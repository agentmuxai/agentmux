// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CEF UI thread tasks — wraps host method calls so they can be safely
// dispatched from non-UI threads (e.g., the axum IPC handler).
//
// CEF browser host methods must be called on the UI thread. Calling them
// from other threads deadlocks the CEF message loop. Use post_task(ThreadId::UI)
// to marshal the call.

use cef::*;
use std::sync::Arc;

use crate::state::AppState;

// ── DevTools ──────────────────────────────────────────────────────────────

wrap_task! {
    pub struct ShowDevToolsTask {
        state: Arc<AppState>,
    }

    impl Task {
        fn execute(&self) {
            let browser = self.state.browser.lock().unwrap();
            if let Some(ref browser) = *browser {
                if let Some(host) = browser.host() {
                    let window_info = WindowInfo {
                        runtime_style: RuntimeStyle::ALLOY,
                        ..Default::default()
                    };
                    host.show_dev_tools(Some(&window_info), None, None, None);
                }
            }
        }
    }
}

// ── Zoom ──────────────────────────────────────────────────────────────────

wrap_task! {
    pub struct SetZoomLevelTask {
        state: Arc<AppState>,
        zoom_level: f64,
    }

    impl Task {
        fn execute(&self) {
            let browser = self.state.browser.lock().unwrap();
            if let Some(ref browser) = *browser {
                if let Some(host) = browser.host() {
                    host.set_zoom_level(self.zoom_level);
                }
            }
        }
    }
}
