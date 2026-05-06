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
/// Excluded:
///   - Pool **inventory** labels (`window-pool-*` in
///     `pool.unpromoted` OR `pool.queue`): no user UI yet. Two
///     sub-states:
///       * `pool.unpromoted` — spawning, renderer not ready.
///       * `pool.queue` — renderer ready, waiting for promote.
///     Both are hidden off-screen and would build stale InstancePanel
///     state from launcher events the user never sees. The bridge
///     uses `state.pool_inventory_labels_snapshot()` (unpromoted ∪
///     queue) so a renderer-ready-but-pre-promote pool window
///     doesn't slip through (`unpromoted_pool_labels_snapshot()`
///     alone misses that case).
///   - Browser-pane labels (`browser-pane-*`): not top-level
///     windows; have no InstancePanel.
///
/// Promoted pool windows (label still has the `window-pool-*`
/// prefix but the entry is in NEITHER pool set) ARE included —
/// they're the user-visible torn-off windows. Pre-fix, a
/// label-prefix-only check excluded them too, so torn-off windows
/// stopped receiving launcher events post-promotion (InstancePanel
/// drift, plus anything else listening to launcher events).
///
/// JSON payload uses `serde_json::to_string`, so any string content
/// from the Event is escaped against quote / backtick injection at
/// the JS-string boundary.
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

    // Snapshot pool inventory once (unpromoted + ready-queue). Hot
    // path — every launcher event runs this loop, and we want to
    // avoid re-locking host_state per browser.
    let pool_inventory = state.pool_inventory_labels_snapshot();

    // Phase H.2.b — reducer-aware iteration with fallback.
    for (label, browser) in state.list_browsers() {
        if label.starts_with("browser-pane-") {
            continue;
        }
        if pool_inventory.contains(label.as_str()) {
            // Pool inventory (unpromoted or ready-queued) — no user
            // UI yet, skip.
            continue;
        }
        if let Some(frame) = browser.main_frame() {
            frame.execute_java_script(Some(&code), Some(&url), 0);
        }
    }
}
