//! Global hotkeys management using the Win32 API.
//!
//! This module is responsible for registering, unregistering, and mapping global hotkeys.
//!
//! By default, the application registers two hotkeys:
//! - **Alt + [**: Move focus to the left Taskbar button ([`HotkeyAction::Left`])
//! - **Alt + ]**: Move focus to the right Taskbar button ([`HotkeyAction::Right`])

use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS,
};

/// Actions that can be triggered by global hotkeys.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HotkeyAction {
    /// Cycle focus to the left window on the Taskbar.
    CycleLeft,
    /// Cycle focus to the right window on the Taskbar.
    CycleRight,
    /// Switch to the Virtual Desktop with the specified index (0-based).
    SwitchVirtualDesktop(u32),
}

/// Stores the details and state of a specific hotkey.
struct Hotkey {
    /// Unique identifier (ID) of the hotkey within the application scope.
    id: i32,
    /// The action to be executed when this hotkey is pressed.
    action: HotkeyAction,
    /// Accompanying modifier keys (such as Alt, Ctrl, Shift).
    modifiers: HOT_KEY_MODIFIERS,
    /// Virtual Key Code of the main key.
    vk: u32,
}

impl Hotkey {
    /// Registers this hotkey with the Windows system.
    ///
    /// # Errors
    /// Returns an error if the hotkey is already in use by another application.
    fn register(&self) -> windows::core::Result<()> {
        unsafe { RegisterHotKey(None, self.id, self.modifiers, self.vk) }
    }

    /// Unregisters this hotkey from the Windows system.
    fn unregister(&self) {
        unsafe {
            let _ = UnregisterHotKey(None, self.id);
        }
    }
}

/// Manager for the application's global hotkeys.
pub struct HotkeyManager {
    /// List of managed hotkey instances.
    hotkeys: Vec<Hotkey>,
}

impl HotkeyManager {
    /// Initializes the manager and registers the default hotkeys with the system.
    ///
    /// Defaults:
    /// - ID 1: `Alt+[` -> Cycle left ([`HotkeyAction::Left`]).
    /// - ID 2: `Alt+]` -> Cycle right ([`HotkeyAction::Right`]).
    /// - ID 11-19: `Alt+1` -> `Alt+9` -> Switch to respective VD ([`HotkeyAction::SwitchVirtualDesktop`]).
    ///
    /// TODO: Allow users to customize hotkeys.
    ///
    /// # Errors
    /// Returns an error if it fails to register one or more hotkeys (usually due to a conflict with
    /// another software).
    pub fn new(config: &crate::config::AppConfig) -> anyhow::Result<Self> {
        let mut hotkeys = vec![];

        if config.cycle_taskbar_based {
            hotkeys.push(Hotkey {
                id: 1,
                action: HotkeyAction::CycleLeft,
                modifiers: HOT_KEY_MODIFIERS(config.hotkey_left_modifiers),
                vk: config.hotkey_left_vk,
            });
            hotkeys.push(Hotkey {
                id: 2,
                action: HotkeyAction::CycleRight,
                modifiers: HOT_KEY_MODIFIERS(config.hotkey_right_modifiers),
                vk: config.hotkey_right_vk,
            });
        }

        // Register Switch Desktop hotkeys if at least 1 modifier key is set
        if config.jump_desktop_modifiers != 0 {
            for i in 1..=9 {
                hotkeys.push(Hotkey {
                    id: 10 + i as i32,
                    action: HotkeyAction::SwitchVirtualDesktop(i as u32 - 1),
                    modifiers: HOT_KEY_MODIFIERS(config.jump_desktop_modifiers),
                    vk: 0x30 + i as u32,
                });
            }
        }

        let this = Self { hotkeys };

        let mut errs = Vec::new();
        for hotkey in &this.hotkeys {
            if let Err(e) = hotkey.register() {
                errs.push(e);
            }
        }

        if !errs.is_empty() {
            anyhow::bail!("Failed to register hotkeys: {:?}", errs);
        }

        Ok(this)
    }

    /// Unregisters all hotkeys established with Windows.
    ///
    /// This method is called automatically when the `HotkeyManager` object is dropped ([`Drop`]).
    pub fn unregister_all(&self) {
        for hotkey in &self.hotkeys {
            hotkey.unregister();
        }
    }

    /// Looks up the action corresponding to the hotkey ID received from the system message.
    pub fn action_from_id(&self, id: i32) -> Option<HotkeyAction> {
        self.hotkeys.iter().find(|h| h.id == id).map(|h| h.action)
    }

    /// Reloads the hotkey configuration: unregisters old ones, loads new ones, and registers them again.
    pub fn reload(&mut self, config: &crate::config::AppConfig) -> anyhow::Result<()> {
        for hotkey in &self.hotkeys {
            hotkey.unregister();
        }

        self.hotkeys.clear();

        if config.cycle_taskbar_based {
            self.hotkeys.push(Hotkey {
                id: 1,
                action: HotkeyAction::CycleLeft,
                modifiers: HOT_KEY_MODIFIERS(config.hotkey_left_modifiers),
                vk: config.hotkey_left_vk,
            });

            self.hotkeys.push(Hotkey {
                id: 2,
                action: HotkeyAction::CycleRight,
                modifiers: HOT_KEY_MODIFIERS(config.hotkey_right_modifiers),
                vk: config.hotkey_right_vk,
            });
        }

        if config.jump_desktop_modifiers != 0 {
            for i in 1..=9 {
                self.hotkeys.push(Hotkey {
                    id: 10 + i as i32,
                    action: HotkeyAction::SwitchVirtualDesktop(i as u32 - 1),
                    modifiers: HOT_KEY_MODIFIERS(config.jump_desktop_modifiers),
                    vk: 0x30 + i as u32,
                });
            }
        }

        let mut errs = Vec::new();
        for hotkey in &self.hotkeys {
            if let Err(e) = hotkey.register() {
                errs.push(format!("ID {}: {}", hotkey.id, e));
            }
        }

        if !errs.is_empty() {
            tracing::warn!("Failed to reload some hotkeys: {:?}", errs);
        }

        Ok(())
    }
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        self.unregister_all();
    }
}
