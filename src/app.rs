//! Application state - điều phối tất cả components: enumerator, hotkey, event hooks.
//!
//! `App` struct là lớp Model/Orchestrator. Khi thêm GUI:
//! - `run()` thay message loop Win32 bằng GUI event loop
//! - Các `handle_*` methods được gọi từ GUI callbacks
//! - Hotkey register qua GUI framework thay vì Win32 `RegisterHotKey`

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{debug, debug_span, error};
use windows::Win32::Foundation::{HWND, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, WM_HOTKEY};

use crate::event::{self, InvalidateSource};
use crate::hotkey::{HotkeyAction, HotkeyManager};
use crate::taskbar::{CycleDirection, TaskbarEnumerator, UncombineManager};

pub struct App {
    enumerator: TaskbarEnumerator,
    hotkey_manager: HotkeyManager,
    uncombine_manager: &'static UncombineManager,
    combine_enabled: bool,
    running: Arc<AtomicBool>,
}

impl App {
    /// Khởi tạo tất cả components.
    ///
    /// # Panics
    ///
    /// Panic nếu không tìm thấy Shell_TrayWnd hoặc không đăng ký được hotkey.
    pub fn new(combine_enabled: bool) -> anyhow::Result<Self> {
        Ok(Self {
            combine_enabled,
            enumerator: TaskbarEnumerator::new()?,
            hotkey_manager: HotkeyManager::new()?,
            uncombine_manager: Box::leak(Box::new(UncombineManager::new())),
            running: Arc::new(AtomicBool::new(true)),
        })
    }

    /// Clone của running flag - dùng cho Ctrl+C handler.
    pub fn running(&self) -> Arc<AtomicBool> {
        self.running.clone()
    }

    /// Main message loop - install hooks, xử lý event, cleanup.
    ///
    /// # Safety
    ///
    /// Phải gọi trên main thread (STA). Hooks dùng unsafe COM/Win32 APIs.
    pub unsafe fn run(&self, main_thread_id: u32) -> anyhow::Result<()> {
        let _win_hook = event::WinEventHook::install(self.uncombine_manager)?;
        self.enumerator.install_uia_hook(main_thread_id)?;

        if !self.combine_enabled {
            self.uncombine_manager.uncombine_all();
        }

        let mut msg = std::mem::zeroed();

        while self.running.load(Ordering::SeqCst) {
            let result = GetMessageW(&mut msg, None, 0, 0);

            if result.0 == 0 {
                break;
            }

            if result.0 == -1 {
                error!("GetMessageW failed");
                break;
            }

            match msg.message {
                WM_HOTKEY => self.handle_hotkey(msg.wParam),
                event::WM_APP_UNCOMBINE => self.handle_uncombine(msg.wParam),
                event::WM_APP_INVALIDATE_CACHE => self.handle_cache_invalidate(msg.wParam),
                _ => {}
            }
        }

        // Cleanup app tại đây, huỷ đăng ký sự kiện, khôi phục trạng thái gốc của window,...
        // RAII: _win_hook tự Drop ở đây, uia_hook được drop bởi enumerator
        self.hotkey_manager.unregister_all();
        if !self.combine_enabled {
            self.uncombine_manager.restore_all();
        }

        Ok(())
    }
}

impl App {
    fn handle_hotkey(&self, wparam: WPARAM) {
        match self.hotkey_manager.action_from_id(wparam.0 as i32) {
            Some(HotkeyAction::Left) => {
                if let Err(e) = self
                    .enumerator
                    .cycle_to_neighbor(self.combine_enabled, CycleDirection::Backward)
                {
                    error!("Error cycling taskbar: {e}");
                }
            }
            Some(HotkeyAction::Right) => {
                if let Err(e) = self
                    .enumerator
                    .cycle_to_neighbor(self.combine_enabled, CycleDirection::Forward)
                {
                    error!("Error cycling taskbar: {e}");
                }
            }
            None => {}
        }

        // Khi người dùng bấm giữ phím, Windows auto-repeat sẽ sinh ra hàng loạt sự kiện WM_HOTKEY
        // trong lúc main thread đang bị block bởi lệnh cycle_to_neighbor.
        //
        // Dọn sạch hàng đợi (PeekMessage) loại bỏ các lệnh thừa này để tránh trượt cửa sổ khi
        // thả tay.
        unsafe {
            let mut msg = std::mem::zeroed();
            while windows::Win32::UI::WindowsAndMessaging::PeekMessageW(
                &mut msg,
                None,
                WM_HOTKEY,
                WM_HOTKEY,
                windows::Win32::UI::WindowsAndMessaging::PM_REMOVE,
            )
            .as_bool()
            {}
        }
    }

    fn handle_uncombine(&self, wparam: WPARAM) {
        let hwnd = HWND(wparam.0 as *mut _);
        let _guard = debug_span!("winevent", event = "UNCOMBINE").entered();
        debug!("hwnd={:?}", hwnd);
        if !self.combine_enabled {
            self.uncombine_manager
                .uncombine_one(hwnd, || self.enumerator.invalidate_cache());
        }
    }

    fn handle_cache_invalidate(&self, wparam: WPARAM) {
        let source = InvalidateSource::from_wparam(wparam.0);
        let _guard = debug_span!("winevent", event = "INVALIDATE_CACHE", %source).entered();
        self.enumerator.invalidate_cache();
        event::reset_cache_invalidated_flag();
    }
}
