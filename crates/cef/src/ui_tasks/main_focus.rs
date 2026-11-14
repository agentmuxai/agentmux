// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What `MainFocusReclaimTask` (window.rs) needs on Windows: whether the window may reclaim at all, which widget is
// the main page's, and whether focus is already there (then there is nothing to do).


use cef::*;
use crate::state::AppState;
use std::sync::Arc;

/// Every browser pane's outer HWND, so the walk for the main render widget can skip widgets that descend from
/// them. Panes are siblings of main under the Views top-level, so a naive EnumChildWindows would pick up their
/// Chrome_RenderWidgetHostHWND and focus the wrong target.
///
/// Two sources are combined: the live registered panes from `state.list_browsers()`, and the pane outer HWNDs
/// still tracked in BROWSER_PANE_HWND_CONTEXT, which covers the window between BrowserUnregistered and CEF's
/// on_before_close (deferred teardown), during which the HWND is still live but the label is gone from
/// `state.browsers`. Without it, panes_excluded=0 and find_main_render_widget picks the pane's render widget:
/// an infinite focus storm.
pub(super) fn pane_outer_hwnds(state: &Arc<AppState>) -> Vec<*mut std::ffi::c_void> {
    let mut hwnds: Vec<*mut std::ffi::c_void> = state
        .list_browsers()
        .into_iter()
        .filter(|(k, _)| crate::browser_pane::label::is_pane_label(k))
        .filter_map(|(_, b)| {
            b.host().and_then(|h| {
                let wh = h.window_handle();
                if wh.0.is_null() { None } else { Some(wh.0 as *mut std::ffi::c_void) }
            })
        })
        .collect();
    for h in crate::browser_pane::hwnd::pane_outer_hwnds_from_context() {
        if !hwnds.contains(&h) {
            hwnds.push(h);
        }
    }
    hwnds
}

/// Whether `MainFocusReclaimTask` may act on a window whose top-level HWND is
/// `target_root`, given the foreground window's root. Only the foreground
/// window: reclaiming in any other window would activate it. A target whose
/// top-level can't be resolved keeps the old behaviour (nothing to activate).
pub(super) fn reclaim_allowed(target_root: Option<isize>, foreground_root: isize) -> bool {
    match target_root {
        None => true,
        Some(t) => t == foreground_root,
    }
}

/// Whether Win32 focus (`held`) already is the main page's render widget (`render`), so a reclaim has nothing to do.
pub(super) fn focus_already_on(render: isize, held: isize) -> bool {
    render != 0 && render == held
}

/// Whether Win32 focus already is the main render widget `render` (the host and the widget are focused, so a
/// reclaim has nothing to do). A focusin inside the app, a click into a composer say, fires the reclaim on every
/// click; re-focusing during a mouse press made clicks into an input box lag where clicks into a pane's body did not.
pub(super) fn main_focus_already_held(render: Option<*mut std::ffi::c_void>) -> bool {
    let held = unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus() };
    render.is_some_and(|r| focus_already_on(r as isize, held as isize))
}

#[cfg(test)]
mod reclaim_tests {
    use super::{focus_already_on, reclaim_allowed};

    #[test]
    fn skips_when_the_main_render_widget_holds_focus() {
        assert!(focus_already_on(0x300, 0x300));
    }

    #[test]
    fn reclaims_when_a_pane_or_nothing_holds_focus() {
        assert!(!focus_already_on(0x300, 0x400));
        assert!(!focus_already_on(0x300, 0));
    }

    #[test]
    fn never_skips_without_a_render_widget_to_compare() {
        assert!(!focus_already_on(0, 0));
    }

    #[test]
    fn reclaims_in_the_foreground_window() {
        assert!(reclaim_allowed(Some(0x100), 0x100));
    }

    #[test]
    fn never_reclaims_in_a_background_window() {
        assert!(!reclaim_allowed(Some(0x100), 0x200));
    }

    #[test]
    fn never_reclaims_when_no_window_of_ours_is_foreground() {
        assert!(!reclaim_allowed(Some(0x100), 0));
    }

    #[test]
    fn keeps_the_old_behaviour_when_the_target_window_is_unknown() {
        assert!(reclaim_allowed(None, 0x200));
    }
}

/// Walk descendants of `root` and return the first Chrome_RenderWidgetHostHWND
/// whose ancestor chain does NOT pass through any of `pane_outer_hwnds`.
/// Panes are siblings of main under the Views top-level, so without this
/// filter the walk would happily pick a pane's render widget.
pub(super) unsafe fn find_main_render_widget(
    root: *mut std::ffi::c_void,
    pane_outer_hwnds: &[*mut std::ffi::c_void],
) -> Option<*mut std::ffi::c_void> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, GetClassNameW, GetParent,
    };

    struct Finder<'a> {
        found: *mut std::ffi::c_void,
        panes: &'a [*mut std::ffi::c_void],
    }
    let mut finder = Finder { found: std::ptr::null_mut(), panes: pane_outer_hwnds };

    unsafe extern "system" fn cb(hwnd: *mut std::ffi::c_void, lparam: isize) -> i32 {
        let finder = &mut *(lparam as *mut Finder);
        let mut buf = [0u16; 64];
        let n = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if n > 0 {
            let class = String::from_utf16_lossy(&buf[..n as usize]);
            if class == "Chrome_RenderWidgetHostHWND" {
                // Walk ancestors; if we pass through any pane outer HWND,
                // this widget belongs to a pane, not main.
                let mut descends_from_pane = false;
                let mut cursor = GetParent(hwnd);
                while !cursor.is_null() {
                    if finder.panes.iter().any(|p| *p == cursor) {
                        descends_from_pane = true;
                        break;
                    }
                    cursor = GetParent(cursor);
                }
                if !descends_from_pane {
                    finder.found = hwnd;
                    return 0; // stop
                }
            }
        }
        1
    }

    EnumChildWindows(root, Some(cb), &mut finder as *mut _ as isize);
    if finder.found.is_null() { None } else { Some(finder.found) }
}
