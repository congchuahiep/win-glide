//! Enumerates buttons on the Windows 11 taskbar in correct order from left to right.
//!
//! # Why not use `FindWindow` directly?
//!
//! On Windows 10, taskbar buttons are `ToolbarWindow32`, a standard Windows control.
//! We can use the `TB_GETBUTTON` message to get information directly. But on **Windows 11**,
//! Microsoft rewrote the taskbar using **XAML** (UWP/WinRT). Buttons are no longer distinct `HWND`s,
//! they are **XAML elements** inside `Windows.UI.Composition.DesktopWindowContentBridge`.
//!
//! Therefore we must use **UI Automation (UIAutomation)**, a COM-based API that allows accessing UI
//! elements regardless of the underlying technology (Win32, XAML, WebView, etc.).
//!
//! # Key concept: IUIAutomation
//!
//! **IUIAutomation** is like a "screen reader" for the visually impaired. It describes everything on the screen as a **hierarchy tree**:
//!
//! ```text
//! Root (Desktop)
//!  └── Shell_TrayWnd (Taskbar)
//!       └── Windows.UI.Composition.DesktopWindowContentBridge
//!            └── Taskbar.TaskListButtonAutomationPeer  ← these are the buttons!
//!            └── Taskbar.TaskListButtonAutomationPeer
//!            └── ...
//! ```

use std::cell::{Cell, RefCell};
use std::time::Instant;
use tracing::{debug, error, instrument, warn};
use windows::core::w;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, HMONITOR, MONITOR_DEFAULTTONULL};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, CLSCTX_LOCAL_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    AutomationElementMode_None, CUIAutomation, IUIAutomation, IUIAutomationCacheRequest,
    IUIAutomationCondition, IUIAutomationElementArray, TreeScope_Descendants,
    UIA_AutomationIdPropertyId, UIA_BoundingRectanglePropertyId, UIA_ClassNamePropertyId,
    UIA_NamePropertyId, UIA_ProcessIdPropertyId,
};
use windows::Win32::UI::Shell::IVirtualDesktopManager;
use windows::Win32::UI::Shell::VirtualDesktopManager;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, FindWindowW, GetForegroundWindow};

use super::activate::force_activate;
use super::button_window::ButtonWindowMap;
use super::explorer::invalidate_explorer_pid_cache;
use super::window::find_visible_windows;
use crate::event::UiaEventHook;
use crate::taskbar::window_context::WindowContext;
use crate::types::{TargetWindow, TaskbarButton};
use crate::utils::truncate;

/// Cache TTL: 1 second. If no WinEvent invalidates it, the cache auto-expires after 1s.
const CACHE_TTL_SECS: f64 = 1.0;

/// Button cache with timestamp.
struct ButtonCache {
    buttons: Vec<TaskbarButton>,
    created_at: Instant,
}

/// Cycle direction: left or right on the taskbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleDirection {
    Forward,
    Backward,
}

/// Enumerates taskbar buttons on Windows 11.
pub struct TaskbarEnumerator {
    /// COM interface IUIAutomation, UI "scanner".
    automation: IUIAutomation,

    /// Caches the button list to avoid re-enumerating on every keystroke.
    ///
    /// [`RefCell`] allows mutation from `&self` methods (no need for `&mut self`).
    /// The cache is invalidated by UIA events or when TTL (1 second) expires.
    button_cache: RefCell<Option<ButtonCache>>,

    /// Virtual Desktop ID of the most recent foreground window.
    ///
    /// Used to detect desktop switching, if ID changes -> invalidate cache.
    last_desktop_id: RefCell<Option<windows::core::GUID>>,

    /// List of HWNDs of the taskbar windows (both primary and secondary monitors' taskbars).
    taskbars: RefCell<Vec<HWND>>,

    /// UIA StructureChanged event hooks.
    uia_hooks: RefCell<Vec<UiaEventHook>>,

    /// UIA StructureChanged event hook - auto subscribes/unsubscribes.
    ///
    /// Upon explorer restart, [`Self::refresh_taskbar_hwnd`] will drop the old and create a new one.
    uia_thread_id: Cell<u32>,

    /// Index of the last active button.
    last_active_index: Cell<Option<usize>>,
}

/// Life cycle implementation
impl TaskbarEnumerator {
    /// Creates a new enumerator and initializes COM (STA apartment).
    ///
    /// # COM Apartments
    ///
    /// Windows COM has 2 apartment types:
    /// - **STA (Single-Threaded Apartment)**: Each thread has its own message queue, using
    /// `GetMessageW`.
    /// - **MTA (Multi-Threaded Apartment)**: No message queue, using
    /// `CoWaitForMultipleObjects`.
    ///
    /// IUIAutomation works well with both, but STA is recommended for simplicity.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let enumerator = TaskbarEnumerator::new()?;
    /// let buttons = enumerator.enumerate_current_monitor_buttons()?;
    /// ```
    pub fn new() -> anyhow::Result<Self> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;

            let taskbars = Self::get_all_taskbar_hwnds();
            if taskbars.is_empty() {
                anyhow::bail!(
                    "No taskbar found, possibly running in portable mode or the taskbar is disabled"
                )
            }

            Ok(Self {
                automation,
                taskbars: RefCell::new(taskbars),
                button_cache: RefCell::new(None),
                last_desktop_id: RefCell::new(None),
                uia_hooks: RefCell::new(Vec::new()),
                uia_thread_id: Cell::new(0),
                last_active_index: Cell::new(None),
            })
        }
    }

    /// Registers UIA StructureChanged event handler on taskbar elements.
    ///
    /// Called once from `App::run()`. After explorer restarts, [`Self::refresh_taskbar_hwnd`]
    /// automatically re-subscribes.
    pub fn install_uia_hook(&self, main_thread_id: u32) -> anyhow::Result<()> {
        self.uia_thread_id.set(main_thread_id);
        let taskbars = self.taskbars.borrow();
        let mut hooks = self.uia_hooks.borrow_mut();
        hooks.clear();
        for &hwnd in taskbars.iter() {
            if let Ok(hook) =
                unsafe { UiaEventHook::install(&self.automation, hwnd, main_thread_id) }
            {
                hooks.push(hook);
            }
        }
        Ok(())
    }

    /// Self-recovers after explorer.exe crashes and restarts.
    ///
    /// NOTE: This logic is called at [`Self::enumerate_buttons`], which might sound incorrect
    /// because this method should ideally be called when the explorer restart event occurs. The
    /// reason for not that approach is because it's a bit cumbersome, but in the future it's still
    /// better to catch the event instead of calling it manually.
    pub fn refresh_taskbar_hwnd(&self) -> anyhow::Result<()> {
        let taskbars = Self::get_all_taskbar_hwnds();
        if taskbars.is_empty() {
            anyhow::bail!("No taskbars found after explorer restart");
        }

        *self.taskbars.borrow_mut() = taskbars;
        self.invalidate_cache();
        invalidate_explorer_pid_cache();

        let mut hooks = self.uia_hooks.borrow_mut();
        hooks.clear();
        for &hwnd in self.taskbars.borrow().iter() {
            if let Ok(hook) =
                unsafe { UiaEventHook::install(&self.automation, hwnd, self.uia_thread_id.get()) }
            {
                hooks.push(hook);
            }
        }

        debug!("Taskbar HWNDs refreshed, UIA re-subscribed");
        Ok(())
    }
}

/// Core implementation
impl TaskbarEnumerator {
    /// Cycles to the next window on the taskbar and activates the window.
    ///
    /// # Errors
    ///
    /// Returns an error if no window could be found to cycle to.
    #[instrument(level = "debug", skip_all)]
    pub fn cycle_to_neighbor(
        &self,
        uncombine_enabled: bool,
        direction: CycleDirection,
    ) -> anyhow::Result<()> {
        let foreground = unsafe { GetForegroundWindow() };
        let target = self.find_neighbor_window(foreground, uncombine_enabled, direction)?;

        match target {
            Some(target) => {
                debug!(
                    "Activating '{}' (grouped={})",
                    truncate(&target.name, 30),
                    target.is_grouped,
                );

                let ok = unsafe { force_activate(target.hwnd) };
                if !ok {
                    warn!("Cannot activate window '{}'", truncate(&target.name, 30),);
                }
            }
            None => warn!("No window found to cycle to"),
        }

        Ok(())
    }

    /// Finds the next window to the left/right of `source` on the taskbar.
    ///
    /// Flow: enumerate buttons -> find active index -> compute next index ->
    /// match target button with window (via `ButtonWindowMap`) -> return target.
    #[instrument(level = "debug", skip_all)]
    fn find_neighbor_window(
        &self,
        source: HWND,
        uncombine_enabled: bool,
        direction: CycleDirection,
    ) -> anyhow::Result<Option<TargetWindow>> {
        /// Returns the index of the next valid button, or `None` if no valid button is found.
        fn get_next_valid_button_index(
            buttons: &[TaskbarButton],
            button_map: &ButtonWindowMap,
            source_index: usize,
            direction: CycleDirection,
        ) -> Option<usize> {
            let mut target_index = source_index;
            let mut attempts = 0;

            loop {
                attempts += 1;
                if attempts > buttons.len() {
                    return None;
                }

                target_index = match direction {
                    CycleDirection::Forward => (target_index + 1) % buttons.len(),
                    CycleDirection::Backward => {
                        if target_index == 0 {
                            buttons.len() - 1
                        } else {
                            target_index - 1
                        }
                    }
                };

                let target_button = &buttons[target_index];
                if !button_map.find_windows_by_button(target_button).is_empty() {
                    return Some(target_index);
                }
            }
        }

        let current_context = WindowContext::current_state();
        let buttons = self.enumerate_buttons_by_monitor(current_context.monitor)?;
        if buttons.is_empty() {
            return Ok(None);
        }

        let current_monitor_windows = find_visible_windows(Some(&current_context));
        let button_map = ButtonWindowMap::new(&buttons, &current_monitor_windows);

        let source_index = match button_map.find_button_index_by_hwnd(source) {
            Some(i) => {
                self.last_active_index.set(Some(i));
                i
            }
            None => self
                .last_active_index
                .get()
                .filter(|&i| i < buttons.len())
                .unwrap_or(0),
        };

        let target_index =
            match get_next_valid_button_index(&buttons, &button_map, source_index, direction) {
                Some(idx) => idx,
                None => return Ok(None),
            };
        let target_button = &buttons[target_index];

        debug!(
            "Source button [{}]: '{}' AUMID '{:?}'",
            source_index, buttons[source_index].name, buttons[source_index].automation_id
        );

        debug!(
            "Target button [{}]: '{}' AUMID '{:?}'",
            target_index, target_button.name, target_button.automation_id
        );

        match uncombine_enabled {
            true => Ok(button_map
                .find_window_by_button(target_button)
                .map(|w| TargetWindow {
                    name: w.title,
                    hwnd: w.hwnd,
                    is_grouped: false,
                })),
            false => {
                let windows = button_map.find_windows_by_button(target_button);
                let is_grouped = windows.len() > 1;

                Ok(windows.into_iter().next().map(|w| TargetWindow {
                    name: w.title,
                    hwnd: w.hwnd,
                    is_grouped,
                }))
            }
        }
    }

    /// Gets all HWNDs of taskbars (primary and secondary).
    fn get_all_taskbar_hwnds() -> Vec<HWND> {
        let mut hwnds = Vec::new();
        unsafe {
            if let Ok(primary) = FindWindowW(w!("Shell_TrayWnd"), None) {
                if !primary.0.is_null() {
                    hwnds.push(primary);
                }
            }

            let mut secondary = HWND::default();
            loop {
                secondary =
                    FindWindowExW(None, Some(secondary), w!("Shell_SecondaryTrayWnd"), None)
                        .unwrap_or_default();

                if secondary.0.is_null() {
                    break;
                }
                hwnds.push(secondary);
            }
        }
        hwnds
    }

    /// Returns taskbar buttons on the current monitor (containing the cursor or foreground window)
    ///
    /// Cached with a 1-second TTL, invalidated by UIA events or desktop switches
    pub fn enumerate_buttons_by_monitor(
        &self,
        monitor: HMONITOR,
    ) -> anyhow::Result<Vec<TaskbarButton>> {
        if self.desktop_changed() {
            self.invalidate_cache();
        }

        if let Some(ref cache) = *self.button_cache.borrow() {
            let age = cache.created_at.elapsed().as_secs_f64();
            if age < CACHE_TTL_SECS {
                debug!("Using cached buttons (age: {:.0}ms)", age * 1000.0);
                return Ok(cache.buttons.clone());
            }
        }

        unsafe {
            let taskbars = self.taskbars.borrow();
            let target_taskbar = taskbars
                .iter()
                .find(|&&hwnd| {
                    let hmonitor_tb = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL);
                    !hmonitor_tb.is_invalid() && hmonitor_tb.0 == monitor.0
                })
                .copied()
                .unwrap_or_else(|| *taskbars.first().unwrap_or(&HWND::default()));

            let buttons = self.enumerate_buttons(target_taskbar)?;
            *self.button_cache.borrow_mut() = Some(ButtonCache {
                buttons: buttons.clone(),
                created_at: Instant::now(),
            });
            Ok(buttons)
        }
    }

    /// Checks if the virtual desktop has changed via `IVirtualDesktopManager::GetWindowDesktopId`.
    fn desktop_changed(&self) -> bool {
        let fg = unsafe { GetForegroundWindow() };
        if fg.0.is_null() {
            return false;
        }

        let mgr: IVirtualDesktopManager =
            match unsafe { CoCreateInstance(&VirtualDesktopManager, None, CLSCTX_LOCAL_SERVER) } {
                Ok(m) => m,
                Err(_) => return false,
            };

        let current_id = match unsafe { mgr.GetWindowDesktopId(fg) } {
            Ok(id) => id,
            Err(_) => return false,
        };

        let mut last = self.last_desktop_id.borrow_mut();
        let changed = match *last {
            Some(ref prev) => prev != &current_id,
            None => false,
        };

        *last = Some(current_id);
        changed
    }

    /// Detects `EVENT_E_ALL_SUBSCRIBERS_FAILED (0x80040201)` error upon explorer restart.
    fn is_subscribers_failed(e: &anyhow::Error) -> bool {
        e.chain().any(|c| c.to_string().contains("0x80040201"))
    }

    /// Enumerates taskbar buttons of a specific taskbar HWND via UIA.
    ///
    /// Fallback: if `FindAllBuildCache` returns empty, tries `enumerate_via_bridge_windows`.
    /// Self-recovers if `EVENT_E_ALL_SUBSCRIBERS_FAILED` (explorer restart) is encountered.
    #[instrument(level = "debug", skip_all)]
    unsafe fn enumerate_buttons(&self, target_taskbar: HWND) -> anyhow::Result<Vec<TaskbarButton>> {
        /// Creates a CacheRequest to batch 4 UIA properties (name, rect, PID, automation_id)
        /// so it only takes 1 COM call instead of 4 separate calls.
        unsafe fn create_button_cache_request(
            automation: &IUIAutomation,
        ) -> anyhow::Result<IUIAutomationCacheRequest> {
            let cache = automation.CreateCacheRequest()?;

            cache.AddProperty(UIA_NamePropertyId)?;
            cache.AddProperty(UIA_BoundingRectanglePropertyId)?;
            cache.AddProperty(UIA_ProcessIdPropertyId)?;
            cache.AddProperty(UIA_AutomationIdPropertyId)?;

            cache.SetAutomationElementMode(AutomationElementMode_None)?;

            Ok(cache)
        }

        let try_enumerate_buttons = |taskbar_hwnd| -> anyhow::Result<Vec<TaskbarButton>> {
            let class_condition = self.automation.CreatePropertyCondition(
                UIA_ClassNamePropertyId,
                &VARIANT::from("Taskbar.TaskListButtonAutomationPeer"),
            )?;

            let cache_request = create_button_cache_request(&self.automation)?;

            let root_element = self.automation.ElementFromHandle(taskbar_hwnd)?;
            let items = root_element.FindAllBuildCache(
                TreeScope_Descendants,
                &class_condition,
                &cache_request,
            )?;

            let mut all_buttons = Vec::new();
            self.collect_buttons(&items, &mut all_buttons)?;

            if all_buttons.is_empty() {
                self.enumerate_via_bridge_windows(
                    taskbar_hwnd,
                    &class_condition,
                    &cache_request,
                    &mut all_buttons,
                )?;
            }

            all_buttons.sort_by_key(|b| b.rect.left);
            Ok(all_buttons)
        };

        match try_enumerate_buttons(target_taskbar) {
            Ok(buttons) => Ok(buttons),
            Err(ref e) if Self::is_subscribers_failed(e) => {
                warn!("UIA subscribers failed (explorer restart?), recovering...");
                self.refresh_taskbar_hwnd().map_err(|e2| {
                    error!("Failed to recover taskbar: {e2}");
                    anyhow::anyhow!("{e}")
                })?;

                let new_target = self
                    .taskbars
                    .borrow()
                    .first()
                    .copied()
                    .unwrap_or(target_taskbar);
                try_enumerate_buttons(new_target)
            }
            Err(e) => Err(e),
        }
    }

    /// Extracts information from the UIA element array via cached properties.
    ///
    /// Uses `Cached*` methods (no separate COM calls needed) because properties were already
    /// batch-read via `CacheRequest` in `FindAllBuildCache`.
    #[instrument(level = "debug", skip_all)]
    unsafe fn collect_buttons(
        &self,
        items: &IUIAutomationElementArray,
        buttons: &mut Vec<TaskbarButton>,
    ) -> anyhow::Result<()> {
        let count = items.Length()?;

        debug!("collect_buttons: count={}", count);

        for i in 0..count {
            let item = items.GetElement(i)?;

            let name = item
                .CachedName()
                .ok()
                .map(|b| b.to_string())
                .unwrap_or_default();

            let rect = match item.CachedBoundingRectangle() {
                Ok(r) => r,
                Err(_) => continue,
            };

            let process_id = item.CachedProcessId().unwrap_or(0);
            let automation_id = item.CachedAutomationId().ok().map(|s| s.to_string());

            buttons.push(TaskbarButton {
                name,
                rect,
                process_id,
                automation_id,
            });
        }

        Ok(())
    }

    /// Fallback: finds buttons via `DesktopWindowContentBridge` child windows of the taskbar.
    ///
    /// Win11 can render buttons inside the bridge window (XAML island)
    /// instead of directly under Shell_TrayWnd.
    #[instrument(level = "debug", skip_all)]
    unsafe fn enumerate_via_bridge_windows(
        &self,
        root_hwnd: HWND,
        condition: &IUIAutomationCondition,
        cache_request: &IUIAutomationCacheRequest,
        buttons: &mut Vec<TaskbarButton>,
    ) -> anyhow::Result<()> {
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
                if let Ok(items) = bridge_element.FindAllBuildCache(
                    TreeScope_Descendants,
                    condition,
                    cache_request,
                ) {
                    self.collect_buttons(&items, buttons)?;
                }
            }

            if !buttons.is_empty() {
                break;
            }
        }

        Ok(())
    }
}

/// Cached implementation
impl TaskbarEnumerator {
    /// Invalidates button cache - called upon receiving UIA StructureChanged event.
    pub fn invalidate_cache(&self) {
        let mut cache = self.button_cache.borrow_mut();
        if cache.is_some() {
            debug!("Button cache invalidated (event)");
            *cache = None;
        }
    }
}
