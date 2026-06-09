//! Quản lý phím nóng toàn cục (Global Hotkeys) bằng API Win32.
//!
//! Module chịu trách nhiệm đăng ký, hủy đăng ký và ánh xạ các phím nóng toàn cục.
//!
//! Mặc định, ứng dụng đăng ký hai phím nóng:
//! - **Alt + [**: Di chuyển tiêu điểm sang nút Taskbar bên trái ([`HotkeyAction::Left`])
//! - **Alt + ]**: Di chuyển tiêu điểm sang nút Taskbar bên phải ([`HotkeyAction::Right`])

use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT,
};

/// Các hành động có thể kích hoạt bởi phím nóng toàn cục
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HotkeyAction {
    /// Di chuyển tiêu điểm sang cửa sổ bên trái trên Taskbar.
    CycleLeft,
    /// Di chuyển tiêu điểm sang cửa sổ bên phải trên Taskbar.
    CycleRight,
    /// Chuyển đổi sang Virtual Desktop với index (0-based) chỉ định.
    SwitchVirtualDesktop(u32),
}

/// Lưu trữ thông tin chi tiết và trạng thái của một phím nóng cụ thể.
struct Hotkey {
    /// Mã định danh duy nhất (ID) của phím nóng trong phạm vi ứng dụng.
    id: i32,
    /// Hành động sẽ được thực thi khi phím nóng này được nhấn.
    action: HotkeyAction,
    /// Các phím bổ trợ đi kèm (như phím Alt, Ctrl, Shift).
    modifiers: HOT_KEY_MODIFIERS,
    /// Mã phím ảo (Virtual Key Code) của phím chính.
    vk: u32,
}

impl Hotkey {
    /// Đăng ký phím nóng này với hệ thống Windows.
    ///
    /// # Lỗi (Errors)
    /// Trả về lỗi nếu phím nóng đã bị chiếm dụng bởi ứng dụng khác.
    fn register(&self) -> windows::core::Result<()> {
        unsafe { RegisterHotKey(None, self.id, self.modifiers, self.vk) }
    }

    /// Hủy đăng ký phím nóng này khỏi hệ thống Windows.
    fn unregister(&self) {
        unsafe {
            let _ = UnregisterHotKey(None, self.id);
        }
    }
}

/// Trình quản lý danh sách các phím nóng toàn cục của ứng dụng.
pub struct HotkeyManager {
    /// Danh sách các thực thể phím nóng đang được quản lý.
    hotkeys: Vec<Hotkey>,
}

impl HotkeyManager {
    /// Khởi tạo trình quản lý và đăng ký các phím nóng mặc định với hệ thống.
    ///
    /// Mặc định:
    /// - ID 1: `Alt+[` -> Di chuyển trái ([`HotkeyAction::Left`]).
    /// - ID 2: `Alt+]` -> Di chuyển phải ([`HotkeyAction::Right`]).
    /// - ID 11-19: `Alt+1` -> `Alt+9` -> Chuyển VD tương ứng ([`HotkeyAction::SwitchVirtualDesktop`]).
    ///
    /// TODO: Cho phép người dùng tự điều chỉnh được phím tắt
    ///
    /// # Errors
    /// Trả về lỗi nếu không thể đăng ký một hoặc nhiều phím nóng (thường do xung đột phím nóng với
    /// phần mềm khác).
    pub fn new(config: &crate::config::AppConfig) -> anyhow::Result<Self> {
        let mut hotkeys = vec![
            Hotkey {
                id: 1,
                action: HotkeyAction::CycleLeft,
                modifiers: HOT_KEY_MODIFIERS(config.hotkey_left_modifiers),
                vk: config.hotkey_left_vk,
            },
            Hotkey {
                id: 2,
                action: HotkeyAction::CycleRight,
                modifiers: HOT_KEY_MODIFIERS(config.hotkey_right_modifiers),
                vk: config.hotkey_right_vk,
            },
        ];

        // Đăng ký Alt + 1 đến Alt + 9 (Virtual Key 0x31 - 0x39)
        for i in 1..=9 {
            hotkeys.push(Hotkey {
                id: 10 + i as i32,
                action: HotkeyAction::SwitchVirtualDesktop(i as u32 - 1),
                modifiers: MOD_ALT,
                vk: 0x30 + i as u32,
            });
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

    /// Hủy đăng ký toàn bộ các phím nóng đã được thiết lập với Windows.
    ///
    /// Phương thức này được gọi tự động khi đối tượng `HotkeyManager` bị hủy ([`Drop`])
    pub fn unregister_all(&self) {
        for hotkey in &self.hotkeys {
            hotkey.unregister();
        }
    }

    /// Tìm kiếm hành động tương ứng với ID phím nóng nhận được từ tin nhắn hệ thống
    pub fn action_from_id(&self, id: i32) -> Option<HotkeyAction> {
        self.hotkeys.iter().find(|h| h.id == id).map(|h| h.action)
    }

    /// Tải lại cấu hình phím tắt: gỡ phím tắt cũ, nạp mới và đăng ký lại
    pub fn reload(&mut self, config: &crate::config::AppConfig) -> anyhow::Result<()> {
        for hotkey in &self.hotkeys {
            hotkey.unregister();
        }

        self.hotkeys.clear();

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

        for i in 1..=9 {
            self.hotkeys.push(Hotkey {
                id: 10 + i as i32,
                action: HotkeyAction::SwitchVirtualDesktop(i as u32 - 1),
                modifiers: MOD_ALT,
                vk: 0x30 + i as u32,
            });
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
