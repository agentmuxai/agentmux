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
        // The store could not be read: refuse (503), so the MCP fails the
        // tool call instead of sending a bare name that a same-named live
        // agent could take (Codex P1 on #3563).
        Ok(Err(e)) => (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": e }))),
        Ok(Ok(resolution)) => {
            let mut body = serde_json::to_value(&resolution).unwrap_or_else(|_| json!({ "resolution": "none" }));
            // A name nobody answers to: the ids it most likely meant, so the refusal can offer them.
            if matches!(resolution, crate::backend::name_resolution::NameResolution::None) {
                body["did_you_mean"] = json!(did_you_mean(&state, &req.name).await);
            }
            (StatusCode::OK, Json(body))
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("resolve task failed: {e}") })),
        ),
    }
}

/// The ids of the agents on this computer that a typed name nobody answers to most likely meant, best first.
async fn did_you_mean(state: &AppState, typed: &str) -> Vec<String> {
    let (mstore, registry, typed) = (state.mstore.clone(), state.reactive_handler, typed.to_string());
    // A synchronous store read, so off the async worker (incident #1782).
    tokio::task::spawn_blocking(move || {
        let ids = crate::backend::name_resolution::known_agent_ids(&mstore, registry);
        crate::backend::name_resolution::closest_ids(&typed, &ids)
    })
    .await
    .unwrap_or_default()
}

/// A message `body` queued at the relay for `typed`, with the ids `typed` most likely meant when it is a name
/// that answers to no agent here (a typo, a workspace folder's form), for the sender to send it again to.
pub(crate) async fn with_did_you_mean(state: &AppState, typed: &str, mut body: Value) -> Value {
    let close = did_you_mean(state, typed).await;
    if !close.is_empty() {
        body["did_you_mean"] = json!(close);
    }
    body
}

/// The same resolution for a name typed by an agent of another machine in a remote operation
/// (`remote_ops`), which is as much a boundary as the MCP tool argument: that agent can't know a uid
/// here. Internal code still never calls `resolve_name_to_uid` itself (`check-name-resolver-callers.sh`).
pub(crate) async fn resolve_typed_name(
    state: &AppState,
    name: &str,
) -> Result<crate::backend::name_resolution::NameResolution, String> {
    let mstore = state.mstore.clone();
    let registry = state.reactive_handler;
    let name = name.to_string();
    // A synchronous store read, so off the async worker (incident #1782).
    tokio::task::spawn_blocking(move || crate::backend::name_resolution::resolve_name_to_uid(&mstore, registry, &name))
        .await
        .map_err(|e| format!("resolve task failed: {e}"))?
}
