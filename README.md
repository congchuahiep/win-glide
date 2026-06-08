# Better Windows Navigate

A powerful, lightweight utility built in Rust designed to enhance navigation and multitasking on Windows 11. It provides seamless keyboard-driven taskbar navigation, uncombines taskbar buttons, and offers quick virtual desktop switching.

## Features

- **Taskbar Navigation**: Use `Alt + [` and `Alt + ]` to instantly cycle through open applications on your taskbar (Left/Right).
- **Uncombine Taskbar Buttons**: Prevents taskbar buttons from being grouped together, giving you individual buttons for each window.
- **Virtual Desktop Indicator**: Displays a visual indicator of your current virtual desktop position directly on the taskbar.
- **Quick Switch Virtual Desktop**: Quickly jump to a specific virtual desktop using `Alt + <index>` (starting from 1).
- **System Tray Integration**: Easily manage the app via a convenient system tray menu.
- **Lightweight & Fast**: Built with Rust for maximum performance and minimal resource usage. Single binary execution.
- **Free**

## CLI Options

You can run the application from the command line with the following flags:

- `-v` or `--verbose`: Enable debug-level logging.
- `--debug`: Attach a console window for debugging.

## Technology Stack

| Component               | Technology                                  |
| ----------------------- | ------------------------------------------- |
| **Language**            | Rust (Edition 2021)                         |
| **Windows API**         | `windows-rs` 0.61                           |
| **Taskbar Enumeration** | IUIAutomation (UIA)                         |
| **Window Matching**     | `EnumWindows` + `GetWindowTextW`            |
| **Window Activation**   | `SetForegroundWindow` + `AttachThreadInput` |
| **Global Hotkeys**      | `RegisterHotKey` + `GetMessageW`            |
| **Virtual Desktops**    | `winvd`                                     |

## Limitations & Requirements

- **Windows 11 Only**: Relies on the modern Windows 11 XAML taskbar implementation (`Taskbar.TaskListButtonAutomationPeer`).

## Development Commands

```bash
# Build the application
cargo build

# Build release version
cargo build --release

# Quick type-check
cargo check

# Run normally
cargo run --release

# Run with verbose logging
cargo run --release -- -v
```
