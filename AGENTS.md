# AGENTS.md - WinGlide

## Project summary

Rust (edition 2021) app for **Windows 11 only** - cycles through taskbar buttons using global hotkeys (`Alt+[` / `Alt+]`). Single binary, no workspace/monorepo. Uses IUIAutomation because Win11 taskbar is XAML, not Win32 HWNDs.

## Developer commands

```bash
cargo build --release              # binary -> target/release/WinGlide.exe
cargo build                        # debug build
cargo check                        # quick type-check (no codegen needed)
./target/release/WinGlide.exe            # normal run
./target/release/WinGlide.exe -v         # with debug logging
./target/release/WinGlide.exe --combine-mode  # keep taskbar buttons grouped
```

No lint/formatter config exists in the repo - only `cargo check` / `cargo build` are available.

## CLI args (manual parsing, no clap)

- `-v` / `--verbose` - enable debug-level console/file logging
- `--combine-mode` - keep taskbar buttons combined (skip uncombine)

## Architecture

```
main.rs                     -> entry: parse_args -> setup_logger -> App::new -> App::run (message loop)
app.rs                      -> orchestrator: wires hotkey_manager + enumerator + uncombine_manager
hotkey.rs                   -> RegisterHotKey (Alt+[/Alt+]), dispatches HotkeyAction::Left/Right
taskbar/
├── mod.rs                  -> module doc + re-export: TaskbarEnumerator, CycleDirection, UncombineManager
├── enumerator.rs           -> IUIAutomation: enumerate buttons, cache with 1s TTL, cycle_to_neighbor
├── button_window.rs        -> ButtonWindowMap: ánh xạ button ↔ window (AUMID -> PID -> Title -> Process)
├── window.rs               -> EnumWindows: find_visible_windows, get_app_user_model_id, get_process_name
├── activate.rs             -> force_activate (SetForegroundWindow + AttachThreadInput)
├── explorer.rs             -> get_explorer_pid, invalidate_explorer_pid_cache
└── uncombine.rs            -> UncombineManager: sets unique AppUserModelID per window
event/uia.rs                -> UIA StructureChanged hook -> WM_APP_INVALIDATE_CACHE
event/winevent.rs           -> WinEvent EVENT_OBJECT_SHOW hook -> WM_APP_UNCOMBINE
types.rs                    -> shared data structs (TaskbarButton, WindowInfo, TargetWindow), no logic
utils.rs                    -> clean_button_name, truncate, is_system_class
temp.rs                     -> dead placeholder (add_one function) - ignore
logging/                    -> tracing-subscriber + tracing-forest, file output to ./logs/
```

## Key facts agents will miss

### COM threading

- App runs **STA apartment** (`CoInitializeEx(COINIT_APARTMENTTHREADED)`). Windows message loop (`GetMessageW`) must run on the same thread that initialized COM.

### UncombineManager lifetime

- `UncombineManager` is `Box::leak`'d in `App::new()` to get a `&'static` reference. This is intentional - the WinEvent callback thread accesses it via `AtomicPtr<UncombineManager>`.

### Cache invalidation

- Button cache (1s TTL) is invalidated by UIA `StructureChanged` events (not WinEvent). A `CACHE_INVALIDATED` AtomicBool prevents posting duplicate `WM_APP_INVALIDATE_CACHE` messages when multiple UIA events fire in rapid succession.

### Explorer restart recovery

- `TaskbarEnumerator::enumerate_buttons()` catches `EVENT_E_ALL_SUBSCRIBERS_FAILED (0x80040201)` and auto-recovers via `refresh_taskbar_hwnd()` - re-finds Shell_TrayWnd, re-subscribes UIA hooks, invalidates explorer PID cache.

### Matching strategies (button_window.rs)

Button-to-window matching tries 4 strategies in order:

1. **AppUserModelID** (button `automation_id` vs window `SHGetPropertyStoreForWindow`)
2. **PID** (if button PID ≠ explorer PID)
3. **Title** fuzzy match (after `clean_button_name` stripping)
4. **Process name** (`.exe` stem match, allows windows with empty titles)

### Logging output

- Logs go to **file only** (`./logs/"WinGlide.log` via `tracing-appender`). The console layer is commented out. Use `tracing_forest` for tree-structured output.

### Dependencies

- `windows` 0.61. Uses features from `Win32_UI_Accessibility`, `Win32_UI_Shell_PropertiesSystem`, etc.
- No clap, no serde - minimal dependency set.

### Windows 11 only

- Relies on XAML taskbar class `Taskbar.TaskListButtonAutomationPeer`. Will not work on Windows 10 (which uses `ToolbarWindow32`). Only enumerates the **primary monitor** taskbar.

### Hotkey IDs

- Hotkey ID 1 = `Alt+[` (VK_OEM_4) -> Left
- Hotkey ID 2 = `Alt+]` (VK_OEM_6) -> Right

### Custom window messages

- `WM_APP_UNCOMBINE = WM_USER + 0x100` - uncombine a new window
- `WM_APP_INVALIDATE_CACHE = WM_USER + 0x101` - invalidate button cache
