// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `App history_reclaim` (`GET`/`POST /api/v1/agent/history/reclaim`): a host or container agent asks for the
//! shared pane-history store's free space to be given back at the next start, withdraws that request, or reads
//! where it stands. The window does the same with `history.reclaim` (Warden > History > Storage); nothing in
//! the request touches the host's files from the agent's side: srv writes the one marker beside its own store.
//!
//! The request is recorded as the calling agent (the marker's `requested_by` is its slug, and the HTTP layer
//! records the call under its token, never as the operator). It runs only at an srv start, so the operator
//! sees it pending in Storage and can cancel it there. Asking again keeps the first request.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::backend::history_usage;
use crate::backend::storage::filestore::reclaim;
use crate::server::caller::Caller;
use crate::server::state::AppState;

#[derive(Debug, Deserialize)]
pub(crate) struct ReclaimRequest {
    /// `request` or `cancel`.
    action: String,
}

struct Refusal {
    status: StatusCode,
    message: String,
}

fn refuse(status: StatusCode, message: impl Into<String>) -> Refusal {
    Refusal { status, message: message.into() }
}

/// The slug of the agent the request's token names.
fn caller_slug(state: &AppState, caller: Option<&Caller>) -> Result<String, Refusal> {
    let Some(uid) = caller.and_then(Caller::uid) else {
        return Err(refuse(
            StatusCode::UNAUTHORIZED,
            "no agent token: App history_reclaim acts as the calling agent; the operator uses Warden > History > Storage",
        ));
    };
    let def = state
        .mstore
        .agent_def_get(uid)
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("history reclaim: {e}")))?
        .ok_or_else(|| refuse(StatusCode::UNAUTHORIZED, "the agent token names no agent on this machine"))?;
    Ok(def.slug)
}

/// The shared store's database file.
fn store_db() -> Result<std::path::PathBuf, Refusal> {
    crate::backend::agent_session::global_transcript_store()
        .and_then(|s| s.path().map(|p| p.to_path_buf()))
        .ok_or_else(|| refuse(StatusCode::CONFLICT, "the shared pane-history store isn't open"))
}

/// The request and last result, as the answer says them.
fn status_of(db: &std::path::Path) -> Value {
    let r = history_usage::reclaim_of(db);
    let (marker, _) = reclaim::status(db);
    let mut out = json!({
        "pending": r.pending,
        "waiting_for": r.waiting_for,
        "last_error": r.last_error,
        "last_done_ms": r.last_done_ms,
        "last_before_bytes": r.last_before_bytes,
        "last_after_bytes": r.last_after_bytes,
    });
    if let Some(m) = marker {
        out["requested_by"] = json!(m.requested_by);
        out["requested_at_ms"] = json!(m.requested_at_ms);
    }
    out
}

/// Request, cancel or read the reclaim of the store at `db`, as `by`.
fn change(db: &std::path::Path, action: &str, by: &str, now_ms: u64) -> Result<Value, Refusal> {
    // Whose request a cancel takes back: anyone may withdraw it (the person can in Storage, an agent here),
    // but the answer and the log say whose it was.
    let withdrawn = reclaim::status(db).0.map(|m| m.requested_by);
    let done = match action {
        "request" => reclaim::request(db, by, now_ms),
        "cancel" => reclaim::cancel(db),
        other => return Err(refuse(StatusCode::BAD_REQUEST, format!("INVALID_ARG: unknown action {other:?} (request or cancel)"))),
    };
    done.map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("history reclaim: {e}")))?;
    tracing::info!(agent = by, action, previously_requested_by = ?withdrawn, "history reclaim: {}", if action == "request" { "asked for at the next start" } else { "withdrawn" });
    let mut out = status_of(db);
    if action == "cancel" {
        if let Some(was) = withdrawn {
            out["withdrew_request_of"] = json!(was);
        }
    }
    Ok(out)
}

fn respond(result: Result<Value, Refusal>) -> Response {
    match result {
        Ok(body) => Json(body).into_response(),
        Err(r) => (r.status, Json(json!({ "error": r.message }))).into_response(),
    }
}

/// `POST /api/v1/agent/history/reclaim`: `{ "action": "request" | "cancel" }`.
pub(crate) async fn handle_history_reclaim(
    State(state): State<AppState>,
    caller: Option<Extension<Caller>>,
    Json(req): Json<ReclaimRequest>,
) -> Response {
    let caller = caller.map(|Extension(c)| c);
    respond(caller_slug(&state, caller.as_ref()).and_then(|by| {
        let db = store_db()?;
        change(&db, &req.action, &by, agentmux_common::time::now_ms_u64())
    }))
}

/// `GET /api/v1/agent/history/reclaim`: where the request and the last reclaim stand; changes nothing.
pub(crate) async fn handle_history_reclaim_status(State(state): State<AppState>, caller: Option<Extension<Caller>>) -> Response {
    let caller = caller.map(|Extension(c)| c);
    respond(caller_slug(&state, caller.as_ref()).and_then(|_| Ok(status_of(&store_db()?))))
}

#[cfg(test)]
#[path = "tests/agent_history_reclaim_tests.rs"]
mod tests;
