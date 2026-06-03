use std::collections::HashMap;
use std::sync::Mutex;
use tracing::{debug, error, instrument};
use windows::core::{BOOL, BSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT, TRUE};
use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, BringWindowToTop, EnumWindows, GetForegroundWindow, GetWindowRect,
    GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetForegroundWindow,
    ShowWindow, ASFW_ANY, SW_RESTORE,
};

use crate::taskbar::TaskbarButton;

/// Mở cửa sổ và đưa nó lên đầu danh sách cửa sổ (foreground)
#[instrument(level = "debug", skip_all)]
pub unsafe fn force_activate(target: HWND) -> bool {
    let foreground = GetForegroundWindow();

    if foreground == target {
        return true;
    }

    // Nếu minimize thì restore trước
    if IsIconic(target).as_bool() {
        let _ = ShowWindow(target, SW_RESTORE);
    }

    // Cho phép foreground switch
    let _ = AllowSetForegroundWindow(ASFW_ANY);

    // Thử trực tiếp
    let _ = SetForegroundWindow(target);
    let _ = BringWindowToTop(target);

    // Kiểm tra ngay (không sleep)
    if GetForegroundWindow() == target {
        return true;
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
    pub rect: RECT,
    pub process_name: String,
    pub app_user_model_id: Option<String>,
}

struct EnumData {
    windows: Vec<WindowInfo>,
}

fn get_process_name(pid: u32) -> String {
    use windows::Win32::System::Threading::PROCESS_NAME_FORMAT;

    static CACHE: Mutex<Option<HashMap<u32, String>>> = Mutex::new(None);

    if let Ok(cache) = CACHE.lock() {
        if let Some(map) = cache.as_ref() {
            if let Some(name) = map.get(&pid) {
                return name.clone();
            }
        }
    }

    let name = unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid);
        if handle.is_err() {
            return String::new();
        }
        let handle = handle.unwrap();

        let mut buffer = [0u16; 1024];
        let mut size = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut size,
        );

        let _ = CloseHandle(handle);

        if result.is_ok() && size > 0 {
            String::from_utf16_lossy(&buffer[..size as usize])
                .rsplit('\\')
                .next()
                .unwrap_or("")
                .to_lowercase()
        } else {
            String::new()
        }
    };

    if let Ok(mut cache) = CACHE.lock() {
        cache
            .get_or_insert_with(HashMap::new)
            .insert(pid, name.clone());
    }

    name
}

/// Lấy AppUserModelID từ một window.
/// Đây là cơ chế CHÍNH THỨC Windows dùng để group taskbar buttons.
/// Nếu window không có AppUserModelID, trả về None.
pub fn get_app_user_model_id(hwnd: HWND) -> Option<String> {
    unsafe {
        let store: IPropertyStore = match SHGetPropertyStoreForWindow(hwnd) {
            Ok(s) => s,
            Err(_) => return None,
        };

        let variant: PROPVARIANT = match store.GetValue(&PKEY_AppUserModel_ID) {
            Ok(v) => v,
            Err(_) => return None,
        };

        // PROPVARIANT có thể là VT_EMPTY nếu không có AppUserModelID
        if variant.is_empty() {
            return None;
        }

        let bstr = match BSTR::try_from(&variant) {
            Ok(b) => b,
            Err(_) => return None,
        };

        let s = bstr.to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }
}

#[instrument(level = "debug", skip_all)]
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

    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let _ = GetWindowRect(hwnd, &mut rect);

    let process_name = get_process_name(pid);

    let data = &mut *(lparam.0 as *mut EnumData);
    data.windows.push(WindowInfo {
        hwnd,
        title,
        rect,
        process_name,
        process_id: pid,
        app_user_model_id: None,
    });

    TRUE
}

/// Core matching logic giữa một taskbar button và danh sách windows
///
/// # Thử 4 chiến lược matching theo thứ tự ưu tiên:
/// 1. PID matching (nếu PID là app thực, không phải explorer)
/// 2. AppUserModelID matching (dùng button automation_id để match với window AppUserModelID)
/// 3. Title matching (fuzzy)
/// 4. Process name matching (so khớp tên file thực thi với button name)
///
/// # Returns
/// Danh sách windows matched - 1 phần tử nếu không phải group, N phần tử nếu group.
fn match_windows_for_button(button: &TaskbarButton, all_windows: &[WindowInfo]) -> Vec<WindowInfo> {
    let explorer_pid = get_explorer_pid();
    let pid_is_real_app = button.process_id > 0 && button.process_id != explorer_pid as i32;

    // Thử 1: Match theo PID (nếu PID là app thực)
    if pid_is_real_app {
        let pid_matches: Vec<WindowInfo> = all_windows
            .iter()
            .filter(|w| w.process_id == button.process_id as u32 && !w.title.is_empty())
            .cloned()
            .collect();

        if !pid_matches.is_empty() {
            debug!(
                "Button '{}' have PID match, found {} windows",
                button.name,
                pid_matches.len()
            );

            let mut sorted = pid_matches;
            sorted.sort_by(|a, b| {
                a.rect
                    .left
                    .cmp(&b.rect.left)
                    .then_with(|| a.hwnd.0.cmp(&b.hwnd.0))
            });

            return sorted;
        }
    }

    // Thử 2: AppUserModelID matching (ƯU TIÊN CAO - cơ chế chính thức của Windows)
    // Button automation_id có thể là AppUserModelID hoặc prefix của nó
    // VD: button automation_id "Microsoft Edge" có thể match window với AppUserModelID
    // "Microsoft.MicrosoftEdge_8wekyb3d..."
    if let Some(auto_id) = &button.automation_id {
        if !auto_id.is_empty() {
            let auto_id_lower = auto_id.to_lowercase();
            let appid_matches: Vec<WindowInfo> = all_windows
                .iter()
                .filter(|w| {
                    if w.title.is_empty() {
                        return false;
                    }
                    let window_aumid = match get_app_user_model_id(w.hwnd) {
                        Some(aumid) => aumid,
                        None => return false, // filter closure return
                    };

                    let window_aumid_lower = window_aumid.to_lowercase();
                    window_aumid_lower == auto_id_lower
                        || window_aumid_lower.starts_with(&auto_id_lower)
                        || auto_id_lower.contains(&window_aumid_lower)
                })
                .cloned()
                .collect();

            if !appid_matches.is_empty() {
                debug!(
                    "Button '{}' have AppUserModelID match, found {} windows",
                    button.name,
                    appid_matches.len()
                );

                let mut sorted = appid_matches;
                sorted.sort_by(|a, b| {
                    a.rect
                        .left
                        .cmp(&b.rect.left)
                        .then_with(|| a.hwnd.0.cmp(&b.hwnd.0))
                });

                return sorted;
            }
        }
    }

    // Thử 3: Match theo title (fuzzy)
    let clean_name = crate::taskbar::clean_button_name(&button.name);

    if clean_name.is_empty() {
        error!("Cannot find windows cause there no clue left: !PID, !AppUserModelID, !clean_name");
        return Vec::new();
    }

    let title_matches: Vec<WindowInfo> = all_windows
        .iter()
        .filter(|w| {
            if w.title.is_empty() {
                return false;
            }
            let w_clean = crate::taskbar::clean_button_name(&w.title);
            !w_clean.is_empty() && (w_clean.contains(&clean_name) || clean_name.contains(&w_clean))
        })
        .cloned()
        .collect();

    if !title_matches.is_empty() {
        debug!(
            "Button '{}' have title match, found {} windows",
            button.name,
            title_matches.len()
        );

        let mut sorted = title_matches;
        sorted.sort_by(|a, b| {
            a.rect
                .left
                .cmp(&b.rect.left)
                .then_with(|| a.hwnd.0.cmp(&b.hwnd.0))
        });

        return sorted;
    }

    // Thử 4: Match theo process name
    // VD: button "Edge" → process "msedge.exe" → "msedge" contains "edge" ✓
    let clean_name_lower = clean_name.to_lowercase();
    let process_matches: Vec<WindowInfo> = all_windows
        .iter()
        .filter(|w| {
            if w.title.is_empty() {
                return false;
            }
            let proc_lower = w.process_name.to_lowercase();
            let proc_stem = proc_lower.strip_suffix(".exe").unwrap_or(&proc_lower);
            !proc_stem.is_empty()
                && (proc_stem.contains(&clean_name_lower) || clean_name_lower.contains(proc_stem))
        })
        .cloned()
        .collect();

    if !process_matches.is_empty() {
        debug!(
            "Button '{}' have process match, found {} windows",
            button.name,
            process_matches.len()
        );

        let mut sorted = process_matches;
        sorted.sort_by(|a, b| {
            a.rect
                .left
                .cmp(&b.rect.left)
                .then_with(|| a.hwnd.0.cmp(&b.hwnd.0))
        });

        return sorted;
    }

    error!("Cannot find windows cause there no clue left: !PID, !AppUserModelID, !title, !process");
    Vec::new()
}

/// Hàm này trả về một [`WindowInfo`] (hoặc không có) đã được match với button. Hàm này thường sử
/// dụng trong uncombine mode, khi button không phải là group và chỉ cần trả về một window.
///
/// Nếu cần tìm kiếm window trong một button group, sử dụng [`find_windows_for_button`] thay vì
/// [`find_window_for_button`].
pub fn find_window_for_button(
    button: &TaskbarButton,
    all_windows: &[WindowInfo],
) -> Option<WindowInfo> {
    match_windows_for_button(button, all_windows)
        .into_iter()
        .next()
}

/// Hàm này trả về danh sách các [`WindowInfo`] đã được match với button. Hàm này thường sử dụng
/// trong combine mode, khi button có thể là group và cần trả về nhiều window.
///
/// Nếu không cần kiểm tra button group, sử dụng [`find_window_for_button`] thay vì
/// [`find_windows_for_button`].
pub fn find_windows_for_button(
    button: &TaskbarButton,
    all_windows: &[WindowInfo],
) -> Vec<WindowInfo> {
    match_windows_for_button(button, all_windows)
}

/// Lấy PID của explorer.exe từ tasklist.
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
