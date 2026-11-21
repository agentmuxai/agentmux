// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `GET /agentmux/discovery` with the live facts' parameters (spec §5): the same answer as ever with no
//! parameter, this host's agents narrowed, paged and given the groups asked for with `fields`, `detail`
//! and the filters, or, with `view=status`, the compact status (`Agents status`). The same answer is what
//! the LAN op `agents.status` gives a peer (`answer`).

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use super::query::{apply, status, Params};
use crate::server::AppState;

fn bad_request(message: String) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": message }))).into_response()
}

/// Which answer a query asks for: the discovery list (the default) or the compact status.
pub(crate) fn is_status_view(query: &HashMap<String, String>) -> Result<bool, String> {
    match query.get("view").map(|v| v.trim()) {
        None | Some("") | Some("list") => Ok(false),
        Some("status") => Ok(true),
        Some(other) => Err(format!("unknown view {other:?}: use list or status")),
    }
}

/// This machine's answer to `query`: the discovery body with its agents narrowed and paged, or the compact
/// status. With `host=all` it also has `peers`, the LAN peers a caller may ask the same question.
pub(crate) async fn answer(state: &AppState, query: &HashMap<String, String>) -> Result<Value, String> {
    let params = Params::from_map(query, &state.hostname)?;
    let compact = is_status_view(query)?;
    let Json(mut body) = crate::server::http_health::handle_discovery(State(state.clone())).await;
    // Nothing asked for: the answer as it always was, plus each row's short model.
    let rows: Vec<Value> = match body.pointer_mut("/host/agents").map(Value::take) {
        Some(Value::Array(rows)) => rows,
        _ => Vec::new(),
    };
    let now_ms = agentmux_common::time::now_ms_u64();
    let worker = state.clone();
    let all_hosts = params.all_hosts;
    let mut out = if compact {
        tokio::task::spawn_blocking(move || status(&worker, rows, &params, now_ms)).await.map_err(|e| e.to_string())?
    } else {
        let (agents, page) = tokio::task::spawn_blocking(move || apply(&worker, rows, &params, now_ms)).await.map_err(|e| e.to_string())?;
        body["host"]["agents"] = Value::Array(agents);
        body["host"]["page"] = page;
        body
    };
    if all_hosts {
        out["peers"] = json!(state.lan_discovery.get_instances());
    }
    Ok(out)
}

pub(crate) async fn handle(State(state): State<AppState>, Query(query): Query<HashMap<String, String>>) -> Response {
    match answer(&state, &query).await {
        Ok(body) => Json(body).into_response(),
        Err(e) => bad_request(e),
    }
}
