//! Cache PID của explorer.exe - dùng để phân biệt button của explorer
//! với button của app thực trong matching logic.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use tracing::debug;

/// Lấy PID của explorer.exe, cache bằng atomic.
///
/// Cache valid đến khi [`invalidate_explorer_pid_cache`] được gọi
/// (từ `TaskbarEnumerator::refresh_taskbar_hwnd` khi explorer restart).
pub(super) fn get_explorer_pid() -> u32 {
    use std::process::Command;

    if EXPLORER_PID_VALID.load(Ordering::Relaxed) {
        return EXPLORER_PID_CACHE.load(Ordering::Relaxed);
    }

    let pid = if let Ok(output) = Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq explorer.exe", "/FO", "CSV", "/NH"])
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

/// Invalidate explorer PID cache - gọi khi explorer restart.
pub(super) fn invalidate_explorer_pid_cache() {
    EXPLORER_PID_VALID.store(false, Ordering::Relaxed);
    debug!("Explorer PID cache invalidated");
}

static EXPLORER_PID_VALID: AtomicBool = AtomicBool::new(false);
static EXPLORER_PID_CACHE: AtomicU32 = AtomicU32::new(0);
