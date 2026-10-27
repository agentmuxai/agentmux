// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! An agent's moves on the Kanban other than accept and send back
//! (`work_delegate.rs`): edit, move, start, answer, cancel, retry, reopen,
//! take back and reassign, with `Tasks`:
//!
//!   POST /agentmux/work/act   { agent_id, action, id, ... }
//!
//! The agent's token decides who acts. These are the same moves the Kanban
//! makes for the operator (`app_api/kanban.rs`); the history carries the agent
//! as the one who did it, and a message to another agent arrives as the
//! acting agent's, never as the operator's. `target` names the agent a move
//! is for (`agent` in the Kanban's own vocabulary is the asker here).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use super::app_api::kanban::{act_as, Actor};
use super::caller::Caller;
use super::AppState;

pub(crate) async fn handle_work_act(
    State(state): State<AppState>,
    caller: Option<axum::Extension<Caller>>,
    Json(mut body): Json<Value>,
) -> Response {
    let Some(uid) = caller.as_deref().and_then(Caller::uid).map(str::to_string) else {
        return (StatusCode::FORBIDDEN, Json(json!({ "error": "FORBIDDEN: this acts for an agent, so the request must carry that agent's token" }))).into_response();
    };
    let claimed = body.get("agent_id").and_then(Value::as_str).unwrap_or_default().to_string();
    let mstore = state.mstore.clone();
    let slug = tokio::task::spawn_blocking(move || super::actor::proven_slug(&mstore, &uid, &claimed)).await.ok().flatten();
    let Some(slug) = slug else {
        return (StatusCode::FORBIDDEN, Json(json!({ "error": "FORBIDDEN: the token's agent doesn't answer to that agent_id" }))).into_response();
    };
    if let Some(target) = body.get("target").cloned() {
        body["agent"] = target;
    }
    match act_as(&state, body, &Actor::agent(&slug)).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::CONFLICT, Json(json!({ "error": e }))).into_response(),
    }
}
