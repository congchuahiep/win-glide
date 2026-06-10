//! Provides utilities for checking and requesting Windows Administrator privileges.
//!
//! This module contains functions to query the current process token for elevation status
//! and to restart the application requesting elevated privileges via the UAC prompt.

use std::env;
use std::os::windows::ffi::OsStrExt;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::ShellExecuteW;

/// Checks if the current process is running with Administrator privileges (Elevated).
///
/// Uses the Windows `GetTokenInformation` API to query `TokenElevation`.
///
/// Returns `true` if the process is elevated, `false` otherwise.
pub fn is_running_as_admin() -> bool {
    unsafe {
        let mut token: HANDLE = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }

        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut size = std::mem::size_of::<TOKEN_ELEVATION>() as u32;

        let res = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut core::ffi::c_void),
            size,
            &mut size,
        );

        let _ = windows::Win32::Foundation::CloseHandle(token);

        if res.is_ok() {
            elevation.TokenIsElevated != 0
        } else {
            false
        }
    }
}

/// Restarts the application requesting Administrator privileges.
///
/// Uses `ShellExecuteW` with the "runas" verb to trigger the Windows UAC (User Account Control) prompt.
/// If the user accepts, a new elevated instance of the application is spawned, and the current process exits immediately.
///
/// # Arguments
///
/// * `reopen_ui` - If `true`, passes the `--reopen-ui` flag to the new instance so that the settings UI is automatically reopened.
///
/// # Errors
///
/// Returns an error if `ShellExecuteW` fails or if the user declines the UAC prompt.
pub fn restart_as_admin(reopen_ui: bool) -> anyhow::Result<()> {
    unsafe {
        let exe_path = env::current_exe()?;
        let mut exe_path_u16: Vec<u16> = exe_path.into_os_string().encode_wide().collect();
        exe_path_u16.push(0);

        let mut args_u16: Vec<u16> = if reopen_ui {
            w!("--reopen-ui").as_wide().to_vec()
        } else {
            vec![0]
        };
        args_u16.push(0);

        // Run as admin using the 'runas' verb which triggers the UAC prompt
        let res = ShellExecuteW(
            None,
            w!("runas"),
            PCWSTR(exe_path_u16.as_ptr()),
            if reopen_ui { PCWSTR(args_u16.as_ptr()) } else { PCWSTR::null() },
            PCWSTR::null(),
            windows::Win32::UI::WindowsAndMessaging::SW_SHOW,
        );

        if res.0 as isize > 32 {
            std::process::exit(0);
        } else {
            anyhow::bail!("Failed to restart as administrator. Error code: {}", res.0 as isize);
        }
    }
}
