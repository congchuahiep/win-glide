//! Activates a window, bringing it to the foreground using SetForegroundWindow and AttachThreadInput
//! fallback.

use tracing::instrument;
use windows::Win32::{
    Foundation::HWND,
    System::Threading::{AttachThreadInput, GetCurrentThreadId},
    UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId,
        IsIconic, SetForegroundWindow, ShowWindowAsync, ASFW_ANY, SW_RESTORE,
    },
};

/// Brings the target window to the foreground.
///
/// Tries SetForegroundWindow first, if it fails, uses AttachThreadInput to attach the foreground thread
/// to the current thread.
#[instrument(level = "debug", skip_all)]
pub(super) unsafe fn force_activate(target: HWND) -> bool {
    let foreground = GetForegroundWindow();
    if foreground == target {
        return true;
    }

    if IsIconic(target).as_bool() {
        let _ = ShowWindowAsync(target, SW_RESTORE);
    }

    let _ = AllowSetForegroundWindow(ASFW_ANY);

    let _ = SetForegroundWindow(target);
    let _ = BringWindowToTop(target);

    if GetForegroundWindow() == target {
        return true;
    }

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
