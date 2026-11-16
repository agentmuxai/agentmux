// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The two ways a window is answered from outside: the agent's `Agents ready`
//! (`POST /api/v1/agent/self/ready`) and the person's "Switch now" (`agent.prep.now`).

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::COMMAND_AGENT_PREP_NOW;
use crate::server::caller::Caller;
use crate::server::AppState;

use super::ledger::{self, Answer};

#[derive(Debug, Deserialize)]
pub struct ReadyRequest {
    /// The caller's slug, which its token must name.
    agent: String,
    #[serde(default)]
    handoff: Option<String>,
    #[serde(default)]
    more_time: bool,
    #[serde(default)]
    why: Option<String>,
}

fn refused(status: StatusCode, why: &str) -> Response {
    (status, Json(json!({ "error": why }))).into_response()
}

/// `POST /api/v1/agent/self/ready`: the caller answers the window open for its own agent. Acts only
/// on the pane its token names, never on a name, so a prep id that leaks is no use to another agent.
pub async fn handle_ready_self(State(state): State<AppState>, caller: Option<Extension<Caller>>, Json(req): Json<ReadyRequest>) -> Response {
    let Some(block_id) = crate::server::actor::own_pane_by_token(&state.mstore, caller.as_deref(), req.agent.trim()) else {
        return refused(StatusCode::UNAUTHORIZED, "no pane for this agent token: Agents ready answers only the calling agent's own change");
    };
    let def_id = match state.mstore.get::<crate::backend::obj::Block>(&block_id) {
        Ok(Some(b)) => crate::backend::obj::meta_get_string(&b.meta, "agentId", ""),
        _ => String::new(),
    };
    if def_id.is_empty() {
        return refused(StatusCode::BAD_REQUEST, "the pane is not an agent's");
    }
    let slug = req.agent.trim().to_string();
    let answer = Answer { handoff: req.handoff, more_time: req.more_time, why: req.why };
    let result = tokio::task::spawn_blocking(move || ledger::answer(&state, &def_id, &slug, answer)).await;
    match result {
        Ok(Ok(body)) => Json(body).into_response(),
        Ok(Err(why)) => refused(StatusCode::CONFLICT, &why),
        Err(e) => refused(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

/// `agent.prep.now`: end the window the agent is in and let the change run.
#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AgentPrepNowReq {
    pub agent_id: String,
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AgentPrepNowResult {
    /// Whether a window was open for the agent.
    pub ended: bool,
}

/// Registers `agent.prep.now`. Open to agents as well as the window, as every account change is: the
/// audit row of the change names whoever asked.
pub fn register(engine: &Arc<WshRpcEngine>, _state: &AppState) {
    engine.register_typed(COMMAND_AGENT_PREP_NOW, move |req: AgentPrepNowReq, ctx| async move {
        if ctx.agent_id.is_empty() {
            ctx.operator_only("only the AgentMux window or an agent ends a change's window")?;
        }
        Ok(AgentPrepNowResult { ended: ledger::person_says_now(req.agent_id.trim()).is_ok() })
    });
}
