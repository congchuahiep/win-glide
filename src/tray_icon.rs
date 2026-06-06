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

const ICON_BYTES: &[u8] = include_bytes!("../assets/icon.ico");

pub const IDM_EXIT: u32 = 1;
pub const IDM_COMBINE_MODE: u32 = 2;

const TEXT_COMBINE_MODE: PCWSTR = w!("Combine Mode");
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
    pub fn show(&self, combine_enabled: bool) -> anyhow::Result<()> {
        let hwnd = self.data.hWnd;
        let mut cursor = POINT::default();
        unsafe {
            // Đưa cửa sổ ẩn lên foreground để khi click ra ngoài menu, menu sẽ tự động đóng lại (bắt buộc theo tài liệu Win32).
            let _ = SetForegroundWindow(hwnd);
            GetCursorPos(&mut cursor)?;
            let hmenu = Self::create_menu(combine_enabled)?;
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

    /// Tạo và thiết lập thông tin cấu hình mặc định `NOTIFYICONDATAW` cho biểu tượng khay hệ thống
    ///
    /// Hàm thực hiện parse tệp biểu tượng từ mảng byte tĩnh được nhúng (`ICON_BYTES`) thông qua các
    /// hàm `LookupIconIdFromDirectoryEx` và `CreateIconFromResourceEx`
    fn create_nid() -> NOTIFYICONDATAW {
        // Tìm offset của biểu tượng phù hợp nhất từ bộ đệm tài nguyên
        let offset = unsafe {
            LookupIconIdFromDirectoryEx(ICON_BYTES.as_ptr(), true, 0, 0, LR_DEFAULTCOLOR)
        };
        let icon_data = &ICON_BYTES[offset as usize..];
        // Tạo HICON từ dữ liệu thô
        let hicon = unsafe {
            CreateIconFromResourceEx(icon_data, true, 0x00030000, 0, 0, LR_DEFAULTCOLOR)
                .expect("Failed to load embedded icon")
        };

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
    /// - Tùy chọn "Combine Mode" (sử dụng checkbox tích chọn tùy vào giá trị của `combine_enabled`)
    /// - Đường phân cách (Separator)
    /// - Tùy chọn "Exit" để đóng ứng dụng
    fn create_menu(combine_enabled: bool) -> anyhow::Result<HMENU> {
        unsafe {
            let hmenu = CreatePopupMenu()?;
            let combine_flags = MF_STRING
                | if combine_enabled {
                    MF_CHECKED
                } else {
                    MF_UNCHECKED
                };

            AppendMenuW(
                hmenu,
                combine_flags,
                IDM_COMBINE_MODE as usize,
                TEXT_COMBINE_MODE,
            )?;
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
