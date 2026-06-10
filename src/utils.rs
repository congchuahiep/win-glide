use windows::core::w;

/// Truncates string `s` to `max_len` characters, appending `...` if it is longer.
pub fn truncate(s: &str, max_len: usize) -> String {
    match s.char_indices().nth(max_len) {
        Some((idx, _)) => format!("{}...", &s[..idx]),
        None => s.to_string(),
    }
}

/// Strips the suffix " - N running window(s)" from the taskbar button name.
///
/// Format on Win11:
/// - Single app: `"Notepad"` -> `"Notepad"`
/// - Multiple windows: `"Chrome - 3 running windows"` -> `"Chrome"`
/// - Pinned: `"Notepad - Pinned"` -> `"Notepad - Pinned"` (unchanged)
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

/// Checks whether a given class name is a system class.
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

/// Checks whether the system is using a light or dark theme.
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
