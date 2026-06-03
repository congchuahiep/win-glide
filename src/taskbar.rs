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
use std::time::Instant;
use tracing::{debug, instrument};
use windows::core::w;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, CLSCTX_LOCAL_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    AutomationElementMode_None, CUIAutomation, IUIAutomation, IUIAutomationCacheRequest,
    IUIAutomationCondition, IUIAutomationElementArray, TreeScope_Descendants,
    UIA_AutomationIdPropertyId, UIA_BoundingRectanglePropertyId, UIA_ClassNamePropertyId,
    UIA_NamePropertyId, UIA_ProcessIdPropertyId,
};
use windows::Win32::UI::Shell::IVirtualDesktopManager;
use windows::Win32::UI::Shell::VirtualDesktopManager;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, FindWindowW, GetForegroundWindow};

use crate::switcher::{
    find_visible_windows, find_window_for_button, find_windows_for_button, WindowInfo,
};

/// Cache TTL: 1 giây. Nếu không có WinEvent invalidate, cache tự expire sau 2s.
///
/// Đây là safety net, trường hợp hiếm khi WinEvent bị miss.
/// Bình thường, cache bị invalidate ngay khi nhận event từ taskbar.
const CACHE_TTL_SECS: f64 = 1.0;

/// Button cache với timestamp.
struct ButtonCache {
    buttons: Vec<TaskbarButton>,
    created_at: Instant,
}

/// Hướng cycle: trái hoặc phải trên taskbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleDirection {
    Forward,  // Alt+] — sang phải
    Backward, // Alt+[ — sang trái
}

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
#[derive(Debug, Clone)]
pub struct CycleEntry {
    /// Tên hiển thị (window title)
    pub name: String,
    /// HWND của window cần activate
    pub hwnd: HWND,
    /// Có thuộc grouped button không
    pub is_grouped: bool,
}

/// Result của việc enumerate taskbar buttons.
pub struct TaskbarEnumerator {
    /// COM interface IUIAutomation, "máy quét" UI.
    automation: IUIAutomation,

    /// Cache button list để tránh re-enumerate mỗi lần bấm phím.
    ///
    /// `RefCell` cho phép mutate từ `&self` methods (không cần `&mut self`).
    /// Cache bị invalidate bởi UIA event hoặc khi TTL (1 giây) hết hạn.
    button_cache: RefCell<Option<ButtonCache>>,

    /// Virtual Desktop ID của foreground window gần nhất.
    /// Dùng để phát hiện chuyển desktop — nếu ID thay đổi → invalidate cache.
    last_desktop_id: RefCell<Option<windows::core::GUID>>,

    /// HWND của taskbar window.
    taskbar_hwnd: HWND,
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
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;

            let taskbar_hwnd = FindWindowW(w!("Shell_TrayWnd"), None)?;
            if taskbar_hwnd.0.is_null() {
                anyhow::bail!("Shell_TrayWnd not found, có thể đang chạy portable mode hoặc taskbar bị disabled")
            }

            Ok(Self {
                automation,
                taskbar_hwnd,
                button_cache: RefCell::new(None),
                last_desktop_id: RefCell::new(None),
            })
        }
    }

    /// Liệt kê tất cả taskbar buttons trên **primary monitor** (taskbar chính).
    ///
    /// Sử dụng cache nếu còn hợp lệ (< CACHE_TTL và chưa bị invalidate,
    /// và virtual desktop chưa thay đổi).
    /// Nếu cache miss, enumerate mới và lưu vào cache.
    pub fn enumerate_primary_buttons(&self) -> anyhow::Result<Vec<TaskbarButton>> {
        // Kiểm tra virtual desktop switch trước khi dùng cache
        if self.desktop_changed() {
            self.invalidate_cache();
        }

        if let Some(ref cache) = *self.button_cache.borrow() {
            let age = cache.created_at.elapsed().as_secs_f64();
            if age < CACHE_TTL_SECS {
                debug!("Using cached buttons (age: {:.0}ms)", age * 1000.0);
                return Ok(cache.buttons.clone());
            }
        }

        unsafe {
            let buttons = self.enumerate_buttons_for_hwnd()?;
            *self.button_cache.borrow_mut() = Some(ButtonCache {
                buttons: buttons.clone(),
                created_at: Instant::now(),
            });
            Ok(buttons)
        }
    }

    /// Kiểm tra virtual desktop có thay đổi không.
    ///
    /// So sánh [`IVirtualDesktopManager::GetWindowDesktopId`] của foreground window
    /// với `last_desktop_id`. Nếu khác -> trả về `true` (cache cần invalidate).
    ///
    /// Lưu ý: chỉ trả về `true` 1 lần khi desktop thay đổi, sau đó cập nhật
    /// `last_desktop_id` nên lần gọi sau sẽ trả về `false`.
    fn desktop_changed(&self) -> bool {
        let fg = unsafe { GetForegroundWindow() };
        if fg.0.is_null() {
            return false;
        }

        let mgr: IVirtualDesktopManager =
            match unsafe { CoCreateInstance(&VirtualDesktopManager, None, CLSCTX_LOCAL_SERVER) } {
                Ok(m) => m,
                Err(_) => return false,
            };

        let current_id = match unsafe { mgr.GetWindowDesktopId(fg) } {
            Ok(id) => id,
            Err(_) => return false,
        };

        let mut last = self.last_desktop_id.borrow_mut();
        let changed = match *last {
            Some(ref prev) => prev != &current_id,
            None => false,
        };

        *last = Some(current_id);
        changed
    }

    /// Invalidate button cache — gọi khi nhận UIA StructureChanged event hoặc
    /// WinEvent từ taskbar.
    ///
    /// UIA event (ChildAdded, ChildRemoved, ...) từ taskbar báo hiệu
    /// rằng button list có thể đã thay đổi. Cache phải bị invalidate
    /// để lần cycle tiếp theo re-enumerate.
    pub fn invalidate_cache(&self) {
        let mut cache = self.button_cache.borrow_mut();
        if cache.is_some() {
            debug!("Button cache invalidated (event)");
            *cache = None;
        }
    }

    /// Install UIA StructureChanged event handler trên taskbar.
    ///
    /// Gọi 1 lần sau khi tạo TaskbarEnumerator.
    pub fn install_uia_handler(&self, main_thread_id: u32) -> anyhow::Result<()> {
        unsafe {
            crate::uia_events::install_uia_handler(
                &self.automation,
                self.taskbar_hwnd,
                main_thread_id,
            )
        }
    }

    /// Uninstall UIA event handler.
    ///
    /// Gọi khi app exit.
    pub fn uninstall_uia_handler(&self) {
        unsafe {
            crate::uia_events::uninstall_uia_handler(&self.automation, self.taskbar_hwnd);
        }
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
        let buttons = self.enumerate_primary_buttons()?;

        if buttons.is_empty() {
            return Ok(None);
        }

        let all_windows = find_visible_windows();

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
                is_grouped,
            }))
        } else {
            Ok(
                find_window_for_button(&target_button, &all_windows).map(|w| CycleEntry {
                    name: w.title,
                    hwnd: w.hwnd,
                    is_grouped: false,
                }),
            )
        }
    }

    /// Tạo CacheRequest chứa các properties cần thiết cho taskbar buttons.
    ///
    /// Thay vì đọc từng property riêng lẻ (4 COM cross-process calls/button),
    /// CacheRequest batch tất cả vào 1 lần duyệt — UIA lấy properties
    /// cùng lúc với tree traversal.
    unsafe fn create_button_cache_request(&self) -> anyhow::Result<IUIAutomationCacheRequest> {
        let cache = self.automation.CreateCacheRequest()?;

        cache.AddProperty(UIA_NamePropertyId)?;
        cache.AddProperty(UIA_BoundingRectanglePropertyId)?;
        cache.AddProperty(UIA_ProcessIdPropertyId)?;
        cache.AddProperty(UIA_AutomationIdPropertyId)?;

        cache.SetAutomationElementMode(AutomationElementMode_None)?;

        Ok(cache)
    }

    /// Core enumeration logic — tìm tất cả TaskListButtonAutomationPeer.
    ///
    /// Dùng `FindAllBuildCache` thay vì `FindAll` để batch property reads.
    /// Thay vì 4 COM calls/button (CurrentName, CurrentBoundingRectangle, ...),
    /// UIA lấy tất cả properties trong 1 lần tree traversal.
    #[instrument(level = "debug", skip_all)]
    unsafe fn enumerate_buttons_for_hwnd(&self) -> anyhow::Result<Vec<TaskbarButton>> {
        let taskbar_hwnd = self.taskbar_hwnd;
        let t0 = Instant::now();

        let class_condition = self.automation.CreatePropertyCondition(
            UIA_ClassNamePropertyId,
            &VARIANT::from("Taskbar.TaskListButtonAutomationPeer"),
        )?;
        debug!(
            "CreatePropertyCondition: {:.2}ms",
            t0.elapsed().as_secs_f64() * 1000.0
        );

        let t1 = Instant::now();
        let cache_request = self.create_button_cache_request()?;
        debug!(
            "CreateCacheRequest: {:.2}ms",
            t1.elapsed().as_secs_f64() * 1000.0
        );

        let t2 = Instant::now();
        let root_element = self.automation.ElementFromHandle(taskbar_hwnd)?;
        debug!(
            "ElementFromHandle: {:.2}ms",
            t2.elapsed().as_secs_f64() * 1000.0
        );

        let t3 = Instant::now();
        let items = root_element.FindAllBuildCache(
            TreeScope_Descendants,
            &class_condition,
            &cache_request,
        )?;
        debug!(
            "FindAllBuildCache: {:.2}ms",
            t3.elapsed().as_secs_f64() * 1000.0
        );

        let t4 = Instant::now();
        let mut all_buttons = Vec::new();
        self.collect_buttons(&items, &mut all_buttons)?;
        debug!(
            "collect_buttons ({} items): {:.2}ms",
            all_buttons.len(),
            t4.elapsed().as_secs_f64() * 1000.0
        );

        if all_buttons.is_empty() {
            self.enumerate_via_bridge_windows(
                taskbar_hwnd,
                &class_condition,
                &cache_request,
                &mut all_buttons,
            )?;
        }

        all_buttons.sort_by_key(|b| b.rect.left);

        debug!(
            "enumerate_buttons_for_hwnd TOTAL: {:.2}ms",
            t0.elapsed().as_secs_f64() * 1000.0
        );

        Ok(all_buttons)
    }

    /// Trích xuất thông tin từ UIA element array.
    ///
    /// Vì dùng `CreatePropertyCondition(UIA_ClassNamePropertyId)`,
    /// `FindAllBuildCache` chỉ trả về `Taskbar.TaskListButtonAutomationPeer` elements.
    /// Properties đã được cached sẵn — đọc qua `Cached*` methods, không cần COM call riêng.
    #[instrument(level = "debug", skip_all)]
    unsafe fn collect_buttons(
        &self,
        items: &IUIAutomationElementArray,
        buttons: &mut Vec<TaskbarButton>,
    ) -> anyhow::Result<()> {
        let count = items.Length()?;

        debug!("collect_buttons: count={}", count);

        for i in 0..count {
            let item = items.GetElement(i)?;

            let name = item
                .CachedName()
                .ok()
                .map(|b| b.to_string())
                .unwrap_or_default();

            let rect = match item.CachedBoundingRectangle() {
                Ok(r) => r,
                Err(_) => continue,
            };

            let process_id = item.CachedProcessId().unwrap_or(0);

            let automation_id = item.CachedAutomationId().ok().map(|s| s.to_string());

            buttons.push(TaskbarButton {
                name,
                rect,
                process_id,
                automation_id,
            });
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
    /// 3. FindAllBuildCache từ bridge element (với CacheRequest)
    /// 4. collect_buttons đọc cached properties
    /// ```
    ///
    /// # Tại sao cần vòng lặp?
    ///
    /// Có thể có NHIỀU bridge windows (một số ẩn hoặc không chứa buttons).
    /// Code duyệt đến khi tìm thấy buttons HOẶC hết child windows.
    #[instrument(level = "debug", skip_all)]
    unsafe fn enumerate_via_bridge_windows(
        &self,
        root_hwnd: HWND,
        condition: &IUIAutomationCondition,
        cache_request: &IUIAutomationCacheRequest,
        buttons: &mut Vec<TaskbarButton>,
    ) -> anyhow::Result<()> {
        let mut child_hwnd = HWND::default();

        loop {
            child_hwnd = FindWindowExW(
                Some(root_hwnd),
                Some(child_hwnd),
                w!("Windows.UI.Composition.DesktopWindowContentBridge"),
                None,
            )
            .unwrap_or_default();

            if child_hwnd.0.is_null() {
                break;
            }

            if let Ok(bridge_element) = self.automation.ElementFromHandle(child_hwnd) {
                if let Ok(items) = bridge_element.FindAllBuildCache(
                    TreeScope_Descendants,
                    condition,
                    cache_request,
                ) {
                    self.collect_buttons(&items, buttons)?;
                }
            }

            if !buttons.is_empty() {
                break;
            }
        }

        Ok(())
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

        // Fast path: match foreground AUMID với button automation_id
        // Tránh duyệt tất cả windows cho mỗi button — chỉ 1 COM call
        if let Some(fg_aumid) = crate::switcher::get_app_user_model_id(foreground_hwnd) {
            let fg_aumid_lower = fg_aumid.to_lowercase();
            for (i, button) in buttons.iter().enumerate() {
                if let Some(auto_id) = &button.automation_id {
                    if !auto_id.is_empty() {
                        let auto_id_lower = auto_id.to_lowercase();
                        if auto_id_lower == fg_aumid_lower
                            || fg_aumid_lower.starts_with(&auto_id_lower)
                            || auto_id_lower.contains(&fg_aumid_lower)
                        {
                            debug!(
                                "Active button [{}]: '{}' fast-match AUMID '{}' vs fg AUMID '{}'",
                                i, button.name, auto_id, fg_aumid
                            );
                            return Some(i);
                        }
                    }
                }
            }
        }

        // Slow path: reverse matching qua find_windows_for_button
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
