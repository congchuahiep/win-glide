# WinGlide

A powerful, lightweight utility built in Rust designed to enhance navigation and multitasking on Windows 11. It provides seamless keyboard-driven taskbar navigation, uncombines taskbar buttons, and offers quick virtual desktop switching.

## Features

<table>
  <tr>
    <td>
      <b>Cycle windows based on taskbar buttons</b>: Use <code>Alt + [</code> and <code>Alt + ]</code> to instantly cycle through open applications on your taskbar (Left/Right).
    </td>
    <td>
      <video src="https://github.com/user-attachments/assets/46faf7d0-0227-45b4-b0c4-01e6a4e78797" controls width="320"></video>
    </td>
  </tr>
  <tr>
    <td>
      <b>Uncombine Taskbar Buttons</b>: Prevents taskbar buttons from being grouped together, giving you individual buttons for each window.
    </td>
    <td>
      <video src="https://github.com/user-attachments/assets/c2d74899-5966-47dc-a678-f8df698e1485" controls width="320"></video>
    </td>
  </tr>
  <tr>
    <td>
      <b>Virtual Desktop Indicator</b>: Displays a visual indicator of your current virtual desktop position directly on the taskbar.
    </td>
    <td>
      <video src="https://github.com/user-attachments/assets/1d41a096-5139-45ff-8675-7cea09de6e1a" controls width="320"></video>
    </td>
  </tr>
  <tr>
    <td>
      <b>Jump to Desktop</b>: Quickly jump to a specific virtual desktop using <code>Alt + &lt;index&gt;</code> (starting from 1).
    </td>
    <td>
      <video src="https://github.com/user-attachments/assets/6f77acad-0fb5-42c3-adc2-1f8916c1f351" controls width="320"></video>
    </td>
  </tr>
  <tr>
    <td>
      <b>System Tray Integration</b>: Easily manage the app via a convenient system tray menu.
    </td>
    <td></td>
  </tr>
  <tr>
    <td>
      <b>Lightweight & Fast</b>: Built with Rust for maximum performance and minimal resource usage. Single binary execution.
    </td>
    <td></td>
  </tr>
  <tr>
    <td>
      <b>Free</b>, why not?
    </td>
    <td></td>
  </tr>
</table>


## Installation

You can download the latest version of WinGlide from our [GitHub Releases](https://github.com/congchuahiep/WinGlide/releases/latest) page. 

We offer two ways to run the application:

* **Installer (`.msi`)**: The standard installation experience. Download the file, run it, and follow the setup wizard to install WinGlide on your system.
* **Portable (`.zip`)**: A standalone version requiring no installation. Simply download, extract the contents to your preferred folder, and run the executable directly.

> [!WARNING]
> 
> WinGlide is currently not code-signed with a paid developer certificate. Because of this, Windows Defender SmartScreen or your antivirus software might flag the application as "unrecognized" or potentially malicious. This is a common false positive for new, open-source executables. 
> 
> If you trust the source code, you can bypass Windows SmartScreen by clicking **"More info"** and then **"Run anyway"**. 
> 
> **If you are unable to run or install the `.msi` file due to strict system policies or antivirus blocks, we recommend using the portable `.zip` version.**

## Development

```bash
# Build the application
cargo build

# Build release version
cargo build --release

# Quick type-check
cargo check

# Run normally
cargo run --release

# Run with debug
cargo run -- --debug --verbose
```

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


