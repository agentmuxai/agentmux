// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Rust -> JS event emission via CEF's execute_javascript.
//
// Events are dispatched as CustomEvents on `window`, matching the pattern
// used by the frontend's `listenEvent()` in platform/ipc.ts:
//
//   window.dispatchEvent(new CustomEvent('agentmux-event', {
//     detail: { event: 'event-name', payload: ... }
//   }))

use cef::{Browser, CefString, ImplBrowser, ImplFrame};

/// Emit an event to the frontend via CEF's execute_javascript.
///
/// The event will be dispatched as a `CustomEvent` named `agentmux-event`
/// with `detail.event` set to the event name and `detail.payload` set to
/// the serialized payload.
pub fn emit_event(browser: &Browser, event: &str, payload: &serde_json::Value) {
    if let Some(frame) = browser.main_frame() {
        let payload_str = serde_json::to_string(payload).unwrap_or_else(|_| "null".to_string());
        let js = format!(
            "window.dispatchEvent(new CustomEvent('agentmux-event', {{ detail: {{ event: '{}', payload: {} }} }}));",
            event, payload_str
        );
        let code = CefString::from(js.as_str());
        let url = CefString::from("");
        frame.execute_java_script(Some(&code), Some(&url), 0);
    }
}

/// Emit an event to the "main" browser stored in AppState.
/// This is a convenience wrapper for use from command handlers and background tasks.
/// Returns whether a browser was found to deliver to.
pub fn emit_event_from_state(
    state: &crate::state::AppState,
    event: &str,
    payload: &serde_json::Value,
) -> bool {
    // Phase H.2.b — reducer-aware lookup with fallback.
    if let Some(browser) = state.get_browser("main") {
        emit_event(&browser, event, payload);
        true
    } else if let Some((_label, browser)) = state.first_browser() {
        // Fallback: emit to any available browser
        emit_event(&browser, event, payload);
        true
    } else {
        tracing::warn!("Cannot emit event '{}': no browser handle in state", event);
        false
    }
}

/// Emit an event to ALL browser windows (for cross-window drag broadcasts).
pub fn emit_event_all_windows(state: &crate::state::AppState, event: &str, payload: &serde_json::Value) {
    // Phase H.2.b — reducer-aware iteration with fallback.
    let all = state.list_browsers();
    if all.is_empty() {
        tracing::warn!("Cannot broadcast event '{}': no browsers", event);
        return;
    }
    for (_label, browser) in all {
        emit_event(&browser, event, payload);
    }
}

/// Emit an event to every TOP-LEVEL window — i.e. the host frontend
/// renderers, excluding pane child browsers. Use this instead of
/// `emit_event_all_windows` when the payload carries metadata that
/// shouldn't be visible to untrusted remote content loaded inside a
/// browser pane. The host frontend's `listenEvent` lives in the
/// top-level renderer; pane main frames load arbitrary URLs.
pub fn emit_event_to_top_level_windows(
    state: &crate::state::AppState,
    event: &str,
    payload: &serde_json::Value,
) {
    let all = state.list_top_level_browsers();
    if all.is_empty() {
        tracing::warn!("Cannot broadcast event '{}': no top-level browsers", event);
        return;
    }
    for (_label, browser) in all {
        emit_event(&browser, event, payload);
    }
}

/// Emit an event to a specific browser window by label.
/// Used by the tear-off Phase 4 hook to push merge-candidate
/// changes to the destination renderer.
pub fn emit_event_to_window(
    state: &crate::state::AppState,
    label: &str,
    event: &str,
    payload: &serde_json::Value,
) -> bool {
    // Phase H.2.b — reducer-aware lookup with fallback.
    match state.get_browser(label) {
        Some(browser) => {
            emit_event(&browser, event, payload);
            true
        }
        None => {
            tracing::warn!(
                "Cannot emit event '{}' to label '{}': no such browser",
                event,
                label
            );
            false
        }
    }
}

/// Emit a browser-pane event to the top-level window that OWNS the pane.
///
/// `emit_event_from_state` always delivers into the window labelled `main`
/// (first-available fallback). A pane living in any OTHER top-level window
/// -- recreated in a promoted pool window after a tab tear-off, or opened
/// directly in a second window -- has its view model in THAT window's
/// renderer, so an event pushed to `main` is dropped on the floor: no
/// listener for that `block_id` exists there, and no warning fires because
/// `main` resolves fine. Symptom: the pane's loading overlay never clears
/// and its address bar / tab title never update, while outbound navigation
/// (request/response, window-agnostic) keeps working. PR #2597 fixed the
/// same class of bug for `browser-pane-clicked` / shortcut / context-menu;
/// this covers the remaining push sites (nav-state x3, title, favicon).
///
/// Falls back to the legacy `main` routing (with a warning) only when the
/// pane has no reducer entry or its `window_label` is empty (the legacy
/// `EnqueueBrowserPaneCreate` path), so nothing that worked before regresses.
/// Returns whether a browser was found to deliver to.
pub fn emit_browser_pane_event(
    state: &crate::state::AppState,
    block_id: &str,
    event: &str,
    payload: &serde_json::Value,
) -> bool {
    // Resolve under the lock, then DROP it before emitting: `emit_event_to_window`
    // re-locks `host_state` for its own browser lookup.
    let target = {
        let host = state.host_state.lock();
        browser_pane_event_target(&host, block_id)
    };
    match target {
        Some(label) => emit_event_to_window(state, &label, event, payload),
        None => {
            tracing::warn!(
                "[browser-pane] no owning window recorded for block_id={} -- falling back to legacy main-window routing for '{}'",
                block_id,
                event
            );
            emit_event_from_state(state, event, payload)
        }
    }
}

/// Pure routing decision behind `emit_browser_pane_event`: the label of the
/// top-level window a pane's events must be delivered to, or `None` when the
/// pane is unknown or recorded no window (caller falls back). Kept free of
/// CEF handles so the reducer tests can pin it.
pub(crate) fn browser_pane_event_target(
    host: &crate::reducer::HostState,
    block_id: &str,
) -> Option<String> {
    host.browser_panes
        .get(block_id)
        .map(|e| e.window_label.as_str())
        .filter(|label| !label.is_empty())
        .map(str::to_string)
}
