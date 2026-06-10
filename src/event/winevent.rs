//! WinEvent hook - monitors EVENT_OBJECT_SHOW to uncombine new windows.

use std::fmt::{self, Display};
use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
use tracing::debug;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetClassNameW, IsWindowVisible, PostThreadMessageW, EVENT_OBJECT_SHOW, GA_ROOT,
    INDEXID_CONTAINER, OBJID_WINDOW, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};

use crate::taskbar::UncombineManager;
use crate::utils::is_system_class;

pub const WM_APP_UNCOMBINE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_USER + 0x100;

static MAIN_THREAD_ID: AtomicU32 = AtomicU32::new(0);
static UNCOMBINE: AtomicPtr<UncombineManager> = AtomicPtr::new(std::ptr::null_mut());

/// WinEvent hook RAII - installs on creation, auto-uninstalls on Drop.
pub struct WinEventHook {
    hook_handle: HWINEVENTHOOK,
}

impl WinEventHook {
    /// Installs the WinEvent hook for EVENT_OBJECT_SHOW.
    ///
    /// # Safety
    /// Must be called on the main thread (STA).
    pub unsafe fn install(uncombine: &UncombineManager) -> anyhow::Result<Self> {
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

    /// Uninstalls the WinEvent hook. This should be called when the hook is no longer needed.
    /// Automatically called when the hook is dropped.
    pub unsafe fn uninstall(&mut self) {
        let _ = UnhookWinEvent(self.hook_handle);
        UNCOMBINE.store(std::ptr::null_mut(), Ordering::SeqCst);
        debug!("WinEvent hook uninstalled");
    }
}

impl Drop for WinEventHook {
    fn drop(&mut self) {
        unsafe {
            self.uninstall();
        }
    }
}

/// WinEvent callback - only processes EVENT_OBJECT_SHOW for uncombining.
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

    let mut class_buf = [0u16; 256];
    let class_len = GetClassNameW(hwnd, &mut class_buf);
    if class_len > 0 {
        let class_name = String::from_utf16_lossy(&class_buf[..class_len as usize]);
        if is_system_class(&class_name) {
            return;
        }
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

/// The type of event that caused the cache invalidation (passed via WPARAM).
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
    /// Converts from WPARAM `usize` to the enum (without transmute).
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
