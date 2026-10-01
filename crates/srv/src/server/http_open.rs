// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pane and agent open.
//! Split out of server/mod.rs unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.1).

use super::*;

/// `POST /api/v1/pane/open` — open a pane (editor/term/browser/…).
///
/// Called by `agentmux-mcp`'s `OpenEditor` tool. Thin HTTP wrapper over
/// `app_api::open_pane` (the same logic the WebSocket `pane.open` RPC uses):
/// creates the block, enqueues the layout action, and broadcasts the updates
/// so the frontend renders the pane. Body is `CommandPaneOpenData`.
pub(super) async fn handle_pane_open(
    State(state): State<AppState>,
    Json(req): Json<crate::backend::rpc_types::CommandPaneOpenData>,
) -> impl IntoResponse {
    match app_api::open_pane(&state, req).await {
        Ok(result) => (StatusCode::OK, Json(json!(result))).into_response(),
        Err(e) => {
            // Argument/validation errors from build_pane_meta are the caller's
            // fault (400); everything else is a server-side failure (500).
            let status = if e.starts_with("MISSING_ARG") || e.starts_with("INVALID_VIEW") {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, Json(json!({ "error": e }))).into_response()
        }
    }
}

/// `POST /api/v1/agent/open` — launch an agent into a pane (idempotent: an
/// agent already open in the target tab returns its existing block with
/// `created: false`, same as the RPC path).
///
/// Called by `agentmux-mcp`'s `OpenAgent` tool. Thin HTTP wrapper over
/// `app_api::open_agent_impl` — the same logic the WebSocket `agent.open`
/// RPC uses. Body is `CommandAgentOpenData` (`agent_id` by id or name;
/// optional `tab_id`/`focus`/split placement).
pub(super) async fn handle_agent_open(
    State(state): State<AppState>,
    Json(req): Json<crate::backend::rpc_types::CommandAgentOpenData>,
) -> impl IntoResponse {
    match app_api::open_agent_impl(&state, req).await {
        Ok(result) => (StatusCode::OK, Json(json!(result))).into_response(),
        Err(e) => (open_error_status(&e), Json(json!({ "error": e }))).into_response(),
    }
}

/// The HTTP status for `agent.open`'s own error vocabulary. AGENT_NOT_FOUND /
/// TEMPLATE_NOT_OPENABLE / INVALID_PROVIDER / CLI_NOT_AVAILABLE are the caller's
/// problem (a bad target, or a CLI that could not be installed); CLI_INSTALLING
/// is neither — the CLI is on its way, so the same request will succeed shortly
/// (503, retryable); anything else is a server-side failure.
fn open_error_status(e: &str) -> StatusCode {
    if e.starts_with("CLI_INSTALLING") {
        StatusCode::SERVICE_UNAVAILABLE
    } else if e.starts_with("AGENT_NOT_FOUND")
        || e.starts_with("TEMPLATE_NOT_OPENABLE")
        || e.starts_with("INVALID_PROVIDER")
        || e.starts_with("CLI_NOT_AVAILABLE")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

#[cfg(test)]
mod open_error_status_tests {
    use super::*;

    #[test]
    fn a_cli_that_is_still_installing_is_retryable_not_the_callers_fault() {
        assert_eq!(
            open_error_status("CLI_INSTALLING: claude 2.1.285 is still being installed"),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn the_callers_own_mistakes_are_400s() {
        for e in [
            "AGENT_NOT_FOUND: x",
            "TEMPLATE_NOT_OPENABLE: x",
            "INVALID_PROVIDER: x",
            "CLI_NOT_AVAILABLE: x could not be installed",
        ] {
            assert_eq!(open_error_status(e), StatusCode::BAD_REQUEST, "{e}");
        }
    }

    #[test]
    fn anything_else_is_a_server_failure() {
        assert_eq!(
            open_error_status("agent.open: meta serialize: boom"),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
