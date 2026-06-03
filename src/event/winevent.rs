//! WinEvent hook — theo dõi EVENT_OBJECT_SHOW để uncombine cửa sổ mới.
//!
//! # Cache invalidation
//!
//! Cache invalidation đã chuyển sang UIA events (uia.rs).
//! WinEvent không còn tham gia cache invalidation.

use std::fmt::{self, Display};
use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
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

static MAIN_THREAD_ID: AtomicU32 = AtomicU32::new(0);
static UNCOMBINE: AtomicPtr<UncombineManager> = AtomicPtr::new(std::ptr::null_mut());

/// WinEvent hook RAII — install khi tạo, auto-uninstall khi Drop.
pub struct WinEventHook {
    hook_handle: HWINEVENTHOOK,
}

impl WinEventHook {
    /// Cài đặt WinEvent hook cho EVENT_OBJECT_SHOW.
    ///
    /// # Safety
    /// Phải gọi trên main thread (STA).
    pub unsafe fn install(uncombine: &'static UncombineManager) -> anyhow::Result<Self> {
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

        debug!("WinEvent hook installed (EVENT_OBJECT_SHOW)");
        Ok(Self { hook_handle: hook })
    }
}

impl Drop for WinEventHook {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWinEvent(self.hook_handle);
        }
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

impl Display for InvalidateSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            InvalidateSource::ButtonAdded => "button added",
            InvalidateSource::ButtonRemoved => "button removed",
            InvalidateSource::ButtonInvalidated => "button invalidated",
            InvalidateSource::ButtonBulkAdded => "button bulk added",
            InvalidateSource::ButtonBulkRemoved => "button bulk removed",
            InvalidateSource::DesktopSwitch => "desktop switch",
        };
        write!(f, "{}", name)
    }
}

impl InvalidateSource {
    /// Chuyển từ WPARAM `usize` về enum (không transmute).
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
