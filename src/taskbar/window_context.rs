use windows::Win32::{
    Foundation::{HWND, POINT},
    Graphics::Gdi::{MonitorFromPoint, MonitorFromWindow, HMONITOR, MONITOR_DEFAULTTONEAREST},
    UI::WindowsAndMessaging::{GetCursorPos, GetForegroundWindow},
};
use winvd::{get_current_desktop, Desktop};

/// The context of something in Windows? (can you get a better name?) Including the
/// foreground window, monitor, and virtual desktop.
pub struct WindowContext {
    pub foreground_window: HWND,
    pub monitor: HMONITOR,
    pub virtual_desktop: Option<Desktop>,
}

impl WindowContext {
    /// Returns the current state of the window context
    ///
    /// The monitor is detected by the cursor position first, then by the foreground window.
    pub fn current_state() -> WindowContext {
        unsafe {
            let fg = GetForegroundWindow();

            let mut pt = POINT::default();
            let hmonitor = GetCursorPos(&mut pt)
                .map(|_| MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST))
                .unwrap_or_else(|_| {
                    if fg.0.is_null() {
                        HMONITOR::default()
                    } else {
                        MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST)
                    }
                });

            let virtual_desktop = get_current_desktop().ok();

            WindowContext {
                foreground_window: fg,
                monitor: hmonitor,
                virtual_desktop,
            }
        }
    }
}
