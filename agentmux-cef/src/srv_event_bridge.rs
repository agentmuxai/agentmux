// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.2c.5a — host outbound JS bridge for srv typed events.
//
// Parallel to `launcher_event_bridge`, but for the srv reducer's
// pipe (workspace / tab / block / layout lifecycle). Single function
// `dispatch_to_renderers(state, event)` called from `srv_ipc`'s
// read loop after each line is parsed.
//
// Renderer-side handler: `window.__agentmux_srv_event(<json>)`. The
// renderer dispatcher (separate frontend PR — E.2c.5b) installs that
// handler and routes events into atom domains.
//
// Filtering: pool windows (`window-pool-*`) and browser-pane child
// HWNDs (`browser-pane-*`) are skipped — they have no UI.

use std::sync::Arc;

use agentmux_common::ipc::Event;
use cef::{CefString, ImplBrowser, ImplFrame};

/// Forward an srv event to every top-level renderer.
pub fn dispatch_to_renderers(state: &Arc<crate::state::AppState>, event: &Event) {
    let json = match serde_json::to_string(event) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                target: "srv-event-bridge",
                "[srv-event-bridge] serialize failed: {}",
                e
            );
            return;
        }
    };

    let script = format!(
        "if (window.__agentmux_srv_event) {{ try {{ window.__agentmux_srv_event({}) }} catch(e) {{ console.error('[srv-event] dispatch failed', e) }} }}",
        json
    );
    let code = CefString::from(script.as_str());
    let url = CefString::from("");

    let browsers = state.browsers.lock();
    for (label, browser) in browsers.iter() {
        if label.starts_with("window-pool-") || label.starts_with("browser-pane-") {
            continue;
        }
        if let Some(frame) = browser.main_frame() {
            frame.execute_java_script(Some(&code), Some(&url), 0);
        }
    }
}
