use windows_reactor::*;

pub fn settings_app(_cx: &mut RenderCx) -> Element {
    vstack((
        text_block("Better Windows navigate's settings")
            .font_size(24.0)
            .bold(),
        text_block("This is the new settings GUI using windows-reactor.").font_size(14.0),
    ))
    .spacing(12.0)
    .into()
}

/// Runs the settings UI application. Only run it in the main thread.
pub fn run() -> Result<()> {
    let _bootstrap_handle = bootstrap::initialize()?;

    App::new()
        .title("Better Windows navigate's settings")
        .backdrop(Backdrop::Mica)
        .inner_size(600., 800.)
        .render(settings_app)
}

/// Opens the settings UI by spawning a new process.
pub fn show() {
    let exe_path = std::env::current_exe().expect("Failed to get executable path");

    std::process::Command::new(exe_path)
        .arg("--settings-ui")
        .spawn()
        .expect("Failed to spawn settings UI process");
}
