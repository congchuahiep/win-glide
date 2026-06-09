//! Module quản lý trạng thái ứng dụng và điều phối tất cả các thành phần.
//!
//! Struct [`App`], đóng vai trò là trung tâm điều khiển (orchestrator) của ứng dụng. Nó tích hợp và
//!  quản lý vòng đời của:
//! - Quản lý phím nóng toàn cục ([`HotkeyManager`]) để lắng nghe tổ hợp phím Alt+[/].
//! - Duyệt tìm các nút trên Taskbar ([`TaskbarEnumerator`]) thông qua UI Automation (UIA).
//! - Điều khiển tính năng nhóm/tách nút ([`UncombineManager`]) cho các cửa sổ.
//! - Tạo khay hệ thống ([`TrayIcon`]) và cửa sổ ẩn để giao tiếp với Windows Message Loop.

#[cfg(doc)]
use aquamarine::aquamarine;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{debug, debug_span, error, info};
use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::AppConfig;
use crate::event::{self, InvalidateSource};
use crate::hotkey::{HotkeyAction, HotkeyManager};
use crate::indicator::IndicatorWindow;
use crate::logging::console::{self, CONSOLE_VISIBLE};
use crate::setting;
use crate::taskbar::{CycleDirection, TaskbarEnumerator, UncombineManager};
use crate::tray_icon::{TrayIcon, IDM_EXIT, IDM_SETTINGS, IDM_SHOW_CONSOLE, IDM_UNCOMBINE_MODE};

/// Định danh thông điệp Windows động "TaskbarCreated".
/// Thông điệp này được gửi khi tiến trình Explorer khởi động lại.
static mut WM_TASKBARCREATED: u32 = 0;

/// Hằng số định danh thông điệp khay hệ thống gửi đến cửa sổ ẩn.
const WM_USER_TRAYICON: u32 = WM_USER + 0x200;

/// Đại diện cho toàn bộ trạng thái của ứng dụng Taskbar Switcher.
///
/// Struct này duy trì các kết nối phần cứng và phần mềm, bao gồm các hook sự kiện,
/// phím nóng, khay hệ thống và thông tin cửa sổ ẩn Win32 để lắng nghe thông điệp hệ thống.
///
/// ### Luồng Xử Lý Windows Message Loop (Windows API)
///
/// ```mermaid
/// sequenceDiagram
///     autonumber
///     actor OS as Windows OS
///     participant MsgLoop as App::run (Message Loop)
///     participant WndProc as App::window_proc (Static Callback)
///     participant App as App (Instance)
///     Note over MsgLoop: Vòng lặp tin nhắn chạy cho đến khi running = false
///     MsgLoop->>OS: Gọi GetMessageW() để lấy tin nhắn kế tiếp
///     OS-->>MsgLoop: Trả về cấu trúc tin nhắn (MSG)
///     alt msg.hwnd.0.is_null() (Thread Message)
///         MsgLoop->>App: dispatch_thread_message(&msg)
///         alt WM_HOTKEY
///             App->>App: handle_hotkey(wParam)
///         else WM_APP_UNCOMBINE
///             App->>App: handle_uncombine(wParam)
///         else WM_APP_INVALIDATE_CACHE
///             App->>App: handle_cache_invalidate(wParam)
///         end
///     else msg.hwnd.0.is_null() là false (Window Message)
///         MsgLoop->>OS: TranslateMessage(&msg) & DispatchMessageW(&msg)
///         OS->>WndProc: Kích hoạt callback: window_proc(hwnd, msg, wparam, lparam)
///         Note over WndProc: Truy xuất con trỏ App từ GWLP_USERDATA
///         WndProc->>App: handle_window_message(msg, wparam, lparam)
///         alt WM_USER_TRAYICON
///             App->>App: tray_icon.show(...)
///         else WM_COMMAND
///             App->>App: Xử lý lệnh menu (Exit / Toggle Combine Mode)
///         else WM_DESTROY
///             App->>App: Thiết lập running = false
///         else WM_TASKBARCREATED
///             App->>App: tray_icon.reregister()
///         end
///         App-->>WndProc: Trả về kết quả (LRESULT)
///         WndProc-->>OS: Trả về kết quả
///         OS-->>MsgLoop: Hoàn thành DispatchMessageW()
///     end
/// ```
#[cfg_attr(doc, aquamarine)]
pub struct App {
    /// Trình duyệt và điều hướng các nút trên thanh Taskbar.
    enumerator: TaskbarEnumerator,

    /// Trình quản lý đăng ký và gỡ bỏ phím nóng toàn cục (Alt+[` / Alt+`]).
    hotkey_manager: HotkeyManager,

    /// Trình quản lý cấu hình tách/gộp nhóm (Uncombine) của các cửa sổ Taskbar.
    /// Được leak tĩnh (`&'static`) để an toàn khi chia sẻ giữa các luồng/callback WinEvent.
    uncombine_manager: Box<UncombineManager>,

    /// Cờ xác định chế độ gộp nhóm (Combine mode) hiện tại có bật hay không.
    uncombine_enabled: AtomicBool,

    /// Cờ điều khiển việc duy trì chạy vòng lặp tin nhắn (Message Loop).
    running: Arc<AtomicBool>,

    /// Khay hệ thống đại diện cho ứng dụng trên thanh Taskbar phụ.
    tray_icon: TrayIcon,

    /// Cửa sổ ẩn xử lý thông điệp hệ thống (Hidden Window).
    hidden_window: HiddenWindow,

    /// Cửa sổ Indicator hiển thị trạng thái Virtual Desktop.
    indicator_window: IndicatorWindow,
}

impl App {
    /// Khởi tạo và liên kết tất cả các thành phần cốt lõi của ứng dụng.
    ///
    /// Tham số `combine_enabled` xác định trạng thái ban đầu của việc gộp nhóm nút.
    ///
    /// # Errors
    ///
    /// Hàm sẽ trả về lỗi nếu không thể khởi tạo [`TaskbarEnumerator`], [`HotkeyManager`],
    /// tạo cửa sổ ẩn Win32 hoặc đăng ký khay hệ thống ([`TrayIcon`]).
    pub fn new(config: &AppConfig) -> anyhow::Result<Self> {
        let enumerator = TaskbarEnumerator::new()?;
        let hotkey_manager = HotkeyManager::new(config)?;
        let uncombine_manager = Box::new(UncombineManager::new());

        let mut tray_icon = TrayIcon::create();
        let hidden_window = Self::create_hidden_window()?;

        unsafe {
            WM_TASKBARCREATED = RegisterWindowMessageW(w!("TaskbarCreated"));
        }

        tray_icon.register(hidden_window.hwnd)?;

        let uncombine_enabled = AtomicBool::new(config.uncombine_mode);

        let indicator_window = unsafe { IndicatorWindow::new()? };

        Ok(Self {
            enumerator,
            hotkey_manager,
            uncombine_manager,
            uncombine_enabled,
            running: Arc::new(AtomicBool::new(true)),
            tray_icon,
            hidden_window,
            indicator_window,
        })
    }

    /// Khởi chạy vòng lặp tin nhắn chính (Main Message Loop) của ứng dụng.
    ///
    /// Thực hiện việc cài đặt các Hook sự kiện (WinEventHook và UI Automation Hook),
    /// áp dụng chế độ Uncombine ban đầu và bắt đầu nhận/xử lý các tin nhắn từ Windows.
    /// Khi ứng dụng kết thúc, hàm sẽ thực hiện dọn dẹp và khôi phục trạng thái hệ thống.
    ///
    /// # Safety
    ///
    /// Hàm này bắt buộc phải được chạy trên luồng main đã khởi tạo COM dưới dạng STA
    /// (`COINIT_APARTMENTTHREADED`).
    pub unsafe fn run(&mut self, main_thread_id: u32) -> anyhow::Result<()> {
        self.indicator_window.run();

        SetWindowLongPtrW(
            self.hidden_window.hwnd,
            GWLP_USERDATA,
            self as *mut Self as isize,
        );

        let _win_hook = event::WinEventHook::install(&self.uncombine_manager)?;
        self.enumerator.install_uia_hook(main_thread_id)?;

        if self.uncombine_enabled.load(Ordering::SeqCst) {
            self.uncombine_manager.uncombine_all();
        }

        let mut msg = std::mem::zeroed();
        while self.running.load(Ordering::SeqCst) {
            let result = GetMessageW(&mut msg, None, 0, 0);

            match result.0 {
                0 => break, // WM_QUIT
                -1 => {
                    error!("GetMessageW failed");
                    break;
                }
                _ => {}
            }

            match msg.hwnd.0.is_null() {
                true => self.dispatch_thread_message(&msg),
                false => {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }

        Ok(())
    }

    /// Xử lý và điều phối các tin nhắn luồng (Thread Messages).
    ///
    /// Các tin nhắn được gửi qua [`PostThreadMessageW`] không có handle cửa sổ cụ thể.
    /// Chúng ta cần tự phân tích và định tuyến đến các hàm xử lý tương ứng:
    /// - `WM_HOTKEY`: Xử lý khi người dùng nhấn tổ hợp phím nóng.
    /// - `WM_APP_UNCOMBINE`: Xử lý việc tách nhóm cho một cửa sổ mới.
    /// - `WM_APP_INVALIDATE_CACHE`: Xóa bộ nhớ đệm danh sách các nút Taskbar.
    fn dispatch_thread_message(&mut self, msg: &MSG) {
        match msg.message {
            WM_HOTKEY => self.handle_hotkey(msg.wParam),
            event::WM_APP_UNCOMBINE => self.handle_uncombine(msg.wParam),
            event::WM_APP_INVALIDATE_CACHE => self.handle_cache_invalidate(msg.wParam),
            event::WM_APP_RELOAD_CONFIG => self.handle_reload_config(),
            _ => {}
        }
    }

    /// Xử lý các thông điệp Win32 hướng cửa sổ (Window Messages).
    ///
    /// Trả về `Some(LRESULT)` nếu thông điệp đã được xử lý và không cần chuyển tiếp đến `DefWindowProcW`.
    pub fn handle_window_message(
        &mut self,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        match msg {
            WM_USER_TRAYICON => {
                let mouse_event = lparam.0 as u32;
                // Hiển thị menu ngữ cảnh tại vị trí con trỏ chuột
                if mouse_event == WM_LBUTTONUP || mouse_event == WM_RBUTTONUP {
                    if let Err(e) = self.tray_icon.show(
                        self.uncombine_enabled.load(Ordering::SeqCst),
                        CONSOLE_VISIBLE.load(Ordering::SeqCst),
                    ) {
                        error!("show_context_menu: {e}");
                    }
                }
                Some(LRESULT(0))
            }
            event::WM_APP_RELOAD_CONFIG => {
                self.handle_reload_config();
                Some(LRESULT(0))
            }
            // Các lệnh Command được gửi từ Menu ngữ cảnh của Khay hệ thống
            WM_COMMAND => {
                let id = loword(wparam.0 as u32);
                match id {
                    IDM_EXIT => {
                        info!("Exit from tray menu");
                        self.running.store(false, Ordering::SeqCst);
                        unsafe {
                            PostQuitMessage(0);
                        }
                    }
                    IDM_UNCOMBINE_MODE => {
                        let was = self.uncombine_enabled.load(Ordering::SeqCst);
                        self.uncombine_enabled.store(!was, Ordering::SeqCst);
                        let new = !was;
                        info!(
                            "Uncombine mode: {}",
                            if new { "enabled" } else { "disabled" }
                        );
                        match new {
                            true => self.uncombine_manager.uncombine_all(),
                            false => self.uncombine_manager.restore_all(),
                        }
                    }
                    IDM_SHOW_CONSOLE => {
                        console::toggle();
                    }
                    IDM_SETTINGS => {
                        info!("Opening settings UI");
                        setting::show_ui();
                    }
                    _ => {}
                }
                Some(LRESULT(0))
            }

            // Hủy cửa sổ ẩn (ví dụ: khi hệ thống tắt cửa sổ này)
            WM_DESTROY => {
                self.running.store(false, Ordering::SeqCst);
                Some(LRESULT(0))
            }

            // Windows shutdown forced
            WM_QUERYENDSESSION => Some(LRESULT(1)),

            // Windows shutdown đang diễn ra!! Cleanup gấp!!
            WM_ENDSESSION => {
                if wparam.0 != 0 {
                    self.running.store(false, Ordering::SeqCst);
                    unsafe {
                        PostQuitMessage(0);
                    }
                }
                Some(LRESULT(0))
            }

            // Thông điệp đặc biệt từ Windows Explorer thông báo thanh Taskbar đã được tạo lại.
            // Điều này xảy ra khi tiến trình explorer.exe khởi động lại.
            _ if msg == unsafe { WM_TASKBARCREATED } => {
                info!("TaskbarCreated — re‑registering tray icon");
                if let Err(e) = self.tray_icon.reregister() {
                    error!("TrayIcon reregister: {e}");
                }
                Some(LRESULT(0))
            }

            _ => None,
        }
    }
}

impl App {
    /// Tạo một cửa sổ Win32 ẩn để nhận các sự kiện hệ thống. Đại diện ứng dụng window hiện tại
    /// để chạy ngầm
    ///
    /// Cửa sổ này không hiển thị trên màn hình và có thuộc tính `WS_EX_TOOLWINDOW` để
    /// không xuất hiện trên Taskbar hoặc trình chuyển đổi Alt+Tab.
    fn create_hidden_window() -> anyhow::Result<HiddenWindow> {
        let hinstance = unsafe { GetModuleHandleW(None) }?;

        let wnd_class = WNDCLASSW {
            hInstance: HINSTANCE(hinstance.0),
            lpszClassName: w!("TaskbarSwitcherTray"),
            lpfnWndProc: Some(Self::window_proc),
            ..Default::default()
        };

        let atom = unsafe { RegisterClassW(&wnd_class) };
        if atom == 0 {
            anyhow::bail!("RegisterClassW failed");
        }

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW,
                w!("TaskbarSwitcherTray"),
                w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                None,
                None,
                Some(hinstance.into()),
                None,
            )?
        };

        Ok(HiddenWindow { hwnd })
    }

    /// Thủ tục cửa sổ tĩnh (Window Procedure) nhận các sự kiện từ hệ thống và chuyển tiếp tới `App`.
    ///
    /// Sử dụng con trỏ của đối tượng `App` được lưu trữ trong `GWLP_USERDATA` để gọi phương thức
    /// `handle_window_message` tương ứng.
    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let app_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
        if app_ptr != 0 {
            let app = &mut *(app_ptr as *mut Self);
            if let Some(result) = app.handle_window_message(msg, wparam, lparam) {
                return result;
            }
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

impl App {
    fn handle_reload_config(&mut self) {
        info!("Reloading configuration...");
        let config = crate::config::AppConfig::load();

        self.uncombine_enabled
            .store(config.uncombine_mode, Ordering::SeqCst);

        match config.uncombine_mode {
            true => self.uncombine_manager.uncombine_all(),
            false => self.uncombine_manager.restore_all(),
        }

        if let Err(e) = self.hotkey_manager.reload(&config) {
            error!("Failed to reload hotkeys: {}", e);
        }

        info!("Configuration reloaded successfully.");
    }

    /// Xử lý sự kiện nhấn phím nóng toàn cục
    ///
    /// Chuyển đổi ID phím nóng thành hành động di chuyển trái/phải trên thanh Taskbar.
    /// Ngoài ra, cơ chế này tự động dọn dẹp các sự kiện WM_HOTKEY lặp lại do cơ chế
    /// auto-repeat của Windows sinh ra khi giữ phím lâu để tránh di chuyển vượt tầm kiểm soát.
    fn handle_hotkey(&self, wparam: WPARAM) {
        match self.hotkey_manager.action_from_id(wparam.0 as i32) {
            Some(HotkeyAction::CycleLeft) => {
                match self.enumerator.cycle_to_neighbor(
                    self.uncombine_enabled.load(Ordering::SeqCst),
                    CycleDirection::Backward,
                ) {
                    Ok(_) => { /* Thành công */ }
                    Err(e) => error!("Error cycling taskbar: {e}"),
                }
            }
            Some(HotkeyAction::CycleRight) => {
                match self.enumerator.cycle_to_neighbor(
                    self.uncombine_enabled.load(Ordering::SeqCst),
                    CycleDirection::Forward,
                ) {
                    Ok(_) => { /* Thành công */ }
                    Err(e) => error!("Error cycling taskbar: {e}"),
                }
            }
            Some(HotkeyAction::SwitchVirtualDesktop(index)) => {
                let _guard = debug_span!("hotkey", action = "switch_virtual_desktop", index);
                if let Err(e) = winvd::switch_desktop(index) {
                    error!("Failed to switch virtual desktop {}: {:?}", index, e);
                }
            }
            None => {}
        }

        // Khi người dùng bấm giữ phím, Windows auto-repeat sẽ sinh ra hàng loạt sự kiện WM_HOTKEY
        // trong lúc main thread đang bị block bởi lệnh cycle_to_neighbor.
        //
        // Dọn sạch hàng đợi (PeekMessage) loại bỏ các lệnh thừa này để tránh trượt cửa sổ khi
        // thả tay.
        unsafe {
            let mut msg = std::mem::zeroed();
            while PeekMessageW(&mut msg, None, WM_HOTKEY, WM_HOTKEY, PM_REMOVE).as_bool() {}
        }
    }

    /// Xử lý sự kiện yêu cầu tách nhóm (Uncombine) cho một cửa sổ mới xuất hiện.
    ///
    /// Hàm này được kích hoạt khi hook WinEvent bắt được sự kiện hiển thị cửa sổ mới.
    fn handle_uncombine(&self, wparam: WPARAM) {
        let hwnd = HWND(wparam.0 as *mut _);
        let _guard = debug_span!("winevent", event = "UNCOMBINE").entered();
        debug!("hwnd={:?}", hwnd);
        if self.uncombine_enabled.load(Ordering::SeqCst) {
            self.uncombine_manager
                .uncombine_one(hwnd, || self.enumerator.invalidate_cache());
        }
    }

    /// Xóa bộ nhớ đệm (cache) lưu các nút Taskbar khi phát hiện cấu trúc Taskbar thay đổi.
    ///
    /// Đồng thời, thiết lập lại cờ báo hiệu đã xử lý xong để chuẩn bị cho lần invalidate tiếp theo.
    fn handle_cache_invalidate(&self, wparam: WPARAM) {
        let source = InvalidateSource::from_wparam(wparam.0);
        let _guard = debug_span!("winevent", event = "INVALIDATE_CACHE", %source).entered();
        self.enumerator.invalidate_cache();
        event::reset_cache_invalidated_flag();
    }
}

impl Drop for App {
    fn drop(&mut self) {
        unsafe {
            SetWindowLongPtrW(self.hidden_window.hwnd, GWLP_USERDATA, 0);
        }
    }
}

/// Lấy phần byte thấp (16-bit) của một số 32-bit (tương tự vĩ lệnh LOWORD trong C++).
fn loword(value: u32) -> u32 {
    value & 0xFFFF
}

struct HiddenWindow {
    hwnd: HWND,
}

impl Drop for HiddenWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
        debug!("HiddenWindow destroyed");
    }
}
