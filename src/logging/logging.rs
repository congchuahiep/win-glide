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

    let file_appender = tracing_appender::rolling::daily("./logs", "better-windows-navigate.log");
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

/// Trích xuất tên module từ event metadata.
fn module_tag(event: &tracing::Event) -> Option<Tag> {
    let target: &'static str = event.metadata().target();
    let short: &'static str = target.rsplit("::").next().unwrap_or(target);

    Some(Tag::builder().prefix(short).suffix("").icon(' ').build())
}
