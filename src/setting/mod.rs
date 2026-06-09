//! Cung cấp các thành phần giao diện cài đặt (Settings UI) cho ứng dụng.
//!
//! Module này chứa các tệp định nghĩa giao diện người dùng dựa trên thư viện `windows-reactor`,
//! bao gồm cài đặt thanh Taskbar, các component tuỳ chỉnh như Expander và giao diện chính.

mod hotkey_button;
mod setting_item;
mod ui;

pub use ui::*;
