// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// `run_command` IPC handler — dispatches a command ID to the frontend registry.
//
// The Rust side does not own the command registry; it simply forwards the ID
// to the frontend via a CEF `CustomEvent`. The frontend's `commandRegistry.run(id)`
// handles validation and execution.

use std::sync::Arc;

use cef::{CefString, ImplBrowser, ImplFrame};

use crate::state::AppState;

/// Dispatch a command palette ID to the frontend of the target window.
///
/// Args:
///   `id`          — stable command ID (e.g. `"open:terminal"`)
///   `windowLabel` — (optional) which window to target; defaults to `"main"`
pub fn run_command(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let id = args["id"]
        .as_str()
        .ok_or_else(|| "run_command: missing 'id' field".to_string())?;

    let window_label = args["windowLabel"].as_str().unwrap_or("main");

    let js = format!(
        "window.dispatchEvent(new CustomEvent('agentmux-run-command', {{ detail: {{ id: {:?} }} }}));",
        id
    );

    let browsers = state.browsers.lock();
    let browser = browsers
        .get(window_label)
        .or_else(|| browsers.values().next());

    if let Some(browser) = browser {
        if let Some(frame) = browser.main_frame() {
            let code = CefString::from(js.as_str());
            let url = CefString::from("");
            frame.execute_java_script(Some(&code), Some(&url), 0);
            tracing::debug!(id = %id, window = %window_label, "[palette] dispatched run_command");
        }
    } else {
        tracing::warn!(id = %id, "[palette] run_command: no browser available");
    }

    Ok(serde_json::Value::Null)
}
