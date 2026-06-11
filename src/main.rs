//! WinGlide - A modern taskbar window switcher for Windows 11.
//!
//! This is the main entry point of the application. It configures the Windows subsystem,
//! sets up panic handlers for crash reporting, and dispatches the execution flow based
//! on the command-line arguments (e.g., running as a background app, settings UI, or console worker).

#![windows_subsystem = "windows"]

mod admin;
mod app;
mod autostart;
mod bootstrap;
mod cli;
mod config;
mod event;
mod hotkey;
mod logging;
mod setting;
mod taskbar;
mod tray_icon;
mod types;
mod updater;
mod utils;
mod virtual_desktop;
mod win32;

/// Displays a fatal error message box to the user.
///
/// This function uses the native Windows API `MessageBoxW` to show an error dialog.
/// It is primarily used when the application fails to initialize or encounters a panic,
/// ensuring the user is informed even if the logger or UI is not available.
///
/// # Arguments
///
/// * `error_msg` - The error message string to display in the dialog body.
fn show_fatal_error(error_msg: &str) {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND,
    };

    let msg_utf16: Vec<u16> = std::ffi::OsStr::new(error_msg)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let title_utf16: Vec<u16> = std::ffi::OsStr::new("WinGlide - Fatal Error")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let _ = MessageBoxW(
            None,
            windows::core::PCWSTR(msg_utf16.as_ptr()),
            windows::core::PCWSTR(title_utf16.as_ptr()),
            MB_ICONERROR | MB_OK | MB_SETFOREGROUND,
        );
    }
}

/// The main entry point of the executable.
///
/// This function sets up a global panic hook to catch and display unhandled panics
/// using a native Windows message box. After configuring the panic hook, it calls
/// the `dispatch` function to handle the actual application logic based on the run mode.
fn main() {
    // Register panic hook to show a dialog when the application crashes
    std::panic::set_hook(Box::new(|info| {
        let payload = info.payload();
        let message = if let Some(s) = payload.downcast_ref::<&str>() {
            s
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.as_str()
        } else {
            "Unknown error"
        };

        let location = info
            .location()
            .map_or("unknown location".to_string(), |loc| {
                format!("{}:{}:{}", loc.file(), loc.line(), loc.column())
            });

        let error_msg = format!(
            "The application has encountered a critical error and must be closed (Crash)!\n\nError Details:\n{}\n\nLocation:\n{}",
            message, location
        );

        tracing::error!("PANIC: {} at {}", message, location);
        show_fatal_error(&error_msg);
    }));

    if let Err(e) = dispatch() {
        tracing::error!("Application Error: {:?}", e);
        let error_msg = format!("Cannot start or run the application!\n\nError:\n{}", e);
        show_fatal_error(&error_msg);
        std::process::exit(1);
    }
}

/// Dispatches the execution flow based on parsed command-line arguments.
///
/// This function determines the application's run mode:
/// - `ConsoleWorker`: Runs the detached debug console worker process.
/// - `SettingsUi`: Launches the XAML-based settings interface.
/// - `BackgroundApp`: Starts the main background WinGlide engine.
///
/// It also handles single-instance enforcement, logger initialization, and DPI awareness.
fn dispatch() -> anyhow::Result<()> {
    let args = cli::parse_args();

    if args.debug {
        bootstrap::attach_debug_console();
    }

    match args.mode {
        cli::RunMode::ConsoleWorker => {
            logging::console::run_worker();
        }
        cli::RunMode::SettingsUi => {
            if bootstrap::ensure_single_instance(bootstrap::InstanceType::SettingsUI) {
                let _guard = logging::setup_logger(args.verbose);
                tracing::info!("Starting settings UI process");
                setting::run()?;
            }
        }
        cli::RunMode::BackgroundApp => {
            if bootstrap::ensure_single_instance(bootstrap::InstanceType::Background) {
                cli::print_help(&args);
                let _guard = logging::setup_logger(args.verbose);
                bootstrap::setup_dpi_awareness();

                let config = crate::config::AppConfig::load();
                let mut app = app::App::new(&config)?;

                if args.reopen_ui {
                    setting::show_ui();
                }

                unsafe {
                    app.run(windows::Win32::System::Threading::GetCurrentThreadId())?;
                }
                tracing::info!("WinGlide stopped");
            }
        }
    }

    Ok(())
}
