// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! What an agent can do about a workspace that is going to stop (`App` `workspace` and `keep_running`).
//!
//! - `GET /api/v1/app/workspace`: whether a stop is pending, when and why (the reason ids and phrases of the
//!   `workspace:stop` event), and what an agent last asked for.
//! - `POST /api/v1/agent/keep-running { minutes }`: ask for the pending stop to be pushed back by 1 to 240
//!   minutes. It is recorded as the agent that asked (`agent:<id>`); srv hands the request to its registered
//!   external host with the next activity answer, and the host decides whether to honour it. With no stop
//!   pending there is nothing to put back and it is refused. Both work for host and container agents alike.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::server::caller::Caller;
use crate::server::host_control::{self, KeepRefusal, KeepRunning, Stop};
use crate::server::AppState;

/// The workspace's stop as an agent reads it.
pub(crate) fn workspace_view(stop: Option<Stop>, keep: Option<KeepRunning>) -> Value {
    json!({
        "stopping": stop.is_some(),
        "stop_at_ms": stop.map(|s| s.at_ms),
        "reason_id": stop.map(|s| s.reason.id()),
        "reason": stop.map(|s| s.reason.phrase()),
        "keep_running": keep.map(|k| json!({ "minutes": k.minutes, "by": k.by, "at_ms": k.at_ms })),
    })
}

/// `GET /api/v1/app/workspace`.
pub(crate) async fn handle_workspace() -> Response {
    let (stop, keep) = host_control::current();
    Json(workspace_view(stop, keep)).into_response()
}

#[derive(Debug, Deserialize)]
pub(crate) struct KeepRunningRequest {
    minutes: u32,
}

fn refusal(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "ok": false, "error": message }))).into_response()
}

/// `POST /api/v1/agent/keep-running`.
pub(crate) async fn handle_keep_running(
    State(state): State<AppState>,
    caller: Option<Extension<Caller>>,
    Json(req): Json<KeepRunningRequest>,
) -> Response {
    let Some(uid) = caller.as_deref().and_then(Caller::uid) else {
        return refusal(StatusCode::UNAUTHORIZED, "no agent token: keep_running is recorded as the agent that asks");
    };
    let slug = match state.mstore.agent_def_get(uid) {
        Ok(Some(def)) => def.slug,
        Ok(None) => return refusal(StatusCode::UNAUTHORIZED, "the agent token names no agent on this machine"),
        Err(e) => return refusal(StatusCode::INTERNAL_SERVER_ERROR, &format!("keep_running: {e}")),
    };
    match host_control::ask_keep_running(req.minutes, &format!("agent:{slug}")) {
        Ok(k) => {
            tracing::info!(by = %k.by, minutes = k.minutes, "an agent asked for the workspace's stop to be put back");
            Json(json!({
                "ok": true,
                "minutes": k.minutes,
                "by": k.by,
                "note": "recorded as yours; the workspace's timer decides whether to honour it: read App workspace to see the stop time",
            }))
            .into_response()
        }
        Err(e @ KeepRefusal::Minutes) => refusal(StatusCode::BAD_REQUEST, e.message()),
        Err(e @ KeepRefusal::NoStop) => refusal(StatusCode::CONFLICT, e.message()),
    }
}

#[cfg(test)]
#[path = "workspace_stop_tests.rs"]
mod tests;
