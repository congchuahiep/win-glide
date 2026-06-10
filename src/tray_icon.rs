//! System Tray Icon management via the `Shell_NotifyIconW` API.
//!
//! This module is responsible for displaying and managing the application's icon in the Windows
//! System Tray.
//!
//! Implementation pattern is referenced from [window-switcher](https://github.com/sigoden/window-switcher/blob/main/src/trayicon.rs).

use tracing::debug;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::utils::is_light_theme;

const ICON_LIGHT_BYTES: &[u8] = include_bytes!("../assets/icon-light.ico");
const ICON_DARK_BYTES: &[u8] = include_bytes!("../assets/icon-dark.ico");

pub const IDM_EXIT: u32 = 1;
pub const IDM_SHOW_CONSOLE: u32 = 3;
pub const IDM_SETTINGS: u32 = 4;

const TEXT_SHOW_CONSOLE: PCWSTR = w!("Debug Console");
const TEXT_SETTINGS: PCWSTR = w!("Settings...");
const TEXT_EXIT: PCWSTR = w!("Exit");

/// Manages the lifecycle and behavior of the icon in the Windows system tray.
pub struct TrayIcon {
    /// Data structure containing the configuration information of the system tray icon.
    data: NOTIFYICONDATAW,
}

impl TrayIcon {
    /// Creates a new `TrayIcon` instance not yet associated with any window.
    ///
    /// Loads the icon file from assets and sets the default tooltip.
    pub fn create() -> Self {
        let data = Self::create_nid();
        Self { data }
    }

    /// Updates the icon based on the current system theme.
    pub fn update_theme(&mut self) {
        let new_hicon = Self::get_hicon();
        // Cannot compare HICON directly using == because it does not implement PartialEq.
        // However, we just need to update the new icon and destroy the old one.
        let old_hicon = self.data.hIcon;
        self.data.hIcon = new_hicon;
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &self.data);
            let _ = DestroyIcon(old_hicon);
        }
        debug!("TrayIcon theme updated (light_mode={})", is_light_theme());
    }

    /// Registers the system tray icon with the Windows Shell and associates it with the message receiving window.
    ///
    /// # Errors
    /// Returns an error if the `Shell_NotifyIconW` API fails to add the icon (`NIM_ADD`).
    pub fn register(&mut self, hwnd: HWND) -> anyhow::Result<()> {
        self.data.hWnd = hwnd;
        unsafe {
            Shell_NotifyIconW(NIM_ADD, &self.data)
                .ok()
                .map_err(|e| anyhow::anyhow!("Shell_NotifyIconW(NIM_ADD): {e}"))?;
        }
        debug!("TrayIcon registered, hwnd={:?}", hwnd);
        Ok(())
    }

    /// Checks if the current system tray icon exists.
    ///
    /// Performed by sending a modify command (`NIM_MODIFY`). Returns `true` if successful.
    #[allow(dead_code)]
    pub fn exists(&mut self) -> bool {
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &self.data) }.as_bool()
    }

    /// Displays a Popup context menu at the current mouse cursor position.
    ///
    /// # Important Note (Thread-safety)
    /// This function **must be called from the WndProc thread** (the same STA thread managing the hidden message
    /// window). The reason is that the `TrackPopupMenu` API needs to process `WM_COMMAND` messages
    /// synchronously.
    pub fn show(&self, console_visible: bool) -> anyhow::Result<()> {
        let hwnd = self.data.hWnd;
        let mut cursor = POINT::default();
        unsafe {
            // Bring the hidden window to the foreground so that when clicking outside the menu, the menu automatically closes
            // (required by Win32 documentation).
            let _ = SetForegroundWindow(hwnd);
            GetCursorPos(&mut cursor)?;
            let hmenu = Self::create_menu(console_visible)?;
            let _ = TrackPopupMenu(
                hmenu,
                TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON,
                cursor.x,
                cursor.y,
                None,
                hwnd,
                None,
            );
            DestroyMenu(hmenu)?;
        }
        Ok(())
    }

    /// Reregisters the icon with the system tray.
    ///
    /// Usually called after receiving the `WM_TASKBARCREATED` message notifying that Windows
    /// Explorer (explorer.exe) has just restarted and the system tray was cleared beforehand.
    pub fn reregister(&mut self) -> anyhow::Result<()> {
        unsafe {
            Shell_NotifyIconW(NIM_ADD, &self.data)
                .ok()
                .map_err(|e| anyhow::anyhow!("Shell_NotifyIconW(NIM_ADD) reregister: {e}"))?;
        }
        debug!("TrayIcon re-registered after TaskbarCreated");
        Ok(())
    }

    fn get_hicon() -> HICON {
        let bytes = if is_light_theme() {
            ICON_LIGHT_BYTES
        } else {
            ICON_DARK_BYTES
        };
        let offset =
            unsafe { LookupIconIdFromDirectoryEx(bytes.as_ptr(), true, 0, 0, LR_DEFAULTCOLOR) };
        let icon_data = &bytes[offset as usize..];
        unsafe {
            CreateIconFromResourceEx(icon_data, true, 0x00030000, 0, 0, LR_DEFAULTCOLOR)
                .expect("Failed to load embedded icon")
        }
    }

    /// Creates and sets the default `NOTIFYICONDATAW` configuration information for the system tray icon.
    fn create_nid() -> NOTIFYICONDATAW {
        let hicon = Self::get_hicon();

        let mut tooltip: Vec<u16> = "Taskbar Switcher".encode_utf16().collect();
        tooltip.resize(128, 0);
        let tooltip: [u16; 128] = tooltip.try_into().expect("tooltip too long");

        NOTIFYICONDATAW {
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_USER + 0x200, // WM_USER_TRAYICON
            hIcon: hicon,
            szTip: tooltip,
            ..Default::default()
        }
    }

    /// Creates a Popup menu containing application configuration options.
    ///
    /// The menu includes:
    /// - "Debug Console" option
    /// - "Settings..." option
    /// - Separator
    /// - "Exit" option to close the application
    fn create_menu(console_visible: bool) -> anyhow::Result<HMENU> {
        unsafe {
            let hmenu = CreatePopupMenu()?;

            let console_flags = MF_STRING
                | if crate::logging::console::DEBUG_CLI_MODE
                    .load(std::sync::atomic::Ordering::SeqCst)
                {
                    MF_GRAYED | MF_DISABLED
                } else if console_visible {
                    MF_CHECKED
                } else {
                    MF_UNCHECKED
                };
            AppendMenuW(
                hmenu,
                console_flags,
                IDM_SHOW_CONSOLE as usize,
                TEXT_SHOW_CONSOLE,
            )?;

            AppendMenuW(hmenu, MF_STRING, IDM_SETTINGS as usize, TEXT_SETTINGS)?;
            AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null())?;
            AppendMenuW(hmenu, MF_STRING, IDM_EXIT as usize, TEXT_EXIT)?;

            Ok(hmenu)
        }
    }
}

/// When the `TrayIcon` instance is destroyed (Out of scope / Drop), automatically remove the icon from the system tray.
impl Drop for TrayIcon {
    fn drop(&mut self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.data);
        }
        debug!("TrayIcon dropped, icon removed");
    }
}
