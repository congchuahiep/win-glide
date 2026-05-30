use anyhow::Result;
use windows::core::w;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationCondition, IUIAutomationElementArray,
    TreeScope_Descendants,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, FindWindowExW};

const TASKBAR_BUTTON_CLASS: &str = "Taskbar.TaskListButtonAutomationPeer";

#[derive(Debug, Clone)]
pub struct TaskbarButton {
    pub name: String,
    pub rect: windows::Win32::Foundation::RECT,
    pub process_id: i32,
}

pub struct TaskbarEnumerator {
    automation: IUIAutomation,
    com_initialized: bool,
}

impl TaskbarEnumerator {
    pub fn new() -> Result<Self> {
        unsafe {
            let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let com_initialized = hr.is_ok();

            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;

            Ok(Self {
                automation,
                com_initialized,
            })
        }
    }

    pub fn enumerate_primary_buttons(&self) -> Result<Vec<TaskbarButton>> {
        let taskbar_hwnd = self.find_primary_taskbar_hwnd()?;
        unsafe { self.enumerate_buttons_for_hwnd(taskbar_hwnd) }
    }

    unsafe fn enumerate_buttons_for_hwnd(&self, root_hwnd: HWND) -> Result<Vec<TaskbarButton>> {
        let true_condition = self.automation.CreateTrueCondition()?;

        let mut all_buttons = Vec::new();

        let root_element = self.automation.ElementFromHandle(root_hwnd)?;

        let items = root_element.FindAll(TreeScope_Descendants, &true_condition)?;
        self.collect_buttons(&items, &mut all_buttons)?;

        if all_buttons.is_empty() {
            self.enumerate_via_bridge_windows(root_hwnd, &true_condition, &mut all_buttons)?;
        }

        all_buttons.sort_by_key(|b| b.rect.left);

        Ok(all_buttons)
    }

    unsafe fn collect_buttons(
        &self,
        items: &IUIAutomationElementArray,
        buttons: &mut Vec<TaskbarButton>,
    ) -> Result<()> {
        let count = items.Length()?;

        for i in 0..count {
            let item = items.GetElement(i)?;

            let class_name = item
                .CurrentClassName()
                .ok()
                .map(|b| b.to_string())
                .unwrap_or_default();

            if class_name == TASKBAR_BUTTON_CLASS {
                let name = item
                    .CurrentName()
                    .ok()
                    .map(|b| b.to_string())
                    .unwrap_or_default();

                let rect = match item.CurrentBoundingRectangle() {
                    Ok(r) => r,
                    Err(_) => continue,
                };

                let process_id = item.CurrentProcessId().unwrap_or(0);

                buttons.push(TaskbarButton {
                    name,
                    rect,
                    process_id,
                });
            }
        }

        Ok(())
    }

    unsafe fn enumerate_via_bridge_windows(
        &self,
        root_hwnd: HWND,
        condition: &IUIAutomationCondition,
        buttons: &mut Vec<TaskbarButton>,
    ) -> Result<()> {
        let mut child_hwnd = HWND::default();

        loop {
            child_hwnd = FindWindowExW(
                Some(root_hwnd),
                Some(child_hwnd),
                w!("Windows.UI.Composition.DesktopWindowContentBridge"),
                None,
            )
            .unwrap_or_default();

            if child_hwnd.0.is_null() {
                break;
            }

            if let Ok(bridge_element) = self.automation.ElementFromHandle(child_hwnd) {
                if let Ok(items) = bridge_element.FindAll(TreeScope_Descendants, condition) {
                    self.collect_buttons(&items, buttons)?;
                }
            }

            if !buttons.is_empty() {
                break;
            }
        }

        Ok(())
    }

    fn find_primary_taskbar_hwnd(&self) -> Result<HWND> {
        unsafe {
            let hwnd = FindWindowW(w!("Shell_TrayWnd"), None).unwrap_or_default();
            if hwnd.0.is_null() {
                anyhow::bail!("Shell_TrayWnd not found");
            }
            Ok(hwnd)
        }
    }

    pub fn find_active_button_index(
        &self,
        buttons: &[TaskbarButton],
        foreground_hwnd: HWND,
    ) -> Option<usize> {
        unsafe {
            let fg_element = self.automation.ElementFromHandle(foreground_hwnd).ok()?;

            let fg_pid = fg_element.CurrentProcessId().ok().unwrap_or(-1);
            let fg_name = fg_element
                .CurrentName()
                .ok()
                .map(|b| b.to_string())
                .unwrap_or_default();

            for (i, button) in buttons.iter().enumerate() {
                if button.process_id == fg_pid && fg_pid > 0 {
                    return Some(i);
                }
            }

            let fg_clean = clean_button_name(&fg_name);
            for (i, button) in buttons.iter().enumerate() {
                let btn_clean = clean_button_name(&button.name);
                if !btn_clean.is_empty()
                    && (fg_clean.contains(&btn_clean) || btn_clean.contains(&fg_clean))
                {
                    return Some(i);
                }
            }

            None
        }
    }
}

pub fn clean_button_name(name: &str) -> String {
    if let Some(pos) = name.rfind(" running window") {
        let before = &name[..pos];
        if let Some(dash_pos) = before.rfind(" - ") {
            return before[..dash_pos].to_string();
        }
        if let Some(dash_pos) = before.rfind(" \u{2014} ") {
            return before[..dash_pos].to_string();
        }
        return before.to_string();
    }
    name.to_string()
}

impl Drop for TaskbarEnumerator {
    fn drop(&mut self) {
        if self.com_initialized {
            unsafe {
                CoUninitialize();
            }
        }
    }
}