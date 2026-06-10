//! Handles the reading and writing of user preferences.
//!
//! This module defines the `AppConfig` struct and provides serialization/deserialization
//! capabilities to persist the application's settings to a JSON file in the user's AppData directory.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, VK_OEM_4, VK_OEM_6};

/// Main configuration struct for the application.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    /// prevents the taskbar buttons groups from being combined on the taskbar
    pub uncombine_mode: bool,
    /// Cycle mode based on the order of taskbar buttons
    pub cycle_taskbar_based: bool,

    /// Virtual key code (VK) of the hotkey to move left
    pub hotkey_left_vk: u32,
    /// Modifiers for the hotkey to move left
    pub hotkey_left_modifiers: u32,

    /// Virtual key code (VK) of the hotkey to move right
    pub hotkey_right_vk: u32,
    /// Modifiers for the hotkey to move right
    pub hotkey_right_modifiers: u32,

    /// Allows displaying the Desktop Indicator
    pub desktop_indicator: bool,

    /// Modifiers for the Jump to Desktop hotkey (e.g. Alt + <number>)
    pub jump_desktop_modifiers: u32,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            uncombine_mode: true,
            cycle_taskbar_based: true,
            hotkey_left_vk: VK_OEM_4.0 as u32,
            hotkey_left_modifiers: MOD_ALT.0 as u32,
            hotkey_right_vk: VK_OEM_6.0 as u32,
            hotkey_right_modifiers: MOD_ALT.0 as u32,
            desktop_indicator: true,
            jump_desktop_modifiers: MOD_ALT.0 as u32,
        }
    }
}

impl AppConfig {
    /// Get the path to the configuration file (in AppData/Roaming/"WinGlide")
    pub fn config_path() -> PathBuf {
        let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push("WinGlide");
        fs::create_dir_all(&path).ok();
        path.push("config.json");
        path
    }

    /// Load the configuration from the file, or return the default configuration if the file is not found or an error occurs.
    pub fn load() -> Self {
        let path = Self::config_path();
        match fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<Self>(&content) {
                Ok(mut config) => {
                    // Force uncombine_mode to true if cycle_taskbar_based is true
                    if config.cycle_taskbar_based {
                        config.uncombine_mode = true;
                    }
                    config
                }
                Err(e) => {
                    tracing::error!("Failed to parse config.json: {}. Using default.", e);
                    Self::default()
                }
            },
            Err(_) => {
                let default_config = Self::default();
                default_config.save();
                default_config
            }
        }
    }

    /// Save the current configuration to the file.
    pub fn save(&self) {
        let path = Self::config_path();
        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = fs::write(&path, json) {
                    tracing::error!("Failed to write config.json: {}", e);
                } else {
                    tracing::info!("Config saved to {:?}", path);
                }
            }
            Err(e) => {
                tracing::error!("Failed to serialize config: {}", e);
            }
        }
    }
}
