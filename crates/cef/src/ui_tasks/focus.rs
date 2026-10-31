// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `focus_window` with an answer: bring a window back and say what happened
//! (`docs/reports/REPORT_ACTION_RESULTS_ALWAYS_VISIBLE_ACROSS_WINDOWS_2026_10_31.md`
//! §4 cases 6-7, step 7).
//!
//! A window that is minimized is restored before it is raised, and one that
//! lies off every display (a monitor that was unplugged, a saved position that
//! no longer exists) is moved onto one. The caller gets
//! `{ found, raised, restored, moved, foreground }`: `foreground` is read back
//! a moment after the raise, so "the OS refused" (Windows lets only the
//! foreground process raise a window; Wayland lets nobody) is something the
//! caller can see and answer, instead of a call that returns null either way.
//!
//! What is decided from geometry alone ([`onscreen_rect`]) is plain code and
//! tested on every platform; the Win32 and CEF calls around it are not.

use std::sync::Arc;

use cef::*;

use super::get_window_on_ui;
use crate::state::AppState;

/// A rectangle in screen pixels, `right` and `bottom` exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    fn width(self) -> i32 {
        self.right - self.left
    }

    fn height(self) -> i32 {
        self.bottom - self.top
    }

    fn centre(self) -> (i64, i64) {
        ((self.left as i64 + self.right as i64) / 2, (self.top as i64 + self.bottom as i64) / 2)
    }
}

/// How much of a window has to be on a display to count as reachable: enough of its title bar to grab.
const MIN_VISIBLE_W: i32 = 120;
const MIN_VISIBLE_H: i32 = 40;

fn overlap(a: Rect, b: Rect) -> (i32, i32) {
    let w = a.right.min(b.right) - a.left.max(b.left);
    let h = a.bottom.min(b.bottom) - a.top.max(b.top);
    (w.max(0), h.max(0))
}

/// Where `window` has to go to be reachable, or `None` when enough of it is on a display already.
///
/// `displays` are the monitors' work areas. A window that is off all of them moves, at the size it has (or the
/// display's, if it is larger), to the middle of the display nearest to where it was; with no display known it
/// stays where it is.
pub fn onscreen_rect(window: Rect, displays: &[Rect]) -> Option<Rect> {
    if displays.is_empty() || window.width() <= 0 || window.height() <= 0 {
        return None;
    }
    let reachable = displays.iter().any(|d| {
        let (w, h) = overlap(window, *d);
        w >= MIN_VISIBLE_W.min(window.width()) && h >= MIN_VISIBLE_H.min(window.height())
    });
    if reachable {
        return None;
    }
    let (cx, cy) = window.centre();
    let nearest = displays.iter().min_by_key(|d| {
        let (dx, dy) = d.centre();
        (dx - cx).pow(2) + (dy - cy).pow(2)
    })?;
    let (w, h) = (window.width().min(nearest.width()), window.height().min(nearest.height()));
    let left = nearest.left + (nearest.width() - w) / 2;
    let top = nearest.top + (nearest.height() - h) / 2;
    Some(Rect { left, top, right: left + w, bottom: top + h })
}

/// What the raise did, as the UI thread saw it.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FocusOutcome {
    pub found: bool,
    pub raised: bool,
    pub restored: bool,
    pub moved: bool,
    /// The window's top-level handle (Windows), for the read-back; 0 elsewhere.
    pub hwnd: isize,
}

/// Restore, bring onto a display and raise the window `label`. On the UI thread.
pub(crate) fn focus_on_ui(state: &Arc<AppState>, label: &str) -> FocusOutcome {
    // Floating panes are raw WS_POPUP HWNDs with no CEF Views Window.
    #[cfg(target_os = "windows")]
    if let Some(hwnd) = crate::floating_pane::floater_hwnd_for_label(label) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE};
        let restored = unsafe {
            // SetForegroundWindow on a minimized window activates it without un-minimizing.
            let minimized = IsIconic(hwnd as _) != 0;
            if minimized {
                ShowWindow(hwnd as _, SW_RESTORE);
            }
            SetForegroundWindow(hwnd as _);
            minimized
        };
        return FocusOutcome { found: true, raised: true, restored, moved: false, hwnd: hwnd as isize };
    }
    let Some(window) = get_window_on_ui(state, label) else {
        return FocusOutcome::default();
    };
    let mut out = FocusOutcome { found: true, ..Default::default() };
    #[cfg(target_os = "windows")]
    {
        out.hwnd = window.window_handle().0 as isize;
        let (restored, moved) = restore_and_bring_onscreen(out.hwnd);
        out.restored = restored;
        out.moved = moved;
    }
    #[cfg(not(target_os = "windows"))]
    if window.is_minimized() != 0 {
        window.restore();
        out.restored = true;
    }
    window.activate();
    out.raised = true;
    out
}

/// Un-minimize `hwnd` and move it onto a display if it is off all of them: (restored, moved).
#[cfg(target_os = "windows")]
fn restore_and_bring_onscreen(hwnd: isize) -> (bool, bool) {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, IsIconic, IsZoomed, SetWindowPos, ShowWindow, SWP_NOACTIVATE, SWP_NOZORDER, SW_RESTORE,
    };
    if hwnd == 0 {
        return (false, false);
    }
    unsafe {
        let restored = IsIconic(hwnd as _) != 0;
        if restored {
            ShowWindow(hwnd as _, SW_RESTORE);
        }
        // A maximized window fills a display by definition.
        if IsZoomed(hwnd as _) != 0 {
            return (restored, false);
        }
        let mut r: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd as _, &mut r) == 0 {
            return (restored, false);
        }
        let window = Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
        let Some(to) = onscreen_rect(window, &display_work_areas()) else {
            return (restored, false);
        };
        let moved = SetWindowPos(hwnd as _, std::ptr::null_mut(), to.left, to.top, to.width(), to.height(), SWP_NOZORDER | SWP_NOACTIVATE) != 0;
        tracing::info!(?window, ?to, moved, "[focus_window] the window was off every display and was brought onto one");
        (restored, moved)
    }
}

/// The work area of every connected monitor.
#[cfg(target_os = "windows")]
fn display_work_areas() -> Vec<Rect> {
    use windows_sys::Win32::Foundation::{LPARAM, RECT};
    use windows_sys::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO};
    unsafe extern "system" fn each(monitor: HMONITOR, _dc: HDC, _rect: *mut RECT, out: LPARAM) -> i32 {
        let areas = &mut *(out as *mut Vec<Rect>);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) != 0 {
            let w = info.rcWork;
            areas.push(Rect { left: w.left, top: w.top, right: w.right, bottom: w.bottom });
        }
        1
    }
    let mut areas: Vec<Rect> = Vec::new();
    unsafe {
        EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), Some(each), &mut areas as *mut Vec<Rect> as LPARAM);
    }
    areas
}

wrap_task! {
    // [`focus_on_ui`], with its outcome sent back to the caller waiting for it.
    pub struct FocusWindowReportTask {
        state: Arc<AppState>,
        label: String,
        tx: std::sync::mpsc::SyncSender<FocusOutcome>,
    }

    impl Task {
        fn execute(&self) {
            // Capacity 1, one per call: try_send never blocks the UI thread.
            let _ = self.tx.try_send(focus_on_ui(&self.state, &self.label));
        }
    }
}

/// Whether the window is the foreground window now (Windows: its top-level window is the foreground one).
#[cfg(target_os = "windows")]
fn is_foreground(_state: &Arc<AppState>, _label: &str, hwnd: isize) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetAncestor, GetForegroundWindow, GA_ROOT};
    if hwnd == 0 {
        return false;
    }
    unsafe {
        let fg = GetForegroundWindow();
        !fg.is_null() && (GetAncestor(fg, GA_ROOT) as isize == hwnd || fg as isize == hwnd)
    }
}

#[cfg(not(target_os = "windows"))]
wrap_task! {
    struct WindowActiveTask {
        state: Arc<AppState>,
        label: String,
        tx: std::sync::mpsc::SyncSender<bool>,
    }

    impl Task {
        fn execute(&self) {
            let active = get_window_on_ui(&self.state, &self.label).is_some_and(|w| w.is_active() != 0);
            let _ = self.tx.try_send(active);
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn is_foreground(state: &Arc<AppState>, label: &str, _hwnd: isize) -> bool {
    let (tx, rx) = std::sync::mpsc::sync_channel::<bool>(1);
    let mut task = WindowActiveTask::new(state.clone(), label.to_string(), tx);
    post_task(ThreadId::UI, Some(&mut task));
    rx.recv_timeout(std::time::Duration::from_millis(250)).unwrap_or(false)
}

/// How long the UI thread has to do the raise before the caller stops waiting.
const RAISE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1000);
/// Between the raise and the read-back: the OS takes a moment to make a window the foreground one.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(150);

/// What `focus_window` answers.
pub(crate) fn report(outcome: Option<FocusOutcome>, foreground: bool) -> serde_json::Value {
    match outcome {
        Some(o) => serde_json::json!({
            "found": o.found,
            "raised": o.raised,
            "restored": o.restored,
            "moved": o.moved,
            "foreground": foreground,
        }),
        // The UI thread didn't answer: the window may still come forward, but nothing is known.
        None => serde_json::json!({ "found": true, "raised": false, "restored": false, "moved": false, "foreground": false, "timed_out": true }),
    }
}

/// Raise the window `label` and wait for what happened. From the IPC thread (never the UI thread).
pub(crate) fn focus_window_blocking(state: &Arc<AppState>, label: &str) -> serde_json::Value {
    // A held "main" (a notification click, the macOS Dock) is shown, not raised: AgentMux itself reveals it.
    if label == crate::start_hidden::HELD_LABEL && crate::start_hidden::release_for_request(state, "focus_window") {
        return serde_json::json!({ "found": true, "raised": true, "restored": false, "moved": false, "foreground": true, "held_main_shown": true });
    }
    let (tx, rx) = std::sync::mpsc::sync_channel::<FocusOutcome>(1);
    let mut task = FocusWindowReportTask::new(state.clone(), label.to_string(), tx);
    post_task(ThreadId::UI, Some(&mut task));
    let outcome = rx.recv_timeout(RAISE_TIMEOUT).ok();
    let foreground = match outcome {
        Some(o) if o.found && o.raised => {
            std::thread::sleep(SETTLE);
            is_foreground(state, label, o.hwnd)
        }
        _ => false,
    };
    report(outcome, foreground)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect { left, top, right, bottom }
    }

    const MAIN: Rect = Rect { left: 0, top: 0, right: 1920, bottom: 1040 };
    const SECOND: Rect = Rect { left: 1920, top: 0, right: 3840, bottom: 1040 };

    #[test]
    fn a_window_on_a_display_stays_where_it_is() {
        assert_eq!(onscreen_rect(r(100, 100, 1100, 800), &[MAIN]), None);
        // On the second of two displays.
        assert_eq!(onscreen_rect(r(2000, 50, 3000, 700), &[MAIN, SECOND]), None);
        // Half off the edge, with its title bar still reachable.
        assert_eq!(onscreen_rect(r(1500, 100, 2500, 800), &[MAIN]), None);
    }

    #[test]
    fn a_window_on_an_unplugged_monitor_comes_to_the_nearest_display() {
        // It was on a monitor to the right of two displays that are now gone.
        let to = onscreen_rect(r(5000, 100, 6000, 800), &[MAIN, SECOND]).unwrap();
        assert_eq!((to.width(), to.height()), (1000, 700), "its size is kept");
        assert!(to.left >= SECOND.left && to.right <= SECOND.right, "the nearest display is the second one: {to:?}");
        // It was left of the only display.
        let to = onscreen_rect(r(-4000, 200, -3000, 900), &[MAIN]).unwrap();
        assert!(to.left >= MAIN.left && to.right <= MAIN.right && to.top >= MAIN.top && to.bottom <= MAIN.bottom, "{to:?}");
    }

    #[test]
    fn a_sliver_on_a_display_is_not_reachable() {
        // 10 px of the window's left edge is on the display.
        assert!(onscreen_rect(r(1910, 100, 2910, 800), &[MAIN]).is_some());
        // Its title bar is below the bottom edge.
        assert!(onscreen_rect(r(100, 1030, 1100, 1700), &[MAIN]).is_some());
    }

    #[test]
    fn a_window_larger_than_the_display_is_fitted() {
        let to = onscreen_rect(r(9000, 9000, 12000, 11000), &[MAIN]).unwrap();
        assert_eq!((to.width(), to.height()), (1920, 1040));
        assert_eq!((to.left, to.top), (0, 0));
    }

    #[test]
    fn nothing_is_decided_without_displays_or_a_size() {
        assert_eq!(onscreen_rect(r(5000, 0, 6000, 700), &[]), None);
        assert_eq!(onscreen_rect(r(0, 0, 0, 0), &[MAIN]), None);
    }

    #[test]
    fn the_answer_says_what_was_done_and_whether_the_os_let_it_through() {
        let out = FocusOutcome { found: true, raised: true, restored: true, moved: false, hwnd: 0 };
        let ok = report(Some(out), true);
        assert_eq!(ok["restored"], true);
        assert_eq!(ok["foreground"], true);
        // The raise was called and the OS refused it.
        let refused = report(Some(out), false);
        assert_eq!((refused["raised"].as_bool(), refused["foreground"].as_bool()), (Some(true), Some(false)));
        // No such window.
        let none = report(Some(FocusOutcome::default()), false);
        assert_eq!((none["found"].as_bool(), none["raised"].as_bool()), (Some(false), Some(false)));
        // The UI thread didn't answer.
        assert_eq!(report(None, false)["timed_out"], true);
    }
}
