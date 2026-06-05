//! UIA StructureChanged event handler, phát hiện thêm/xoá taskbar buttons.
//!
//! # Reorder detection
//!
//! StructureChangeType_ChildrenReordered không fire trên Win11 XAML taskbar.
//! Reorder được xử lý bởi TTL cache (safety net trong taskbar.rs).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
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
use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;

use super::winevent::InvalidateSource;

pub const WM_APP_INVALIDATE_CACHE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_USER + 0x101;

/// Cờ chống gửi message trùng khi nhiều UIA event fire liên tiếp.
pub static CACHE_INVALIDATED: AtomicBool = AtomicBool::new(false);

static MAIN_THREAD_ID: AtomicU32 = AtomicU32::new(0);

/// COM event handler cho StructureChanged.
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

        if let Some(source) = match changetype {
            StructureChangeType_ChildAdded => Some(InvalidateSource::ButtonAdded),
            StructureChangeType_ChildRemoved => Some(InvalidateSource::ButtonRemoved),
            StructureChangeType_ChildrenInvalidated => Some(InvalidateSource::ButtonInvalidated),
            StructureChangeType_ChildrenBulkAdded => Some(InvalidateSource::ButtonBulkAdded),
            StructureChangeType_ChildrenBulkRemoved => Some(InvalidateSource::ButtonBulkRemoved),
            _ => None,
        } {
            if !CACHE_INVALIDATED.swap(true, Ordering::SeqCst) {
                unsafe {
                    let _ = PostThreadMessageW(
                        thread_id,
                        WM_APP_INVALIDATE_CACHE,
                        WPARAM(source as usize),
                        LPARAM(0),
                    );
                }
            }
        }

        Ok(())
    }
}

/// UIA event hook RAII - install khi tạo, auto-uninstall khi Drop.
///
/// Handler COM object được leak (Box::leak) để giữ alive suốt app lifetime.
pub struct UiaEventHook {
    automation: IUIAutomation,
    taskbar_element: IUIAutomationElement,
    handler: &'static IUIAutomationStructureChangedEventHandler,
}

impl UiaEventHook {
    /// Đăng ký StructureChanged event handler trên taskbar element.
    ///
    /// # Safety
    /// Phải gọi trên main thread (STA).
    pub unsafe fn install(
        automation: &IUIAutomation,
        taskbar_hwnd: HWND,
        main_thread_id: u32,
    ) -> anyhow::Result<Self> {
        MAIN_THREAD_ID.store(main_thread_id, Ordering::SeqCst);

        let taskbar_element = automation.ElementFromHandle(taskbar_hwnd)?;
        let handler: IUIAutomationStructureChangedEventHandler = StructureChangedHandler {}.into();
        let handler: &'static IUIAutomationStructureChangedEventHandler =
            Box::leak(Box::new(handler));

        automation.AddStructureChangedEventHandler(
            &taskbar_element,
            TreeScope_Subtree,
            None::<&IUIAutomationCacheRequest>,
            handler,
        )?;

        debug!("UIA StructureChanged event handler installed");
        Ok(Self {
            automation: automation.clone(),
            taskbar_element,
            handler,
        })
    }
}

impl Drop for UiaEventHook {
    fn drop(&mut self) {
        unsafe {
            let _ = self
                .automation
                .RemoveStructureChangedEventHandler(&self.taskbar_element, self.handler);
        }
        debug!("UIA StructureChanged event handler uninstalled");
    }
}

/// Reset cờ "cache invalidated" sau khi main thread đã xử lý.
pub fn reset_cache_invalidated_flag() {
    CACHE_INVALIDATED.store(false, Ordering::SeqCst);
}
