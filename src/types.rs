//! Kiểu dữ liệu dùng chung — chỉ chứa data structures thuần, không logic.
//!
//! File này không import từ bất kỳ module nào khác trong project.
//! Mọi module khác import từ đây → không có circular dependency.

use windows::Win32::Foundation::{HWND, RECT};

/// Taskbar button trên Windows 11 XAML taskbar, đọc từ UI Automation.
///
/// Không có HWND riêng vì XAML elements không phải Win32 windows.
#[derive(Debug, Clone)]
pub struct TaskbarButton {
    /// Tên hiển thị của button (VD: "Chrome", "Edge - 3 running windows")
    pub name: String,

    /// Vị trí và kích thước trên màn hình (pixel)
    pub rect: RECT,

    /// PID của process sở hữu button (thường là explorer.exe trên Win11)
    pub process_id: i32,

    /// Automation ID từ UIA, có thể chứa AppUserModelID
    pub automation_id: Option<String>,
}

/// Cửa sổ visible trên desktop, dùng để matching button ↔ window.
#[derive(Debug, Clone)]
pub struct WindowInfo {
    /// Handle của cửa sổ (HWND)
    pub hwnd: HWND,

    /// Window title
    pub title: String,

    /// Process ID
    pub process_id: u32,

    /// Vị trí và kích thước
    pub rect: RECT,

    /// Tên file thực thi (VD: "chrome.exe")
    pub process_name: String,

    /// AppUserModelID — dùng để match với taskbar button automation_id
    pub app_user_model_id: Option<String>,
}
