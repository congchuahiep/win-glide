//! Manages the application's auto-start behavior on Windows.
//!
//! This module interacts with the Windows Registry (`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`)
//! to check, enable, or disable the application from launching automatically at user login.

use std::env;
use std::os::windows::ffi::OsStrExt;
use windows::core::{w, PCWSTR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_ALL_ACCESS, KEY_READ, REG_SAM_FLAGS, REG_SZ, REG_VALUE_TYPE,
    RRF_RT_REG_SZ,
};

const APP_NAME: PCWSTR = w!("WinGlide");
const REG_RUN_PATH: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");

/// Checks if the application is currently configured to start automatically on login.
///
/// Returns `true` if the registry key exists and contains a value for this application,
/// otherwise returns `false`.
pub fn is_autostart_enabled() -> bool {
    unsafe {
        let mut hkey = HKEY::default();
        let status = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            REG_RUN_PATH,
            Some(0),
            REG_SAM_FLAGS(KEY_READ.0),
            &mut hkey,
        );

        if status.is_err() {
            return false;
        }

        let mut data_type = REG_VALUE_TYPE(0);
        let mut data_size: u32 = 0;

        let res = RegGetValueW(
            hkey,
            PCWSTR::null(),
            APP_NAME,
            RRF_RT_REG_SZ,
            Some(&mut data_type as *mut REG_VALUE_TYPE),
            None,
            Some(&mut data_size as *mut u32),
        );

        let _ = RegCloseKey(hkey);
        res.is_ok()
    }
}

/// Enables or disables the auto-start functionality.
///
/// # Arguments
///
/// * `enabled` - If `true`, adds the application executable path to the registry run key.
///   If `false`, removes the application from the registry run key.
///
/// # Errors
///
/// Returns an error if the registry key cannot be opened or modified.
pub fn set_autostart(enabled: bool) -> anyhow::Result<()> {
    unsafe {
        let mut hkey = HKEY::default();
        let status = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            REG_RUN_PATH,
            Some(0),
            REG_SAM_FLAGS(KEY_ALL_ACCESS.0),
            &mut hkey,
        );
        status.ok()?;

        if enabled {
            let exe_path = env::current_exe()?;
            let exe_path_u16: Vec<u16> = exe_path.into_os_string().encode_wide().collect();
            // Wrap the path in quotes in case there are spaces
            let mut val_u16 = vec![b'"' as u16];
            val_u16.extend(exe_path_u16);
            val_u16.push(b'"' as u16);
            val_u16.push(0);

            let size = (val_u16.len() * 2) as u32;

            RegSetValueExW(
                hkey,
                APP_NAME,
                Some(0),
                REG_SZ,
                Some(std::slice::from_raw_parts(
                    val_u16.as_ptr() as *const u8,
                    size as usize,
                )),
            )
            .ok()?;
        } else {
            let _ = RegDeleteValueW(hkey, APP_NAME);
        }

        let _ = RegCloseKey(hkey);
        Ok(())
    }
}
