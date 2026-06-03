mod hotkey;
mod logging;
mod switcher;
mod taskbar;
mod temp;
mod uia_events;
mod uncombine;
mod utils;
mod winevent;

use crate::hotkey::{HotkeyAction, HotkeyManager};
use crate::uncombine::UncombineManager;
use crate::utils::truncate;
use crate::winevent::InvalidateSource;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use taskbar::{CycleDirection, TaskbarEnumerator};
use tracing::{debug, debug_span, error, info, instrument, warn};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetMessageW, PostThreadMessageW, WM_HOTKEY, WM_QUIT,
};

#[instrument(level = "debug", skip_all)]
fn cycle_taskbar(
    enumerator: &TaskbarEnumerator,
    combine_enabled: bool,
    direction: CycleDirection,
) -> anyhow::Result<()> {
    let foreground = unsafe { GetForegroundWindow() };

    let target = enumerator.cycle_to_neighbor(foreground, combine_enabled, direction)?;

    match target {
        Some(entry) => {
            debug!(
                "Activating '{}' (grouped={})",
                truncate(&entry.name, 30),
                entry.is_grouped,
            );

            let ok = unsafe { switcher::force_activate(entry.hwnd) };
            if !ok {
                warn!("force_activate returned false (foreground lock may be active)");
            }
        }
        None => {
            warn!("No window found to cycle to");
        }
    }

    Ok(())
}

fn print_help(verbose: bool, uncombine_enabled: bool) {
    let mut info = String::from(
        "\nTaskbar Switcher started:\
        \n\tAlt+[  : cycle left\
        \n\tAlt+]  : cycle right\
        \n\tCtrl-C : quit\
        \n\
        \n\t-v/--verbose: enable debug logging\
        \n\t--uncombine: enable uncombine mode",
    );

    if verbose {
        info.push_str("\nEnable verbose logging");
    }

    if uncombine_enabled {
        info.push_str("\nEnable uncombine mode");
    }

    println!("{}\n", info);
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let _verbose = args.iter().any(|arg| arg == "-v" || arg == "--verbose");
    let verbose = true;
    let _combine_enabled = args.iter().any(|a| a == "--combine-mode");
    let combine_enabled = false;

    print_help(verbose, combine_enabled);

    let _file_graud = logging::setup_logger(verbose);
    let enumerator = TaskbarEnumerator::new()?;
    let hotkey_manager = HotkeyManager::new()?;

    let main_thread_id = unsafe { GetCurrentThreadId() };

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        info!("Ctrl-C received, exiting...");
        r.store(false, Ordering::SeqCst);
        unsafe {
            let _ = PostThreadMessageW(
                main_thread_id,
                WM_QUIT,
                WPARAM::default(),
                LPARAM::default(),
            );
        }
    })
    .unwrap_or_else(|e| error!("Ctrl-C handler failed: {e}"));

    unsafe {
        let uncombine: &'static UncombineManager = Box::leak(Box::new(UncombineManager::new()));
        let mut msg = std::mem::zeroed();
        winevent::install_hook(uncombine)?;
        enumerator.install_uia_handler(main_thread_id)?;

        if !combine_enabled {
            uncombine.uncombine_all();
        }

        while running.load(Ordering::SeqCst) {
            let result = GetMessageW(&mut msg, None, 0, 0);

            if result.0 == 0 {
                enumerator.uninstall_uia_handler();
                winevent::uninstall_hook();
                hotkey_manager.unregister_all();

                if !combine_enabled {
                    uncombine.restore_all();
                }

                break;
            }

            if result.0 == -1 {
                break;
            }

            match msg.message {
                WM_HOTKEY => match hotkey_manager.action_from_id(msg.wParam.0 as i32) {
                    Some(HotkeyAction::Left) => {
                        if let Err(e) =
                            cycle_taskbar(&enumerator, combine_enabled, CycleDirection::Backward)
                        {
                            error!("Error cycling taskbar: {e}");
                        }
                    }
                    Some(HotkeyAction::Right) => {
                        if let Err(e) =
                            cycle_taskbar(&enumerator, combine_enabled, CycleDirection::Forward)
                        {
                            error!("Error cycling taskbar: {e}");
                        }
                    }
                    None => continue,
                },
                winevent::WM_APP_UNCOMBINE => {
                    let hwnd = HWND(msg.wParam.0 as *mut _);
                    let _guard = debug_span!("winevent", event = "UNCOMBINE").entered();
                    debug!("hwnd={:?}", hwnd);
                    if !combine_enabled {
                        uncombine.uncombine_one(hwnd, || enumerator.invalidate_cache());
                    }
                }
                winevent::WM_APP_INVALIDATE_CACHE => {
                    let source = InvalidateSource::from_wparam(msg.wParam.0);
                    let _guard =
                        debug_span!("winevent", event = "INVALIDATE_CACHE", %source).entered();
                    enumerator.invalidate_cache();
                    winevent::reset_cache_invalidated_flag();
                }
                _ => {}
            }
        }
    }

    info!("Taskbar Switcher stopped");
    Ok(())
}
