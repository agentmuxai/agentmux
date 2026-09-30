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

    // Phase H.2.b — reducer-aware iteration with fallback. Pool-side skip is
    // BY TYPE (reducer is_pool flag) so an adopted pool window's foreign
    // `window-{uuid}` label (SPEC_POOL_ADOPTION_AND_WINDOW_ROW_CRUMB Residual
    // 1) is skipped exactly like a `window-pool-*` one — its parked renderer
    // shows the pool boot page and has no srv-event consumers. `window-pool-`
    // stays as a fast-path prefix (covers spawns racing browser
    // registration); `browser-pane-` is prefix-only as before.
    let tab_pool_labels = state.pool_side_top_level_labels();
    for (label, browser) in state.list_browsers() {
        if label.starts_with("window-pool-")
            || label.starts_with("browser-pane-")
            || tab_pool_labels.contains(&label)
        {
            continue;
        }
        if let Some(frame) = browser.main_frame() {
            frame.execute_java_script(Some(&code), Some(&url), 0);
        }
    }
}
