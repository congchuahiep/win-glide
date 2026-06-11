//! Taskbar system - the core module of WinGlide.
//!
//! Manages the entire taskbar lifecycle: enumerating buttons via UI Automation,
//! mapping buttons to windows, activating windows, and uncombining buttons.

mod button_window;
mod enumerator;
mod uncombine;

pub use enumerator::{CycleDirection, TaskbarEnumerator};
pub use uncombine::UncombineManager;
