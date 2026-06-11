//! Maps taskbar buttons to windows through multiple strategies: AUMID, PID, title, process name.

use std::collections::{HashMap, HashSet};

use tracing::{debug, error, instrument};
use windows::Win32::Foundation::HWND;

use super::explorer;
use crate::{
    types::{TaskbarButton, WindowInfo},
    utils,
};

/// Mapping between taskbar buttons and visible windows.
///
/// Initialized once per cycle, used for:
/// - Finding window(s) for a button (`find_window_by_button`, `find_windows_by_button`)
/// - Finding the button index for the foreground window (`find_button_index_by_hwnd`)
pub(super) struct ButtonWindowMap<'a> {
    buttons: &'a [TaskbarButton],
    windows: &'a [WindowInfo],

    /// The result
    mapping: HashMap<usize, Vec<WindowInfo>>,
}

impl<'a> ButtonWindowMap<'a> {
    /// Creates a new instance with the current list of buttons and windows.
    pub(super) fn new(buttons: &'a [TaskbarButton], windows: &'a [WindowInfo]) -> Self {
        let mut consumed_hwnds = HashSet::new();

        let mut this = Self {
            buttons,
            windows,
            mapping: HashMap::new(),
        };

        for (i, button) in buttons.iter().enumerate() {
            let candidates = this.match_windows_for_button(button);

            let mut assigned_windows = Vec::new();

            for w in candidates {
                // NẾU CỬA SỔ NÀY CHƯA AI LẤY
                if !consumed_hwnds.contains(&w.hwnd.0) {
                    assigned_windows.push(w.clone());
                    consumed_hwnds.insert(w.hwnd.0); // ĐÁNH DẤU ĐÃ LẤY!
                }
            }

            // Gắn danh sách cửa sổ hợp pháp vào Button này
            this.mapping.insert(i, assigned_windows);
        }

        this
    }

    /// Finds a window corresponding to the button. Used for uncombine mode.
    pub(super) fn find_window_by_button(
        &self,
        target_button: &TaskbarButton,
    ) -> Option<WindowInfo> {
        // Tra cứu xem target_button nằm ở index thứ mấy
        if let Some(pos) = self
            .buttons
            .iter()
            .position(|b| std::ptr::eq(b, target_button))
        {
            // Trả về danh sách cửa sổ đã được chia từ trước
            if let Some(assigned) = self.mapping.get(&pos) {
                return assigned.iter().next().cloned();
            }
        }

        None
    }

    /// Finds all windows corresponding to the button. Used for combine mode (grouped buttons).
    pub(super) fn find_windows_by_button(&self, target_button: &TaskbarButton) -> Vec<WindowInfo> {
        // Tra cứu xem target_button nằm ở index thứ mấy
        if let Some(pos) = self
            .buttons
            .iter()
            .position(|b| std::ptr::eq(b, target_button))
        {
            // Trả về danh sách cửa sổ đã được chia từ trước
            if let Some(assigned) = self.mapping.get(&pos) {
                return assigned.clone();
            }
        }
        Vec::new()
    }

    /// Finds the index of the taskbar button corresponding to the target window.
    pub(super) fn find_button_index_by_hwnd(&self, target_hwnd: HWND) -> Option<usize> {
        for (index, windows) in &self.mapping {
            if windows.iter().any(|w| w.hwnd == target_hwnd) {
                return Some(*index);
            }
        }
        None
    }

    /// Core matching logic: maps button -> window(s) through 5 strategies.
    ///
    /// 1. **AppUserModelID** - matches `button.automation_id` with `window.AppUserModelID`
    /// 2. **PID** - if button PID is a real app (not explorer)
    /// 3. **Implicit AUMID EXE** - matches executable file name with button name (for win32 app)
    /// 4. **Title** - fuzzy match after `clean_button_name`
    /// 5. **Process name** - matches executable file name with button name
    #[instrument(level = "debug", skip_all)]
    fn match_windows_for_button(&self, button: &TaskbarButton) -> Vec<WindowInfo> {
        if let Some(matches) = self.match_by_aumid(button) {
            return matches;
        }
        if let Some(matches) = self.match_by_pid(button) {
            return matches;
        }
        if let Some(matches) = self.match_by_implicit_aumid_exe(button) {
            return matches;
        }

        let clean_name = utils::clean_button_name(&button.name);
        if clean_name.is_empty() {
            error!("Cannot find windows: no PID, AppUserModelID, or clean name available");
            return Vec::new();
        }

        if let Some(matches) = self.match_by_title(button, &clean_name) {
            return matches;
        }
        if let Some(matches) = self.match_by_process_name(button, &clean_name) {
            return matches;
        }

        debug!("Matching failed");
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

    /// Sorts matched windows left-to-right (based on screen coordinates) and by HWND as fallback.
    ///
    /// This finalizes may not useful as all?
    ///
    /// Returns `Some(matches)` if not empty, otherwise `None`.
    fn finalize_matches(
        &self,
        button: &TaskbarButton,
        mut matches: Vec<WindowInfo>,
        strategy: &str,
    ) -> Option<Vec<WindowInfo>> {
        if matches.is_empty() {
            return None;
        }
        debug!(
            "Button '{}' button_pid={}, auto_id={:?} have {} match, found {} windows",
            button.name,
            button.process_id,
            button.automation_id,
            strategy,
            matches.len()
        );
        matches.sort_by(|a, b| {
            a.rect
                .left
                .cmp(&b.rect.left)
                .then_with(|| a.hwnd.0.cmp(&b.hwnd.0))
        });
        Some(matches)
    }

    /// Checks if a window's explicit AUMID is compatible with the button's AUMID.
    /// Used to prevent stealing windows that have been assigned a different custom AUMID.
    fn is_aumid_compatible(&self, w: &WindowInfo, button: &TaskbarButton) -> bool {
        if let Some(win_aumid) = w.aumid.as_ref() {
            let w_lower = win_aumid.to_lowercase();
            let is_match = match &button.automation_id {
                Some(btn_aumid) => {
                    let b_lower = btn_aumid.to_lowercase();
                    w_lower == b_lower || b_lower.contains(&w_lower) || w_lower.contains(&b_lower)
                }
                None => false,
            };
            if !is_match {
                return false;
            }
        }
        true
    }

    /// Strategy 1: Matches using explicit `AppUserModelID`.
    ///
    /// The most reliable method. Direct match between button's `automation_id` and window's AUMID.
    fn match_by_aumid(&self, button: &TaskbarButton) -> Option<Vec<WindowInfo>> {
        let auto_id = button.automation_id.as_ref()?;
        if auto_id.is_empty() {
            return None;
        }

        let auto_id_lower = auto_id.to_lowercase();
        let matches: Vec<WindowInfo> = self
            .windows
            .iter()
            .filter(|w| {
                let window_aumid = match w.aumid.as_ref() {
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

        self.finalize_matches(button, matches, "AppUserModelID")
    }

    /// Strategy 2: Matches using Process ID (`PID`).
    ///
    /// Relies on the process ID if the taskbar button does not belong to explorer.exe.
    fn match_by_pid(&self, button: &TaskbarButton) -> Option<Vec<WindowInfo>> {
        let explorer_pid = explorer::get_explorer_pid();
        if button.process_id == 0 || button.process_id == explorer_pid as i32 {
            return None;
        }

        let matches: Vec<WindowInfo> = self
            .windows
            .iter()
            .filter(|w| w.process_id == button.process_id as u32 && !w.title.is_empty())
            .cloned()
            .collect();

        self.finalize_matches(button, matches, "PID")
    }

    /// Strategy 3: Implicit AUMID Executable Match.
    ///
    /// Parses the button's implicit AUMID generated by Windows for legacy Win32 apps.
    /// Extracts the `.exe` name from the AUMID and matches it against the window's process name.
    fn match_by_implicit_aumid_exe(&self, button: &TaskbarButton) -> Option<Vec<WindowInfo>> {
        let auto_id = button.automation_id.as_ref()?;
        let auto_id_lower = auto_id.to_lowercase();

        if !auto_id_lower.ends_with(".exe") {
            return None;
        }
        let idx = auto_id_lower.rfind('\\')?;
        let extracted_exe = &auto_id_lower[idx + 1..];

        let matches: Vec<WindowInfo> = self
            .windows
            .iter()
            .filter(|w| {
                if !self.is_aumid_compatible(w, button) {
                    return false;
                }
                w.process_name.to_lowercase() == extracted_exe
            })
            .cloned()
            .collect();

        self.finalize_matches(button, matches, "Implicit AUMID EXE")
    }

    /// Strategy 4: Matches using Window Title (Fuzzy).
    ///
    /// Cross-checks the cleaned button name against the window's title.
    /// Protected by `is_aumid_compatible` to prevent stealing uncombined windows.
    fn match_by_title(&self, button: &TaskbarButton, clean_name: &str) -> Option<Vec<WindowInfo>> {
        let matches: Vec<WindowInfo> = self
            .windows
            .iter()
            .filter(|w| {
                if w.title.is_empty() {
                    return false;
                }
                if !self.is_aumid_compatible(w, button) {
                    return false;
                }

                let w_clean = utils::clean_button_name(&w.title);
                !w_clean.is_empty()
                    && (w_clean.contains(clean_name) || clean_name.contains(&w_clean))
            })
            .cloned()
            .collect();

        self.finalize_matches(button, matches, "Title")
    }

    /// Strategy 5: Matches using Process Name (Fuzzy).
    ///
    /// Cross-checks the cleaned button name against the window's process name.
    /// Protected by `is_aumid_compatible` to prevent stealing uncombined windows.
    fn match_by_process_name(
        &self,
        button: &TaskbarButton,
        clean_name: &str,
    ) -> Option<Vec<WindowInfo>> {
        let clean_name_lower = clean_name.to_lowercase();
        let matches: Vec<WindowInfo> = self
            .windows
            .iter()
            .filter(|w| {
                if w.process_name.is_empty() {
                    return false;
                }
                if !self.is_aumid_compatible(w, button) {
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

        self.finalize_matches(button, matches, "Process")
    }
}
