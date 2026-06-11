//! Manages the Debug Console window. Used for real-time logging.
//!
//! ## Architecture: "Self-Invoking Console Worker"
//!
//! Because the main application runs with `#![windows_subsystem = "windows"]` (GUI App), it has no stdout.
//! When calling a subprocess using the `Command` builder with `Stdio::piped()`, stdout defaults to `NULL`.
//!
//! Solution: The application **calls itself** with the `--console-worker` parameter.
//! The child process:
//! 1. Calls `AllocConsole()` to create a new Console Window (GUI Apps don't have one by default).
//! 2. Opens the special file `CONOUT$` (writes directly to the current Console's buffer, bypassing the `NULL` stdout).
//! 3. Enables the `ENABLE_VIRTUAL_TERMINAL_PROCESSING` flag to display ANSI colors.
//! 4. Continuously reads the `stdin` stream and prints to the screen. When `stdin` is closed (main app exits) -> the child process terminates.

use std::io::{Read, Write};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

extern "system" {
    fn AllocConsole() -> i32;
    fn GetConsoleMode(hConsoleHandle: isize, lpMode: *mut u32) -> i32;
    fn SetConsoleMode(hConsoleHandle: isize, dwMode: u32) -> i32;
    fn SetConsoleTitleW(lpConsoleTitle: *const u16) -> i32;
}

/// Synchronization flag for the Console visibility state.
pub static CONSOLE_VISIBLE: AtomicBool = AtomicBool::new(false);

/// Flag indicating the application is running in CLI debug mode (logging to stdout).
pub static DEBUG_CLI_MODE: AtomicBool = AtomicBool::new(false);

/// Handle to the Console child process, protected by a Mutex.
static CHILD_PROCESS: Mutex<Option<Child>> = Mutex::new(None);

/// Shared stdin pipe for `MakeWriter`, protected by a Mutex.
pub static CONSOLE_PIPE: Mutex<Option<std::process::ChildStdin>> = Mutex::new(None);

/// Toggles the Debug Console window.
pub fn toggle() {
    if DEBUG_CLI_MODE.load(Ordering::SeqCst) {
        return;
    }

    let was_visible = CONSOLE_VISIBLE.load(Ordering::SeqCst);
    let new_visible = !was_visible;
    CONSOLE_VISIBLE.store(new_visible, Ordering::SeqCst);

    match new_visible {
        true => spawn_console(),
        false => kill_console(),
    }
}

/// Runs the console worker loop (only called from the child process)
pub fn run_worker() {
    unsafe {
        AllocConsole();
    }

    // Set the window title
    let title = "Debug Console - WinGlide";
    let mut title_u16: Vec<u16> = title.encode_utf16().collect();
    title_u16.push(0);
    unsafe {
        SetConsoleTitleW(title_u16.as_ptr());
    }

    // Open CONOUT$ to write directly (since the GUI app's forwarded stdout is NULL)
    // Must be opened with read/write access for GetConsoleMode to work
    let out_res = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("CONOUT$");
    if out_res.is_err() {
        return;
    }
    let mut out = out_res.unwrap();

    // Enable ANSI color support
    let handle = out.as_raw_handle() as isize;
    let mut mode = 0;
    unsafe {
        if GetConsoleMode(handle, &mut mode) != 0 {
            SetConsoleMode(handle, mode | 0x0004);
        }
    }

    // Disable selection (Quick Edit Mode) on CONIN$
    if let Ok(in_file) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("CONIN$")
    {
        let in_handle = in_file.as_raw_handle() as isize;
        let mut in_mode = 0;
        unsafe {
            if GetConsoleMode(in_handle, &mut in_mode) != 0 {
                SetConsoleMode(in_handle, (in_mode & !0x0040) | 0x0080);
            }
        }
    }

    // Continuously read from Stdin and write to the screen
    let mut stdin = std::io::stdin();
    let mut buf = [0u8; 1024];
    loop {
        match stdin.read(&mut buf) {
            Ok(0) => break, // EOF -> main app has closed
            Ok(n) => {
                if out.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

/// Spawns the `--console-worker` child process
fn spawn_console() {
    kill_console();

    let exe_path = std::env::current_exe().expect("Failed to get executable path");
    let result = Command::new(exe_path)
        .arg("--console-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NEW_PROCESS_GROUP) // New process group so Ctrl+C doesn't kill the main app
        .spawn();

    match result {
        Ok(mut child) => {
            let stdin = child.stdin.take();
            *CHILD_PROCESS.lock().unwrap() = Some(child);
            *CONSOLE_PIPE.lock().unwrap() = stdin;
        }
        Err(e) => {
            CONSOLE_VISIBLE.store(false, Ordering::SeqCst);
            eprintln!("Failed to spawn debug console worker: {e}");
        }
    }
}

/// Kills the child console process.
fn kill_console() {
    // Close the pipe first so the child process receives EOF
    let _ = CONSOLE_PIPE.lock().unwrap().take();

    if let Some(mut child) = CHILD_PROCESS.lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Windows process creation flags.
const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
