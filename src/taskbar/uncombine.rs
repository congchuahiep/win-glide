//! Manages uncombining taskbar buttons.
//!
//! On Windows, the taskbar defaults to grouping (`combine`) multiple windows of the same app into a single button.
//! This mechanism is based on the **AppUserModelID (AUMID)**: windows with the same AUMID are grouped by Windows.
//!
//! `UncombineManager` assigns each window a **unique** AUMID in the format `TaskbarSwitcher_<HWND>`,
//! making Windows treat each window as a separate app -> each window gets its own taskbar button.
//!
//! The original AUMID of each window is saved so it can be **restored** upon app exit (to avoid corrupting system state).
//!
//! # Workflow
//!
//! ```text
//! 1. App start -> uncombine_all() iterates all windows, sets unique AUMID
//! 2. WinEvent hook -> detects new window -> uncombine_one(hwnd)
//! 3. App exit (Ctrl+C) -> restore_all() restores original AUMID
//! ```

use std::collections::HashMap;
use std::sync::Mutex;
use tracing::{debug, error, instrument};
use windows::core::HSTRING;
use windows::Win32::Foundation::HWND;
use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
use windows::Win32::System::Com::StructuredStorage::{
    InitPropVariantFromStringAsVector, PROPVARIANT,
};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};

use super::window::{find_visible_windows, get_app_user_model_id};
use crate::utils::truncate;

/// Manages uncombine: saves original AUMID and sets unique AUMID for each window.
///
/// # Thread safety
///
/// `original_aumids` is protected by `Mutex` because:
/// - Main thread calls `uncombine_all()` / `uncombine_one()` / `restore_all()`
/// - WinEvent callback thread calls `is_tracked()` to filter before posting message
pub struct UncombineManager {
    /// `hwnd_val (isize) -> original AUMID`.
    ///
    /// - `Some("App.Aumid")` - window has an original AUMID
    /// - `None` - window didn't have an AUMID (e.g., console window)
    original_aumids: Mutex<HashMap<isize, Option<String>>>,
}

impl UncombineManager {
    /// Creates a new instance with an empty map.
    pub fn new() -> Self {
        Self {
            original_aumids: Mutex::new(HashMap::new()),
        }
    }

    /// Checks if a window is already tracked by uncombine.
    ///
    /// Used in WinEvent callback for quick filtering before posting message to avoid sending useless
    /// messages for already uncombined windows.
    pub fn is_tracked(&self, hwnd: HWND) -> bool {
        self.original_aumids
            .lock()
            .unwrap()
            .contains_key(&(hwnd.0 as isize))
    }

    /// Uncombines **all** visible windows on the desktop.
    ///
    /// Only uncombines when 2 or more windows of the same app (same AUMID or process name) are detected.
    /// The first window (anchor) will keep its original AUMID to not lose its icon.
    #[instrument(level = "debug", skip_all)]
    pub fn uncombine_all(&self) {
        let windows = find_visible_windows(None);
        let mut map = self.original_aumids.lock().unwrap();

        let mut group_counts = HashMap::new();
        let mut window_keys = Vec::new();

        for w in &windows {
            let key = match get_app_user_model_id(w.hwnd) {
                Some(aumid) => format!("AUMID:{}", aumid),
                None => format!("EXE:{}", w.process_name),
            };
            *group_counts.entry(key.clone()).or_insert(0) += 1;
            window_keys.push((key, w));
        }

        let mut spared_groups = std::collections::HashSet::new();

        for (key, w) in window_keys {
            let hwnd_val = w.hwnd.0 as isize;

            if map.contains_key(&hwnd_val) {
                continue;
            }

            let count = group_counts.get(&key).unwrap_or(&1);
            if *count <= 1 {
                continue;
            }

            if !spared_groups.contains(&key) {
                spared_groups.insert(key);
                continue;
            }

            let original = get_app_user_model_id(w.hwnd);
            let new_aumid = format!("TaskbarSwitcher_{}", hwnd_val);
            map.insert(hwnd_val, original.clone());

            match set_aumid(w.hwnd, Some(&new_aumid)) {
                Ok(()) => debug!(
                    "'{}' has been uncombined '{}' to '{}'",
                    truncate(&w.title, 30),
                    original.as_deref().unwrap_or("None"),
                    new_aumid
                ),
                Err(e) => error!(
                    "Failed to set AUMID for {}: {:?}",
                    truncate(&w.title, 30),
                    e,
                ),
            }
        }
    }

    /// Uncombines a **single** new window.
    ///
    /// Only uncombines if there is another window of the same app already visible (anchor window).
    #[instrument(level = "debug", skip_all)]
    pub fn uncombine_one(&self, hwnd: HWND, on_success: impl FnOnce()) {
        let hwnd_val = hwnd.0 as isize;
        let mut map = self.original_aumids.lock().unwrap();

        if map.contains_key(&hwnd_val) {
            return;
        }

        let windows = find_visible_windows(None);
        let target_w = windows.iter().find(|w| w.hwnd == hwnd);
        if target_w.is_none() {
            return;
        }
        let target_w = target_w.unwrap();

        let target_key = match get_app_user_model_id(hwnd) {
            Some(aumid) => format!("AUMID:{}", aumid),
            None => format!("EXE:{}", target_w.process_name),
        };

        let mut count = 0;
        for w in &windows {
            let key = match get_app_user_model_id(w.hwnd) {
                Some(aumid) => format!("AUMID:{}", aumid),
                None => format!("EXE:{}", w.process_name),
            };
            if key == target_key {
                count += 1;
            }
        }

        if count <= 1 {
            debug!(
                "uncombine_one: Spared {:?} (key: {}), count is {}",
                hwnd, target_key, count
            );
            return;
        }

        let original = get_app_user_model_id(hwnd);
        let new_aumid = format!("TaskbarSwitcher_{}", hwnd_val);
        map.insert(hwnd_val, original.clone());

        match set_aumid(hwnd, Some(&new_aumid)) {
            Ok(()) => {
                debug!(
                    "New window {:?} has been uncombined '{}' to '{}'",
                    hwnd,
                    original.as_deref().unwrap_or("None"),
                    new_aumid
                );
                on_success();
            }
            Err(e) => error!("Failed to set AUMID for {:?}: {:?}", hwnd, e),
        }
    }

    /// Restores **all** original AUMIDs.
    ///
    /// Iterates the entire map, for each window:
    /// - If it had an original AUMID: restores it
    /// - If `None` (no original AUMID): clears the AUMID property (assigns VT_EMPTY)
    #[instrument(level = "debug", skip_all)]
    pub fn restore_all(&self) {
        debug!("Restoring original AppUserModelIDs");

        let mut map = self.original_aumids.lock().unwrap();

        for (&hwnd_val, original) in map.iter() {
            let hwnd = HWND(hwnd_val as *mut _);

            debug!("Restoring {:?}: '{:?}'", hwnd, original);
            let _ = set_aumid(hwnd, original.as_deref());
        }

        map.clear();
    }
}

impl Drop for UncombineManager {
    fn drop(&mut self) {
        self.restore_all();
    }
}

/// Sets the AppUserModelID for a window.
fn set_aumid(hwnd: HWND, aumid: Option<&str>) -> Result<(), windows::core::Error> {
    unsafe {
        let store: IPropertyStore = SHGetPropertyStoreForWindow(hwnd)?;
        let prop = match aumid {
            Some(s) => InitPropVariantFromStringAsVector(&HSTRING::from(s))?,
            None => PROPVARIANT::default(),
        };
        store.SetValue(&PKEY_AppUserModel_ID, &prop)
    }
}
