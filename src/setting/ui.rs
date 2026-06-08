//! Root UI module cho Settings application.
//! Nơi khởi tạo cấu trúc layout chính bao gồm Header, các mục Settings, Logging và Footer.

use windows_reactor::*;

use crate::setting::setting_item::{setting_item, SettingItemProps};

/// Khung chính của ứng dụng cài đặt (Settings App).
/// Gồm một luồng (`vstack`) chứa toàn bộ các khối nội dung, padding đều ra xung quanh.
pub fn settings_app(cx: &mut RenderCx) -> Element {
    vstack((header(), taskbar_settings(cx), logging(), footer()))
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
    vstack((
        body_strong("Taskbar").margin(Thickness {
            bottom: 10.,
            ..Default::default()
        }),
        setting_item(
            &SettingItemProps {
                icon: Some('\u{E7C4}'),
                title: "Combine Mode".into(),
                description: Some("Group windows together on the taskbar".into()),
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
                icon: Some('\u{E8AB}'),
                title: "Cycle windows based on taskbar buttons".into(),
                description: None,
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
                children: Some(vec![
                    SettingItemProps {
                        icon: None,
                        title: "Cycle Left".into(),
                        description: None,
                        action: Some(button("Alt + [").accent().into()),
                        children: None,
                    },
                    SettingItemProps {
                        icon: None,
                        title: "Cycle Right".into(),
                        description: None,
                        action: Some(button("Alt + ]").accent().into()),
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
