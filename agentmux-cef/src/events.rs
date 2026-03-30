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
pub fn emit_event_from_state(state: &crate::state::AppState, event: &str, payload: &serde_json::Value) {
    let browsers = state.browsers.lock().unwrap();
    if let Some(browser) = browsers.get("main") {
        emit_event(browser, event, payload);
    } else if let Some((_label, browser)) = browsers.iter().next() {
        // Fallback: emit to any available browser
        emit_event(browser, event, payload);
    } else {
        tracing::warn!("Cannot emit event '{}': no browser handle in state", event);
    }
}

/// Emit an event to ALL browser windows (for cross-window drag broadcasts).
pub fn emit_event_all_windows(state: &crate::state::AppState, event: &str, payload: &serde_json::Value) {
    let browsers = state.browsers.lock().unwrap();
    if browsers.is_empty() {
        tracing::warn!("Cannot broadcast event '{}': no browsers", event);
        return;
    }
    for (_label, browser) in browsers.iter() {
        emit_event(browser, event, payload);
    }
}
