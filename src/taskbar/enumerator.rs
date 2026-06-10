//! Liệt kê các nút (buttons) trên Windows 11 taskbar theo đúng thứ tự từ trái sang phải.
//!
//! # Tại sao không dùng `FindWindow` trực tiếp?
//!
//! Trên Windows 10, taskbar buttons là các `ToolbarWindow32`, một control tiêu chuẩn của Windows.
//! Ta có thể dùng `TB_GETBUTTON` message để lấy thông tin trực tiếp. Nhưng trên **Windows 11**,
//! Microsoft viết lại taskbar bằng **XAML** (UWP/WinRT). Các nút không còn là `HWND` riêng biệt
//! nữa, chúng là **XAML elements** bên trong `Windows.UI.Composition.DesktopWindowContentBridge`.
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

use std::cell::{Cell, RefCell};
use std::time::Instant;
use tracing::{debug, error, instrument, warn};
use windows::core::w;
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::{
    MonitorFromPoint, MonitorFromWindow, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL,
};
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
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetCursorPos, GetForegroundWindow,
};

use super::activate::force_activate;
use super::button_window::ButtonWindowMap;
use super::explorer::invalidate_explorer_pid_cache;
use super::window::find_visible_windows;
use crate::event::UiaEventHook;
use crate::types::{TargetWindow, TaskbarButton};
use crate::utils::truncate;

/// Cache TTL: 1 giây. Nếu không có WinEvent invalidate, cache tự expire sau 2s.
const CACHE_TTL_SECS: f64 = 1.0;

/// Button cache với timestamp.
struct ButtonCache {
    buttons: Vec<TaskbarButton>,
    created_at: Instant,
}

/// Hướng cycle: trái hoặc phải trên taskbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleDirection {
    Forward,
    Backward,
}

/// Liệt kê các taskbar buttons trên Windows 11.
pub struct TaskbarEnumerator {
    /// COM interface IUIAutomation, "máy quét" UI.
    automation: IUIAutomation,

    /// Cache button list để tránh re-enumerate mỗi lần bấm phím.
    ///
    /// [`RefCell`] cho phép mutate từ `&self` methods (không cần `&mut self`).
    /// Cache bị invalidate bởi UIA event hoặc khi TTL (1 giây) hết hạn.
    button_cache: RefCell<Option<ButtonCache>>,

    /// Virtual Desktop ID của foreground window gần nhất.
    ///
    /// Dùng để phát hiện chuyển desktop, nếu ID thay đổi -> invalidate cache.
    last_desktop_id: RefCell<Option<windows::core::GUID>>,

    /// Danh sách HWND của các taskbar window (cả chính và các taskbar của màn hình phụ).
    taskbars: RefCell<Vec<HWND>>,

    /// UIA StructureChanged event hooks.
    uia_hooks: RefCell<Vec<UiaEventHook>>,

    /// UIA StructureChanged event hook - tự subscribe/unsubscribe.
    ///
    /// Khi explorer restart, [`Self::refresh_taskbar_hwnd`] sẽ drop old + create new.
    uia_thread_id: Cell<u32>,

    /// Index của button đang active lần cuối cùng.
    last_active_index: Cell<Option<usize>>,
}

/// Life cycle implementation
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
    /// let buttons = enumerator.enumerate_current_monitor_buttons()?;
    /// ```
    pub fn new() -> anyhow::Result<Self> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;

            let taskbars = Self::get_all_taskbar_hwnds();
            if taskbars.is_empty() {
                anyhow::bail!(
                    "No taskbar found, có thể đang chạy portable mode hoặc taskbar bị disabled"
                )
            }

            Ok(Self {
                automation,
                taskbars: RefCell::new(taskbars),
                button_cache: RefCell::new(None),
                last_desktop_id: RefCell::new(None),
                uia_hooks: RefCell::new(Vec::new()),
                uia_thread_id: Cell::new(0),
                last_active_index: Cell::new(None),
            })
        }
    }

    /// Đăng ký UIA StructureChanged event handler trên taskbar elements.
    ///
    /// Gọi 1 lần từ `App::run()`. Sau explorer restart, [`Self::refresh_taskbar_hwnd`] tự động
    /// re-subscribe.
    pub fn install_uia_hook(&self, main_thread_id: u32) -> anyhow::Result<()> {
        self.uia_thread_id.set(main_thread_id);
        let taskbars = self.taskbars.borrow();
        let mut hooks = self.uia_hooks.borrow_mut();
        hooks.clear();
        for &hwnd in taskbars.iter() {
            if let Ok(hook) =
                unsafe { UiaEventHook::install(&self.automation, hwnd, main_thread_id) }
            {
                hooks.push(hook);
            }
        }
        Ok(())
    }

    /// Tự phục hồi sau khi explorer.exe crash và restart.
    ///
    /// NOTE: Logic này được gọi tại [`Self::enumerate_buttons`], nghe có vẻ không đúng lắm vì đáng
    /// lẽ ra phương thức này phải gọi khi sự kiện explorer restart xảy ra. Lý do không chọn phương
    /// pháp bắt sự kiện đó là vì cách đó hơi rườm rà, nhưng mai sau vẫn nên bắt sự kiện hơn thay vì
    /// tự gọi thủ công
    pub fn refresh_taskbar_hwnd(&self) -> anyhow::Result<()> {
        let taskbars = Self::get_all_taskbar_hwnds();
        if taskbars.is_empty() {
            anyhow::bail!("No taskbars found after explorer restart");
        }

        *self.taskbars.borrow_mut() = taskbars;
        self.invalidate_cache();
        invalidate_explorer_pid_cache();

        let mut hooks = self.uia_hooks.borrow_mut();
        hooks.clear();
        for &hwnd in self.taskbars.borrow().iter() {
            if let Ok(hook) =
                unsafe { UiaEventHook::install(&self.automation, hwnd, self.uia_thread_id.get()) }
            {
                hooks.push(hook);
            }
        }

        debug!("Taskbar HWNDs refreshed, UIA re-subscribed");
        Ok(())
    }
}

/// Core implementation
impl TaskbarEnumerator {
    /// Cycle đến window kế tiếp trên taskbar và activate window.
    ///
    /// # Errors
    ///
    /// Returns an error if no window could be found to cycle to.
    #[instrument(level = "debug", skip_all)]
    pub fn cycle_to_neighbor(
        &self,
        uncombine_enabled: bool,
        direction: CycleDirection,
    ) -> anyhow::Result<()> {
        let foreground = unsafe { GetForegroundWindow() };
        let target = self.find_neighbor_window(foreground, uncombine_enabled, direction)?;

        match target {
            Some(target) => {
                debug!(
                    "Activating '{}' (grouped={})",
                    truncate(&target.name, 30),
                    target.is_grouped,
                );

                let ok = unsafe { force_activate(target.hwnd) };
                if !ok {
                    warn!("force_activate returned false");
                }
            }
            None => warn!("No window found to cycle to"),
        }

        Ok(())
    }

    /// Tìm window kế tiếp bên trái/phải của `source` trên taskbar.
    ///
    /// Luồng: enumerate buttons -> find active index -> compute next index ->
    /// match target button với window (qua `ButtonWindowMap`) -> return target.
    #[instrument(level = "debug", skip_all)]
    fn find_neighbor_window(
        &self,
        source: HWND,
        uncombine_enabled: bool,
        direction: CycleDirection,
    ) -> anyhow::Result<Option<TargetWindow>> {
        let buttons = self.enumerate_current_monitor_buttons()?;

        if buttons.is_empty() {
            return Ok(None);
        }

        let all_windows = find_visible_windows();
        let button_map = ButtonWindowMap::new(&buttons, &all_windows);

        let source_index = match button_map.find_button_index_by_hwnd(source) {
            Some(i) => {
                self.last_active_index.set(Some(i));
                i
            }
            None => self
                .last_active_index
                .get()
                .filter(|&i| i < buttons.len())
                .unwrap_or(0),
        };

        let target_index = match direction {
            CycleDirection::Forward if source_index + 1 >= buttons.len() => 0,
            CycleDirection::Forward => source_index + 1,
            CycleDirection::Backward if source_index == 0 => buttons.len() - 1,
            CycleDirection::Backward => source_index - 1,
        };

        let target_button = &buttons[target_index];

        match uncombine_enabled {
            true => {
                debug!(
                    "Target button [{}]: '{}' AUMID '{:?}'",
                    target_index, target_button.name, target_button.automation_id
                );

                Ok(button_map
                    .find_window_by_button(target_button)
                    .map(|w| TargetWindow {
                        name: w.title,
                        hwnd: w.hwnd,
                        is_grouped: false,
                    }))
            }
            false => {
                let windows = button_map.find_windows_by_button(target_button);
                let is_grouped = windows.len() > 1;

                Ok(windows.into_iter().next().map(|w| TargetWindow {
                    name: w.title,
                    hwnd: w.hwnd,
                    is_grouped,
                }))
            }
        }
    }

    /// Lấy tất cả HWND của các taskbar (chính và phụ).
    fn get_all_taskbar_hwnds() -> Vec<HWND> {
        let mut hwnds = Vec::new();
        unsafe {
            if let Ok(primary) = FindWindowW(w!("Shell_TrayWnd"), None) {
                if !primary.0.is_null() {
                    hwnds.push(primary);
                }
            }

            let mut secondary = HWND::default();
            loop {
                secondary =
                    FindWindowExW(None, Some(secondary), w!("Shell_SecondaryTrayWnd"), None)
                        .unwrap_or_default();

                if secondary.0.is_null() {
                    break;
                }
                hwnds.push(secondary);
            }
        }
        hwnds
    }

    /// Trả về taskbar buttons trên monitor hiện tại (chứa cursor hoặc foreground window).
    ///
    /// Cache với TTL 1 giây, bị invalidate bởi UIA event hoặc desktop switch.
    pub fn enumerate_current_monitor_buttons(&self) -> anyhow::Result<Vec<TaskbarButton>> {
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
            let mut pt = POINT::default();
            let hmonitor = GetCursorPos(&mut pt)
                .map(|_| MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST))
                .unwrap_or_else(|_| {
                    let fg = GetForegroundWindow();
                    if fg.0.is_null() {
                        HMONITOR::default()
                    } else {
                        MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST)
                    }
                });

            let taskbars = self.taskbars.borrow();
            let target_taskbar = taskbars
                .iter()
                .find(|&&hwnd| {
                    let hmonitor_tb = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL);
                    !hmonitor_tb.is_invalid() && hmonitor_tb.0 == hmonitor.0
                })
                .copied()
                .unwrap_or_else(|| *taskbars.first().unwrap_or(&HWND::default()));

            let buttons = self.enumerate_buttons(target_taskbar)?;
            *self.button_cache.borrow_mut() = Some(ButtonCache {
                buttons: buttons.clone(),
                created_at: Instant::now(),
            });
            Ok(buttons)
        }
    }

    /// Kiểm tra virtual desktop có thay đổi không qua `IVirtualDesktopManager::GetWindowDesktopId`.
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

    /// Phát hiện lỗi `EVENT_E_ALL_SUBSCRIBERS_FAILED (0x80040201)` khi explorer restart.
    fn is_subscribers_failed(e: &anyhow::Error) -> bool {
        e.chain().any(|c| c.to_string().contains("0x80040201"))
    }

    /// Liệt kê taskbar buttons của một taskbar HWND cụ thể qua UIA.
    ///
    /// Fallback: nếu `FindAllBuildCache` trả về rỗng, thử `enumerate_via_bridge_windows`.
    /// Tự phục hồi nếu gặp `EVENT_E_ALL_SUBSCRIBERS_FAILED` (explorer restart).
    #[instrument(level = "debug", skip_all)]
    unsafe fn enumerate_buttons(&self, target_taskbar: HWND) -> anyhow::Result<Vec<TaskbarButton>> {
        /// Tạo CacheRequest batch 4 UIA properties (name, rect, PID, automation_id)
        /// để chỉ cần 1 COM call thay vì 4 lần gọi riêng.
        unsafe fn create_button_cache_request(
            automation: &IUIAutomation,
        ) -> anyhow::Result<IUIAutomationCacheRequest> {
            let cache = automation.CreateCacheRequest()?;

            cache.AddProperty(UIA_NamePropertyId)?;
            cache.AddProperty(UIA_BoundingRectanglePropertyId)?;
            cache.AddProperty(UIA_ProcessIdPropertyId)?;
            cache.AddProperty(UIA_AutomationIdPropertyId)?;

            cache.SetAutomationElementMode(AutomationElementMode_None)?;

            Ok(cache)
        }

        let try_enumerate_buttons = |taskbar_hwnd| -> anyhow::Result<Vec<TaskbarButton>> {
            let class_condition = self.automation.CreatePropertyCondition(
                UIA_ClassNamePropertyId,
                &VARIANT::from("Taskbar.TaskListButtonAutomationPeer"),
            )?;

            let cache_request = create_button_cache_request(&self.automation)?;

            let root_element = self.automation.ElementFromHandle(taskbar_hwnd)?;
            let items = root_element.FindAllBuildCache(
                TreeScope_Descendants,
                &class_condition,
                &cache_request,
            )?;

            let mut all_buttons = Vec::new();
            self.collect_buttons(&items, &mut all_buttons)?;

            if all_buttons.is_empty() {
                self.enumerate_via_bridge_windows(
                    taskbar_hwnd,
                    &class_condition,
                    &cache_request,
                    &mut all_buttons,
                )?;
            }

            all_buttons.sort_by_key(|b| b.rect.left);
            Ok(all_buttons)
        };

        match try_enumerate_buttons(target_taskbar) {
            Ok(buttons) => Ok(buttons),
            Err(ref e) if Self::is_subscribers_failed(e) => {
                warn!("UIA subscribers failed (explorer restart?), recovering...");
                self.refresh_taskbar_hwnd().map_err(|e2| {
                    error!("Failed to recover taskbar: {e2}");
                    anyhow::anyhow!("{e}")
                })?;

                let new_target = self
                    .taskbars
                    .borrow()
                    .first()
                    .copied()
                    .unwrap_or(target_taskbar);
                try_enumerate_buttons(new_target)
            }
            Err(e) => Err(e),
        }
    }

    /// Trích xuất thông tin từ UIA element array qua cached properties.
    ///
    /// Dùng `Cached*` methods (không cần COM call riêng) vì properties đã được
    /// batch-read qua `CacheRequest` trong `FindAllBuildCache`.
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

    /// Fallback: tìm buttons qua `DesktopWindowContentBridge` child windows của taskbar.
    ///
    /// Win11 có thể render buttons bên trong bridge window (XAML island)
    /// thay vì trực tiếp dưới Shell_TrayWnd.
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
}

/// Cached implementation
impl TaskbarEnumerator {
    /// Invalidate button cache - gọi khi nhận UIA StructureChanged event.
    pub fn invalidate_cache(&self) {
        let mut cache = self.button_cache.borrow_mut();
        if cache.is_some() {
            debug!("Button cache invalidated (event)");
            *cache = None;
        }
    }
}
