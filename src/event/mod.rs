//! Event module - manages WinEvent hooks and UI Automation StructureChanged events.
//!
//! WinEvent: monitors `EVENT_OBJECT_SHOW` to uncombine newly created windows.
//! UIA: monitors `StructureChanged` to invalidate the taskbar button cache.

mod uia;
mod winevent;

pub use uia::*;
pub use winevent::*;
pub const WM_APP_RELOAD_CONFIG: u32 = windows::Win32::UI::WindowsAndMessaging::WM_USER + 0x102;
pub const WM_APP_RESTART_AS_ADMIN: u32 = windows::Win32::UI::WindowsAndMessaging::WM_USER + 0x103;
