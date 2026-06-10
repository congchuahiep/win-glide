use nu_ansi_term::Color;
use std::{fmt::Write, sync::atomic::Ordering};
use tracing::Level;
use tracing_forest::printer::Formatter;
use tracing_forest::tree::Tree;
use tracing_subscriber::fmt::MakeWriter;

use crate::logging::console::{CONSOLE_PIPE, CONSOLE_VISIBLE};

pub struct CleanFormatter;

impl Formatter for CleanFormatter {
    type Error = std::fmt::Error;

    fn fmt(&self, tree: &Tree) -> Result<String, Self::Error> {
        let mut w = String::with_capacity(256);
        Self::format_tree(tree, None, &mut Vec::new(), &mut w)?;
        Ok(w)
    }
}

enum Indent {
    Null,
    Line,
    Fork,
    Turn,
}

impl Indent {
    fn repr(&self) -> &'static str {
        match self {
            Self::Null => "   ",
            Self::Line => "│  ",
            Self::Fork => "├─ ",
            Self::Turn => "└─ ",
        }
    }
}

impl CleanFormatter {
    fn format_tree(
        tree: &Tree,
        duration_root: Option<f64>,
        indent: &mut Vec<Indent>,
        w: &mut String,
    ) -> std::fmt::Result {
        match tree {
            Tree::Event(event) => {
                write!(w, "{} ", ColorLevel(event.level()))?;
                Self::format_indent(indent, w)?;

                if let Some(prefix) = event.tag().and_then(|t| t.prefix()) {
                    let dim = Color::White.dimmed();
                    write!(w, "{}{}:{} ", dim.prefix(), prefix, dim.suffix())?;
                }

                if let Some(msg) = event.message() {
                    w.write_str(msg)?;
                }

                for f in event.fields() {
                    write!(w, " | {}: {}", f.key(), f.value())?;
                }
                writeln!(w)
            }
            Tree::Span(span) => {
                let total = span.total_duration().as_nanos() as f64;
                let inner = span.inner_duration().as_nanos() as f64;
                let root = duration_root.unwrap_or(total);

                write!(w, "{} ", ColorLevel(span.level()))?;
                Self::format_indent(indent, w)?;

                let cyan = Color::Cyan;
                let yellow = Color::Yellow;
                write!(
                    w,
                    "{}{}{} {}[ {} | ",
                    cyan.prefix(),
                    span.name(),
                    cyan.suffix(),
                    yellow.prefix(),
                    Self::fmt_dur(total)
                )?;

                if inner > 0.0 {
                    let base = span.base_duration().as_nanos() as f64;
                    write!(w, "{:.2}% / ", 100.0 * base / root)?;
                }
                write!(w, "{:.2}% ]{}", 100.0 * total / root, yellow.suffix())?;

                for f in span.fields() {
                    write!(w, " | {}: {}", f.key(), f.value())?;
                }
                writeln!(w)?;

                let nodes: Vec<_> = span.nodes().iter().collect();
                if let Some((last, rest)) = nodes.split_last() {
                    if let Some(edge) = indent.last_mut() {
                        *edge = match edge {
                            Indent::Turn => Indent::Null,
                            Indent::Fork => Indent::Line,
                            _ => Indent::Null,
                        };
                    }
                    indent.push(Indent::Fork);
                    for tree in rest {
                        if let Some(e) = indent.last_mut() {
                            *e = Indent::Fork;
                        }
                        Self::format_tree(tree, Some(root), indent, w)?;
                    }
                    if let Some(e) = indent.last_mut() {
                        *e = Indent::Turn;
                    }
                    Self::format_tree(last, Some(root), indent, w)?;
                    indent.pop();
                }
                Ok(())
            }
        }
    }

    fn format_indent(indent: &[Indent], w: &mut String) -> std::fmt::Result {
        let cyan = Color::Cyan.dimmed();

        for i in indent {
            write!(w, "\t{}{}{}", cyan.prefix(), i.repr(), cyan.suffix())?;
        }
        Ok(())
    }

    fn fmt_dur(mut t: f64) -> String {
        for unit in ["ns", "µs", "ms", "s"] {
            if t < 10.0 {
                return format!("{t:.2}{unit}");
            } else if t < 100.0 {
                return format!("{t:.1}{unit}");
            } else if t < 1000.0 {
                return format!("{t:.0}{unit}");
            }
            t /= 1000.0;
        }
        format!("{:.0}s", t * 1000.0)
    }
}

struct ColorLevel(Level);

impl std::fmt::Display for ColorLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let color = match self.0 {
            Level::TRACE => Color::Purple,
            Level::DEBUG => Color::Blue,
            Level::INFO => Color::Green,
            Level::WARN => Color::Rgb(252, 234, 160), // light orange
            Level::ERROR => Color::Red,
        };
        let style = color.bold();
        write!(f, "{}{:<6}{}", style.prefix(), self.0, style.suffix())
    }
}

// ---------------------------------------------------------------------------
// MakeWriter implementation for tracing-forest
// ---------------------------------------------------------------------------

/// Writer that logs to the stdin pipe of the child console process.
///
/// If the pipe does not exist (console not toggled or already closed), the data is
/// silently discarded without causing a panic or blocking the main application.
pub struct ConsoleWriter;

impl<'a> MakeWriter<'a> for ConsoleWriter {
    type Writer = PipeWriter;

    fn make_writer(&'a self) -> Self::Writer {
        PipeWriter
    }
}

/// Writer that executes data writing to the stdin pipe.
///
/// **Crucial rule**: The `write()` function must **NEVER** return an `Err`.
///
/// Reason: `tracing-forest` uses `.expect()` on the result of `Processor::process()`.
/// If `write()` returns an error (e.g., `BrokenPipe` when the console is closed), it will
/// propagate to `Printer::process()` -> `ForestLayer::on_event()` -> `.expect()` -> **panic**.
///
/// When a broken pipe is detected, the writer automatically cleans up the pipe (sets to `None`) and
/// updates `CONSOLE_VISIBLE = false`, then returns `Ok(buf.len())` to swallow the data.
pub struct PipeWriter;

impl std::io::Write for PipeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if crate::logging::console::DEBUG_CLI_MODE.load(Ordering::SeqCst) {
            return std::io::stdout().write(buf);
        }

        if let Ok(mut guard) = CONSOLE_PIPE.lock() {
            if let Some(ref mut pipe) = *guard {
                match pipe.write(buf) {
                    Ok(n) => return Ok(n),
                    Err(_) => {
                        // Pipe broken (console closed) -> clean up
                        *guard = None;
                        CONSOLE_VISIBLE.store(false, Ordering::SeqCst);
                    }
                }
            }
        }
        // Pipe does not exist or is broken -> swallow the data
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if crate::logging::console::DEBUG_CLI_MODE.load(Ordering::SeqCst) {
            return std::io::stdout().flush();
        }

        if let Ok(mut guard) = CONSOLE_PIPE.lock() {
            if let Some(ref mut pipe) = *guard {
                if pipe.flush().is_err() {
                    *guard = None;
                    CONSOLE_VISIBLE.store(false, Ordering::SeqCst);
                }
            }
        }
        Ok(())
    }
}
