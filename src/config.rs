use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, VK_OEM_4, VK_OEM_6};

/// Cấu hình chính của ứng dụng
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    /// Chế độ Uncombine (Tránh group cửa sổ lại với nhau trên Taskbar)
    pub uncombine_mode: bool,
    /// Chế độ Cycle dựa trên thứ tự các nút trên Taskbar
    pub cycle_taskbar_based: bool,

    /// Mã phím ảo (VK) của phím nóng di chuyển trái
    pub hotkey_left_vk: u32,
    /// Các phím bổ trợ (Modifiers) của phím nóng di chuyển trái
    pub hotkey_left_modifiers: u32,

    /// Mã phím ảo (VK) của phím nóng di chuyển phải
    pub hotkey_right_vk: u32,
    /// Các phím bổ trợ (Modifiers) của phím nóng di chuyển phải
    pub hotkey_right_modifiers: u32,

    /// Cho phép hiển thị Desktop Indicator
    pub desktop_indicator: bool,

    /// Các phím bổ trợ cho Jump to Desktop (vd: Alt + <number>)
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
    /// Lấy đường dẫn lưu file cấu hình (trong AppData/Roaming/"better-windows-navigate)
    pub fn config_path() -> PathBuf {
        let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push("better-windows-navigate");
        fs::create_dir_all(&path).ok();
        path.push("config.json");
        path
    }

    /// Tải cấu hình từ file, nếu lỗi hoặc file không tồn tại thì trả về cấu hình mặc định
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
                // File chưa tồn tại, tạo mới với default
                let default_config = Self::default();
                default_config.save();
                default_config
            }
        }
    }

    /// Lưu cấu hình hiện tại xuống file
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
