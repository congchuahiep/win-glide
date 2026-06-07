//! Cửa sổ hiển thị trạng thái (Indicator) trên Taskbar.

use std::slice::from_raw_parts_mut;

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::utils;

const WM_APP_VD_EVENT: u32 = WM_USER + 0x100;

/// Cửa sổ hiển thị trạng thái (Indicator) trên Taskbar.
/// Nó sử dụng cờ `WS_EX_LAYERED` kết hợp với `UpdateLayeredWindow` để vẽ đồ họa 32-bit có kênh Alpha (trong suốt).
pub struct IndicatorWindow {
    pub hwnd: HWND,
    _desktop_event_thread: Option<winvd::DesktopEventThread>,
}

impl IndicatorWindow {
    /// Khởi tạo cửa sổ Indicator mới.
    ///
    /// **WARN: Vấn đề Task View (Win+Tab)**
    /// Hiện tại cửa sổ này được đặt làm Owned Window của `Shell_TrayWnd` (Taskbar) để nó luôn nằm
    /// trên Taskbar. Tuy nhiên, trên Windows 11, khi mở Task View (Win+Tab), hệ thống DWM sẽ tự
    /// động dùng kỹ thuật "Cloaking" để ẩn tất cả các cửa sổ Owned của Taskbar. Kết quả là
    /// Indicator sẽ biến mất trong lúc Task View đang mở, và thường chỉ hiện lại khi Taskbar nhận
    /// được focus. Đây là hạn chế kỹ thuật hiện tại chưa có cách khắc phục triệt để
    pub unsafe fn new() -> anyhow::Result<Self> {
        let hinstance = GetModuleHandleW(None)?;
        let class_name = w!("TaskbarSwitcherIndicator");

        let wnd_class = WNDCLASSW {
            hInstance: HINSTANCE(hinstance.0),
            lpszClassName: class_name,
            lpfnWndProc: Some(Self::window_proc),
            ..Default::default()
        };

        let _ = RegisterClassW(&wnd_class);

        let taskbar_hwnd = FindWindowW(w!("Shell_TrayWnd"), None)?;
        let mut tray_rect = RECT::default();
        let _ = GetWindowRect(taskbar_hwnd, &mut tray_rect);
        let taskbar_height = tray_rect.bottom - tray_rect.top;

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE,
            class_name,
            w!("Indicator"),
            WS_POPUP | WS_VISIBLE,
            tray_rect.left + 10,
            tray_rect.top,
            128,
            taskbar_height,
            Some(taskbar_hwnd),
            None,
            Some(hinstance.into()),
            None,
        )?;

        let this = Self {
            hwnd,
            _desktop_event_thread: None,
        };

        // Initial render
        Self::render(hwnd);

        Ok(this)
    }

    /// Khởi chạy luồng theo dõi sự kiện đổi Desktop (Virtual Desktop).
    /// Khi phát hiện người dùng chuyển Desktop, nó sẽ gửi tin nhắn `WM_APP_VD_EVENT` về luồng UI chính
    /// để yêu cầu vẽ lại (render) các chấm Indicator, hiển thị đúng Desktop hiện tại.
    pub fn run(&mut self) {
        let (tx, rx) = std::sync::mpsc::channel::<winvd::DesktopEvent>();
        let hwnd_ind_ptr = self.hwnd.0 as isize;

        match winvd::listen_desktop_events(tx) {
            Ok(thread) => {
                std::thread::spawn(move || {
                    while let Ok(_event) = rx.recv() {
                        unsafe {
                            let hwnd_ind = windows::Win32::Foundation::HWND(hwnd_ind_ptr as *mut _);
                            let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                                Some(hwnd_ind),
                                WM_APP_VD_EVENT,
                                windows::Win32::Foundation::WPARAM(0),
                                windows::Win32::Foundation::LPARAM(0),
                            );
                        }
                    }
                });

                self._desktop_event_thread = Some(thread);
            }
            Err(e) => tracing::error!("Failed to start winvd desktop event listener: {:?}", e),
        }
    }

    /// Hàm vẽ (render) nội dung Indicator lên vùng nhớ đệm, sau đó đẩy trực tiếp lên màn hình.
    ///
    /// Thay vì dùng GDI tiêu chuẩn (gây lỗi viền đen răng cưa khi kết hợp với `LWA_COLORKEY`),
    /// hàm này khởi tạo một bitmap 32-bit ARGB (DIBSection), vẽ các chấm tròn bằng thuật toán SDF
    /// (Signed Distance Field) để có hiệu ứng khử răng cưa mượt mà, sau đó dùng `UpdateLayeredWindow`
    /// để áp dụng toàn bộ kênh Alpha lên Desktop
    ///
    /// Thú thật thì tôi không biết hàm này hoạt động như thế nào nữa :P
    pub fn render(hwnd: HWND) {
        unsafe {
            let mut tray_rect = RECT::default();
            if let Ok(taskbar_hwnd) = FindWindowW(w!("Shell_TrayWnd"), None) {
                let _ = GetWindowRect(taskbar_hwnd, &mut tray_rect);
            }
            let width = 128;
            let height = tray_rect.bottom - tray_rect.top;

            if height <= 0 {
                return;
            }

            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height, // top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0 as u32,
                    ..Default::default()
                },
                ..Default::default()
            };

            let mut ppvbits: *mut std::ffi::c_void = std::ptr::null_mut();
            let screen_dc = GetDC(None);
            let mem_dc = CreateCompatibleDC(Some(screen_dc));
            let hbitmap =
                CreateDIBSection(Some(mem_dc), &bmi, DIB_RGB_COLORS, &mut ppvbits, None, 0);

            if let Ok(bmp) = hbitmap {
                let old_bmp = SelectObject(mem_dc, HGDIOBJ(bmp.0 as _));

                // Trỏ mảng rust vào bộ đệm của hình ảnh
                // Điền nền màu đen nhưng có độ trong suốt 0 (Alpha = 0) => Tàng hình 100%
                let buffer = from_raw_parts_mut(ppvbits as *mut u32, (width * height) as usize);
                buffer.fill(0);

                let light_mode = utils::is_light_theme();
                let theme_color = match light_mode {
                    true => (20, 20, 20),
                    false => (255, 255, 255),
                };

                // Taskbar ở 1080p thường cao 48px, ở 4K (200%) cao 96px.
                // Việc bám theo chiều cao Taskbar giúp tỷ lệ luôn chuẩn xác 100% trên mọi màn hình.
                let radius = height as f32 * 0.08; // Bán kính = 8% chiều cao (tương đương ~3.8px ở 1080p)
                let spacing = radius * 4.5;
                let start_x = 10.0 + radius;
                let cy = height as f32 / 2.0;

                let count = winvd::get_desktop_count().unwrap_or(1) as usize;
                let current = winvd::get_current_desktop().ok();
                let desktops = winvd::get_desktops().unwrap_or_default();
                let mut current_idx = 0;
                if let Some(c) = current {
                    for (i, d) in desktops.iter().enumerate() {
                        if *d == c {
                            current_idx = i;
                            break;
                        }
                    }
                }

                for i in 0..count {
                    let cx = start_x + (i as f32) * spacing;
                    let base_alpha = match i == current_idx {
                        true => 1.0,
                        false => 0.5,
                    };

                    Self::draw_aa_circle(
                        buffer,
                        width,
                        height,
                        cx,
                        cy,
                        radius,
                        theme_color,
                        base_alpha,
                    );
                }

                // Cập nhật lên màn hình
                let mut pt_src = POINT { x: 0, y: 0 };
                let mut size = SIZE {
                    cx: width,
                    cy: height,
                };
                let mut pt_dst = POINT {
                    x: tray_rect.left + 10,
                    y: tray_rect.top,
                };
                let mut blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };

                let _ = UpdateLayeredWindow(
                    hwnd,
                    None,
                    Some(&mut pt_dst as *mut _),
                    Some(&mut size as *mut _),
                    Some(mem_dc),
                    Some(&mut pt_src as *mut _),
                    COLORREF(0),
                    Some(&mut blend as *mut _),
                    ULW_ALPHA,
                );

                SelectObject(mem_dc, old_bmp);
                let _ = DeleteObject(HGDIOBJ(bmp.0 as _));
            }

            let _ = DeleteDC(mem_dc);
            ReleaseDC(None, screen_dc);
        }
    }

    /// Vẽ một hình tròn SDF có khử răng cưa trực tiếp lên buffer ARGB 32-bit.
    fn draw_aa_circle(
        buffer: &mut [u32],
        width: i32,
        height: i32,
        cx: f32,
        cy: f32,
        radius: f32,
        color: (u8, u8, u8),
        base_alpha: f32,
    ) {
        let (r, g, b) = color;

        let min_x = (cx - radius - 1.0).floor().max(0.0) as i32;
        let max_x = (cx + radius + 1.0).ceil().min((width - 1) as f32) as i32;
        let min_y = (cy - radius - 1.0).floor().max(0.0) as i32;
        let max_y = (cy + radius + 1.0).ceil().min((height - 1) as f32) as i32;

        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let dx = px - cx;
                let dy = py - cy;
                let distance = (dx * dx + dy * dy).sqrt();

                // Khử răng cưa (SDF)
                let mut alpha = if distance <= radius - 0.5 {
                    1.0
                } else if distance >= radius + 0.5 {
                    0.0
                } else {
                    0.5 - (distance - radius)
                };

                alpha *= base_alpha;

                if alpha > 0.0 {
                    let a = (alpha * 255.0) as u32;
                    let pr = (r as f32 * alpha) as u32;
                    let pg = (g as f32 * alpha) as u32;
                    let pb = (b as f32 * alpha) as u32;

                    let new_pixel = (a << 24) | (pr << 16) | (pg << 8) | pb;
                    let idx = (y * width + x) as usize;

                    if idx < buffer.len() {
                        buffer[idx] = new_pixel;
                    }
                }
            }
        }
    }

    /// Hàm xử lý tin nhắn (Message Procedure) cốt lõi của cửa sổ Indicator.
    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_APP_VD_EVENT => {
                Self::render(hwnd);
                LRESULT(0)
            }
            WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
                // Thoát khỏi "Input Sync Call" trước khi gọi render (COM), việc này giúp khi
                // rerender lại được lấy state mới nhất
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                        Some(hwnd),
                        WM_APP_VD_EVENT,
                        windows::Win32::Foundation::WPARAM(0),
                        windows::Win32::Foundation::LPARAM(0),
                    );
                }
                LRESULT(0)
            }
            WM_NCHITTEST => LRESULT(-1),
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

impl Drop for IndicatorWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
