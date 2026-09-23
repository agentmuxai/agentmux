// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /agentmux/agents/resolve` — the HTTP face of the one place a typed
//! agent name is interpreted (identity M3, spec §5). Called by `agentmux-mcp`
//! before `WorkEnqueue` and `CronCreate` so those tools can store a UID at
//! authoring time (§5.4) and hand an ambiguous name back to the model with
//! the candidates (§5.2) instead of enqueueing something that can never be
//! delivered unambiguously.
//!
//! Auth-gated like every other loopback route. The body names a *target*; it
//! never asserts the caller's own identity.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::AppState;

#[derive(Debug, Deserialize)]
pub(super) struct ResolveRequest {
    pub name: String,
}

pub(super) async fn handle_resolve_agent_name(
    State(state): State<AppState>,
    Json(req): Json<ResolveRequest>,
) -> (StatusCode, Json<Value>) {
    if req.name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "name is required" })),
        );
    }
    // A synchronous store read, so off the async worker (incident #1782).
    let mstore = state.mstore.clone();
    let registry = state.reactive_handler;
    let name = req.name.clone();
    match tokio::task::spawn_blocking(move || {
        crate::backend::name_resolution::resolve_name_to_uid(&mstore, registry, &name)
    })
    .await
    {
        Ok(resolution) => (
            StatusCode::OK,
            Json(
                serde_json::to_value(&resolution)
                    .unwrap_or_else(|_| json!({ "resolution": "none" })),
            ),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("resolve task failed: {e}") })),
        ),
    }
}
