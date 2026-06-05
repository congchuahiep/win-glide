# Taskbar Switcher

Ứng dụng Rust thuần cho **Windows 11** giúp chuyển đổi qua lại giữa các nút taskbar theo thứ tự trên thanh taskbar (khác với Alt+Tab dùng để chuyển cửa sổ mở gần nhất).

```
Alt + [    →  Chuyển sang nút taskbar bên trái
Alt + ]    →  Chuyển sang nút taskbar bên phải
Ctrl + C   →  Thoát
```

> Windows 11 mặc định **không có phím tắt** để cycle qua các nút trên taskbar. Ứng dụng này mang lại trải nghiệm giống như **Mouse Wheel on Taskbar** trên Windhawk, nhưng là phiên bản standalone không cần DLL injection.

## Tính năng

- **Cycle qua các nút taskbar**: Dùng phím Alt+[ hoặc Alt+] để chuyển sang nút taskbar bên trái hoặc bên phải.
- **Uncombine taskbar button**:  Chặn việc taskbar button bị gom nhóm lại với nhau

## Kiến trúc

```
src/
├── main.rs           # Entry point, parse CLI args, setup logger, message loop
├── app.rs            # Orchestrator: điều phối hotkey + enumerator + uncombine
├── hotkey.rs         # RegisterHotKey (Alt+[ / Alt+]), dispatches HotkeyAction
├── taskbar/
│   ├── mod.rs        # Module chính — re-export TaskbarEnumerator, UncombineManager
│   ├── enumerator.rs # IUIAutomation: liệt kê buttons, cache 1s TTL, cycle_to_neighbor
│   ├── button_window.rs # ButtonWindowMap: ánh xạ button ↔ window (4 chiến lược)
│   ├── window.rs     # EnumWindows: find_visible_windows, get_app_user_model_id
│   ├── activate.rs   # force_activate (SetForegroundWindow + AttachThreadInput)
│   ├── explorer.rs   # Cache PID của explorer.exe
│   └── uncombine.rs  # UncombineManager: gán AUMID riêng cho từng window
├── event/
│   ├── uia.rs        # UIA StructureChanged hook → WM_APP_INVALIDATE_CACHE
│   └── winevent.rs   # WinEvent EVENT_OBJECT_SHOW hook → WM_APP_UNCOMBINE
├── types.rs           # TaskbarButton, WindowInfo, TargetWindow (data-only)
└── utils.rs           # clean_button_name, truncate, is_system_class
```

### Luồng hoạt động (Cycle)

```
1. Alt+] được nhấn
         ↓
2. IUIAutomation → liệt kê Taskbar.TaskListButtonAutomationPeer (có cache 1s TTL)
         ↓
3. ButtonWindowMap → tìm button index của foreground window (AUMID fast path)
         ↓
4. Tính target index (active + 1 hoặc -1, wrap around)
         ↓
5. ButtonWindowMap → tìm window ứng với target button (AUMID → PID → Title → Process)
         ↓
6. force_activate(hwnd) → đưa target lên foreground
```

## Công nghệ sử dụng

| Thành phần          | Công nghệ                               |
| ------------------- | --------------------------------------- |
| Ngôn ngữ            | Rust (Edition 2021)                     |
| Windows API         | windows-rs 0.61                         |
| Taskbar enumeration | IUIAutomation (UIA)                     |
| Tìm window          | EnumWindows + GetWindowTextW            |
| Activate window     | SetForegroundWindow + AttachThreadInput |
| Global hotkeys      | RegisterHotKey + GetMessageW            |

## Build

```bash
cargo build --release
```

Binary ở `target/release/taskbar_switcher.exe`.

## Chạy

```bash
# Chế độ uncombine (mặc định) — mỗi window có button riêng
./target/release/taskbar_switcher.exe

# Debug logging
./target/release/taskbar_switcher.exe -v

# Giữ taskbar buttons gộp như mặc định
./target/release/taskbar_switcher.exe --combine-mode
```


## Giới hạn

- Chỉ hỗ trợ **Windows 11**
