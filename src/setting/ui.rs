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
    vstack((
        header(),
        taskbar_settings(cx),
        virtual_desktop_settings(cx),
        logging(),
        footer(),
    ))
    .spacing(24.0)
    .margin(24.0)
    .into()
}

/// Khối Header hiển thị Tiêu đề và Mô tả chính của App.
fn header() -> Element {
    vstack((
        title("Taskbar Switcher"),
        body("Settings configuration for your taskbar switcher application."),
    ))
    .spacing(12.0)
    .into()
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
                title: "Uncombine Mode".into(),
                description: Some("Disallow group windows on the taskbar".into()),
                action: Some(
                    ToggleSwitch::new(config.uncombine_mode)
                        .on_content("")
                        .off_content("")
                        .min_width(0.0)
                        .margin(Thickness {
                            right: -10.,
                            ..Default::default()
                        })
                        .on_changed({
                            let set_config = set_config.clone();
                            let config_state = config.clone();
                            move |is_on: bool| {
                                let mut new_config = config_state.clone();
                                new_config.uncombine_mode = is_on;
                                new_config.save();
                                set_config.call(new_config);
                                send_reload_signal();
                            }
                        })
                        .into(),
                ),
                children: None,
            },
            cx,
        ),
        setting_item(
            &SettingItemProps {
                icon: Some('\u{E8AB}'),
                title: "Cycle taskbar buttons".into(),
                description: Some("Switch windows based on taskbar buttons".into()),
                action: Some(
                    ToggleSwitch::new(config.cycle_taskbar_based)
                        .on_content("")
                        .off_content("")
                        .min_width(0.0)
                        .margin(Thickness {
                            right: -10.,
                            ..Default::default()
                        })
                        .on_changed({
                            let set_config = set_config.clone();
                            let config_state = config.clone();
                            move |is_on: bool| {
                                let mut new_config = config_state.clone();
                                new_config.cycle_taskbar_based = is_on;
                                new_config.save();
                                set_config.call(new_config);
                                send_reload_signal();
                            }
                        })
                        .into(),
                ),
                children: Some(vec![
                    SettingItemProps {
                        icon: None,
                        title: "Cycle left".into(),
                        description: None,
                        action: Some(hotkey_button::render_hotkey_button(
                            (config.hotkey_left_modifiers, config.hotkey_left_vk),
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
                        children: None,
                    },
                    SettingItemProps {
                        icon: None,
                        title: "Cycle right".into(),
                        description: None,
                        action: Some(hotkey_button::render_hotkey_button(
                            (config.hotkey_right_modifiers, config.hotkey_right_vk),
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
                        children: None,
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

    vstack((
        body_strong("Virtual Desktop").margin(Thickness {
            bottom: 10.,
            ..Default::default()
        }),
        setting_item(
            &SettingItemProps {
                icon: Some('\u{E712}'),
                title: "Desktop Indicator".into(),
                description: Some("Show the virtual desktop indicator on the taskbar".into()),
                action: Some(
                    ToggleSwitch::new(true)
                        .on_content("")
                        .off_content("")
                        .min_width(0.0)
                        .margin(Thickness {
                            right: -10.,
                            ..Default::default()
                        })
                        .into(),
                ),
                children: None,
            },
            cx,
        ),
        setting_item(
            &SettingItemProps {
                icon: Some('\u{E7B5}'),
                title: "Jump to Desktop".into(),
                description: Some("Quickly jump to the desktop by index".into()),
                action: None,
                children: None,
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
        body("Developer: congchuahiep"),
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
        .title("Better Windows navigate's settings")
        .backdrop(Backdrop::Mica)
        .inner_size(500., 600.)
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
