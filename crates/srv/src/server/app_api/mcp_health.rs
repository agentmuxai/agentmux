// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agentmux-mcp doctor`'s two calls (docs/specs/SPEC_AGENT_MCP_SELF_HEAL_2026_10_10.md
//! §7): what srv knows about the caller's own `agentmux` MCP server, and
//! "repair it now".
//!
//! `doctor` runs in the agent's shell, which has no MCP signing key (only the
//! MCP process does), so the caller's pane comes from its agent token
//! (`actor::own_pane_by_token`). There is no target: an agent can only look
//! at, and heal, its own pane.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::json;

use crate::server::caller::Caller;
use crate::server::state::AppState;

#[derive(Debug, Deserialize)]
pub(crate) struct SelfRequest {
    /// The caller's slug (`AGENTMUX_AGENT_SLUG`), which its token must name.
    agent: String,
}

fn own_pane(state: &AppState, caller: Option<&Caller>, agent: &str) -> Option<String> {
    crate::server::actor::own_pane_by_token(&state.mstore, caller, agent.trim())
}

fn no_pane() -> Response {
    let error = "no pane for this agent token: run this from the agent's own shell \
                 (it needs AGENTMUX_AGENT_TOKEN and AGENTMUX_AGENT_SLUG)";
    (StatusCode::UNAUTHORIZED, Json(json!({ "error": error }))).into_response()
}

/// `POST /api/v1/agent/self/mcp-health`
pub(crate) async fn handle_mcp_health(
    State(state): State<AppState>,
    caller: Option<Extension<Caller>>,
    Json(req): Json<SelfRequest>,
) -> Response {
    let Some(block_id) = own_pane(&state, caller.as_deref(), &req.agent) else {
        return no_pane();
    };
    let health = crate::backend::mcp_health::snapshot(&block_id);
    Json(json!({
        "block_id": block_id,
        "health": health,
        "safe_profile": crate::backend::mcp_health::is_safe_profile(&block_id),
        "auto": crate::backend::mcp_health::auto_enabled(),
    }))
    .into_response()
}

/// `POST /api/v1/agent/self/mcp-heal`: 202 `healing` when a repair started,
/// else 200 with why not (`connected`, `already_healing`, `no_status_yet`,
/// `no_process`).
pub(crate) async fn handle_mcp_heal(
    State(state): State<AppState>,
    caller: Option<Extension<Caller>>,
    Json(req): Json<SelfRequest>,
) -> Response {
    use crate::backend::blockcontroller::persistent::PersistentSubprocessController;
    let Some(block_id) = own_pane(&state, caller.as_deref(), &req.agent) else {
        return no_pane();
    };
    let Some(ctrl) = crate::backend::blockcontroller::get_controller(&block_id) else {
        return Json(json!({ "status": "no_process" })).into_response();
    };
    let Some(ctrl) = ctrl.as_any().downcast_ref::<PersistentSubprocessController>() else {
        return Json(json!({ "status": "no_process" })).into_response();
    };
    let outcome = ctrl.mcp_heal_now();
    tracing::info!(block_id = %block_id, agent = %req.agent, outcome = ?outcome, "agentmux-mcp doctor --heal");
    match outcome {
        Ok(()) => (StatusCode::ACCEPTED, Json(json!({ "status": "healing" }))).into_response(),
        Err(why) => Json(json!({ "status": why })).into_response(),
    }
}
