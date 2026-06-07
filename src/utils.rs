use windows::core::w;

/// Cắt chuỗi `s` thành `max_len` ký tự, thêm `...` nếu chuỗi dài hơn.
pub fn truncate(s: &str, max_len: usize) -> String {
    match s.char_indices().nth(max_len) {
        Some((idx, _)) => format!("{}...", &s[..idx]),
        None => s.to_string(),
    }
}

/// Strip suffix " - N running window(s)" khỏi tên taskbar button.
///
/// Format trên Win11:
/// - App đơn: `"Notepad"` -> `"Notepad"`
/// - Nhiều windows: `"Chrome - 3 running windows"` -> `"Chrome"`
/// - Pinned: `"Notepad - Pinned"` -> `"Notepad - Pinned"` (không đổi)
/// - VS Code: `"VS Code - main.rs - 1 running window"` -> `"VS Code - main.rs"`
pub fn clean_button_name(name: &str) -> String {
    if let Some(pos) = name.rfind(" running window") {
        let before = &name[..pos];

        if let Some(dash_pos) = before.rfind(" - ") {
            return before[..dash_pos].to_string();
        }

        if let Some(dash_pos) = before.rfind(" \u{2014} ") {
            return before[..dash_pos].to_string();
        }

        return before.to_string();
    }

    name.to_string()
}

/// Kiểm tra xem một class name có phải là class hệ thống hay không.
pub fn is_system_class(class_name: &str) -> bool {
    matches!(
        class_name,
        "Progman" | "WorkerW" |  // Desktop
        "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" |  // Taskbar
        "Windows.UI.Composition.DesktopWindowContentBridge" |  // XAML bridge
        "ApplicationFrame" |  // UWP frame
        "IME" | "MSCTFIME UI" | "IMEUI" |  // IME
        "tooltips_class32" |  // Tooltips
        "DwmWindowComposition" |  // DWM
        "SysAnimate32" // Animation
    )
}

/// Kiểm tra xem hệ thống đang sử dụng theme light hay dark.
pub fn is_light_theme() -> bool {
    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let res = unsafe {
        windows::Win32::System::Registry::RegGetValueW(
            windows::Win32::System::Registry::HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("SystemUsesLightTheme"),
            windows::Win32::System::Registry::RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut _ as *mut _),
            Some(&mut size),
        )
    };
    res.is_ok() && value != 0
}
