//! Caches the PID of explorer.exe - used to distinguish explorer buttons
//! from actual app buttons in the matching logic.

use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use tracing::debug;

/// Gets the PID of explorer.exe, cached using an atomic.
///
/// The cache is valid until [`invalidate_explorer_pid_cache`] is called
/// (from `TaskbarEnumerator::refresh_taskbar_hwnd` when explorer restarts).
pub(super) fn get_explorer_pid() -> u32 {
    if EXPLORER_PID_VALID.load(Ordering::Relaxed) {
        return EXPLORER_PID_CACHE.load(Ordering::Relaxed);
    }

    let pid = if let Ok(output) = Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq explorer.exe", "/FO", "CSV", "/NH"])
        .creation_flags(0x08000000)
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout
            .lines()
            .find(|l| l.contains("explorer.exe"))
            .and_then(|l| l.split(',').nth(1))
            .and_then(|s| s.trim_matches('"').trim().parse().ok())
            .unwrap_or(0)
    } else {
        0
    };

    EXPLORER_PID_CACHE.store(pid, Ordering::Relaxed);
    EXPLORER_PID_VALID.store(true, Ordering::Relaxed);
    pid
}

/// Invalidates the explorer PID cache - called when explorer restarts.
pub(super) fn invalidate_explorer_pid_cache() {
    EXPLORER_PID_VALID.store(false, Ordering::Relaxed);
    debug!("Explorer PID cache invalidated");
}

static EXPLORER_PID_VALID: AtomicBool = AtomicBool::new(false);
static EXPLORER_PID_CACHE: AtomicU32 = AtomicU32::new(0);
