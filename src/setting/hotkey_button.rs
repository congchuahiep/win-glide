use std::cell::RefCell;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, VK_CONTROL, VK_ESCAPE, VK_LWIN,
    VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, WH_KEYBOARD_LL,
};
use windows_reactor::{button, Element, RenderCx};

const WM_KEYDOWN: u32 = 0x0100;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_KEYUP: u32 = 0x0101;
const WM_SYSKEYUP: u32 = 0x0105;

thread_local! {
    static HOOK_HANDLE: RefCell<Option<HHOOK>> = RefCell::new(None);
    static CALLBACK: RefCell<Option<Box<dyn FnOnce(Option<(u32, u32)>)>>> = RefCell::new(None);
    static MODIFIERS_STATE: RefCell<u32> = RefCell::new(0);
}

/// The hotkey to display and capture.
///
/// # Args
/// - `hotkey`: `(modifiers, virtual key code)`
pub fn render_hotkey_button(
    hotkey: (u32, u32),
    enabled: bool,
    on_capture: impl Fn(u32, u32) + 'static,
    cx: &mut RenderCx,
) -> Element {
    let (is_listening, set_listening) = cx.use_state(false);
    let on_capture = std::rc::Rc::new(on_capture);

    if is_listening {
        button("Listening... (Press Esc to cancel)").accent().enabled(enabled).into()
    } else {
        button(format_hotkey(hotkey.0, hotkey.1))
            .accent()
            .enabled(enabled)
            .on_click(move || {
                set_listening.call(true);
                let set_listening_clone = set_listening.clone();
                let on_capture_clone = on_capture.clone();
                capture_hotkey(move |result| {
                    if let Some((mods, vk)) = result {
                        on_capture_clone(mods, vk);
                    }
                    set_listening_clone.call(false);
                });
            })
            .into()
    }
}

/// Bắt phím nóng từ người dùng thông qua Global Keyboard Hook (WH_KEYBOARD_LL)
/// Ưu điểm: Chặn được phím (Swallow key) để các app khác (hoặc app ngầm) không bị kích hoạt ngoài
/// ý muốn.
pub fn capture_hotkey<F>(on_captured: F)
where
    F: FnOnce(Option<(u32, u32)>) + 'static,
{
    // Hủy hook cũ nếu có
    HOOK_HANDLE.with(|h| {
        if let Some(hook) = h.borrow_mut().take() {
            let _ = unsafe { UnhookWindowsHookEx(hook) };
        }
    });

    CALLBACK.with(|cb| {
        *cb.borrow_mut() = Some(Box::new(on_captured));
    });

    // Lấy trạng thái Modifier hiện tại phòng trường hợp người dùng đang giữ phím
    MODIFIERS_STATE.with(|m| {
        let mut mods = 0;
        if unsafe { GetAsyncKeyState(VK_CONTROL.0 as i32) } as u16 & 0x8000 != 0 {
            mods |= MOD_CONTROL.0 as u32;
        }
        if unsafe { GetAsyncKeyState(VK_MENU.0 as i32) } as u16 & 0x8000 != 0 {
            mods |= MOD_ALT.0 as u32;
        }
        if unsafe { GetAsyncKeyState(VK_SHIFT.0 as i32) } as u16 & 0x8000 != 0 {
            mods |= MOD_SHIFT.0 as u32;
        }
        if unsafe { GetAsyncKeyState(VK_LWIN.0 as i32) } as u16 & 0x8000 != 0
            || unsafe { GetAsyncKeyState(VK_RWIN.0 as i32) } as u16 & 0x8000 != 0
        {
            mods |= MOD_WIN.0 as u32;
        }
        *m.borrow_mut() = mods;
    });

    let hmod = unsafe {
        GetModuleHandleW(None)
            .ok()
            .map(|h| windows::Win32::Foundation::HINSTANCE(h.0))
    };
    let hook_result = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), hmod, 0) };

    if let Ok(hook) = hook_result {
        HOOK_HANDLE.with(|h| *h.borrow_mut() = Some(hook));
    } else {
        finish_capture(None);
    }
}

unsafe extern "system" fn hook_proc(n_code: i32, w_param: WPARAM, l_param: LPARAM) -> LRESULT {
    if n_code >= 0 {
        let msg = w_param.0 as u32;
        let kbd = &*(l_param.0 as *const KBDLLHOOKSTRUCT);
        let vk = kbd.vkCode;

        let mod_flag = match vk {
            16 | 160 | 161 => MOD_SHIFT.0 as u32,
            17 | 162 | 163 => MOD_CONTROL.0 as u32,
            18 | 164 | 165 => MOD_ALT.0 as u32,
            91 | 92 => MOD_WIN.0 as u32,
            _ => 0,
        };

        if msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN {
            if vk == VK_ESCAPE.0 as u32 {
                finish_capture(None);
                return LRESULT(1);
            }

            if mod_flag != 0 {
                // Cập nhật trạng thái Modifier
                MODIFIERS_STATE.with(|m| *m.borrow_mut() |= mod_flag);
                return LRESULT(1); // Chặn phím
            } else {
                // Phím bình thường
                let current_mods = MODIFIERS_STATE.with(|m| *m.borrow());
                if current_mods != 0 {
                    finish_capture(Some((current_mods, vk)));
                }
                return LRESULT(1); // Chặn phím
            }
        } else if msg == WM_KEYUP || msg == WM_SYSKEYUP {
            if mod_flag != 0 {
                MODIFIERS_STATE.with(|m| *m.borrow_mut() &= !mod_flag);
            }
            return LRESULT(1); // Chặn sự kiện nhả phím
        }
    }
    CallNextHookEx(None, n_code, w_param, l_param)
}

fn finish_capture(result: Option<(u32, u32)>) {
    HOOK_HANDLE.with(|h| {
        if let Some(hook) = h.borrow_mut().take() {
            let _ = unsafe { UnhookWindowsHookEx(hook) };
        }
    });

    CALLBACK.with(|cb| {
        if let Some(callback) = cb.borrow_mut().take() {
            callback(result);
        }
    });
}

/// Chuyển đổi mã VK và Modifier thành chuỗi hiển thị
pub fn format_hotkey(modifiers: u32, vk: u32) -> String {
    let mut parts = Vec::new();
    if modifiers & MOD_WIN.0 as u32 != 0 {
        parts.push("Win".to_string());
    }
    if modifiers & MOD_CONTROL.0 as u32 != 0 {
        parts.push("Ctrl".to_string());
    }
    if modifiers & MOD_ALT.0 as u32 != 0 {
        parts.push("Alt".to_string());
    }
    if modifiers & MOD_SHIFT.0 as u32 != 0 {
        parts.push("Shift".to_string());
    }

    let key = match vk {
        0x30..=0x39 => format!("{}", (vk - 0x30) as u8 as char), // 0-9
        0x41..=0x5A => format!("{}", vk as u8 as char),          // A-Z
        219 => "[".to_string(),                                  // VK_OEM_4
        221 => "]".to_string(),                                  // VK_OEM_6
        188 => ",".to_string(),                                  // VK_OEM_COMMA
        190 => ".".to_string(),                                  // VK_OEM_PERIOD
        191 => "/".to_string(),                                  // VK_OEM_2
        186 => ";".to_string(),                                  // VK_OEM_1
        222 => "'".to_string(),                                  // VK_OEM_7
        192 => "`".to_string(),                                  // VK_OEM_3
        _ => format!("VK_{}", vk),
    };

    parts.push(key);
    parts.join(" + ")
}
