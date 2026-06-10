//! Configures the application's structured logging system.
//!
//! This module sets up `tracing_subscriber` with custom formatters to output logs
//! to both a rolling file and a detached console window via Named Pipes.

mod formatter;
mod logging;
pub mod console;

pub use formatter::*;
pub use logging::*;
