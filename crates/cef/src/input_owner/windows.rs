// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Windows adapter for the input owner: turns window messages, IPC and
//! pane layout into machine events, and runs the machine's effects on the UI
//! thread. Every focus change goes through here; nothing else sets a pane's or
//! the app's keyboard focus on Windows.
//! docs/specs/SPEC_BROWSER_PANE_INPUT_OWNER_2026_10_15.md §3.2.

use std::sync::{Arc, OnceLock};

use cef::*;

use crate::state::{AppState, BrowserPaneLifecycle};

use super::machine::{Effect, Event, Owner, Surface};
use super::registry;

/// The app state, for the top-level window hook, which Windows calls without one.
static STATE: OnceLock<Arc<AppState>> = OnceLock::new();

/// Give the top-level window hook the app state. Idempotent.
pub fn set_state(state: &Arc<AppState>) {
    let _ = STATE.set(state.clone());
}

/// Step `window`'s machine and run what it returns.
pub fn dispatch(state: &Arc<AppState>, window: &str, event: Event) {
    for effect in registry::step(window, event) {
        run(state, window, effect);
    }
}

// ── Events ───────────────────────────────────────────────────────────────

/// The frontend selected pane `block` (`browser_pane_focus`).
pub fn select_pane(state: &Arc<AppState>, block: &str) {
    match state.browser_pane_window_label(block) {
        Some(window) => dispatch(state, &window, Event::Select(Owner::Pane(block.to_string()))),
        None => tracing::warn!("[input] select of pane {block} with no window"),
    }
}

/// The frontend selected the app's own content in `window` (`main_window_focus`).
pub fn select_app(state: &Arc<AppState>, window: &str) {
    dispatch(state, window, Event::Select(Owner::App));
}

/// A click in pane `block`'s page (`WM_LBUTTONDOWN` in its windows).
pub fn pane_pointer_down(state: &Arc<AppState>, block: &str) {
    if let Some(window) = state.browser_pane_window_label(block) {
        dispatch(state, &window, Event::PanePointerDown(block.to_string()));
    }
}

/// `WM_SETFOCUS` reached one of pane `block`'s windows (after Chromium handled it).
pub fn pane_focus_gained(state: &Arc<AppState>, block: &str) {
    if let Some(window) = state.browser_pane_window_label(block) {
        dispatch(state, &window, Event::FocusGained(Surface::Pane(block.to_string())));
    }
}

/// Pane `block`'s browser was created.
pub fn pane_live(state: &Arc<AppState>, block: &str) {
    if let Some(window) = state.browser_pane_window_label(block) {
        dispatch(state, &window, Event::PaneLive(block.to_string()));
    }
}

/// Pane `block` was laid out in `window`: at 0×0 (its tab isn't the visible one)
/// or with a size.
pub fn pane_laid_out(state: &Arc<AppState>, window: &str, block: &str, collapsed: bool) {
    let event = if collapsed { Event::PaneCollapsed(block.to_string()) } else { Event::PaneShown(block.to_string()) };
    dispatch(state, window, event);
}

/// Pane `block` is about to be destroyed: if it has the keyboard, the app takes
/// it before its window goes.
pub fn pane_closing(state: &Arc<AppState>, block: &str) {
    if let Some(window) = registry::window_of_pane(block) {
        dispatch(state, &window, Event::PaneClosing(block.to_string()));
    }
}

/// Pane `block` was destroyed.
pub fn pane_gone(state: &Arc<AppState>, block: &str) {
    if let Some(window) = registry::window_of_pane(block) {
        dispatch(state, &window, Event::PaneGone(block.to_string()));
    }
    // A pane redocked out of a floating window dies with that window, whose
    // machine can't hand the keyboard on. If nothing holds it once the pane is
    // gone, the app of the window the user is in takes it.
    let mut task = OrphanedFocusTask::new(state.clone());
    post_delayed_task(ThreadId::UI, Some(&mut task), ORPHAN_CHECK_AFTER_MS);
}

/// How long after a pane is destroyed to check that some window still has the keyboard.
const ORPHAN_CHECK_AFTER_MS: i64 = 50;

/// The top-level window `hwnd` was activated or deactivated (`WM_ACTIVATE`).
/// Called before CEF's own handling; the effects are posted, so they run after it.
/// `visible`: an invisible pool window's activation is handed back elsewhere.
pub fn window_activation(hwnd: *mut std::ffi::c_void, active: bool, visible: bool) {
    let Some(state) = STATE.get() else { return };
    let Some(window) = state.label_for_hwnd(hwnd as _) else { return };
    if active && !visible {
        return;
    }
    dispatch(state, &window, if active { Event::Activated } else { Event::Deactivated });
}

/// The top-level window `hwnd` was shown, or restored from minimized.
pub fn window_shown(hwnd: *mut std::ffi::c_void) {
    let Some(state) = STATE.get() else { return };
    if let Some(window) = state.label_for_hwnd(hwnd as _) {
        dispatch(state, &window, Event::WindowShown);
    }
}

// ── Effects ──────────────────────────────────────────────────────────────

fn run(state: &Arc<AppState>, window: &str, effect: Effect) {
    match effect {
        Effect::FocusApp => {
            let mut task = crate::ui_tasks::MainFocusReclaimTask::new(state.clone(), window.to_string());
            post_task(ThreadId::UI, Some(&mut task));
        }
        Effect::FocusPane(block) => {
            let mut task = FocusPaneTask::new(state.clone(), block);
            post_task(ThreadId::UI, Some(&mut task));
        }
        Effect::RaisePanes => {
            let mut task = RaisePanesTask::new(state.clone(), window.to_string());
            post_task(ThreadId::UI, Some(&mut task));
        }
        Effect::Verify { seq, after_ms } => {
            let mut task = VerifyTask::new(state.clone(), window.to_string(), seq);
            post_delayed_task(ThreadId::UI, Some(&mut task), after_ms as i64);
        }
        Effect::Warn(reason) => tracing::warn!("[input] win={window} {reason}"),
    }
}

wrap_task! {
    struct FocusPaneTask {
        state: Arc<AppState>,
        block: String,
    }
    impl Task {
        fn execute(&self) {
            self.state.browser_panes.give_keyboard(&self.block, &self.state);
        }
    }
}

wrap_task! {
    struct RaisePanesTask {
        state: Arc<AppState>,
        window: String,
    }
    impl Task {
        fn execute(&self) {
            raise_panes(&self.state, &self.window);
        }
    }
}

wrap_task! {
    struct VerifyTask {
        state: Arc<AppState>,
        window: String,
        seq: u64,
    }
    impl Task {
        fn execute(&self) {
            let surface = focus_surface(&self.state, &self.window);
            dispatch(&self.state, &self.window, Event::Verify { seq: self.seq, surface });
        }
    }
}

wrap_task! {
    struct OrphanedFocusTask {
        state: Arc<AppState>,
    }
    impl Task {
        fn execute(&self) {
            use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus;
            use windows_sys::Win32::UI::WindowsAndMessaging::{GetAncestor, GetForegroundWindow, GA_ROOT};
            if !unsafe { GetFocus() }.is_null() {
                return;
            }
            let fg = unsafe { GetForegroundWindow() };
            if fg.is_null() {
                return;
            }
            let root = unsafe { GetAncestor(fg, GA_ROOT) };
            if let Some(window) = self.state.label_for_hwnd(root) {
                tracing::info!("[input] win={window} the keyboard was nowhere after a pane closed");
                dispatch(&self.state, &window, Event::Select(Owner::App));
            }
        }
    }
}

/// Where Win32 keyboard focus is, as seen from `window`.
fn focus_surface(state: &Arc<AppState>, window: &str) -> Surface {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetAncestor, GA_ROOT};
    let focus = unsafe { GetFocus() };
    if focus.is_null() {
        return Surface::Elsewhere;
    }
    let root = unsafe { GetAncestor(focus, GA_ROOT) } as isize;
    if let Some(own) = state.window_hwnd(window) {
        if own != root {
            return Surface::Elsewhere;
        }
    }
    match crate::browser_pane::hwnd::pane_block_for_hwnd(focus) {
        Some(block) => Surface::Pane(block),
        None => Surface::App,
    }
}

/// Put every live, laid-out pane of `window` above the app's own child windows,
/// so a visible pane takes its wheel and clicks. Logs a pane that something
/// covered: the window the pointer would have reached at the pane's centre.
fn raise_panes(state: &Arc<AppState>, window: &str) {
    use windows_sys::Win32::Foundation::{POINT, RECT};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetWindowRect, IsChild, SetWindowPos, WindowFromPoint, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOSIZE,
    };
    let panes: Vec<(String, String)> = state
        .host_state
        .lock()
        .browser_panes
        .values()
        .filter(|e| e.window_label == window && e.lifecycle == BrowserPaneLifecycle::Live)
        .map(|e| (e.block_id.clone(), e.label.clone()))
        .collect();
    for (block, label) in panes {
        let Some(wrapper) = crate::browser_pane::wrapper::peek_wrapper_hwnd(&label) else { continue };
        let wrapper = wrapper as *mut std::ffi::c_void;
        let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        if unsafe { GetWindowRect(wrapper, &mut rect) } == 0 || rect.right <= rect.left || rect.bottom <= rect.top {
            continue;
        }
        let centre = POINT { x: (rect.left + rect.right) / 2, y: (rect.top + rect.bottom) / 2 };
        let hit = unsafe { WindowFromPoint(centre) };
        if !hit.is_null() && hit != wrapper && unsafe { IsChild(wrapper, hit) } == 0 {
            let mut class = [0u16; 64];
            let n = unsafe { GetClassNameW(hit, class.as_mut_ptr(), class.len() as i32) };
            tracing::info!(
                "[input] win={window} pane:{} was covered at its centre by {:p} class={}",
                block.get(..8).unwrap_or(&block),
                hit,
                String::from_utf16_lossy(&class[..n.max(0) as usize]),
            );
        }
        unsafe {
            SetWindowPos(wrapper, std::ptr::null_mut(), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }
}
