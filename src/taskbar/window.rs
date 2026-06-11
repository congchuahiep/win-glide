//! Finds windows and reads window properties (AppUserModelID, process name).

use crate::{
    taskbar::{aumid::get_aumid, window_context::WindowContext},
    types::WindowInfo,
    utils::is_system_class,
};
use std::{collections::HashMap, sync::Mutex};
use tracing::{debug, instrument};
use windows::Win32::{
    Foundation::{CloseHandle, HWND, LPARAM, RECT, TRUE},
    Graphics::{
        Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED},
        Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONULL},
    },
    System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    },
    UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindow, GetWindowLongW, GetWindowRect, GetWindowTextW,
        GetWindowThreadProcessId, IsWindowVisible, GWL_EXSTYLE, GW_OWNER, WS_EX_APPWINDOW,
        WS_EX_TOOLWINDOW,
    },
};
use windows_core::BOOL;

struct EnumData<'a> {
    windows: Vec<WindowInfo>,
    context_filter: Option<&'a WindowContext>,
}

/// Enumerates all visible windows on the desktop, ignoring system windows.
///
/// # Arguments
///
/// * `filter` - Optional filter to exclude windows from the result.
#[instrument(level = "debug", skip_all)]
pub(super) fn find_visible_windows(filter: Option<&WindowContext>) -> Vec<WindowInfo> {
    let mut data = EnumData {
        windows: Vec::new(),
        context_filter: filter,
    };

    unsafe {
        let _ = EnumWindows(
            Some(enum_windows_callback),
            LPARAM(&mut data as *mut _ as isize),
        );
    }

    for window in &mut data.windows {
        debug!("{:?}", window);
    }

    data.windows
}

unsafe extern "system" fn enum_windows_callback(
    hwnd: windows::Win32::Foundation::HWND,
    lparam: LPARAM,
) -> BOOL {
    if !IsWindowVisible(hwnd).as_bool() {
        return TRUE;
    }

    let mut cloaked = 0u32;
    let _ = DwmGetWindowAttribute(
        hwnd,
        DWMWA_CLOAKED,
        &mut cloaked as *mut _ as *mut _,
        std::mem::size_of::<u32>() as u32,
    );
    if cloaked != 0 {
        return TRUE;
    }

    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
    let is_tool_window = (ex_style & WS_EX_TOOLWINDOW.0) != 0;
    let is_app_window = (ex_style & WS_EX_APPWINDOW.0) != 0;
    let owner = GetWindow(hwnd, GW_OWNER);

    if is_tool_window {
        return TRUE;
    }

    if owner.is_ok() && !is_app_window {
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

    if let Some(ctx) = data.context_filter {
        // Check if the window is on the same monitor as the context
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL).0;
        if monitor != ctx.monitor.0 {
            return TRUE;
        }

        // Check if this window is on the same virtual desktop as the context
        //
        // Use `transmute` trick to force the window handle to be a the same HWND as the HWND of
        // `winvd`, which have different version (0.58) as this project (0.61)
        let winvd_hwnd = unsafe { std::mem::transmute(hwnd) };
        let is_showed_all_desktop = winvd::is_pinned_window(winvd_hwnd).map_or(false, |r| r);

        if !is_showed_all_desktop {
            let win_desktop = winvd::get_desktop_by_window(winvd_hwnd).ok();

            match (win_desktop, ctx.virtual_desktop) {
                (Some(w_id), Some(ctx_id)) if w_id != ctx_id => return TRUE,
                _ => {}
            }
        }
    }

    data.windows.push(find_window_by_hwnd(hwnd));

    TRUE
}

#[instrument(level = "debug", skip_all)]
pub unsafe fn find_window_by_hwnd(hwnd: HWND) -> WindowInfo {
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

    WindowInfo {
        hwnd,
        title,
        rect,
        process_id: pid,
        process_name: get_process_name(pid),
        aumid: get_aumid(hwnd),
    }
}

/// Gets the process name from PID, with internal caching.
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
