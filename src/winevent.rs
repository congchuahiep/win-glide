//! WinEvent hook — chỉ theo dõi EVENT_OBJECT_SHOW cho uncombine.
//!
//! # Cache invalidation
//!
//! Cache invalidation hiện được xử lý bởi UIA events (uia_events.rs).
//! Các WinEvent HIDE/DESTROY/NAMECHANGE đã bị vô hiệu hoá tạm thời
//! để kiểm tra UIA hoạt động tốt không. Event REORDER bị xoá vì
//! không fire trên Win11 XAML taskbar.

use std::fmt::{self, Display};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};
use tracing::debug;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetWindowTextW, IsWindowVisible, PostThreadMessageW, EVENT_OBJECT_SHOW, GA_ROOT,
    INDEXID_CONTAINER, OBJID_WINDOW, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};

use crate::uncombine::UncombineManager;

pub const WM_APP_UNCOMBINE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_USER + 0x100;
pub const WM_APP_INVALIDATE_CACHE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_USER + 0x101;

static HOOK_HANDLE: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());
static MAIN_THREAD_ID: AtomicU32 = AtomicU32::new(0);
static UNCOMBINE: AtomicPtr<UncombineManager> = AtomicPtr::new(std::ptr::null_mut());

/// Cờ chống gửi message trùng. Dùng bởi UIA events (uia_events.rs).
pub static CACHE_INVALIDATED: AtomicBool = AtomicBool::new(false);

/// Cài đặt WinEvent hook chỉ cho EVENT_OBJECT_SHOW (uncombine).
pub unsafe fn install_hook(uncombine: &'static UncombineManager) -> anyhow::Result<()> {
    MAIN_THREAD_ID.store(GetCurrentThreadId(), Ordering::SeqCst);
    UNCOMBINE.store(uncombine as *const _ as *mut _, Ordering::SeqCst);

    let hook = SetWinEventHook(
        EVENT_OBJECT_SHOW,
        EVENT_OBJECT_SHOW,
        None,
        Some(win_event_proc),
        0,
        0,
        WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
    );

    if hook.is_invalid() {
        anyhow::bail!("Failed to install WinEvent hook");
    }

    HOOK_HANDLE.store(hook.0, Ordering::SeqCst);
    debug!("WinEvent hook installed (EVENT_OBJECT_SHOW only)");
    Ok(())
}

/// Gỡ bỏ WinEvent hook.
pub unsafe fn uninstall_hook() {
    let handle_ptr = HOOK_HANDLE.load(Ordering::SeqCst);
    if !handle_ptr.is_null() {
        let hook = HWINEVENTHOOK(handle_ptr);
        let _ = UnhookWinEvent(hook);
        HOOK_HANDLE.store(std::ptr::null_mut(), Ordering::SeqCst);
        UNCOMBINE.store(std::ptr::null_mut(), Ordering::SeqCst);
        debug!("WinEvent hook uninstalled");
    }
}

/// Callback WinEvent — chỉ xử lý EVENT_OBJECT_SHOW cho uncombine.
unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if id_object != OBJID_WINDOW.0 || id_child != INDEXID_CONTAINER as i32 {
        return;
    }

    if hwnd.0.is_null() {
        return;
    }

    if event != EVENT_OBJECT_SHOW {
        return;
    }

    if !IsWindowVisible(hwnd).as_bool() {
        return;
    }

    if GetAncestor(hwnd, GA_ROOT) != hwnd {
        return;
    }

    let mut title_buf = [0u16; 256];
    if GetWindowTextW(hwnd, &mut title_buf) == 0 {
        return;
    }

    let thread_id = MAIN_THREAD_ID.load(Ordering::SeqCst);
    if thread_id == 0 {
        return;
    }

    let uncombine_ptr = UNCOMBINE.load(Ordering::SeqCst);
    if uncombine_ptr.is_null() {
        return;
    }

    let uncombine = &*uncombine_ptr;
    if uncombine.is_tracked(hwnd) {
        return;
    }

    debug!("WinEvent: SHOW hwnd={:?}", hwnd);
    let _ = PostThreadMessageW(
        thread_id,
        WM_APP_UNCOMBINE,
        WPARAM(hwnd.0 as usize),
        LPARAM(0),
    );
}

/// Reset cờ "cache invalidated" sau khi main thread đã xử lý.
pub fn reset_cache_invalidated_flag() {
    CACHE_INVALIDATED.store(false, Ordering::SeqCst);
}

/// Loại sự kiện gây ra cache invalidation (truyền qua WPARAM).
#[repr(usize)]
#[derive(Debug, Clone, Copy)]
pub enum InvalidateSource {
    ButtonAdded = 0,
    ButtonRemoved = 1,
    ButtonInvalidated = 2,
    ButtonBulkAdded = 3,
    ButtonBulkRemoved = 4,
    DesktopSwitch = 100,
}

impl fmt::Display for InvalidateSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            InvalidateSource::ButtonAdded => "Taskbar button added",
            InvalidateSource::ButtonRemoved => "Taskbar button removed",
            InvalidateSource::ButtonInvalidated => "Taskbar button invalidated",
            InvalidateSource::ButtonBulkAdded => "Taskbar button bulk added",
            InvalidateSource::ButtonBulkRemoved => "Taskbar button bulk removed",
            InvalidateSource::DesktopSwitch => "Desktop switch",
        };

        write!(f, "{}", name)
    }
}

impl InvalidateSource {
    /// Chuyển từ WPARAM `usize` về enum (safe, không transmute).
    pub fn from_wparam(wparam: usize) -> Self {
        match wparam {
            0 => Self::ButtonAdded,
            1 => Self::ButtonRemoved,
            2 => Self::ButtonInvalidated,
            3 => Self::ButtonBulkAdded,
            4 => Self::ButtonBulkRemoved,
            _ => Self::DesktopSwitch,
        }
    }
}
