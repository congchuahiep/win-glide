mod switcher;
mod taskbar;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_NOREPEAT, VK_OEM_4, VK_OEM_6,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetMessageW, WM_HOTKEY};

use taskbar::TaskbarEnumerator;

const HOTKEY_ID_LEFT: i32 = 1;
const HOTKEY_ID_RIGHT: i32 = 2;

fn cycle_taskbar(enumerator: &TaskbarEnumerator, direction: i32) -> anyhow::Result<()> {
    let buttons = enumerator.enumerate_primary_buttons()?;

    if buttons.is_empty() {
        eprintln!("No taskbar buttons found");
        return Ok(());
    }

    let foreground = unsafe { GetForegroundWindow() };

    let active_index = enumerator
        .find_active_button_index(&buttons, foreground)
        .unwrap_or(0);

    let target_index = if direction > 0 {
        if active_index + 1 >= buttons.len() {
            0
        } else {
            active_index + 1
        }
    } else {
        if active_index == 0 {
            buttons.len() - 1
        } else {
            active_index - 1
        }
    };

    let target_button = &buttons[target_index];

    eprintln!(
        "Cycling {} from [{}] '{}' → [{}] '{}' (pid={})",
        if direction > 0 { "→" } else { "←" },
        active_index,
        buttons[active_index].name,
        target_index,
        target_button.name,
        target_button.process_id,
    );

    // Tìm window HWND cho button đích và force_activate
    match switcher::find_window_for_button(&target_button.name, target_button.process_id) {
        Some(hwnd) => {
            eprintln!("  → force_activate(hwnd={:?})", hwnd);
            let ok = unsafe { switcher::force_activate(hwnd) };
            if !ok {
                eprintln!("  ⚠ force_activate returned false (foreground lock may be active)");
            }
        }
        None => {
            eprintln!("  ✗ Could not find HWND for '{}'", target_button.name);
        }
    }

    Ok(())
}

fn main() -> anyhow::Result<()> {
    // Khởi tạo enumerator để lấy danh sách các taskbar buttons (tên, PID, HWND,...)
    let enumerator = TaskbarEnumerator::new()?;

    eprintln!("Taskbar Switcher started.");
    eprintln!("  Alt+[  → cycle left");
    eprintln!("  Alt+]  → cycle right");
    eprintln!("  Ctrl-C → quit");

    let left_ok: bool;
    let right_ok: bool;

    // Đăng ký 2 hotkey toàn cục
    // 2 hotkey này được đăng ký vào 2 id là [`HOTKEY_ID_LEFT`] và [`HOTKEY_ID_RIGHT`]
    unsafe {
        let left_result = RegisterHotKey(
            None,
            HOTKEY_ID_LEFT,
            MOD_ALT | MOD_NOREPEAT,
            VK_OEM_4.0 as u32,
        );
        left_ok = left_result.is_ok();
        if let Err(e) = left_result {
            eprintln!("Warning: Failed to register Alt+[: {e}");
        }

        let right_result = RegisterHotKey(
            None,
            HOTKEY_ID_RIGHT,
            MOD_ALT | MOD_NOREPEAT,
            VK_OEM_6.0 as u32,
        );
        right_ok = right_result.is_ok();
        if let Err(e) = right_result {
            eprintln!("Warning: Failed to register Alt+]: {e}");
        }
    }

    if !left_ok && !right_ok {
        anyhow::bail!("Failed to register any hotkeys");
    }

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .unwrap_or_else(|e| eprintln!("Ctrl-C handler failed: {e}"));

    unsafe {
        let mut msg = std::mem::zeroed();

        while running.load(Ordering::SeqCst) {
            let result = GetMessageW(&mut msg, None, 0, 0);

            if result.0 == 0 || result.0 == -1 {
                break;
            }

            if msg.message == WM_HOTKEY {
                let direction = match msg.wParam.0 as i32 {
                    HOTKEY_ID_LEFT => -1,
                    HOTKEY_ID_RIGHT => 1,
                    _ => continue,
                };

                if let Err(e) = cycle_taskbar(&enumerator, direction) {
                    eprintln!("Error cycling taskbar: {e}");
                }
            }
        }
    }

    unsafe {
        let _ = UnregisterHotKey(None, HOTKEY_ID_LEFT);
        let _ = UnregisterHotKey(None, HOTKEY_ID_RIGHT);
    }

    eprintln!("Taskbar Switcher stopped.");
    Ok(())
}
