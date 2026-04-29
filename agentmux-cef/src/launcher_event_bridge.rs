// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase B.7.3.1 — host outbound JS bridge for launcher typed events.
//
// Single function `dispatch_to_renderers(state, event)` called from
// `launcher_ipc::apply_event_to_shadow` after each event is applied
// to host shadows. Iterates `state.browsers`, calls
// `Frame::ExecuteJavaScript` per top-level browser to invoke
// `window.__agentmux_launcher_event(<json>)` in the renderer.
//
// Filtering: pool windows (`window-pool-*`) and browser-pane child
// HWNDs (`browser-pane-*`) are skipped. They have no UI to react.
//
// Cross-platform: `Frame::ExecuteJavaScript` is portable across
// CEF's Windows / macOS / Linux backends. No platform specifics.
//
// Phase B.7.3.3 — typed events are the SOLE channel for
// InstancePanel state. The bespoke `window-instances-changed`
// event and its 4 sync emit sites in the host are retired.
//
// See `docs/specs/SPEC_B_7_3_LAUNCHER_EVENTS_TO_RENDERER_2026_04_29.md`.

use std::sync::Arc;

use agentmux_common::ipc::Event;
use cef::{CefString, ImplBrowser, ImplFrame};

/// Forward a launcher event to every top-level renderer.
///
/// Pool + browser-pane labels are excluded — they have no UI.
/// The JSON payload uses `serde_json::to_string`, so any string
/// content from the Event is escaped against quote / backtick
/// injection at the JS-string boundary.
pub fn dispatch_to_renderers(state: &Arc<crate::state::AppState>, event: &Event) {
    let json = match serde_json::to_string(event) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                target: "launcher-event-bridge",
                "[launcher-event-bridge] serialize failed: {}",
                e
            );
            return;
        }
    };

    let script = format!(
        "if (window.__agentmux_launcher_event) {{ try {{ window.__agentmux_launcher_event({}) }} catch(e) {{ console.error('[launcher-event] dispatch failed', e) }} }}",
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
