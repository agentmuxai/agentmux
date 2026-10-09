// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Show and Close for a popup window a browser pane's page opened, from the
//! opener's popup-windows strip
//! (docs/specs/SPEC_BROWSER_PANE_NATIVE_POPUPS_AGENT_DRIVEN_2026_10_08.md §7).
//!
//! Popups aren't app windows: CEF owns their window on Windows, and they
//! carry no srv window record. So they get their own tasks rather than
//! `post_focus_window`/`post_close_window`.

use std::sync::Arc;

use cef::*;

use crate::state::AppState;

/// The popup window's browser, if `label` names a live one.
fn popup_browser(state: &AppState, label: &str) -> Option<Browser> {
    state.live_popup_label(label)?;
    state.host_state.lock().browsers.get(label).map(|h| h.browser.clone())
}

wrap_task! {
    pub struct ShowPopupTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            let Some(browser) = popup_browser(&self.state, &self.label) else { return };
            let Some(host) = browser.host() else { return };
            #[cfg(target_os = "windows")]
            {
                let wh = host.window_handle();
                if !wh.0.is_null() {
                    use windows_sys::Win32::UI::WindowsAndMessaging::{
                        GetAncestor, IsIconic, SetForegroundWindow, ShowWindow, GA_ROOT, SW_RESTORE,
                    };
                    unsafe {
                        let root = GetAncestor(wh.0 as _, GA_ROOT);
                        let top = if root.is_null() { wh.0 as _ } else { root };
                        if IsIconic(top) != 0 {
                            ShowWindow(top, SW_RESTORE);
                        }
                        SetForegroundWindow(top);
                    }
                }
            }
            host.set_focus(1);
        }
    }
}

wrap_task! {
    pub struct ClosePopupTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            let Some(browser) = popup_browser(&self.state, &self.label) else { return };
            if let Some(host) = browser.host() {
                // Not forced: the page gets its unload handlers, as when the
                // person closes the window themselves.
                host.close_browser(0);
            }
        }
    }
}

/// Bring the popup window `label` to the front.
pub fn post_show_popup(state: &Arc<AppState>, label: &str) {
    let mut task = ShowPopupTask::new(state.clone(), label.to_string());
    post_task(ThreadId::UI, Some(&mut task));
}

/// Close the popup window `label`.
pub fn post_close_popup(state: &Arc<AppState>, label: &str) {
    let mut task = ClosePopupTask::new(state.clone(), label.to_string());
    post_task(ThreadId::UI, Some(&mut task));
}
