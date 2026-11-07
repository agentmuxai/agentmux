// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/vault/{show,update,verify,health}`: what an agent can ask the Vault about an entry and about itself
//! (docs/specs/SPEC_VAULT_PARITY_ENTRY_HISTORY_REASONS_AND_IMPORT_2026_11_06.md §8). Each answers ids, names, states
//! and counts, for the Vaults the proven agent may use; none returns a value. `update` changes details only and needs
//! the agent's signed identity and a reason, as the record names who changed what.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::vault_handlers::{asking_agent, publish};
use super::AppState;
use crate::backend::vault::inspect::{self, KeyResult, UpdateRequest};
use crate::backend::vault::seal;

type Auth = Option<agentmux_common::api_types::UiAutomationAuth>;

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

/// The agent whose signed identity verified, else none (which sees only the Vaults open to every agent).
fn who(state: &AppState, auth: &Auth) -> Option<String> {
    let (agent, pane) = asking_agent(state, auth.as_ref());
    pane.map(|_| agent)
}

/// The Vault key as it reads now, or why it can't be.
async fn key() -> KeyResult {
    tokio::task::spawn_blocking(|| seal::key(false)).await.unwrap_or_else(|e| Err(e.to_string()))
}

/// Runs `read` on the blocking pool and answers its result as JSON; a refusal is a 404 (the entry isn't one this agent
/// may use, or isn't there) or 500.
async fn answer<T: Serialize + Send + 'static>(read: impl FnOnce() -> Result<T, String> + Send + 'static) -> Response {
    match tokio::task::spawn_blocking(read).await {
        Ok(Ok(out)) => Json(out).into_response(),
        Ok(Err(e)) if e.starts_with("no entry") => error(StatusCode::NOT_FOUND, e),
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, e),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct IdBody {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    auth: Auth,
}

/// `POST /api/v1/vault/show`
pub(crate) async fn handle_vault_show(State(state): State<AppState>, Json(body): Json<IdBody>) -> Response {
    let agent = who(&state, &body.auth);
    let Some(id) = body.id.filter(|i| !i.trim().is_empty()) else { return error(StatusCode::BAD_REQUEST, "show needs an id: the entry to look at (list shows them)") };
    let (store, key) = (state.identity_store.clone(), key().await);
    answer(move || inspect::show(&store, &id, agent.as_deref(), &key, crate::backend::vault::expiry::today())).await
}

/// `POST /api/v1/vault/verify`
pub(crate) async fn handle_vault_verify(State(state): State<AppState>, Json(body): Json<IdBody>) -> Response {
    let agent = who(&state, &body.auth);
    let (store, key) = (state.identity_store.clone(), key().await);
    answer(move || inspect::verify(&store, body.id.as_deref(), agent.as_deref(), &key, crate::backend::vault::expiry::today())).await
}

/// `POST /api/v1/vault/health`
pub(crate) async fn handle_vault_health(State(state): State<AppState>, Json(body): Json<IdBody>) -> Response {
    let agent = who(&state, &body.auth);
    let (store, key) = (state.identity_store.clone(), key().await);
    let backend = crate::identity::secret_store::backend_name();
    answer(move || inspect::health(&store, agent.as_deref(), &key, backend, crate::backend::vault::expiry::today())).await
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateBody {
    #[serde(flatten)]
    request: UpdateRequest,
    #[serde(default)]
    auth: Auth,
}

/// `POST /api/v1/vault/update`
pub(crate) async fn handle_vault_update(State(state): State<AppState>, Json(body): Json<UpdateBody>) -> Response {
    let Some(agent) = who(&state, &body.auth) else {
        return error(StatusCode::FORBIDDEN, "update needs the agent's signed identity: the Vault records who changed each entry");
    };
    let store = state.identity_store.clone();
    let request = body.request;
    let response = answer(move || inspect::update_details(&store, &request, &agent)).await;
    if response.status().is_success() {
        publish(&state);
    }
    response
}
