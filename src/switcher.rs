use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, TRUE};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, BringWindowToTop, EnumWindows, GetForegroundWindow,
    GetWindowThreadProcessId, GetWindowTextW, IsIconic, IsWindowVisible,
    SetForegroundWindow, ShowWindow, SW_RESTORE, ASFW_ANY,
};

pub unsafe fn force_activate(target: HWND) -> bool {
    let foreground = GetForegroundWindow();

    if foreground == target {
        return true;
    }

    // Nếu minimize thì restore trước
    if IsIconic(target).as_bool() {
        let _ = ShowWindow(target, SW_RESTORE);
        // Đợi restore hoàn tất
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    // Cho phép foreground switch
    let _ = AllowSetForegroundWindow(ASFW_ANY);

    // Fast path: thử trực tiếp
    if SetForegroundWindow(target).as_bool() {
        let _ = BringWindowToTop(target);
        return GetForegroundWindow() == target;
    }

    // Fallback: AttachThreadInput dance
    let current_thread = GetCurrentThreadId();
    let foreground_thread = if foreground.0.is_null() {
        0
    } else {
        GetWindowThreadProcessId(foreground, None)
    };

    if foreground_thread == 0 || foreground_thread == current_thread {
        let _ = BringWindowToTop(target);
        // Thử lại sau BringWindowToTop
        let _ = SetForegroundWindow(target);
        return GetForegroundWindow() == target;
    }

    let attached = AttachThreadInput(current_thread, foreground_thread, true).as_bool();

    let _ = SetForegroundWindow(target);
    let _ = BringWindowToTop(target);

    if attached {
        let _ = AttachThreadInput(current_thread, foreground_thread, false);
    }

    GetForegroundWindow() == target
}

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub hwnd: HWND,
    pub title: String,
    pub process_id: u32,
}

struct EnumData {
    windows: Vec<WindowInfo>,
}

pub fn find_visible_windows() -> Vec<WindowInfo> {
    let mut data = EnumData {
        windows: Vec::new(),
    };

    unsafe {
        let _ = EnumWindows(
            Some(enum_windows_callback),
            LPARAM(&mut data as *mut _ as isize),
        );
    }

    data.windows
}

unsafe extern "system" fn enum_windows_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    if !IsWindowVisible(hwnd).as_bool() {
        return TRUE;
    }

    let mut title_buf = [0u16; 512];
    let len = GetWindowTextW(hwnd, &mut title_buf);
    if len == 0 {
        return TRUE;
    }

    let title = String::from_utf16_lossy(&title_buf[..len as usize]);
    if title.is_empty() {
        return TRUE;
    }

    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));

    let data = &mut *(lparam.0 as *mut EnumData);
    data.windows.push(WindowInfo {
        hwnd,
        title,
        process_id: pid,
    });

    TRUE
}

/// Tìm window HWND phù hợp cho một taskbar button.
/// So sánh PID và title để xác định window đích.
pub fn find_window_for_button(button_name: &str, button_pid: i32) -> Option<HWND> {
    let windows = find_visible_windows();

    // Win11 taskbar buttons thường trả về PID của explorer.exe
    // Kiểm tra xem PID có phải app thực không
    let explorer_pid = get_explorer_pid();
    let pid_is_real_app = button_pid > 0 && button_pid != explorer_pid as i32;

    // Thử 1: Match theo PID (nếu PID là app thực)
    if pid_is_real_app {
        let pid_matches: Vec<&WindowInfo> = windows
            .iter()
            .filter(|w| w.process_id == button_pid as u32 && !w.title.is_empty())
            .collect();

        if pid_matches.len() == 1 {
            return Some(pid_matches[0].hwnd);
        }

        // Nhiều window cùng PID → match theo title
        if !pid_matches.is_empty() {
            let clean = crate::taskbar::clean_button_name(button_name);
            for w in &pid_matches {
                let w_clean = crate::taskbar::clean_button_name(&w.title);
                if !w_clean.is_empty() && (w_clean.contains(&clean) || clean.contains(&w_clean))
                {
                    return Some(w.hwnd);
                }
            }
            return Some(pid_matches[0].hwnd);
        }
    }

    // Thử 2: Match theo title
    let clean_name = crate::taskbar::clean_button_name(button_name);
    if clean_name.is_empty() {
        return None;
    }

    // Ưu tiên match chính xác trước
    for w in &windows {
        if w.title.is_empty() {
            continue;
        }
        let w_clean = crate::taskbar::clean_button_name(&w.title);
        if w_clean == clean_name {
            return Some(w.hwnd);
        }
    }

    // Fuzzy match: title chứa nhau
    let mut best_match: Option<(HWND, usize)> = None;
    for w in &windows {
        if w.title.is_empty() {
            continue;
        }
        let w_clean = crate::taskbar::clean_button_name(&w.title);

        let match_len = if w_clean.contains(&clean_name) {
            clean_name.len()
        } else if clean_name.contains(&w_clean) {
            w_clean.len()
        } else {
            0
        };

        if match_len > best_match.map_or(0, |(_, l)| l) {
            best_match = Some((w.hwnd, match_len));
        }
    }

    best_match.map(|(hwnd, _)| hwnd)
}

fn get_explorer_pid() -> u32 {
    use std::process::Command;
    static EXPLORER_PID: std::sync::OnceLock<u32> = std::sync::OnceLock::new();

    *EXPLORER_PID.get_or_init(|| {
        // Lấy PID của explorer.exe
        if let Ok(output) = Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq explorer.exe", "/FO", "CSV", "/NH"])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // Format: "explorer.exe","1234","Console","1","45,678 K"
            for line in stdout.lines() {
                if line.contains("explorer.exe") {
                    let parts: Vec<&str> = line.split(',').collect();
                    if parts.len() >= 2 {
                        let pid_str = parts[1].trim_matches('"').trim();
                        if let Ok(pid) = pid_str.parse::<u32>() {
                            return pid;
                        }
                    }
                }
            }
        }
        0
    })
}