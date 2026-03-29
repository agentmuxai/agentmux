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

/// Emit an event using the browser stored in AppState.
/// This is a convenience wrapper for use from command handlers and background tasks.
pub fn emit_event_from_state(state: &crate::state::AppState, event: &str, payload: &serde_json::Value) {
    let browser = state.browser.lock().unwrap();
    if let Some(ref browser) = *browser {
        emit_event(browser, event, payload);
    } else {
        tracing::warn!("Cannot emit event '{}': no browser handle in state", event);
    }
}
