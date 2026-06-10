//! Maps taskbar buttons to windows through multiple strategies: AUMID, PID, title, process name.

use tracing::{debug, error};
use windows::Win32::Foundation::HWND;

use super::{explorer, window};
use crate::types::{TaskbarButton, WindowInfo};

/// Mapping between taskbar buttons and visible windows.
///
/// Initialized once per cycle, used for:
/// - Finding window(s) for a button (`find_window_by_button`, `find_windows_by_button`)
/// - Finding the button index for the foreground window (`find_button_index_by_hwnd`)
pub(super) struct ButtonWindowMap<'a> {
    buttons: &'a [TaskbarButton],
    windows: &'a [WindowInfo],
}

impl<'a> ButtonWindowMap<'a> {
    /// Creates a new instance with the current list of buttons and windows.
    pub(super) fn new(buttons: &'a [TaskbarButton], windows: &'a [WindowInfo]) -> Self {
        Self { buttons, windows }
    }

    /// Finds the index of the taskbar button corresponding to the target window.
    ///
    /// Fast path: matches the window's AUMID with the button's automation_id.
    /// Slow path: reverse matching via `find_windows_for_button` for each button.
    pub(super) fn find_button_index_by_hwnd(&self, target_hwnd: HWND) -> Option<usize> {
        let fg_info = self.windows.iter().find(|w| w.hwnd == target_hwnd);
        let fg_name = fg_info.map(|w| w.title.as_str()).unwrap_or("<unknown>");

        if let Some(fg_aumid) = window::get_app_user_model_id(target_hwnd) {
            let fg_aumid_lower = fg_aumid.to_lowercase();
            for (i, button) in self.buttons.iter().enumerate() {
                if let Some(amuid) = &button.automation_id {
                    if !amuid.is_empty() {
                        let auto_id_lower = amuid.to_lowercase();
                        if auto_id_lower == fg_aumid_lower
                            || fg_aumid_lower.starts_with(&auto_id_lower)
                            || auto_id_lower.contains(&fg_aumid_lower)
                        {
                            debug!("Active button [{}]: '{}' AUMID '{}'", i, button.name, amuid);
                            return Some(i);
                        }
                    }
                }
            }
        }

        for (i, button) in self.buttons.iter().enumerate() {
            let windows = self.find_windows_by_button(button);

            if windows.iter().any(|w| w.hwnd == target_hwnd) {
                debug!(
                    "Active button [{}]: '{}' matches foreground '{}'",
                    i, button.name, fg_name
                );
                return Some(i);
            }
        }

        debug!(
            "No button match found for foreground '{}' (HWND {:?})",
            fg_name, target_hwnd
        );
        None
    }

    /// Finds a window corresponding to the button. Used for uncombine mode.
    pub(super) fn find_window_by_button(&self, button: &TaskbarButton) -> Option<WindowInfo> {
        self.match_windows_for_button(button).into_iter().next()
    }

    /// Finds all windows corresponding to the button. Used for combine mode (grouped buttons).
    pub(super) fn find_windows_by_button(&self, button: &TaskbarButton) -> Vec<WindowInfo> {
        self.match_windows_for_button(button)
    }

    /// Core matching logic: maps button -> window(s) through 4 strategies.
    ///
    /// 1. **AppUserModelID** - matches `button.automation_id` with `window.AppUserModelID`
    /// 2. **PID** - if button PID is a real app (not explorer)
    /// 3. **Title** - fuzzy match after `clean_button_name`
    /// 4. **Process name** - matches executable file name with button name
    fn match_windows_for_button(&self, button: &TaskbarButton) -> Vec<WindowInfo> {
        // Strategy 1: AppUserModelID
        if let Some(auto_id) = &button.automation_id {
            if !auto_id.is_empty() {
                let auto_id_lower = auto_id.to_lowercase();
                let appid_matches: Vec<WindowInfo> = self
                    .windows
                    .iter()
                    .filter(|w| {
                        let window_aumid = match window::get_app_user_model_id(w.hwnd) {
                            Some(aumid) => aumid,
                            None => return false,
                        };

                        let window_aumid_lower = window_aumid.to_lowercase();
                        window_aumid_lower == auto_id_lower
                            || window_aumid_lower.starts_with(&auto_id_lower)
                            || auto_id_lower.contains(&window_aumid_lower)
                    })
                    .cloned()
                    .collect();

                if !appid_matches.is_empty() {
                    debug!(
                        "Button '{}' have AppUserModelID match, found {} windows",
                        button.name,
                        appid_matches.len()
                    );

                    let mut sorted = appid_matches;
                    sorted.sort_by(|a, b| {
                        a.rect
                            .left
                            .cmp(&b.rect.left)
                            .then_with(|| a.hwnd.0.cmp(&b.hwnd.0))
                    });

                    return sorted;
                }
            }
        }

        // Strategy 2: PID
        let explorer_pid = explorer::get_explorer_pid();
        let pid_is_real_app = button.process_id > 0 && button.process_id != explorer_pid as i32;
        if pid_is_real_app {
            let pid_matches: Vec<WindowInfo> = self
                .windows
                .iter()
                .filter(|w| w.process_id == button.process_id as u32 && !w.title.is_empty())
                .cloned()
                .collect();

            if !pid_matches.is_empty() {
                debug!(
                    "Button '{}' have PID match, found {} windows",
                    button.name,
                    pid_matches.len()
                );

                let mut sorted = pid_matches;
                sorted.sort_by(|a, b| {
                    a.rect
                        .left
                        .cmp(&b.rect.left)
                        .then_with(|| a.hwnd.0.cmp(&b.hwnd.0))
                });

                return sorted;
            }
        }

        // Strategy 3: Title (fuzzy)
        let clean_name = crate::utils::clean_button_name(&button.name);

        if clean_name.is_empty() {
            error!("Cannot find windows: no PID, AppUserModelID, or clean name available");
            return Vec::new();
        }

        let title_matches: Vec<WindowInfo> = self
            .windows
            .iter()
            .filter(|w| {
                if w.title.is_empty() {
                    return false;
                }
                let w_clean = crate::utils::clean_button_name(&w.title);
                !w_clean.is_empty()
                    && (w_clean.contains(&clean_name) || clean_name.contains(&w_clean))
            })
            .cloned()
            .collect();

        if !title_matches.is_empty() {
            debug!(
                "Button '{}' have title match, found {} windows",
                button.name,
                title_matches.len()
            );

            let mut sorted = title_matches;
            sorted.sort_by(|a, b| {
                a.rect
                    .left
                    .cmp(&b.rect.left)
                    .then_with(|| a.hwnd.0.cmp(&b.hwnd.0))
            });

            return sorted;
        }

        // Strategy 4: Process name
        let clean_name_lower = clean_name.to_lowercase();
        let process_matches: Vec<WindowInfo> = self
            .windows
            .iter()
            .filter(|w| {
                if w.process_name.is_empty() {
                    return false;
                }
                let proc_lower = w.process_name.to_lowercase();
                let proc_stem = proc_lower.strip_suffix(".exe").unwrap_or(&proc_lower);
                !proc_stem.is_empty()
                    && (proc_stem.contains(&clean_name_lower)
                        || clean_name_lower.contains(proc_stem))
            })
            .cloned()
            .collect();

        if !process_matches.is_empty() {
            debug!(
                "Button '{}' have process match, found {} windows",
                button.name,
                process_matches.len()
            );

            let mut sorted = process_matches;
            sorted.sort_by(|a, b| {
                a.rect
                    .left
                    .cmp(&b.rect.left)
                    .then_with(|| a.hwnd.0.cmp(&b.hwnd.0))
            });

            return sorted;
        }

        debug!(
            "Button '{}' matching failed: pid_is_real={}, explorer_pid={}, button_pid={}, \
             auto_id={:?}, clean_name='{}'",
            button.name,
            pid_is_real_app,
            explorer_pid,
            button.process_id,
            button.automation_id,
            clean_name,
        );

        for w in self.windows.iter() {
            if w.title.to_lowercase().contains(&clean_name.to_lowercase())
                || w.process_name
                    .to_lowercase()
                    .contains(&clean_name.to_lowercase())
            {
                debug!(
                    "Candidate: title='{}' process='{}' pid={} hwnd={:?}",
                    w.title, w.process_name, w.process_id, w.hwnd
                );
            }
        }

        Vec::new()
    }
}
