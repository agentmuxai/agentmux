// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/agent/status`: `Presence status`. An agent says, in one line,
//! what it is doing now. The line is kept in its pane's block meta with the
//! time (`backend::work_facts::live_phrase`), which is where the Swarm row and
//! `work.goal` read it. A task claim writes the task's title in the same place
//! (`work_queue::handle_work_claim`).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde::Deserialize;
use serde_json::json;

use crate::backend::work_facts::live_phrase;

use super::caller::Caller;
use super::work_claims_handlers::claimer;
use super::AppState;

/// `Presence status`'s arguments, plus who is asking (`agent`, `block`: the
/// MCP's own `AGENTMUX_AGENT_ID` and block id; the request's agent token wins).
#[derive(Debug, Default, Deserialize)]
pub struct StatusParams {
    pub line: Option<String>,
    pub state: Option<String>,
    pub agent: Option<String>,
    pub block: Option<String>,
}

fn bad_request(msg: impl Into<String>) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": msg.into() }))).into_response()
}

/// Write `meta` to the block's own meta, the way a pane title is set, so the
/// change is stored and reaches every window.
pub(super) async fn write_block_meta(state: &AppState, block_id: &str, meta: serde_json::Value) -> Result<(), String> {
    let call = crate::backend::service::WebCallType {
        service: "object".to_string(),
        method: "UpdateObjectMeta".to_string(),
        uicontext: None,
        args: vec![json!(format!("block:{block_id}")), meta],
    };
    match super::service::run_service_call(state, &call).await.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// `POST /api/v1/agent/status`.
pub(crate) async fn handle_agent_status(
    State(state): State<AppState>,
    caller: Option<axum::Extension<Caller>>,
    Json(p): Json<StatusParams>,
) -> Response {
    let (line, cut) = match live_phrase::clean_line(p.line.as_deref().unwrap_or("")) {
        Ok(v) => v,
        Err(e) => return bad_request(e),
    };
    let status = match live_phrase::clean_state(p.state.as_deref()) {
        Ok(v) => v,
        Err(e) => return bad_request(e),
    };
    let Some((agent, _uid, block)) = claimer(&state, caller.as_ref().map(|c| &c.0), p.agent.as_deref(), p.block.as_deref())
    else {
        return bad_request("who is setting the status is unknown: this needs an agent started by AgentMux");
    };
    let Some(block_id) = block else {
        return bad_request("this agent has no pane here, so there is nowhere to keep its status line");
    };
    let now = agentmux_common::time::now_ms();
    let meta = live_phrase::set_meta(&line, status, live_phrase::SRC_AGENT, now);
    if let Err(e) = write_block_meta(&state, &block_id, meta).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("status not saved: {e}") }))).into_response();
    }
    Json(json!({
        "ok": true,
        "agent": agent,
        "line": line,
        "state": status,
        "set_at": now,
        "cut_to_fit": cut,
        "note": "Shown in the Swarm and in Agents conversations until you set another line or claim a task. Set it again when \
                 your plan changes, when you are blocked or waiting, and when you are done.",
    }))
    .into_response()
}

/// A task claim: the task's title becomes the claimer's line (source `task`),
/// replacing a line from the task before. The agent replaces it with its own
/// words by setting a status. Skipped when the claim names no pane.
pub(super) fn task_claimed(state: &AppState, block_id: &str, number: Option<i64>, title: &str) {
    let block_id = block_id.trim();
    let Some(line) = live_phrase::task_line(number.and_then(|n| u64::try_from(n).ok()), title) else { return };
    if block_id.is_empty() {
        return;
    }
    let (state, block_id) = (state.clone(), block_id.to_string());
    tokio::spawn(async move {
        let meta = live_phrase::set_meta(&line, None, live_phrase::SRC_TASK, agentmux_common::time::now_ms());
        if let Err(e) = write_block_meta(&state, &block_id, meta).await {
            tracing::warn!(block_id = %block_id, error = %e, "task claim: could not set the status line");
        }
    });
}

/// A task done or handed back: a line that only came from the task goes, so
/// the row does not go on naming work that ended. A line the agent set itself
/// stays (it said "Done: ..." or something newer).
pub(super) fn task_ended(state: &AppState, agent_id: &str) {
    let (state, agent_id) = (state.clone(), agent_id.trim().to_string());
    if agent_id.is_empty() {
        return;
    }
    tokio::spawn(async move {
        let blocks: Vec<String> = state
            .reactive_handler
            .list_agents()
            .into_iter()
            .filter(|r| r.agent_id.eq_ignore_ascii_case(&agent_id))
            .map(|r| r.block_id)
            .collect();
        for block_id in blocks {
            let mstore = state.mstore.clone();
            let id = block_id.clone();
            let from_task = tokio::task::spawn_blocking(move || {
                mstore
                    .get::<crate::backend::obj::Block>(&id)
                    .ok()
                    .flatten()
                    .and_then(|b| live_phrase::declared_of(&b.meta))
                    .is_some_and(|d| d.src == live_phrase::SRC_TASK)
            })
            .await
            .unwrap_or(false);
            if from_task {
                if let Err(e) = write_block_meta(&state, &block_id, live_phrase::clear_meta()).await {
                    tracing::warn!(block_id = %block_id, error = %e, "task end: could not clear the status line");
                }
            }
        }
    });
}
