//! Application state management module and orchestrator of all components.
//!
//! The [`App`] struct acts as the orchestrator of the application. It integrates and
//! manages the lifecycle of:
//! - Global hotkey manager ([`HotkeyManager`]) to listen for the Alt+[/] key combinations.
//! - Taskbar button enumerator ([`TaskbarEnumerator`]) via UI Automation (UIA).
//! - Combining/uncombining feature control ([`UncombineManager`]) for windows.
//! - Creating the system tray icon ([`TrayIcon`]) and hidden window to communicate with the Windows Message Loop.

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
use crate::virtual_desktop::indicator::IndicatorWindow;
use crate::logging::console::{self, CONSOLE_VISIBLE};
use crate::setting;
use crate::taskbar::{CycleDirection, TaskbarEnumerator, UncombineManager};
use crate::tray_icon::{TrayIcon, IDM_EXIT, IDM_SETTINGS, IDM_SHOW_CONSOLE};

/// Dynamic Windows message identifier "TaskbarCreated".
/// This message is sent when the Explorer process restarts.
static mut WM_TASKBARCREATED: u32 = 0;

/// Constant identifier for the message the system tray sends to the hidden window.
const WM_USER_TRAYICON: u32 = WM_USER + 0x200;

/// Represents the entire state of the WinGlide application.
///
/// This struct maintains hardware and software connections, including event hooks,
/// hotkeys, system tray, and Win32 hidden window information to listen to system messages.
///
/// ### Windows Message Loop Processing Flow (Windows API)
///
/// ```mermaid
/// sequenceDiagram
///     autonumber
///     actor OS as Windows OS
///     participant MsgLoop as App::run (Message Loop)
///     participant WndProc as App::window_proc (Static Callback)
///     participant App as App (Instance)
///     Note over MsgLoop: Message loop runs until running = false
///     MsgLoop->>OS: Calls GetMessageW() to get the next message
///     OS-->>MsgLoop: Returns message structure (MSG)
///     alt msg.hwnd.0.is_null() (Thread Message)
///         MsgLoop->>App: dispatch_thread_message(&msg)
///         alt WM_HOTKEY
///             App->>App: handle_hotkey(wParam)
///         else WM_APP_UNCOMBINE
///             App->>App: handle_uncombine(wParam)
///         else WM_APP_INVALIDATE_CACHE
///             App->>App: handle_cache_invalidate(wParam)
///         end
///     else msg.hwnd.0.is_null() is false (Window Message)
///         MsgLoop->>OS: TranslateMessage(&msg) & DispatchMessageW(&msg)
///         OS->>WndProc: Triggers callback: window_proc(hwnd, msg, wparam, lparam)
///         Note over WndProc: Retrieves App pointer from GWLP_USERDATA
///         WndProc->>App: handle_window_message(msg, wparam, lparam)
///         alt WM_USER_TRAYICON
///             App->>App: tray_icon.show(...)
///         else WM_COMMAND
///             App->>App: Handles menu commands (Exit / Toggle Combine Mode)
///         else WM_DESTROY
///             App->>App: Sets running = false
///         else WM_TASKBARCREATED
///             App->>App: tray_icon.reregister()
///         end
///         App-->>WndProc: Returns result (LRESULT)
///         WndProc-->>OS: Returns result
///         OS-->>MsgLoop: Completes DispatchMessageW()
///     end
/// ```
#[cfg_attr(doc, aquamarine)]
pub struct App {
    /// Enumerates and navigates buttons on the Taskbar.
    enumerator: TaskbarEnumerator,

    /// Manager for registering and unregistering global hotkeys (Alt+[` / Alt+`]).
    hotkey_manager: HotkeyManager,

    /// Manager for configuring combining/uncombining of Taskbar windows.
    /// Statically leaked (`&'static`) for thread-safe sharing between threads/WinEvent callbacks.
    uncombine_manager: Box<UncombineManager>,

    /// Flag determining whether the current uncombine mode is enabled.
    uncombine_enabled: AtomicBool,

    /// Control flag to maintain the running state of the Message Loop.
    running: Arc<AtomicBool>,

    /// System tray icon representing the application on the secondary Taskbar.
    tray_icon: TrayIcon,

    /// Hidden window for processing system messages.
    hidden_window: HiddenWindow,

    /// Indicator window displaying Virtual Desktop status.
    indicator_window: Option<IndicatorWindow>,
}

impl App {
    /// Initializes and links all core components of the application.
    ///
    /// The `combine_enabled` parameter determines the initial state of button grouping.
    ///
    /// # Errors
    ///
    /// Returns an error if it fails to initialize [`TaskbarEnumerator`], [`HotkeyManager`],
    /// create the Win32 hidden window, or register the system tray icon ([`TrayIcon`]).
    pub fn new(config: &AppConfig) -> anyhow::Result<Self> {
        let enumerator = TaskbarEnumerator::new()?;
        let hotkey_manager = HotkeyManager::new(config)?;
        let uncombine_manager = Box::new(UncombineManager::new());
        let mut tray_icon = TrayIcon::create();
        let hidden_window = Self::create_hidden_window()?;

        let indicator_window = if config.desktop_indicator {
            unsafe { Some(IndicatorWindow::new()?) }
        } else {
            None
        };

        unsafe {
            WM_TASKBARCREATED = RegisterWindowMessageW(w!("TaskbarCreated"));
        }

        tray_icon.register(hidden_window.hwnd)?;
        let uncombine_enabled = AtomicBool::new(config.uncombine_mode);

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

    /// Launches the main message loop of the application.
    ///
    /// Installs event hooks (WinEventHook and UI Automation Hook),
    /// applies the initial uncombine mode, and starts receiving/processing Windows messages.
    /// When the application ends, the function will clean up and restore the system state.
    ///
    /// # Safety
    ///
    /// This function must be run on the main thread which has initialized COM as STA
    /// (`COINIT_APARTMENTTHREADED`).
    pub unsafe fn run(&mut self, main_thread_id: u32) -> anyhow::Result<()> {
        if let Some(indicator) = &mut self.indicator_window {
            indicator.run();
        }

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

    /// Processes and dispatches thread messages.
    ///
    /// Messages sent via [`PostThreadMessageW`] do not have a specific window handle.
    /// We need to analyze and route them to their corresponding handlers:
    /// - `WM_HOTKEY`: Handled when the user presses a hotkey combination.
    /// - `WM_APP_UNCOMBINE`: Handled when uncombining a newly appeared window.
    /// - `WM_APP_INVALIDATE_CACHE`: Clears the cached list of Taskbar buttons.
    fn dispatch_thread_message(&mut self, msg: &MSG) {
        match msg.message {
            WM_HOTKEY => self.handle_hotkey(msg.wParam),
            event::WM_APP_UNCOMBINE => self.handle_uncombine(msg.wParam),
            event::WM_APP_INVALIDATE_CACHE => self.handle_cache_invalidate(msg.wParam),
            event::WM_APP_RELOAD_CONFIG => self.handle_reload_config(),
            _ => {}
        }
    }

    /// Processes Win32 window-directed messages.
    ///
    /// Returns `Some(LRESULT)` if the message has been processed and does not need to be forwarded to `DefWindowProcW`.
    pub fn handle_window_message(
        &mut self,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        match msg {
            WM_USER_TRAYICON => {
                let mouse_event = lparam.0 as u32;
                // Show context menu at cursor position
                if mouse_event == WM_LBUTTONUP || mouse_event == WM_RBUTTONUP {
                    if let Err(e) = self.tray_icon.show(CONSOLE_VISIBLE.load(Ordering::SeqCst)) {
                        error!("show_context_menu: {e}");
                    }
                }
                Some(LRESULT(0))
            }
            event::WM_APP_RELOAD_CONFIG => {
                self.handle_reload_config();
                Some(LRESULT(0))
            }
            event::WM_APP_RESTART_AS_ADMIN => {
                let reopen_ui = wparam.0 == 1;
                let _ = crate::admin::restart_as_admin(reopen_ui);
                unsafe { windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0) };
                Some(LRESULT(0))
            }
            // Commands sent from the system tray context menu
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

            // Destroy hidden window (e.g., when the system closes this window)
            WM_DESTROY => {
                self.running.store(false, Ordering::SeqCst);
                Some(LRESULT(0))
            }

            // Windows shutdown forced
            WM_QUERYENDSESSION => Some(LRESULT(1)),

            // Windows shutdown in progress!! Urgent cleanup!!
            WM_ENDSESSION => {
                if wparam.0 != 0 {
                    self.running.store(false, Ordering::SeqCst);
                    unsafe {
                        PostQuitMessage(0);
                    }
                }
                Some(LRESULT(0))
            }

            // Special message from Windows Explorer notifying that the Taskbar has been recreated.
            // This occurs when the explorer.exe process restarts.
            _ if msg == unsafe { WM_TASKBARCREATED } => {
                info!("TaskbarCreated, re-registering tray icon");
                if let Err(e) = self.tray_icon.reregister() {
                    error!("TrayIcon reregister: {e}");
                }
                Some(LRESULT(0))
            }

            // Update UI when the user changes light/dark mode in Windows Settings
            WM_SETTINGCHANGE => {
                let lparam_str = if lparam.0 != 0 {
                    unsafe {
                        let ptr = lparam.0 as *const u16;
                        let mut len = 0;
                        while *ptr.add(len) != 0 {
                            len += 1;
                        }
                        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
                    }
                } else {
                    String::new()
                };

                if lparam_str == "ImmersiveColorSet" {
                    self.tray_icon.update_theme();
                }
                Some(LRESULT(0))
            }

            _ => None,
        }
    }
}

impl App {
    /// Creates a hidden Win32 window to receive system events. Represents the current window application
    /// running in the background.
    ///
    /// This window is not visible on the screen and has the `WS_EX_TOOLWINDOW` attribute so
    /// it doesn't appear on the Taskbar or Alt+Tab switcher.
    fn create_hidden_window() -> anyhow::Result<HiddenWindow> {
        let hinstance = unsafe { GetModuleHandleW(None) }?;

        let wnd_class = WNDCLASSW {
            hInstance: HINSTANCE(hinstance.0),
            lpszClassName: w!("WinGlideTray"),
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
                w!("WinGlideTray"),
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

    /// Static window procedure to receive system events and forward them to `App`.
    ///
    /// Uses the pointer to the `App` object stored in `GWLP_USERDATA` to call the corresponding
    /// `handle_window_message` method.
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

        if config.desktop_indicator && self.indicator_window.is_none() {
            unsafe {
                match IndicatorWindow::new() {
                    Ok(mut ind) => {
                        ind.run();
                        self.indicator_window = Some(ind);
                    }
                    Err(e) => error!("Failed to create IndicatorWindow: {}", e),
                }
            }
        } else if !config.desktop_indicator && self.indicator_window.is_some() {
            self.indicator_window = None;
        }

        info!("Configuration reloaded successfully.");
    }

    /// Handles global hotkey press events.
    ///
    /// Converts hotkey ID to left/right cycle actions on the Taskbar.
    /// Additionally, this mechanism automatically cleans up repeated WM_HOTKEY events generated by
    /// the Windows auto-repeat mechanism when a key is held down to prevent uncontrollable cycling.
    fn handle_hotkey(&self, wparam: WPARAM) {
        match self.hotkey_manager.action_from_id(wparam.0 as i32) {
            Some(HotkeyAction::CycleLeft) => {
                match self.enumerator.cycle_to_neighbor(
                    self.uncombine_enabled.load(Ordering::SeqCst),
                    CycleDirection::Backward,
                ) {
                    Ok(_) => { /* Success */ }
                    Err(e) => error!("Error cycling taskbar: {e}"),
                }
            }
            Some(HotkeyAction::CycleRight) => {
                match self.enumerator.cycle_to_neighbor(
                    self.uncombine_enabled.load(Ordering::SeqCst),
                    CycleDirection::Forward,
                ) {
                    Ok(_) => { /* Success */ }
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

        // When the user holds down a key, Windows auto-repeat generates a flood of WM_HOTKEY events
        // while the main thread is blocked by the cycle_to_neighbor command.
        //
        // Clear the queue (PeekMessage) to discard these extra commands and prevent runaway cycling
        // when the key is released.
        unsafe {
            let mut msg = std::mem::zeroed();
            while PeekMessageW(&mut msg, None, WM_HOTKEY, WM_HOTKEY, PM_REMOVE).as_bool() {}
        }
    }

    /// Handles the request to uncombine a newly appearing window.
    ///
    /// This function is triggered when the WinEvent hook catches a new window display event.
    fn handle_uncombine(&self, wparam: WPARAM) {
        let hwnd = HWND(wparam.0 as *mut _);
        let _guard = debug_span!("winevent", event = "UNCOMBINE").entered();
        debug!("hwnd={:?}", hwnd);
        if self.uncombine_enabled.load(Ordering::SeqCst) {
            self.uncombine_manager
                .uncombine_one(hwnd, || self.enumerator.invalidate_cache());
        }
    }

    /// Clears the cache storing Taskbar buttons when a structural change in the Taskbar is detected.
    ///
    /// Additionally, resets the processed flag to prepare for the next invalidate.
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

/// Retrieves the low-order word (16 bits) from a 32-bit value (similar to the LOWORD macro in C++).
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
