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
/// - App đơn: `"Notepad"` → `"Notepad"`
/// - Nhiều windows: `"Chrome - 3 running windows"` → `"Chrome"`
/// - Pinned: `"Notepad - Pinned"` → `"Notepad - Pinned"` (không đổi)
/// - VS Code: `"VS Code - main.rs - 1 running window"` → `"VS Code - main.rs"`
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
