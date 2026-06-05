//! Hệ thống taskbar - module chính của Taskbar Switcher.
//!
//! Quản lý toàn bộ vòng đời của taskbar: liệt kê buttons qua UI Automation,
//! ánh xạ button với window, kích hoạt window, và tách/gộp buttons (uncombine).

mod activate;
mod button_window;
mod enumerator;
mod explorer;
mod uncombine;
mod window;

pub use enumerator::{CycleDirection, TaskbarEnumerator};
pub use uncombine::UncombineManager;
