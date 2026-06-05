//! Tìm kiếm cửa sổ và đọc thuộc tính window (AppUserModelID, process name).

use std::{collections::HashMap, sync::Mutex};

use tracing::instrument;
use windows::Win32::{
    Foundation::{CloseHandle, HWND, LPARAM, RECT, TRUE},
    Storage::EnhancedStorage::PKEY_AppUserModel_ID,
    System::{
        Com::StructuredStorage::PROPVARIANT,
        Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION},
    },
    UI::{
        Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow},
        WindowsAndMessaging::{
            EnumWindows, GetClassNameW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId,
            IsWindowVisible,
        },
    },
};
use windows_core::{BOOL, BSTR};

use crate::{types::WindowInfo, utils::is_system_class};

struct EnumData {
    windows: Vec<WindowInfo>,
}

/// Liệt kê tất cả cửa sổ visible trên desktop, bỏ qua system windows.
#[instrument(level = "debug", skip_all)]
pub(super) fn find_visible_windows() -> Vec<WindowInfo> {
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

    let mut class_buf = [0u16; 256];
    let class_len = unsafe { GetClassNameW(hwnd, &mut class_buf) };
    if class_len > 0 {
        let class_name = String::from_utf16_lossy(&class_buf[..class_len as usize]);
        if is_system_class(&class_name) {
            return TRUE;
        }
    }

    let data = &mut *(lparam.0 as *mut EnumData);
    data.windows.push(find_window_by_hwnd(hwnd));

    TRUE
}

unsafe fn find_window_by_hwnd(hwnd: HWND) -> WindowInfo {
    let mut title_buf = [0u16; 512];
    let len = GetWindowTextW(hwnd, &mut title_buf);
    let title = if len == 0 {
        String::new()
    } else {
        String::from_utf16_lossy(&title_buf[..len as usize])
    };

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

    WindowInfo {
        hwnd,
        title,
        rect,
        process_name,
        process_id: pid,
    }
}

/// Lấy tên process từ PID, có cache nội bộ.
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

/// Lấy AppUserModelID từ window qua `SHGetPropertyStoreForWindow`.
///
/// Đây là cơ chế chính thức Windows dùng để group taskbar buttons.
pub(super) fn get_app_user_model_id(hwnd: HWND) -> Option<String> {
    unsafe {
        let store: IPropertyStore = match SHGetPropertyStoreForWindow(hwnd) {
            Ok(s) => s,
            Err(_) => return None,
        };

        let variant: PROPVARIANT = match store.GetValue(&PKEY_AppUserModel_ID) {
            Ok(v) => v,
            Err(_) => return None,
        };

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
