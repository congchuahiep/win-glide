use tracing::level_filters::LevelFilter;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_forest::{ForestLayer, Printer, Tag};
use tracing_subscriber::{fmt, prelude::*};

use crate::logging::{CleanFormatter, ConsoleWriter};

pub fn setup_logger(verbose: bool) -> WorkerGuard {
    let max_level = if verbose {
        LevelFilter::DEBUG
    } else {
        LevelFilter::WARN
    };

    let mut log_dir = dirs::config_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    log_dir.push("WinGlide");
    log_dir.push("logs");

    let file_appender = tracing_appender::rolling::daily(log_dir, "WinGlide.log");
    let (non_blocking_file, file_guard) = tracing_appender::non_blocking(file_appender);

    let file_layer = fmt::layer().json().with_writer(non_blocking_file);
    let forest_layer = ForestLayer::new(
        Printer::new()
            .formatter(CleanFormatter)
            .writer(ConsoleWriter),
        module_tag,
    );

    tracing_subscriber::registry()
        .with(forest_layer)
        .with(max_level)
        .with(file_layer)
        .init();

    file_guard
}

/// Extracts the module name from the event metadata.
fn module_tag(event: &tracing::Event) -> Option<Tag> {
    let target: &'static str = event.metadata().target();
    let short: &'static str = target.rsplit("::").next().unwrap_or(target);

    Some(Tag::builder().prefix(short).suffix("").icon(' ').build())
}
