//! Indicator window displaying status on the Taskbar.

use std::slice::from_raw_parts_mut;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse;
use windows::Win32::UI::WindowsAndMessaging::{self, *};

use crate::utils;

const WM_APP_VD_EVENT: u32 = WM_USER + 0x100;

static HOVER_INDEX: AtomicIsize = AtomicIsize::new(-1);

fn get_hovered_index(x: i32) -> Option<usize> {
    unsafe {
        let mut tray_rect = RECT::default();
        if let Ok(taskbar_hwnd) = FindWindowW(w!("Shell_TrayWnd"), None) {
            let _ = GetWindowRect(taskbar_hwnd, &mut tray_rect);
        }
        let height = tray_rect.bottom - tray_rect.top;
        if height <= 0 {
            return None;
        }

        let radius = height as f32 * 0.08;
        let spacing = radius * 4.5;
        let start_x = 10.0 + radius;

        let px = x as f32;

        let count = winvd::get_desktop_count().unwrap_or(1) as usize;
        let half_spacing = spacing / 2.0;
        for i in 0..count {
            let cx = start_x + (i as f32) * spacing;
            // Switch to rectangular Hit-box (like a div block), connected continuously without gaps
            if px >= cx - half_spacing && px <= cx + half_spacing {
                return Some(i);
            }
        }
        None
    }
}

/// Indicator window displaying status on the Taskbar.
/// It uses the `WS_EX_LAYERED` flag combined with `UpdateLayeredWindow` to draw 32-bit graphics with an Alpha (transparent) channel.
pub struct IndicatorWindow {
    pub hwnd: HWND,
    _desktop_event_thread: Option<winvd::DesktopEventThread>,
}

impl IndicatorWindow {
    /// Initializes a new Indicator window.
    ///
    /// **WARN: Task View Issue (Win+Tab)**
    /// Currently this window is set as an Owned Window of `Shell_TrayWnd` (Taskbar) so it always stays
    /// on top of the Taskbar. However, on Windows 11, when opening Task View (Win+Tab), the DWM system automatically
    /// uses "Cloaking" technique to hide all Owned windows of the Taskbar. As a result,
    /// the Indicator will disappear while Task View is open, and usually only reappears when the Taskbar receives
    /// focus. This is a current technical limitation with no complete workaround yet.
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
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
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

    /// Starts a thread to monitor Desktop switching events (Virtual Desktop).
    /// When it detects the user switching Desktops, it sends a `WM_APP_VD_EVENT` message to the main UI thread
    /// to request a re-render of the Indicator dots, accurately displaying the current Desktop.
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

    /// Renders Indicator content to a buffer, then pushes it directly to the screen.
    ///
    /// Instead of using standard GDI (which causes jagged black border artifacts when combined with `LWA_COLORKEY`),
    /// this function initializes a 32-bit ARGB bitmap (DIBSection), draws circular dots using the SDF
    /// (Signed Distance Field) algorithm for smooth anti-aliasing, then uses `UpdateLayeredWindow`
    /// to apply the entire Alpha channel onto the Desktop.
    ///
    /// To be honest, I don't even know how this function works anymore :P
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

                // Point rust array to image buffer
                // Fill with black background but 0 transparency (Alpha = 0) => 100% invisible
                let buffer = from_raw_parts_mut(ppvbits as *mut u32, (width * height) as usize);
                buffer.fill(0);

                let light_mode = utils::is_light_theme();
                let button_theme_color = match light_mode {
                    true => (20, 20, 20),
                    false => (255, 255, 255),
                };
                let hitbox_theme_color = match light_mode {
                    true => (255, 255, 255),
                    false => (100, 100, 100),
                };

                // Taskbar at 1080p is usually 48px high, at 4K (200%) it's 96px high.
                // Binding to Taskbar height keeps the aspect ratio 100% accurate on all screens.
                let radius = height as f32 * 0.07; // Radius = 7% height (equivalent to ~3.36px at 1080p)
                let spacing = radius * 5.;
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

                let hover_idx = HOVER_INDEX.load(Ordering::Relaxed);

                for i in 0..count {
                    let cx = start_x + (i as f32) * spacing;
                    let is_hovered = hover_idx == (i as isize);

                    // Draw invisible Div block (Alpha = 1) as a Hitbox surrounding the dot.
                    // If hovering, draw a slight rounded background (Alpha = 0.15)
                    Self::draw_hitbox_and_bg(
                        buffer,
                        width,
                        height,
                        cx,
                        spacing,
                        is_hovered,
                        hitbox_theme_color,
                    );

                    let is_active = i == current_idx;

                    let mut current_radius = radius;
                    let mut base_alpha = if is_active {
                        current_radius *= 1.25; // Active indicator is as big as when hovered
                        1.0
                    } else {
                        0.5
                    };

                    // Hover effect
                    if is_hovered {
                        current_radius = radius * 1.25; // Ensure 25% enlargement (don't double if both active and hovered)
                        if base_alpha < 0.8 {
                            base_alpha = 0.8; // Brighten up
                        }
                    }

                    Self::draw_aa_circle(
                        buffer,
                        width,
                        height,
                        cx,
                        cy,
                        current_radius,
                        button_theme_color,
                        base_alpha,
                    );
                }

                // Update to screen
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

    /// Draws an "invisible" rectangular block (Hitbox) and blurred rounded background on Hover
    fn draw_hitbox_and_bg(
        buffer: &mut [u32],
        width: i32,
        height: i32,
        cx: f32,
        spacing: f32,
        is_hovered: bool,
        theme_color: (u8, u8, u8),
    ) {
        let half_spacing = spacing / 2.0;
        let min_x = (cx - half_spacing).floor().max(0.0) as i32;
        let max_x = (cx + half_spacing).ceil().min((width - 1) as f32) as i32;
        let min_y = 0;
        let max_y = height - 1;

        let cy = height as f32 / 2.0;

        // bg_rw: Horizontal radius of hover background (Width = bg_rw * 2)
        // Instead of subtracting margin, we leave `spacing / 2.0` so hover backgrounds touch continuously (no gap)
        let bg_rw = spacing / 2.0;

        // bg_rh: Vertical radius of hover background (Height = bg_rh * 2)
        // Subtract 6px to create padding from the top/bottom edges of the Taskbar
        let bg_rh = (height as f32) / 2.0 - 6.0;

        // Corner radius of hover background (Larger means rounder, max is bg_rh)
        let corner_radius = 6.0;

        let inner_w = bg_rw - corner_radius;
        let inner_h = bg_rh - corner_radius;
        let (r, g, b) = theme_color;

        let base_alpha = 0.3; // Transparency of hover background

        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let idx = (y * width + x) as usize;
                if idx >= buffer.len() {
                    continue;
                }

                if is_hovered {
                    let dx = (px - cx).abs() - inner_w;
                    let dy = (py - cy).abs() - inner_h;
                    let dist = dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - corner_radius;

                    let mut alpha = if dist <= -0.5 {
                        1.0
                    } else if dist >= 0.5 {
                        0.0
                    } else {
                        0.5 - dist
                    };

                    alpha *= base_alpha;

                    if alpha > 0.0 {
                        let a = (alpha * 255.0) as u32;
                        let pr = (r as f32 * alpha) as u32;
                        let pg = (g as f32 * alpha) as u32;
                        let pb = (b as f32 * alpha) as u32;
                        buffer[idx] = (a << 24) | (pr << 16) | (pg << 8) | pb;
                        continue;
                    }
                }

                // Invisible hitbox
                if buffer[idx] == 0 {
                    buffer[idx] = 0x01000000;
                }
            }
        }
    }

    /// Draws an anti-aliased SDF circle directly onto a 32-bit ARGB buffer.
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

                // Anti-aliasing (SDF)
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

    /// Core Message Procedure for the Indicator window.
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
                // Exit "Input Sync Call" before calling render (COM), which helps
                // get the latest state when rerendering
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
            WM_MOUSEMOVE => {
                let x = ((lparam.0 as i32) << 16) >> 16;
                let hovered = get_hovered_index(x);
                let old = HOVER_INDEX.load(std::sync::atomic::Ordering::Relaxed);
                let new_val = hovered.map(|i| i as isize).unwrap_or(-1);

                if old != new_val {
                    HOVER_INDEX.store(new_val, std::sync::atomic::Ordering::Relaxed);
                    Self::render(hwnd);

                    if new_val != -1 {
                        let mut tme = KeyboardAndMouse::TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<KeyboardAndMouse::TRACKMOUSEEVENT>() as u32,
                            dwFlags: KeyboardAndMouse::TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        let _ = unsafe { KeyboardAndMouse::TrackMouseEvent(&mut tme) };
                    }
                }
                LRESULT(0)
            }
            0x02A3 /* WM_MOUSELEAVE */ => {
                HOVER_INDEX.store(-1, std::sync::atomic::Ordering::Relaxed);
                Self::render(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let x = ((lparam.0 as i32) << 16) >> 16;
                if let Some(idx) = get_hovered_index(x) {
                    if let Ok(desktops) = winvd::get_desktops() {
                        if idx < desktops.len() {
                            let _ = winvd::switch_desktop(desktops[idx]);
                        }
                    }
                }
                LRESULT(0)
            }
            WindowsAndMessaging::WM_SETCURSOR => {
                unsafe {
                    let cursor = WindowsAndMessaging::LoadCursorW(
                        None,
                        WindowsAndMessaging::IDC_HAND
                    ).unwrap();
                    let _ = WindowsAndMessaging::SetCursor(Some(cursor));
                }
                LRESULT(1)
            }
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

