mod app;
mod event;
mod hotkey;
mod logging;
mod taskbar;
mod temp;
mod types;
mod utils;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{error, info};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};

#[derive(Default)]
struct Args {
    verbose: bool,
    combine_enabled: bool,
}

fn parse_args() -> Args {
    let raw: Vec<String> = std::env::args().collect();
    Args {
        verbose: raw.iter().any(|a| a == "-v" || a == "--verbose"),
        combine_enabled: raw.iter().any(|a| a == "--combine-mode"),
    }
}

fn print_help(args: &Args) {
    let mut info = String::from(
        "\nTaskbar Switcher started:\
        \n\tAlt+[  : cycle left\
        \n\tAlt+]  : cycle right\
        \n\tCtrl-C : quit\
        \n\
        \n\t-v/--verbose: enable debug logging\
        \n\t--combine-mode: enable combine mode",
    );

    if args.verbose {
        info.push_str("\nVerbose logging enabled");
    }

    if args.combine_enabled {
        info.push_str("\nCombine mode enabled");
    }

    println!("{}\n", info);
}

fn setup_ctrlc_handler(running: Arc<AtomicBool>, main_thread_id: u32) {
    ctrlc::set_handler(move || {
        info!("Ctrl-C received, exiting...");
        running.store(false, Ordering::SeqCst);
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
}

fn main() -> anyhow::Result<()> {
    let args = parse_args();
    print_help(&args);

    let _guard = logging::setup_logger(args.verbose);
    let main_thread_id = unsafe { GetCurrentThreadId() };
    let app = app::App::new(args.combine_enabled)?;

    setup_ctrlc_handler(app.running(), main_thread_id);

    unsafe {
        app.run(main_thread_id)?;
    }

    info!("Taskbar Switcher stopped");
    Ok(())
}
