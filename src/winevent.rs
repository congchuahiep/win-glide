//! WinEvent hook để phát hiện cửa sổ mới và uncombine ngay lập tức.
//!
//! # Tại sao dùng WinEvent?
//!
//! Khi app đang chạy ở chế độ uncombine, các cửa sổ **mới mở** sau khi app start
//! cũng cần được uncombine. Thay vì polling (lặp vô hạn quét window), ta dùng
//! Windows Accessibility API `SetWinEventHook` để đăng ký callback.
//!
//! Mỗi khi có cửa sổ mới hiển thị (`EVENT_OBJECT_SHOW`), Windows gọi callback
//! của ta trên một **thread riêng**. Callback gửi custom message (`WM_APP_UNCOMBINE`)
//! đến main thread để xử lý uncombine an toàn.
//!
//! # Luồng hoạt động
//!
//! ```text
//! Windows: cửa sổ mới hiển thị
//!   → win_event_proc() trên thread của Windows
//!     → filter (visible, top-level, có title, chưa track)
//!     → PostThreadMessageW(MAIN_THREAD_ID, WM_APP_UNCOMBINE, hwnd)
//!       → Main thread: GetMessageW() nhận WM_APP_UNCOMBINE
//!         → uncombine.uncombine_one(hwnd)
//! ```

use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
use tracing::debug;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetWindowTextW, IsWindowVisible, PostThreadMessageW, EVENT_OBJECT_CREATE,
    EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE, EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_SHOW, GA_ROOT,
    INDEXID_CONTAINER, OBJID_WINDOW, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};

use crate::uncombine::UncombineManager;

/// Custom message ID gửi từ WinEvent callback đến main thread khi có cửa sổ mới.
///
/// `WM_USER + 0x100` đảm bảo không trùng với message hệ thống.
pub const WM_APP_UNCOMBINE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_USER + 0x100;

/// Custom message gửi từ WinEvent callback đến main thread khi cần invalidate cache
/// (window bị đóng hoặc title thay đổi), không cần uncombine.
pub const WM_APP_INVALIDATE_CACHE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_USER + 0x101;

static HOOK_HANDLE: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());
static MAIN_THREAD_ID: AtomicU32 = AtomicU32::new(0);
static UNCOMBINE: AtomicPtr<UncombineManager> = AtomicPtr::new(std::ptr::null_mut());

/// Cài đặt WinEvent hook để bắt sự kiện cửa sổ mới.
///
/// # Tham số
///
/// * `uncombine` — Reference `'static` đến UncombineManager (tạo bằng `Box::leak`)
///
/// # Safety
///
/// Hàm này là `unsafe` vì gọi Windows API và đăng ký callback C-style.
///
/// # Ví dụ
///
/// ```rust,ignore
/// let uncombine: &'static UncombineManager = Box::leak(Box::new(UncombineManager::new()));
/// unsafe { winevent::install_hook(uncombine)?; }
/// ```
pub unsafe fn install_hook(uncombine: &'static UncombineManager) -> anyhow::Result<()> {
    MAIN_THREAD_ID.store(GetCurrentThreadId(), Ordering::SeqCst);
    UNCOMBINE.store(uncombine as *const _ as *mut _, Ordering::SeqCst);

    let hook = SetWinEventHook(
        EVENT_OBJECT_CREATE,
        EVENT_OBJECT_HIDE,
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
    debug!("WinEvent hook installed (CREATE..NAMECHANGE)");
    Ok(())
}

/// Gỡ bỏ WinEvent hook.
///
/// Gọi khi app thoát. Sau khi gọi, callback sẽ không được trigger nữa.
///
/// # Safety
///
/// Phải được gọi trên cùng thread với `install_hook`.
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

/// Callback được Windows gọi mỗi khi có UI object hiển thị.
///
/// Chạy trên **main thread** (do WINEVENT_OUTOFCONTEXT).
/// Tuy nhiên callback có thể bị gọi bất kỳ lúc nào trong quá trình
/// dispatch message — gây reentrancy nếu gọi COM trực tiếp.
/// Vì vậy chỉ filter nhẹ rồi PostThreadMessageW về main loop.
///
/// # Filter chain
///
/// ```text
/// 1. id_object == 0 && id_child == 0    → chỉ root window object
/// 2. event == EVENT_OBJECT_SHOW         → chỉ event show (không hide/create/destroy)
/// 3. IsWindowVisible(hwnd)              → cửa sổ đang visible
/// 4. GetAncestor(GA_ROOT) == hwnd       → top-level window (không child control)
/// 5. GetWindowTextW > 0                 → có title (lọc system window)
/// 6. !uncombine.is_tracked(hwnd)        → chưa được uncombine
/// ```
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

    let uncombine_ptr = UNCOMBINE.load(Ordering::SeqCst);
    if uncombine_ptr.is_null() {
        return;
    }

    let thread_id = MAIN_THREAD_ID.load(Ordering::SeqCst);
    if thread_id == 0 {
        return;
    }

    match event {
        EVENT_OBJECT_CREATE | EVENT_OBJECT_SHOW => {
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

            let uncombine = &*uncombine_ptr;
            if uncombine.is_tracked(hwnd) {
                return;
            }

            let _ = PostThreadMessageW(
                thread_id,
                WM_APP_UNCOMBINE,
                WPARAM(hwnd.0 as usize),
                LPARAM(0),
            );
        }
        EVENT_OBJECT_HIDE => {
            if GetAncestor(hwnd, GA_ROOT) != hwnd {
                return;
            }

            let mut title_buf = [0u16; 256];
            if GetWindowTextW(hwnd, &mut title_buf) == 0 {
                return;
            }

            let _ = PostThreadMessageW(
                thread_id,
                WM_APP_INVALIDATE_CACHE,
                WPARAM(hwnd.0 as usize),
                LPARAM(EVENT_OBJECT_HIDE as isize),
            );
        }
        EVENT_OBJECT_NAMECHANGE => {
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

            let _ = PostThreadMessageW(
                thread_id,
                WM_APP_INVALIDATE_CACHE,
                WPARAM(hwnd.0 as usize),
                LPARAM(EVENT_OBJECT_NAMECHANGE as isize),
            );
        }
        _ => {}
    }
}
