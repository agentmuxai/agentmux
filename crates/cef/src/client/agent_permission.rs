// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A page's camera or microphone request in a browser pane an agent drives
//! (docs/reports/REPORT_AGENTS_AND_OS_PROMPTS_2026_11_07.md §3, tier 1). The person's own panes keep
//! the prompt in the pane. Here the request stays parked while srv decides (`/api/v1/host/browser_permission`):
//! the site was approved through the sensitive-operation countdown, so the page is allowed; otherwise
//! srv has filed the request and the page is denied now, so the agent and the page are never left waiting.

use std::sync::Arc;

use crate::browser_panes::media_prompt;

/// In a pane an agent drives and hasn't handed to the person: park `cb` and have srv decide.
/// `false` when the pane is not one (the caller prompts the person as before).
pub(super) fn hold(state: &Arc<crate::state::AppState>, cb: &cef::MediaAccessCallback, block_id: &str, origin: &str, requested: u32) -> bool {
    if !super::jsdialog::driven_pane(state, block_id) {
        return false;
    }
    // A pane is known to srv by its block id.
    let id = media_prompt::park(cb.clone(), block_id, origin, requested);
    media_prompt::arm_timeout(id);
    ask(state, id, block_id, origin, requested);
    true
}

/// Ask srv about the parked request `id` on a thread of its own (the caller is a CEF callback that
/// can't wait), then answer it on the UI thread. Anything but an `allow` is a denial, and so is
/// srv not answering.
pub(super) fn ask(state: &Arc<crate::state::AppState>, id: u64, target: &str, origin: &str, requested: u32) {
    let web_endpoint = state.backend_endpoints.lock().web_endpoint.clone();
    let auth_key = state.auth_key.lock().clone();
    let ipc_token = state.ipc_token.clone();
    let body = serde_json::json!({ "target": target, "origin": origin, "requested": requested });
    let started = std::thread::Builder::new().name("browser-permission".into()).spawn(move || {
        let allow = match super::backend_browser_permission(&web_endpoint, &auth_key, &ipc_token, &body) {
            Ok(decision) => decision == "allow",
            Err(e) => {
                tracing::warn!(error = %e, "browser pane: couldn't ask srv about a camera or microphone request; denying");
                false
            }
        };
        media_prompt::resolve_on_ui(id, allow);
    });
    if started.is_err() {
        media_prompt::resolve_on_ui(id, false);
    }
}
