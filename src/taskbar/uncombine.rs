//! Quản lý việc tách (uncombine) các taskbar button.
//!
//! Trên Windows, taskbar mặc định gộp (`combine`) nhiều cửa sổ cùng app vào một button duy nhất.
//! Cơ chế này dựa trên **AppUserModelID (AUMID)**: các cửa sổ có cùng AUMID được Windows gộp vào
//! chung một group.
//!
//! `UncombineManager` gán cho mỗi cửa sổ một AUMID **duy nhất** dạng `TaskbarSwitcher_<HWND>`,
//! khiến Windows coi mỗi cửa sổ là một app riêng biệt -> mỗi cửa sổ có taskbar button riêng.
//!
//! AUMID gốc của mỗi cửa sổ được lưu lại để có thể **khôi phục** khi thoát app (tránh làm hỏng
//! trạng thái hệ thống).
//!
//! # Luồng hoạt động
//!
//! ```text
//! 1. App start -> uncombine_all() duyệt tất cả window, set AUMID riêng
//! 2. WinEvent hook -> phát hiện window mới -> uncombine_one(hwnd)
//! 3. App exit (Ctrl+C) -> restore_all() khôi phục AUMID gốc
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

/// Quản lý uncombine: lưu AUMID gốc và set AUMID riêng cho từng cửa sổ.
///
/// # Thread safety
///
/// `original_aumids` được bảo vệ bởi `Mutex` vì:
/// - Main thread gọi `uncombine_all()` / `uncombine_one()` / `restore_all()`
/// - WinEvent callback thread gọi `is_tracked()` để filter trước khi post message
pub struct UncombineManager {
    /// `hwnd_val (isize) -> AUMID gốc`.
    ///
    /// - `Some("App.Aumid")` - cửa sổ có AUMID gốc
    /// - `None` - cửa sổ vốn không có AUMID (VD: console window)
    original_aumids: Mutex<HashMap<isize, Option<String>>>,
}

impl UncombineManager {
    /// Tạo instance mới với map rỗng.
    pub fn new() -> Self {
        Self {
            original_aumids: Mutex::new(HashMap::new()),
        }
    }

    /// Kiểm tra cửa sổ đã được theo dõi bởi uncombine chưa.
    ///
    /// Dùng tromg WinEvent callback để filter nhanh trước khi post messag tránh gửi message vô ích
    /// cho cửa sổ đã uncombine.
    pub fn is_tracked(&self, hwnd: HWND) -> bool {
        self.original_aumids
            .lock()
            .unwrap()
            .contains_key(&(hwnd.0 as isize))
    }

    /// Uncombine **tất cả** cửa sổ đang visible trên desktop.
    ///
    /// Chỉ uncombine khi phát hiện có từ 2 cửa sổ trở lên thuộc cùng một app (cùng AUMID hoặc tên tiến trình).
    /// Cửa sổ đầu tiên (anchor) sẽ giữ nguyên AUMID để không bị mất icon.
    #[instrument(level = "debug", skip_all)]
    pub fn uncombine_all(&self) {
        let windows = find_visible_windows();
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

    /// Uncombine **một** cửa sổ mới xuất hiện.
    ///
    /// Chỉ uncombine nếu đã có một cửa sổ khác của cùng app đang hiển thị (anchor window).
    #[instrument(level = "debug", skip_all)]
    pub fn uncombine_one(&self, hwnd: HWND, on_success: impl FnOnce()) {
        let hwnd_val = hwnd.0 as isize;
        let mut map = self.original_aumids.lock().unwrap();

        if map.contains_key(&hwnd_val) {
            return;
        }

        let windows = find_visible_windows();
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

    /// Khôi phục **tất cả** AUMID gốc.
    ///
    /// Duyệt toàn bộ map, với mỗi cửa sổ:
    /// - Nếu có AUMID gốc: set lại AUMID gốc
    /// - Nếu `None` (vốn không có AUMID): xóa AUMID property (gán VT_EMPTY)
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

/// Set AppUserModelID cho một cửa sổ.
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
