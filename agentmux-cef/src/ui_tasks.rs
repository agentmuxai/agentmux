// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CEF UI thread task dispatch.
//
// All CEF Views operations (Window::close, minimize, maximize, etc.) must run
// on the CEF UI thread. IPC commands arrive on tokio threads. This module
// provides tasks that can be posted to the UI thread via post_task().
//
// Key insight: don't pass Browser/Window handles across threads. Instead,
// pass Arc<AppState> and look up the browser on the UI thread.
//
// Used on Linux (and macOS). On Windows, Win32 APIs are used directly since
// they are safe to call from any thread.

use std::sync::Arc;
use cef::*;
use crate::state::AppState;

/// Get the CEF Views Window for a browser label on the UI thread.
fn get_window_on_ui(state: &Arc<AppState>, label: &str) -> Option<Window> {
    let browsers = state.browsers.lock().unwrap();
    let mut browser = browsers.get(label)?.clone();
    drop(browsers);
    let browser_view = browser_view_get_for_browser(Some(&mut browser))?;
    browser_view.window()
}

// ── Close ────────────────────────────────────────────────────────────────

wrap_task! {
    pub struct CloseWindowTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                window.close();
            }
        }
    }
}

pub fn post_close_window(state: &Arc<AppState>, label: &str) {
    let mut task = CloseWindowTask::new(state.clone(), label.to_string());
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Minimize ─────────────────────────────────────────────────────────────

wrap_task! {
    pub struct MinimizeWindowTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                window.minimize();
            }
        }
    }
}

pub fn post_minimize_window(state: &Arc<AppState>, label: &str) {
    let mut task = MinimizeWindowTask::new(state.clone(), label.to_string());
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Maximize (toggle) ────────────────────────────────────────────────────

wrap_task! {
    pub struct MaximizeWindowTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                if window.is_maximized() != 0 {
                    window.restore();
                } else {
                    window.maximize();
                }
            }
        }
    }
}

pub fn post_maximize_window(state: &Arc<AppState>, label: &str) {
    let mut task = MaximizeWindowTask::new(state.clone(), label.to_string());
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Focus/Activate ───────────────────────────────────────────────────────

wrap_task! {
    pub struct FocusWindowTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                window.activate();
            }
        }
    }
}

pub fn post_focus_window(state: &Arc<AppState>, label: &str) {
    let mut task = FocusWindowTask::new(state.clone(), label.to_string());
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Drag ─────────────────────────────────────────────────────────────────
// CEF Views does not expose a programmatic drag-initiation API.
// Window dragging on Linux/macOS uses the WindowDelegate draggable regions.
// TODO: implement via X11 _NET_WM_MOVERESIZE for programmatic drag.
pub fn post_start_drag(_state: &Arc<AppState>, _label: &str) {}

// ── Move window ───────────────────────────────────────────────────────────

wrap_task! {
    pub struct MoveWindowTask {
        state: Arc<AppState>,
        label: String,
        dx: i32,
        dy: i32,
    }

    impl Task {
        fn execute(&self) {
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                let bounds = window.bounds();
                window.set_bounds(Some(&Rect {
                    x: bounds.x + self.dx,
                    y: bounds.y + self.dy,
                    width: bounds.width,
                    height: bounds.height,
                }));
            }
        }
    }
}

pub fn post_move_window(state: &Arc<AppState>, label: &str, dx: i32, dy: i32) {
    let mut task = MoveWindowTask::new(state.clone(), label.to_string(), dx, dy);
    post_task(ThreadId::UI, Some(&mut task));
}
