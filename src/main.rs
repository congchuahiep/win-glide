#![windows_subsystem = "windows"]

mod app;
mod config;
mod event;
mod hotkey;
mod indicator;
mod logging;
mod setting;
mod taskbar;
mod tray_icon;
mod types;
mod utils;

use std::sync::atomic::Ordering;

use tracing::info;
use windows::Win32::System::Console::{AllocConsole, AttachConsole, ATTACH_PARENT_PROCESS};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi;

#[derive(Default)]
struct Args {
    debug: bool,
    verbose: bool,
    combine_enabled: bool,
    console_worker: bool,
    settings_ui: bool,
}

fn parse_args() -> Args {
    let raw: Vec<String> = std::env::args().collect();
    Args {
        debug: raw.iter().any(|a| a == "--debug"),
        verbose: raw.iter().any(|a| a == "-v" || a == "--verbose"),
        combine_enabled: raw.iter().any(|a| a == "--combine-mode"),
        console_worker: raw.iter().any(|a| a == "--console-worker"),
        settings_ui: raw.iter().any(|a| a == "--settings-ui"),
    }
}

fn print_help(args: &Args) {
    let mut info = String::from(
        "\nTaskbar Switcher started:\
        \n\tAlt+[  : cycle left\
        \n\tAlt+]  : cycle right\
        \n\tRight-click tray icon : menu\
        \n\
        \n\t-v/--verbose: enable debug logging\
        \n\t--combine-mode: enable combine mode\
        \n\t--debug: attach console for debugging",
    );

    if args.verbose {
        info.push_str("\nVerbose logging enabled");
    }

    if args.combine_enabled {
        info.push_str("\nCombine mode enabled");
    }

    if args.debug {
        info.push_str("\nDebug console enabled");
    }

    println!("{}\n", info);
}

fn main() -> anyhow::Result<()> {
    let args = parse_args();

    if args.debug {
        unsafe {
            if AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
                let _ = AllocConsole();
            }
        }
        crate::logging::console::DEBUG_CLI_MODE.store(true, Ordering::SeqCst);
    }

    if args.console_worker {
        logging::console::run_worker();
        return Ok(());
    }

    if args.settings_ui {
        let _guard = logging::setup_logger(args.verbose);
        tracing::info!("Starting settings UI process");

        match setting::run() {
            Ok(_) => {
                tracing::info!("settings UI exited normally");
            }
            Err(e) => {
                tracing::error!("settings UI Error: {:?}", e);
            }
        }
        return Ok(());
    }

    print_help(&args);

    let _guard = logging::setup_logger(args.verbose);

    // Cấu hình DPI Aware cho tiến trình background
    unsafe {
        let _ =
            HiDpi::SetProcessDpiAwarenessContext(HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let main_thread_id = unsafe { GetCurrentThreadId() };
    let config = crate::config::AppConfig::load();
    let mut app = app::App::new(&config)?;

    unsafe {
        app.run(main_thread_id)?;
    }

    info!("Taskbar Switcher stopped");

    Ok(())
}
