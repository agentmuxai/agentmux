// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `GET /agentmux/discovery` with the live facts' parameters (spec §5): the same answer as ever with no
//! parameter, this host's agents narrowed, paged and given the groups asked for with `fields`, `detail`
//! and the filters, or, with `view=status`, the compact status (`Agents status`).

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

pub(crate) async fn handle(State(state): State<AppState>, Query(query): Query<HashMap<String, String>>) -> Response {
    let params = match Params::from_map(&query, &state.hostname) {
        Ok(p) => p,
        Err(e) => return bad_request(e),
    };
    let compact = match query.get("view").map(|v| v.trim()) {
        None | Some("") | Some("list") => false,
        Some("status") => true,
        Some(other) => return bad_request(format!("unknown view {other:?}: use list or status")),
    };
    let Json(mut body) = crate::server::http_health::handle_discovery(State(state.clone())).await;
    // Nothing asked for: the answer as it always was, plus each row's short model.
    let rows: Vec<Value> = match body.pointer_mut("/host/agents").map(Value::take) {
        Some(Value::Array(rows)) => rows,
        _ => Vec::new(),
    };
    let now_ms = agentmux_common::time::now_ms_u64();
    let worker = state.clone();
    if compact {
        let answer = tokio::task::spawn_blocking(move || status(&worker, rows, &params, now_ms)).await;
        return answer.map_or_else(|e| bad_request(e.to_string()), |a| Json(a).into_response());
    }
    match tokio::task::spawn_blocking(move || apply(&worker, rows, &params, now_ms)).await {
        Ok((agents, page)) => {
            body["host"]["agents"] = Value::Array(agents);
            body["host"]["page"] = page;
            Json(body).into_response()
        }
        Err(e) => bad_request(e.to_string()),
    }
}
