//! Quản lý khay hệ thống (System Tray Icon) thông qua API `Shell_NotifyIconW`.
//!
//! Module này chịu trách nhiệm hiển thị và quản lý biểu tượng của ứng dụng dưới khay hệ thống
//! Windows (System Tray)
//!
//! Pattern triển khai được tham khảo từ [window-switcher](https://github.com/sigoden/window-switcher/blob/main/src/trayicon.rs).

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

/// Quản lý vòng đời và hành vi của biểu tượng trên khay hệ thống Windows.
pub struct TrayIcon {
    /// Cấu trúc dữ liệu chứa thông tin cấu hình của biểu tượng khay hệ thống.
    data: NOTIFYICONDATAW,
}

impl TrayIcon {
    /// Tạo mới một thực thể `TrayIcon` chưa liên kết với cửa sổ nào
    ///
    /// Nạp tệp biểu tượng từ assets và thiết lập tooltip mặc định
    pub fn create() -> Self {
        let data = Self::create_nid();
        Self { data }
    }

    /// Cập nhật icon dựa trên giao diện hệ thống hiện tại
    pub fn update_theme(&mut self) {
        let new_hicon = Self::get_hicon();
        // Không thể so sánh HICON trực tiếp bằng == vì nó không implement PartialEq
        // Tuy nhiên, chúng ta chỉ cần update lại icon mới và hủy icon cũ
        let old_hicon = self.data.hIcon;
        self.data.hIcon = new_hicon;
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &self.data);
            let _ = DestroyIcon(old_hicon);
        }
        debug!("TrayIcon theme updated (light_mode={})", is_light_theme());
    }

    /// Đăng ký biểu tượng khay hệ thống với Windows Shell và liên kết với cửa sổ nhận tin nhắn
    ///
    /// # Errors
    /// Trả về lỗi nếu hàm API `Shell_NotifyIconW` thất bại trong việc thêm biểu tượng (`NIM_ADD`)
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

    /// Kiểm tra xem biểu tượng khay hệ thống hiện tại có đang tồn tại hay không
    ///
    /// Thực hiện bằng cách gửi lệnh chỉnh sửa (`NIM_MODIFY`). Trả về `true` nếu thành công
    #[allow(dead_code)]
    pub fn exists(&mut self) -> bool {
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &self.data) }.as_bool()
    }

    /// Hiển thị menu ngữ cảnh dạng Popup tại vị trí con trỏ chuột hiện tại.
    ///
    /// # Chú ý quan trọng (Thread-safety)
    /// Hàm này **bắt buộc phải được gọi từ luồng WndProc** (cùng luồng STA quản lý cửa sổ ẩn nhận
    /// tin nhắn). Nguyên nhân là do API `TrackPopupMenu` cần xử lý các tin nhắn `WM_COMMAND` một
    /// cách đồng bộ.
    pub fn show(&self, console_visible: bool) -> anyhow::Result<()> {
        let hwnd = self.data.hWnd;
        let mut cursor = POINT::default();
        unsafe {
            // Đưa cửa sổ ẩn lên foreground để khi click ra ngoài menu, menu sẽ tự động đóng lại
            // (bắt buộc theo tài liệu Win32).
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

    /// Đăng ký lại biểu tượng với khay hệ thống.
    ///
    /// Thường được gọi sau khi nhận được thông báo `WM_TASKBARCREATED` thông báo rằng Windows
    /// Explorer (explorer.exe) vừa được khởi động lại và khay hệ thống đã bị xóa sạch trước đó.
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

    /// Tạo và thiết lập thông tin cấu hình mặc định `NOTIFYICONDATAW` cho biểu tượng khay hệ thống
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

    /// Tạo một menu Popup chứa các tùy chọn cấu hình của ứng dụng.
    ///
    /// Menu bao gồm:
    /// - Tùy chọn "Debug Console"
    /// - Tùy chọn "Settings..."
    /// - Đường phân cách (Separator)
    /// - Tùy chọn "Exit" để đóng ứng dụng
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

/// Khi thực thể `TrayIcon` bị hủy (Out of scope / Drop), tự động xóa biểu tượng khỏi khay hệ thống
impl Drop for TrayIcon {
    fn drop(&mut self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.data);
        }
        debug!("TrayIcon dropped, icon removed");
    }
}
