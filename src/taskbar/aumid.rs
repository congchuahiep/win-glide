use windows::Win32::{
    Foundation::{CloseHandle, HWND},
    Storage::{EnhancedStorage::PKEY_AppUserModel_ID, Packaging::Appx::GetApplicationUserModelId},
    System::{
        Com::StructuredStorage::{InitPropVariantFromStringAsVector, PROPVARIANT},
        Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    },
    UI::{
        Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow},
        WindowsAndMessaging::GetWindowThreadProcessId,
    },
};
use windows_core::{BSTR, HSTRING, PWSTR};

/// Sets the AppUserModelID for a window.
pub fn set_aumid(hwnd: HWND, aumid: Option<&str>) -> Result<(), windows::core::Error> {
    unsafe {
        let store: IPropertyStore = SHGetPropertyStoreForWindow(hwnd)?;
        let prop = match aumid {
            Some(s) => InitPropVariantFromStringAsVector(&HSTRING::from(s))?,
            None => PROPVARIANT::default(),
        };
        store.SetValue(&PKEY_AppUserModel_ID, &prop)
    }
}

/// Gets AppUserModelID from a window via `SHGetPropertyStoreForWindow`.
///
/// This is the official mechanism Windows uses to group taskbar buttons.
pub fn get_aumid(hwnd: HWND) -> Option<String> {
    fn get_window_aumid(hwnd: HWND) -> Option<String> {
        unsafe {
            let store: IPropertyStore = match SHGetPropertyStoreForWindow(hwnd) {
                Ok(s) => s,
                Err(_) => return None,
            };

            let variant: PROPVARIANT = match store.GetValue(&PKEY_AppUserModel_ID) {
                Ok(v) => v,
                Err(_) => return None,
            };

            if variant.is_empty() {
                return None;
            }

            let bstr = match BSTR::try_from(&variant) {
                Ok(b) => b,
                Err(_) => return None,
            };

            let s = bstr.to_string();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        }
    }

    fn get_process_aumid(hwnd: HWND) -> Option<String> {
        unsafe {
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == 0 {
                return None;
            }

            let h_process = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                Ok(h) => h,
                Err(_) => return None,
            };

            let mut length: u32 = 0;
            // Gọi lần 1 để lấy độ dài chuỗi cần cấp phát
            let _ = GetApplicationUserModelId(h_process, &mut length, None);

            if length > 0 {
                let mut buffer: Vec<u16> = vec![0; length as usize];
                if GetApplicationUserModelId(
                    h_process,
                    &mut length,
                    Some(PWSTR(buffer.as_mut_ptr())),
                )
                .is_ok()
                {
                    // Độ dài đã bao gồm null terminator nên trừ đi 1
                    let len = (length - 1) as usize;
                    let s = String::from_utf16_lossy(&buffer[0..len]);
                    let _ = CloseHandle(h_process);
                    return Some(s);
                }
            }
            let _ = CloseHandle(h_process);
            None
        }
    }

    if let Some(aumid) = get_window_aumid(hwnd) {
        Some(aumid)
    } else {
        get_process_aumid(hwnd)
    }
}
