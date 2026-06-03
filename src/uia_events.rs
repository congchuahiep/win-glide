//! UIA StructureChanged event handler — phát hiện thêm/xoá taskbar buttons.
//!
//! # Tại sao dùng UIA thay vì WinEvent?
//!
//! UIA StructureChanged trực tiếp theo dõi cây UI của taskbar, phát hiện
//! khi một `TaskListButtonAutomationPeer` được thêm hoặc xoá khỏi tree.
//! Độ chính xác cao hơn WinEvent vì theo dõi chính xác taskbar, không cần
//! filter by HWND.
//!
//! # Reorder detection
//!
//! `StructureChangeType_ChildrenReordered` KHÔNG fire trên Win11 XAML taskbar.
//! Reorder được xử lý bởi TTL cache (safety net trong taskbar.rs).

use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
use tracing::debug;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::SAFEARRAY;
use windows::Win32::UI::Accessibility::{
    IUIAutomation, IUIAutomationCacheRequest, IUIAutomationElement,
    IUIAutomationStructureChangedEventHandler, IUIAutomationStructureChangedEventHandler_Impl,
    StructureChangeType, StructureChangeType_ChildAdded, StructureChangeType_ChildRemoved,
    StructureChangeType_ChildrenBulkAdded, StructureChangeType_ChildrenBulkRemoved,
    StructureChangeType_ChildrenInvalidated, TreeScope_Subtree,
};

use crate::winevent::{CACHE_INVALIDATED, WM_APP_INVALIDATE_CACHE};

static MAIN_THREAD_ID: AtomicU32 = AtomicU32::new(0);
static HANDLER_PTR: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

/// COM event handler cho StructureChanged.
///
/// `#[implement]` macro tạo COM object với IUnknown + IUIAutomationStructureChangedEventHandler.
#[windows_core::implement(IUIAutomationStructureChangedEventHandler)]
struct StructureChangedHandler {}

impl IUIAutomationStructureChangedEventHandler_Impl for StructureChangedHandler_Impl {
    #[allow(non_upper_case_globals)]
    fn HandleStructureChangedEvent(
        &self,
        _sender: windows_core::Ref<'_, IUIAutomationElement>,
        changetype: StructureChangeType,
        _runtimeid: *const SAFEARRAY,
    ) -> windows_core::Result<()> {
        let thread_id = MAIN_THREAD_ID.load(Ordering::SeqCst);
        if thread_id == 0 {
            return Ok(());
        }

        match changetype {
            t if t == StructureChangeType_ChildAdded
                || t == StructureChangeType_ChildRemoved
                || t == StructureChangeType_ChildrenInvalidated
                || t == StructureChangeType_ChildrenBulkAdded
                || t == StructureChangeType_ChildrenBulkRemoved =>
            {
                if !CACHE_INVALIDATED.swap(true, Ordering::SeqCst) {
                    unsafe {
                        let _ = windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                            thread_id,
                            WM_APP_INVALIDATE_CACHE,
                            WPARAM(changetype.0 as usize),
                            LPARAM(0),
                        );
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }
}

/// Đăng ký UIA StructureChanged event handler trên taskbar element.
///
/// Handler được leak để giữ alive suốt app lifetime (chỉ uninstall khi thoát).
///
/// # Safety
///
/// Phải gọi trên cùng thread với message loop.
pub unsafe fn install_uia_handler(
    automation: &IUIAutomation,
    taskbar_hwnd: HWND,
    main_thread_id: u32,
) -> anyhow::Result<()> {
    MAIN_THREAD_ID.store(main_thread_id, Ordering::SeqCst);

    let taskbar_element = automation.ElementFromHandle(taskbar_hwnd)?;

    // Box::leak để giữ handler alive suốt app lifetime
    let handler: IUIAutomationStructureChangedEventHandler = StructureChangedHandler {}.into();
    let handler_ref: &'static IUIAutomationStructureChangedEventHandler =
        Box::leak(Box::new(handler));

    automation.AddStructureChangedEventHandler(
        &taskbar_element,
        TreeScope_Subtree,
        None::<&IUIAutomationCacheRequest>,
        handler_ref,
    )?;

    // Lưu raw pointer cho cleanup
    HANDLER_PTR.store(
        handler_ref as *const IUIAutomationStructureChangedEventHandler as *mut _,
        Ordering::SeqCst,
    );

    debug!("UIA StructureChanged event handler installed");
    Ok(())
}

/// Gỡ bỏ UIA event handler.
///
/// # Safety
///
/// Phải gọi trên cùng thread với `install_uia_handler`.
pub unsafe fn uninstall_uia_handler(automation: &IUIAutomation, taskbar_hwnd: HWND) {
    let handler_ptr = HANDLER_PTR.swap(std::ptr::null_mut(), Ordering::SeqCst);
    if handler_ptr.is_null() {
        return;
    }

    if let Ok(taskbar_element) = automation.ElementFromHandle(taskbar_hwnd) {
        let handler: &IUIAutomationStructureChangedEventHandler = &*(handler_ptr as *const _);
        let _ = automation.RemoveStructureChangedEventHandler(&taskbar_element, handler);
    }

    debug!("UIA StructureChanged event handler uninstalled");
}
