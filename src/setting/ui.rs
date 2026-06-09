//! Root UI module cho Settings application.
//! Nơi khởi tạo cấu trúc layout chính bao gồm Header, các mục Settings, Logging và Footer.

use windows_reactor::*;

use crate::config::AppConfig;
use crate::setting::hotkey_button;
use crate::setting::setting_item::{setting_item, SettingItemProps};

/// Gửi tín hiệu tải lại cấu hình tới tiến trình chạy ngầm
fn send_reload_signal() {
    unsafe {
        if let Ok(hwnd) = windows::Win32::UI::WindowsAndMessaging::FindWindowW(
            windows::core::w!("TaskbarSwitcherTray"),
            windows::core::PCWSTR::null(),
        ) {
            if !hwnd.is_invalid() {
                let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                    Some(hwnd),
                    crate::event::WM_APP_RELOAD_CONFIG,
                    windows::Win32::Foundation::WPARAM(0),
                    windows::Win32::Foundation::LPARAM(0),
                );
            }
        }
    }
}

/// Khung chính của ứng dụng cài đặt (Settings App).
/// Gồm một luồng (`vstack`) chứa toàn bộ các khối nội dung, padding đều ra xung quanh.
pub fn settings_app(cx: &mut RenderCx) -> Element {
    let content = vstack((
        header(),
        taskbar_settings(cx),
        virtual_desktop_settings(cx),
        logging(),
        footer(),
    ))
    .spacing(32.0)
    .margin(24.0);

    scroll_viewer(content).into()
}

/// Khối Header hiển thị Tiêu đề và Mô tả chính của App.
fn header() -> Element {
    vstack((title("Settings"),)).spacing(12.0).into()
}

/// Phân vùng chứa các cài đặt liên quan đến Taskbar.
/// Lưu ý: `cx: &mut RenderCx` được truyền trực tiếp xuống `setting_item` thay vì
/// sử dụng wrapper `component(setting_item)`, nhằm đảm bảo việc Reconciler của
/// `windows-reactor` đánh giá được đầy đủ sự thay đổi state, tránh lỗi skip render
/// khi component bị bọc trong một `vstack` tĩnh.
fn taskbar_settings(cx: &mut RenderCx) -> Element {
    let (config, set_config) = cx.use_state(AppConfig::load());

    vstack((
        body_strong("Taskbar").margin(Thickness {
            bottom: 10.,
            ..Default::default()
        }),
        setting_item(
            &SettingItemProps {
                icon: Some('\u{E7C4}'),
                title: Some("Uncombine taskbar buttons".into()),
                description: Some("Disallow group windows on the taskbar".into()),
                action: Some(
                    ToggleSwitch::new(config.uncombine_mode)
                        .enabled(!config.cycle_taskbar_based)
                        .on_content("")
                        .off_content("")
                        .width(42.)
                        .min_width(0.0)
                        .on_changed({
                            let set_config = set_config.clone();
                            move |is_on: bool| {
                                let mut new_config = crate::config::AppConfig::load();
                                if new_config.uncombine_mode == is_on { return; }
                                new_config.uncombine_mode = is_on;
                                new_config.save();
                                set_config.call(new_config);
                                send_reload_signal();
                            }
                        })
                        .into(),
                ),
                children: None, always_expand: false, enabled: true,
            },
            cx,
        ),
        setting_item(
            &SettingItemProps {
                icon: Some('\u{E8AB}'),
                title: Some("Cycle taskbar buttons".into()),
                description: Some("Switch windows based on taskbar buttons".into()),
                action: Some(
                    ToggleSwitch::new(config.cycle_taskbar_based)
                        .on_content("")
                        .off_content("")
                        .width(42.)
                        .min_width(0.0)
                        .on_changed({
                            let set_config = set_config.clone();
                            move |is_on: bool| {
                                let mut new_config = crate::config::AppConfig::load();
                                if new_config.cycle_taskbar_based == is_on { return; }
                                new_config.cycle_taskbar_based = is_on;
                                if is_on {
                                    new_config.uncombine_mode = true;
                                }
                                new_config.save();
                                set_config.call(new_config);
                                send_reload_signal();
                            }
                        })
                        .into(),
                ),
                always_expand: true, enabled: true,
                children: Some(vec![
                    SettingItemProps {
                        icon: None,
                        title: None,
                        description: Some("By default, 'Cycle taskbar buttons' automatically uncombines buttons because it handles button groups poorly".into()),
                        action: None,
                        children: None, always_expand: false, enabled: config.cycle_taskbar_based,
                    },
                    SettingItemProps {
                        icon: None,
                        title: Some("Cycle left".into()),
                        description: None,
                        action: Some(hotkey_button::render_hotkey_button(
                            (config.hotkey_left_modifiers, config.hotkey_left_vk),
                            config.cycle_taskbar_based,
                            {
                                let set_config = set_config.clone();
                                let config_state = config.clone();
                                move |mods, vk| {
                                    let mut new_config = config_state.clone();

                                    // Handle conflict: swap if the other uses the same hotkey
                                    if new_config.hotkey_right_modifiers == mods
                                        && new_config.hotkey_right_vk == vk
                                    {
                                        new_config.hotkey_right_modifiers =
                                            new_config.hotkey_left_modifiers;
                                        new_config.hotkey_right_vk = new_config.hotkey_left_vk;
                                    }

                                    new_config.hotkey_left_modifiers = mods;
                                    new_config.hotkey_left_vk = vk;
                                    new_config.save();
                                    set_config.call(new_config);
                                    send_reload_signal();
                                }
                            },
                            cx,
                        )),
                        children: None, always_expand: false, enabled: config.cycle_taskbar_based,
                    },
                    SettingItemProps {
                        icon: None,
                        title: Some("Cycle right".into()),
                        description: None,
                        action: Some(hotkey_button::render_hotkey_button(
                            (config.hotkey_right_modifiers, config.hotkey_right_vk),
                            config.cycle_taskbar_based,
                            {
                                let set_config = set_config.clone();
                                let config_state = config.clone();
                                move |mods, vk| {
                                    let mut new_config = config_state.clone();

                                    // Handle conflict: swap if the other uses the same hotkey
                                    if new_config.hotkey_left_modifiers == mods
                                        && new_config.hotkey_left_vk == vk
                                    {
                                        new_config.hotkey_left_modifiers =
                                            new_config.hotkey_right_modifiers;
                                        new_config.hotkey_left_vk = new_config.hotkey_right_vk;
                                    }

                                    new_config.hotkey_right_modifiers = mods;
                                    new_config.hotkey_right_vk = vk;
                                    new_config.save();
                                    set_config.call(new_config);
                                    send_reload_signal();
                                }
                            },
                            cx,
                        )),
                        children: None, always_expand: false, enabled: config.cycle_taskbar_based,
                    },
                ]),
            },
            cx,
        ),
    ))
    .spacing(4.0)
    .into()
}

fn virtual_desktop_settings(cx: &mut RenderCx) -> Element {
    let (config, set_config) = cx.use_state(AppConfig::load());

    // Khởi tạo các cờ cho toggle buttons
    let has_ctrl = config.jump_desktop_modifiers
        & windows::Win32::UI::Input::KeyboardAndMouse::MOD_CONTROL.0 as u32
        != 0;
    let has_alt = config.jump_desktop_modifiers
        & windows::Win32::UI::Input::KeyboardAndMouse::MOD_ALT.0 as u32
        != 0;

    let update_modifier = {
        let set_config = set_config.clone();
        let config_state = config.clone();
        std::rc::Rc::new(move |mod_flag: u32, is_checked: bool| {
            let mut new_config = config_state.clone();
            if is_checked {
                new_config.jump_desktop_modifiers |= mod_flag;
            } else {
                new_config.jump_desktop_modifiers &= !mod_flag;
            }
            new_config.save();
            set_config.call(new_config);
            send_reload_signal();
        })
    };

    let ctrl_btn = toggle_button("Ctrl", has_ctrl).on_changed({
        let update_modifier = update_modifier.clone();
        move |checked| {
            update_modifier(
                windows::Win32::UI::Input::KeyboardAndMouse::MOD_CONTROL.0 as u32,
                checked,
            )
        }
    });
    let alt_btn = toggle_button("Alt", has_alt).on_changed({
        let update_modifier = update_modifier.clone();
        move |checked| {
            update_modifier(
                windows::Win32::UI::Input::KeyboardAndMouse::MOD_ALT.0 as u32,
                checked,
            )
        }
    });

    let jump_modifiers_action = hstack((ctrl_btn, alt_btn))
        .spacing(4.0)
        .margin(Thickness {
            right: 10.,
            ..Default::default()
        })
        .vertical_alignment(VerticalAlignment::Center);

    vstack((
        body_strong("Virtual Desktop").margin(Thickness {
            bottom: 10.,
            ..Default::default()
        }),
        setting_item(
            &SettingItemProps {
                icon: Some('\u{E712}'),
                title: Some("Desktop Indicator".into()),
                description: Some("Show the virtual desktop indicator on the taskbar".into()),
                action: Some(
                    ToggleSwitch::new(config.desktop_indicator)
                        .on_content("")
                        .off_content("")
                        .min_width(0.0)
                        .width(42.)
                        .on_changed({
                            let set_config = set_config.clone();
                            let config_state = config.clone();
                            move |checked| {
                                let mut new_config = config_state.clone();
                                new_config.desktop_indicator = checked;
                                new_config.save();
                                set_config.call(new_config);
                                send_reload_signal();
                            }
                        })
                        .into(),
                ),
                children: None,
                always_expand: false,
                enabled: true,
            },
            cx,
        ),
        setting_item(
            &SettingItemProps {
                icon: Some('\u{E7B5}'),
                title: Some("Jump to Desktop".into()),
                description: Some("Change the desktop by index. e.g Alt+1, Alt+2".into()),
                action: Some(jump_modifiers_action.into()),
                children: None,
                always_expand: false,
                enabled: true,
            },
            cx,
        ),
    ))
    .spacing(4.0)
    .into()
}

/// Khối hiển thị thông tin Debug Logging.
fn logging() -> Element {
    vstack((
        body_strong("Logging"),
        button("Show Debug Console").on_click(|| {
            crate::logging::console::toggle();
        }),
    ))
    .spacing(8.0)
    .into()
}

/// Khối Footer hiển thị thông tin tác giả và Repository.
fn footer() -> Element {
    vstack((
        body_strong("About"),
        body("Version: 0.0.1"),
        body("Author: @congchuahiep"),
        button("GitHub Repository").on_click(|| {
            // Open browser
        }),
    ))
    .spacing(8.0)
    .into()
}

/// Runs the settings UI application. Only run it in the main thread.
pub fn run() -> Result<()> {
    let _bootstrap_handle = bootstrap::initialize()?;

    App::new()
        .title("Better windows navigate")
        .backdrop(Backdrop::Mica)
        .inner_size(500., 720.)
        .inner_constraints(InnerConstraints {
            min_width: Some(500.),
            min_height: Some(540.),
            max_width: None,
            max_height: None,
        })
        .render(settings_app)
}

/// Opens the settings UI by spawning a new process.
pub fn show_ui() {
    let exe_path = std::env::current_exe().expect("Failed to get executable path");

    std::process::Command::new(exe_path)
        .arg("--settings-ui")
        .spawn()
        .expect("Failed to spawn settings UI process");
}
