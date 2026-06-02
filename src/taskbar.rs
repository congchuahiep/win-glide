//! Liệt kê các nút (buttons) trên Windows 11 taskbar theo đúng thứ tự từ trái sang phải.
//!
//! # Tại sao không dùng `FindWindow` trực tiếp?
//!
//! Trên Windows 10, taskbar buttons là các `ToolbarWindow32` — một control tiêu chuẩn của Windows.
//! Ta có thể dùng `TB_GETBUTTON` message để lấy thông tin trực tiếp. Nhưng trên **Windows 11**,
//! Microsoft viết lại taskbar bằng **XAML** (UWP/WinRT). Các nút không còn là `HWND` riêng biệt
//! nữa — chúng là **XAML elements** bên trong `Windows.UI.Composition.DesktopWindowContentBridge`.
//!
//! Do đó ta phải dùng **UI Automation (UIAutomation)**, một COM-based API cho phép truy cập UI
//! elements bất kể underlying technology (Win32, XAML, WebView, etc.).
//!
//! # Khái niệm quan trọng: IUIAutomation
//!
//! **IUIAutomation** giống như một "máy quét màn hình" cho người khiếm thị. Nó mô tả mọi thứ trên màn hình thành một **cây phân cấp** (tree):
//!
//! ```text
//! Root (Desktop)
//!  └── Shell_TrayWnd (Taskbar)
//!       └── Windows.UI.Composition.DesktopWindowContentBridge
//!            └── Taskbar.TaskListButtonAutomationPeer  ← đây là các nút!
//!            └── Taskbar.TaskListButtonAutomationPeer
//!            └── ...
//! ```
//!
//! Mỗi **element** có các **properties**:
//! - `CurrentClassName`: loại element (VD: "Taskbar.TaskListButtonAutomationPeer")
//! - `CurrentName`: tên hiển thị (VD: "Chrome - 3 running windows")
//! - `CurrentBoundingRectangle`: vị trí + kích thước trên màn hình
//! - `CurrentProcessId`: PID của process sở hữu (thường là explorer.exe trên Win11)
//! - `CurrentAutomationId`: ID duy nhất của element
//!
//! # Luồng hoạt động
//!
//! ```rust
//! // 1. Tạo IUIAutomation instance
//! let automation = CoCreateInstance(&CUIAutomation)?;
//!
//! // 2. Tìm Shell_TrayWnd (taskbar window)
//! let taskbar = FindWindowW("Shell_TrayWnd", None)?;
//!
//! // 3. Lấy element gốc của taskbar
//! let root = automation.ElementFromHandle(taskbar)?;
//!
//! // 4. Tìm tất cả descendants là TaskListButtonAutomationPeer
//! let items = root.FindAll(TreeScope_Descendants, true_condition)?;
//!
//! // 5. Lọc, lấy thông tin, sort theo vị trí trái -> phải
//! buttons.sort_by_key(|b| b.rect.left);
//! ```

use std::cell::RefCell;

use tracing::{debug, instrument};
use windows::core::w;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationCondition, IUIAutomationElementArray,
    TreeScope_Descendants,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, FindWindowW};

use crate::switcher::{
    find_visible_windows, find_window_for_button, find_windows_for_button, WindowInfo,
};

/// Hướng cycle: trái hoặc phải trên taskbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleDirection {
    Forward,  // Alt+] — sang phải
    Backward, // Alt+[ — sang trái
}

const TASKBAR_BUTTON_CLASS: &str = "Taskbar.TaskListButtonAutomationPeer";

/// Taskbar button trên Windows 11. Chứa thông tin để xác định vị trí và thứ tự của nút trên
/// taskbar.
///
/// **Không chứa HWND** vì Win11 XAML taskbar buttons không có HWND riêng
#[derive(Debug, Clone)]
pub struct TaskbarButton {
    /// Tên hiển thị của nút.
    ///
    /// Format trên Win11:
    /// - App đơn: `"Chrome"`
    /// - App có nhiều window: `"Chrome - 3 running windows"`
    /// - App đã pin: `"Notepad - Pinned"`
    ///
    /// Dùng [`clean_button_name()`] để strip suffix.
    pub name: String,

    /// Vị trí và kích thước trên màn hình (pixel).
    ///
    /// Dùng `rect.left` để sắp xếp các nút theo thứ tự trái -> phải.
    pub rect: windows::Win32::Foundation::RECT,

    /// Process ID của ứng dụng sở hữu nút này.
    ///
    /// ⚠️ **Quan trọng**: Trên Win11, giá trị này THƯỜNG trả về PID của `explorer.exe`,
    /// không phải PID của ứng dụng thực. Lý do: XAML taskbar chạy trong explorer process.
    ///
    /// Do đó, ta KHÔNG thể dùng PID này trực tiếp để `SetForegroundWindow`.
    /// Phải dùng [`super::switcher::find_window_for_button()`] để tìm HWND thực.
    pub process_id: i32,

    /// Automation ID của button từ UI Automation.
    ///
    /// Trên Win11, đây có thể chứa AppUserModelID, giúp matching windows chính xác hơn.
    pub automation_id: Option<String>,
}

/// Một window target trong danh sách cycle.
/// Mỗi entry tương ứng với 1 window cụ thể (HWND),
/// không phải 1 taskbar button.
/// Dùng cho flat list cycling — grouped buttons được expand.
#[derive(Debug, Clone)]
pub struct CycleEntry {
    /// Tên hiển thị (window title)
    pub name: String,
    /// HWND của window cần activate
    pub hwnd: HWND,
    /// Vị trí trái của taskbar button gốc (để sort theo thứ tự trái→phải)
    pub taskbar_left: i32,
    /// Có thuộc grouped button không
    pub is_grouped: bool,
    /// Vị trí window trên màn hình (dùng để sort windows trong group)
    pub window_rect: RECT,
}

/// Result của việc enumerate taskbar buttons.
pub struct TaskbarEnumerator {
    /// COM interface IUIAutomation, "máy quét" UI.
    ///
    /// Không implements `Send`/`Sync` vì COM objects không an toàn khi share cross-thread.
    automation: IUIAutomation,

    /// Flag: đã tự init COM chưa.
    ///
    /// Nếu `true`, ta phải `CoUninitialize()` khi drop.
    /// Nếu `false`, có thể COM đã được init sẵn bởi thread khác.
    com_initialized: bool,

    /// Cache danh sách taskbar buttons. `None` = chưa cache hoặc đã bị invalidate.
    /// Tự động populate khi lần đầu gọi `cached_buttons()`, tồn tại cho đến khi `invalidate_cache()`.
    button_cache: RefCell<Option<Vec<TaskbarButton>>>,

    /// Cache danh sách visible windows (EnumWindows). Invalidated cùng lúc với button_cache.
    window_cache: RefCell<Option<Vec<WindowInfo>>>,
}

impl TaskbarEnumerator {
    /// Tạo enumerator mới và init COM (STA apartment).
    ///
    /// # COM Apartments
    ///
    /// Windows COM có 2 loại apartment:
    /// - **STA (Single-Threaded Apartment)**: Mỗi thread sở hữu message queue riêng, dùng
    /// `GetMessageW`.
    /// - **MTA (Multi-Threaded Apartment)**: Không có message queue, dùng
    /// `CoWaitForMultipleObjects`.
    ///
    /// IUIAutomation hoạt động tốt với cả 2, nhưng STA được khuyến nghị cho đơn giản.
    ///
    /// # Ví dụ
    ///
    /// ```rust,ignore
    /// let enumerator = TaskbarEnumerator::new()?;
    /// let buttons = enumerator.enumerate_primary_buttons()?;
    /// ```
    pub fn new() -> anyhow::Result<Self> {
        unsafe {
            let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let com_initialized = hr.is_ok();

            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;

            Ok(Self {
                automation,
                com_initialized,
                button_cache: RefCell::new(None),
                window_cache: RefCell::new(None),
            })
        }
    }

    /// Liệt kê tất cả taskbar buttons trên **primary monitor** (taskbar chính).
    ///
    /// # Luồng
    ///
    /// ```text
    /// 1. Tìm Shell_TrayWnd (FindWindowW)
    /// 2. Quét descendants từ root element (ElementFromHandle + FindAll)
    /// 3. Nếu không thấy -> thử qua DesktopWindowContentBridge (Win11 fallback)
    /// 4. Sort theo rect.left (trái -> phải)
    /// ```
    ///
    /// # Tại sao phải thử 2 lần?
    ///
    /// Win11 có 2 cấu trúc taskbar:
    /// 1. **DirectUI** (cũ): XAML buttons nằm trực tiếp trong Shell_TrayWnd tree
    /// 2. **ContentBridge** (mới): XAML buttons nằm trong
    /// `Windows.UI.Composition.DesktopWindowContentBridge`
    ///
    /// Code thử cả 2 path để đảm bảo tìm thấy buttons.
    pub fn enumerate_primary_buttons(&self) -> anyhow::Result<Vec<TaskbarButton>> {
        let taskbar_hwnd = self.find_primary_taskbar_hwnd()?;

        unsafe { self.enumerate_buttons_for_hwnd(taskbar_hwnd) }
    }

    /// Lấy danh sách buttons từ cache. Nếu cache trống, scan UIA và populate.
    ///
    /// Cache tồn tại vĩnh viễn cho đến khi `invalidate_cache()` được gọi
    /// (qua WinEvent CREATE/DESTROY/NAMECHANGE).
    fn cached_buttons(&self) -> anyhow::Result<Vec<TaskbarButton>> {
        {
            let cache = self.button_cache.borrow();
            if let Some(ref buttons) = *cache {
                return Ok(buttons.clone());
            }
        }

        let buttons = self.enumerate_primary_buttons()?;
        *self.button_cache.borrow_mut() = Some(buttons.clone());
        Ok(buttons)
    }

    /// Lấy danh sách visible windows từ cache. Nếu cache trống, gọi EnumWindows và populate.
    ///
    /// Invalidated cùng lúc với button_cache qua `invalidate_cache()`.
    fn cached_windows(&self) -> anyhow::Result<Vec<WindowInfo>> {
        {
            let cache = self.window_cache.borrow();
            if let Some(ref windows) = *cache {
                return Ok(windows.clone());
            }
        }

        let windows = find_visible_windows();
        *self.window_cache.borrow_mut() = Some(windows.clone());
        Ok(windows)
    }

    /// Vô hiệu hoá toàn bộ cache (buttons + windows).
    ///
    /// Gọi khi có sự kiện thay đổi taskbar (window tạo/đóng/đổi title).
    /// Lần cycle tiếp theo sẽ scan UIA và EnumWindows fresh.
    pub fn invalidate_cache(&self) {
        self.button_cache.borrow_mut().take();
        self.window_cache.borrow_mut().take();
        debug!("Cache invalidated");
    }

    /// Tìm button kế tiếp (trái/phải) của foreground window và trả về window cần activate.
    ///
    /// # Khác với `build_cycle_entries`:
    ///
    /// - `build_cycle_entries`: xây **toàn bộ** danh sách (N buttons × M windows) → ~30ms
    /// - `cycle_to_neighbor`: chỉ tìm button hiện tại + button kế bên + **1 window** → ~15ms
    ///   (hoặc <1ms nếu buttons cache hợp lệ)
    ///
    /// # Tham số
    ///
    /// * `foreground`: HWND của cửa sổ đang focus
    /// * `combine_enabled`: `true` = combine mode (button có thể nhóm); `false` = uncombined
    /// * `direction`: `Forward` (phải) hoặc `Backward` (trái)
    ///
    /// # Returns
    ///
    /// `None` nếu không tìm thấy window phù hợp. `Some(CycleEntry)` nếu tìm thấy.
    #[instrument(level = "debug", skip_all)]
    pub fn cycle_to_neighbor(
        &self,
        foreground: HWND,
        combine_enabled: bool,
        direction: CycleDirection,
    ) -> anyhow::Result<Option<CycleEntry>> {
        let buttons = self.cached_buttons()?;

        if buttons.is_empty() {
            return Ok(None);
        }

        let all_windows = self.cached_windows()?;

        let active_index =
            TaskbarEnumerator::find_active_button_index(&buttons, foreground, &all_windows)
                .unwrap_or(0);

        debug!("Current index {active_index}");

        let target_index = match direction {
            CycleDirection::Forward if active_index + 1 >= buttons.len() => 0,
            CycleDirection::Forward => active_index + 1,
            CycleDirection::Backward if active_index == 0 => buttons.len() - 1,
            CycleDirection::Backward => active_index - 1,
        };

        let target_button = &buttons[target_index];

        debug!(
            "Cycling {:?} from [{}] -> [{}] (button='{}')",
            direction, active_index, target_index, target_button.name,
        );

        if combine_enabled {
            let windows = find_windows_for_button(&target_button, &all_windows);

            let is_grouped = windows.len() > 1;

            Ok(windows.into_iter().next().map(|w| CycleEntry {
                name: w.title,
                hwnd: w.hwnd,
                taskbar_left: target_button.rect.left,
                is_grouped,
                window_rect: w.rect,
            }))
        } else {
            Ok(
                find_window_for_button(&target_button, &all_windows).map(|w| CycleEntry {
                    name: w.title,
                    hwnd: w.hwnd,
                    taskbar_left: target_button.rect.left,
                    is_grouped: false,
                    window_rect: w.rect,
                }),
            )
        }
    }

    /// Build danh sách cycle entries, mỗi entry là 1 window cụ thể.
    ///
    /// Grouped buttons (Combine mode ON) được expand thành nhiều entries, mỗi entry tương ứng với
    /// 1 window riêng lẻ. _(WARN: Cơ chế cho group button hiện tại không chính xác, xem phần bên
    /// dưới để biết lý do)_
    ///
    /// # Luồng
    ///
    /// 1. enumerate_primary_buttons() → lấy các taskbar buttons
    /// 2. Với mỗi button → find_all_windows_for_button() → tìm windows
    /// 3. Nếu button có 1 window → 1 CycleEntry
    /// 4. Nếu button có N>1 windows (grouped) → N CycleEntries, sort theo window_rect.left _(WARN:
    /// Cơ chế hiện tại không chính xác, xem phần bên dưới để biết lý do)_
    /// 5. Sort tất cả entries theo taskbar_left (trái → phải)
    ///
    /// # Ví dụ output
    ///
    /// Với taskbar: [Settings] [Chrome(group: 3 windows)] [VScode] [Explorer]
    ///
    /// Output flat list:
    /// ```text
    /// [Settings#1, Chrome#1, Chrome#2, Chrome#3, VScode#1, Explorer#1]
    /// ```
    ///
    /// # Thứ tự trong group
    ///
    /// Cơ chế hiện tại không chính xác: các cửa sổ trong một nhóm taskbar button đang được
    /// sắp xếp theo ID của window (HWND). Vì vậy, thứ tự cửa sổ không khớp với thứ tự hiển thị trên
    /// taskbar, mà chỉ theo ID nội bộ. Chi tiết hơn tại [`find_all_windows_for_button`].
    #[instrument(level = "debug", skip_all)]
    pub fn build_cycle_entries(&self, combine_enabled: bool) -> anyhow::Result<Vec<CycleEntry>> {
        let buttons = self.cached_buttons()?;

        // Cache: gọi EnumWindows 1 lần, dùng cho tất cả button
        let all_windows = find_visible_windows();

        let mut entries = Vec::new();

        for button in &buttons {
            match combine_enabled {
                // Nếu combine_enabled là true, tìm các cửa sổ trong group button/không phải group
                // button và thêm vào entries
                true => {
                    let windows = find_windows_for_button(button, &all_windows);

                    let is_grouped = windows.len() > 1;

                    for w in windows {
                        entries.push(CycleEntry {
                            name: w.title.clone(),
                            hwnd: w.hwnd,
                            taskbar_left: button.rect.left,
                            is_grouped,
                            window_rect: w.rect,
                        });
                    }
                }
                // Nếu combine_enabled là false, chỉ tìm cửa sổ duy nhất và thêm vào entries
                false => {
                    let window = find_window_for_button(button, &all_windows);

                    match window {
                        Some(w) => {
                            entries.push(CycleEntry {
                                name: w.title.clone(),
                                hwnd: w.hwnd,
                                taskbar_left: button.rect.left,
                                is_grouped: false,
                                window_rect: w.rect,
                            });
                        }
                        None => {}
                    }
                }
            }
        }

        entries.sort_by(|a, b| {
            a.taskbar_left
                .cmp(&b.taskbar_left)
                .then_with(|| a.window_rect.left.cmp(&b.window_rect.left))
                .then_with(|| a.hwnd.0.cmp(&b.hwnd.0))
        });

        for (i, e) in entries.iter().enumerate() {
            debug!(
                "Entry[{}]: name='{}', grouped={}, left={}",
                i, e.name, e.is_grouped, e.taskbar_left
            );
        }

        Ok(entries)
    }

    /// Core enumeration logic — tìm tất cả TaskListButtonAutomationPeer.
    ///
    /// # Chi tiết từng bước
    ///
    /// ```rust,ignore
    /// // Tạo condition "lấy tất cả" (không lọc gì)
    /// let true_condition = automation.CreateTrueCondition()?;
    ///
    /// // Lấy element gốc của taskbar (Shell_TrayWnd)
    /// let root = automation.ElementFromHandle(taskbar_hwnd)?;
    ///
    /// // FindAll với TreeScope_Descendants = tìm TẤT CẢ con cháu
    /// let items = root.FindAll(TreeScope_Descendants, &true_condition)?;
    ///
    /// // Duyệt từng element, lọc class_name == TASKBAR_BUTTON_CLASS
    /// for i in 0..count {
    ///     let item = items.GetElement(i)?;
    ///     if item.CurrentClassName() == "Taskbar.TaskListButtonAutomationPeer" {
    ///         buttons.push(extract_info(item));
    ///     }
    /// }
    ///
    /// // Sort theo vị trí trái → phải (thứ tự taskbar)
    /// buttons.sort_by_key(|b| b.rect.left);
    /// ```
    unsafe fn enumerate_buttons_for_hwnd(
        &self,
        root_hwnd: HWND,
    ) -> anyhow::Result<Vec<TaskbarButton>> {
        let true_condition = self.automation.CreateTrueCondition()?;
        let mut all_buttons = Vec::new();
        let root_element = self.automation.ElementFromHandle(root_hwnd)?;
        let items = root_element.FindAll(TreeScope_Descendants, &true_condition)?;

        self.collect_buttons(&items, &mut all_buttons)?;

        if all_buttons.is_empty() {
            self.enumerate_via_bridge_windows(root_hwnd, &true_condition, &mut all_buttons)?;
        }

        // Sort theo vị trí trái -> phải
        //
        // rect.left = tọa độ x của cạnh trái button
        // Taskbar có thể ở trên/dưới/trái/phải màn hình,
        // nhưng với taskbar ngang, left tăng dần từ trái → phải.
        all_buttons.sort_by_key(|b| b.rect.left);

        Ok(all_buttons)
    }

    /// Lọc và trích xuất thông tin từ UIA element array.
    ///
    /// # Tại sao phải lọc?
    ///
    /// `FindAll(TreeScope_Descendants)` trả về **MỌI** element con của taskbar:
    /// - TaskListButtonAutomationPeer (các nút app)
    /// - StartButton
    /// - SearchButton
    /// - ClockButton
    /// - NotificationIcon
    /// - v.v.
    ///
    /// Ta chỉ quan tâm `Taskbar.TaskListButtonAutomationPeer`.
    ///
    /// # Mỗi button cung cấp gì?
    ///
    /// | Property | Ý nghĩa | Ví dụ |
    /// |----------|---------|---------|
    /// | `CurrentClassName` | Loại element | `"Taskbar.TaskListButtonAutomationPeer"` |
    /// | `CurrentName` | Tên hiển thị | `"Chrome - 3 running windows"` |
    /// | `CurrentBoundingRectangle` | Vị trí | `RECT { left: 100, top: 1060, ... }` |
    /// | `CurrentProcessId` | PID | `1234` (thường là explorer.exe) |
    unsafe fn collect_buttons(
        &self,
        items: &IUIAutomationElementArray,
        buttons: &mut Vec<TaskbarButton>,
    ) -> anyhow::Result<()> {
        let count = items.Length()?;

        for i in 0..count {
            let item = items.GetElement(i)?;

            // CurrentClassName: lọc chỉ lấy button
            //
            // Taskbar.TaskListButtonAutomationPeer = nút app trên taskbar Win11
            // StartButton = nút Start
            // SearchButton = nút Search
            // ClockButton = đồng hồ
            let class_name = item
                .CurrentClassName()
                .ok()
                .map(|b| b.to_string())
                .unwrap_or_default();

            if class_name == TASKBAR_BUTTON_CLASS {
                let name = item
                    .CurrentName()
                    .ok()
                    .map(|b| b.to_string())
                    .unwrap_or_default();

                // Lấy vị trí (để sắp xếp thứ tự)
                let rect = match item.CurrentBoundingRectangle() {
                    Ok(r) => r,
                    Err(_) => continue, // Button không có rect hợp lệ -> bỏ qua
                };

                // Lấy PID (để match với window thực, trường hợp này không mấy khả thi)
                let process_id = item.CurrentProcessId().unwrap_or(0);

                // Lấy AutomationID (có thể chứa AppUserModelID hoặc dùng để match)
                let automation_id = item.CurrentAutomationId().ok().map(|s| s.to_string());

                buttons.push(TaskbarButton {
                    name,
                    rect,
                    process_id,
                    automation_id,
                });
            }
        }

        Ok(())
    }

    /// Win11 fallback: Tìm buttons qua DesktopWindowContentBridge.
    ///
    /// Windows 11 có thể render taskbar buttons bên trong một
    /// `DesktopWindowContentBridge` window con của Shell_TrayWnd.
    ///
    /// # Luồng
    ///
    /// ```text
    /// 1. FindWindowEx tìm child window có class "Windows.UI.Composition.DesktopWindowContentBridge"
    /// 2. Gọi ElementFromHandle trên bridge window đó
    /// 3. FindAll từ bridge element
    /// 4. collect_buttons lọc TaskListButtonAutomationPeer
    /// ```
    ///
    /// # Tại sao cần vòng lặp?
    ///
    /// Có thể có NHIỀU bridge windows (một số ẩn hoặc không chứa buttons).
    /// Code duyệt đến khi tìm thấy buttons HOẶC hết child windows.
    unsafe fn enumerate_via_bridge_windows(
        &self,
        root_hwnd: HWND,
        condition: &IUIAutomationCondition,
        buttons: &mut Vec<TaskbarButton>,
    ) -> anyhow::Result<()> {
        // HWND::default() = null pointer
        let mut child_hwnd = HWND::default();

        // FindWindowEx: tìm child window của root_hwnd
        //
        // FindWindowExW(parent, child_after, class_name, window_name)
        // - parent = Shell_TrayWnd
        // - child_after = null (tìm từ đầu)
        // - class_name = "Windows.UI.Composition.DesktopWindowContentBridge"
        // - window_name = null (bất kỳ)
        //
        // Vòng lặp để tìm TẤT CẢ child windows cùng class
        loop {
            child_hwnd = FindWindowExW(
                Some(root_hwnd),
                Some(child_hwnd),
                w!("Windows.UI.Composition.DesktopWindowContentBridge"),
                None,
            )
            .unwrap_or_default();

            // .0.is_null() = kiểm tra HWND có null không
            if child_hwnd.0.is_null() {
                break; // Hết windows
            }

            // Lấy UIA element từ bridge HWND
            if let Ok(bridge_element) = self.automation.ElementFromHandle(child_hwnd) {
                // Tìm descendants (các nút bên trong bridge)
                if let Ok(items) = bridge_element.FindAll(TreeScope_Descendants, condition) {
                    self.collect_buttons(&items, buttons)?;
                }
            }

            // Nếu đã tìm thấy buttons → dừng
            // Tránh duyệt thêm các bridge không cần thiết
            if !buttons.is_empty() {
                break;
            }
        }

        Ok(())
    }

    /// Tìm HWND của primary taskbar (`Shell_TrayWnd`).
    ///
    /// # Shell_TrayWnd là gì?
    ///
    /// `Shell_TrayWnd` (Shell Tray Window) là **top-level window** của taskbar.
    /// Đây là window class tiêu chuẩn của Windows từ Windows 95 đến Win11.
    ///
    /// Các class windows liên quan:
    /// - `Shell_TrayWnd` — taskbar chính
    /// - `Shell_SecondaryTrayWnd` — taskbar trên monitor phụ
    /// - `ReBarWindow32` — container chứa taskbar items (Win10)
    /// - `MSTaskSwWClass` — taskbar switcher (Win10)
    /// - `MSTaskListWClass` — danh sách nhiệm vụ (Win10)
    ///
    /// # Win11 thay đổi gì?
    ///
    /// Win11 ẩn `ReBarWindow32` và dùng XAML-based taskbar.
    /// Nhưng `Shell_TrayWnd` vẫn tồn tại (legacy compatibility).
    fn find_primary_taskbar_hwnd(&self) -> anyhow::Result<HWND> {
        unsafe {
            let hwnd = FindWindowW(w!("Shell_TrayWnd"), None).unwrap_or_default();

            if hwnd.0.is_null() {
                anyhow::bail!("Shell_TrayWnd not found — có thể đang chạy portable mode hoặc taskbar bị disabled");
            }

            Ok(hwnd)
        }
    }

    /// Tìm index của taskbar button tương ứng với foreground window.
    ///
    /// Sử dụng "reverse matching": với mỗi button, tìm các windows thuộc button đó
    /// (qua AUMID, title, process name — logic đã kiểm chứng trong `match_windows_for_button_cached`),
    /// rồi kiểm tra xem foreground HWND có nằm trong danh sách windows đó không.
    ///
    /// Phương pháp này đáng tin cậy hơn so với matching trực tiếp bằng UIA properties
    /// vì button PID trên Win11 = explorer.exe (không phải app PID),
    /// và window title không cùng format với button name.
    fn find_active_button_index(
        buttons: &[TaskbarButton],
        foreground_hwnd: HWND,
        all_windows: &[WindowInfo],
    ) -> Option<usize> {
        let fg_info = all_windows.iter().find(|w| w.hwnd == foreground_hwnd);
        let fg_name = fg_info.map(|w| w.title.as_str()).unwrap_or("<unknown>");

        for (i, button) in buttons.iter().enumerate() {
            let windows = find_windows_for_button(button, all_windows);

            if windows.iter().any(|w| w.hwnd == foreground_hwnd) {
                debug!(
                    "Active button [{}]: '{}' matches foreground '{}'",
                    i, button.name, fg_name
                );
                return Some(i);
            }
        }

        debug!(
            "No button match found for foreground '{}' (HWND {:?})",
            fg_name, foreground_hwnd
        );
        None
    }
}

/// Strip suffix " - N running window(s)" từ taskbar button name.
///
/// Win11 taskbar button name format:
///
/// | Loại | Format | After clean |
/// |------|--------|------------|
/// | App đơn | `"Notepad"` | `"Notepad"` |
/// | Nhiều windows | `"Chrome - 3 running windows"` | `"Chrome"` |
/// | Pinned | `"Notepad - Pinned"` | `"Notepad - Pinned"` |
/// | VS Code split | `"VS Code - main.rs - 1 running window"` | `"VS Code - main.rs"` |
///
/// # Algorithm
///
/// 1. Tìm `" running window"` từ cuối chuỗi
/// 2. Lấy phần trước đó
/// 3. Tìm `" - "` hoặc `" — "` (em dash) làm delimiter cuối
/// 4. Trả về phần trước delimiter
///
/// # Ví dụ
///
/// ```rust
/// assert_eq!(clean_button_name("Chrome - 3 running windows"), "Chrome");
/// assert_eq!(clean_button_name("VS Code - main.rs - 1 running window"), "VS Code - main.rs");
/// assert_eq!(clean_button_name("Notepad"), "Notepad"); // không đổi
/// ```
pub fn clean_button_name(name: &str) -> String {
    // rfind: tìm từ cuối về đầu
    if let Some(pos) = name.rfind(" running window") {
        // Lấy phần trước " running window"
        let before = &name[..pos];

        // Thử dash thường: " - "
        if let Some(dash_pos) = before.rfind(" - ") {
            return before[..dash_pos].to_string();
        }

        // Thử em dash: " — " (Unicode U+2014)
        if let Some(dash_pos) = before.rfind(" \u{2014} ") {
            return before[..dash_pos].to_string();
        }

        // Không có dash → trả về toàn bộ phần trước
        return before.to_string();
    }

    // Không có suffix → trả về nguyên name
    name.to_string()
}

// /// Destructor — giải phóng COM khi TaskbarEnumerator bị drop.
// ///
// /// Nếu `CoInitializeEx` được gọi thành công trong `new()`,
// /// ta phải gọi `CoUninitialize()` để "rút phích cắm COM".
// ///
// /// ⚠️ **Quan trọng**: Chỉ uninitialize nếu chính ta đã init.
// /// Nếu COM đã được init sẵn bởi thread khác, việc uninitialize
// /// có thể gây crash hoặc lỗi cho ứng dụng khác.
// impl Drop for TaskbarEnumerator {
//     fn drop(&mut self) {
//         if self.com_initialized {
//             unsafe {
//                 CoUninitialize();
//             }
//         }
//     }
// }
