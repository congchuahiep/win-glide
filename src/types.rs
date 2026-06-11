//! Shared data types - contains purely data structures, no logic.
//!
//! This file does not import from any other module in the project.
//! All other modules import from here -> no circular dependency.

use windows::Win32::Foundation::{HWND, RECT};

/// Taskbar button on the Windows 11 XAML taskbar, read from UI Automation.
///
/// It does not have a separate HWND because XAML elements are not Win32 windows.
#[derive(Debug, Clone)]
pub struct TaskbarButton {
    /// The display name of the button (e.g., "Chrome", "Edge - 3 running windows")
    pub name: String,

    /// Position and size on the screen (in pixels)
    pub rect: RECT,

    /// PID of the process owning the button (usually explorer.exe on Win11)
    pub process_id: i32,

    /// Automation ID from UIA, which is the AppUserModelID 70% of the time?
    pub automation_id: Option<String>,
}

/// Visible window on the desktop, used for matching buttons to windows.
#[derive(Debug, Clone)]
pub struct WindowInfo {
    /// Window handle (HWND)
    pub hwnd: HWND,

    /// Window title
    pub title: String,

    /// Process ID
    pub process_id: u32,

    /// Position and size
    pub rect: RECT,

    /// Executable file name (e.g., "chrome.exe")
    pub process_name: String,

    /// AppUserModelID (AUMID) of the window
    pub aumid: Option<String>,
}

/// A target window in the cycle list.
/// Each entry corresponds to a specific window (HWND), not a taskbar button.
#[derive(Debug, Clone)]
pub struct TargetWindow {
    /// Display name (window title)
    pub name: String,
    /// HWND of the window to activate
    pub hwnd: HWND,
    /// Whether it belongs to a grouped button
    pub is_grouped: bool,
}
