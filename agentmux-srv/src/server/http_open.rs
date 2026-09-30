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
        Err(e) => {
            // The impl's own error vocabulary: AGENT_NOT_FOUND /
            // TEMPLATE_NOT_OPENABLE / INVALID_PROVIDER / CLI_NOT_AVAILABLE
            // are the caller's problem (bad target or an uninstalled CLI
            // they must remedy first); anything else is a server-side
            // failure.
            let status = if e.starts_with("AGENT_NOT_FOUND")
                || e.starts_with("TEMPLATE_NOT_OPENABLE")
                || e.starts_with("INVALID_PROVIDER")
                || e.starts_with("CLI_NOT_AVAILABLE")
            {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, Json(json!({ "error": e }))).into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Agent App API REST handlers (identity / preset / memory).
//
// `agent_id` is the agent slug, supplied by agentmux-mcp from its trusted
// AGENTMUX_AGENT_ID env. Each handler maps a 4xx for caller/validation errors
// (FORBIDDEN, "not found", "provide …", "not a regular file") and 5xx otherwise,
// then delegates to the shared `app_api::*_impl`.
// ---------------------------------------------------------------------------
