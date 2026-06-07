#![windows_subsystem = "windows"]

mod app;
mod event;
mod hotkey;
mod logging;
mod taskbar;
mod tray_icon;
mod types;
mod utils;

use tracing::info;
use windows::Win32::System::Threading::GetCurrentThreadId;

#[derive(Default)]
struct Args {
    verbose: bool,
    combine_enabled: bool,
    console_worker: bool,
}

fn parse_args() -> Args {
    let raw: Vec<String> = std::env::args().collect();
    Args {
        verbose: raw.iter().any(|a| a == "-v" || a == "--verbose"),
        combine_enabled: raw.iter().any(|a| a == "--combine-mode"),
        console_worker: raw.iter().any(|a| a == "--console-worker"),
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

fn main() -> anyhow::Result<()> {
    let args = parse_args();

    print_help(&args);

    if args.console_worker {
        logging::console::run_worker();
        return Ok(());
    }
    let _guard = logging::setup_logger(args.verbose);

    let main_thread_id = unsafe { GetCurrentThreadId() };
    let mut app = app::App::new(args.combine_enabled)?;

    unsafe {
        app.run(main_thread_id)?;
    }

    info!("Taskbar Switcher stopped");
    Ok(())
}
