// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Vault's routes (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5.4).
//!
//! - `POST /api/v1/host/vault`: everything the pane does, relayed by the
//!   host. `X-Host-Token` must be the host's IPC token, which agents never
//!   see: the instance auth key every agent holds isn't enough, so no agent
//!   can read a tail, or plant, change or delete an entry.
//! - `GET /api/v1/vault`: for agents, each entry's id, label, kind and
//!   section. No value, tail, field or grant.
//! - `POST /api/v1/vault/request`: an agent asks the person for a new entry
//!   and waits for the answer: the new entry's id, or declined.

use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::backend::vault::{self, requests, EntryChange, NewEntry};

use super::AppState;

/// How long one "a Vault pane is showing" note lasts: the pane renews it
/// every 10 seconds while shown, so a crashed window doesn't hold it.
const ON_SCREEN_MS: i64 = 30_000;

const DEFAULT_WAIT: Duration = Duration::from_secs(300);
const MAX_WAIT: Duration = Duration::from_secs(600);

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

/// Tells every UI the Vault or its requests changed. Carries no entry.
pub(crate) fn publish(state: &AppState) {
    let event = crate::backend::mps_events::VaultEvent {
        requests: requests::requests().list(),
        changed_ms: agentmux_common::time::now_ms_u64(),
    };
    state.broker.publish_event::<crate::backend::mps_events::Vault>(&event, vec![], 0);
}

/// What AgentMux already keeps in the keychain, managed elsewhere: named,
/// never read (§5.2 "Kept by AgentMux").
fn kept(state: &AppState) -> Vec<Value> {
    let mut out = Vec::new();
    if let Ok(accounts) = state.id_store.identity_list(None) {
        for a in accounts {
            if matches!(a.secret_ref, crate::backend::storage::identities::SecretRef::Keychain { .. }) {
                let name = if a.display_name.is_empty() { a.name } else { a.display_name };
                out.push(json!({ "label": name, "what": format!("{} account key", a.provider), "managed_in": "accounts" }));
            }
        }
    }
    if let Ok(Some(email)) = state.mstore.vault_kept_muxbus_user() {
        let label = if email.is_empty() { "MuxBus sign-in".to_string() } else { format!("MuxBus sign-in ({email})") };
        out.push(json!({ "label": label, "what": "cloud messages sign-in", "managed_in": "muxbus" }));
    }
    for p in crate::backend::tower_peers::Peers::global().list() {
        out.push(json!({ "label": p.hostname, "what": "paired Tower computer's token", "managed_in": "tower" }));
    }
    out
}

/// `POST /api/v1/host/vault`: the pane's operations, from the host only.
pub(crate) async fn handle_host_vault(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if !super::ui_handlers::from_host(&state, &headers).await {
        return error(StatusCode::FORBIDDEN, "only the AgentMux host can reach the Vault");
    }
    let op = body.get("op").and_then(Value::as_str).unwrap_or("").to_string();
    let store = state.identity_store.clone();
    let st = state.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<(Value, bool), String> {
        match op.as_str() {
            "list" => {
                let (entries, problem) = vault::list_for_pane(&store)?;
                Ok((
                    json!({
                        "entries": entries,
                        "problem": problem,
                        "requests": requests::requests().list(),
                        "backend": crate::identity::secret_store::backend_name(),
                    }),
                    false,
                ))
            }
            "create" => {
                let request_id = body.get("request_id").and_then(Value::as_str).unwrap_or("").to_string();
                let entry: NewEntry = serde_json::from_value(body).map_err(|e| format!("not an entry: {e}"))?;
                let id = vault::create(&store, entry)?;
                if !request_id.is_empty() {
                    if let Some(req) = requests::requests().get(&request_id) {
                        vault::record(&store, &id, &req.label, vault::PERSON, "filled a request", &req.agent, "done");
                    }
                    requests::requests().answer(&request_id, requests::Answer::Created(id.clone()));
                }
                Ok((json!({ "id": id }), true))
            }
            "update" => {
                let change: EntryChange = serde_json::from_value(body).map_err(|e| format!("not a change: {e}"))?;
                vault::update(&store, change)?;
                Ok((json!({}), true))
            }
            "delete" => {
                let id = body.get("id").and_then(Value::as_str).unwrap_or("");
                vault::delete(&store, id)?;
                Ok((json!({}), true))
            }
            "decline_request" => {
                let id = body.get("request_id").and_then(Value::as_str).unwrap_or("");
                if let Some(req) = requests::requests().get(id) {
                    vault::record(&store, "", &req.label, vault::PERSON, "declined a request", &req.agent, "declined");
                }
                requests::requests().answer(id, requests::Answer::Declined);
                Ok((json!({}), true))
            }
            "requests" => Ok((json!({ "requests": requests::requests().list() }), false)),
            // The pane says it is showing (or no longer is). Only the host can
            // say so: an agent clearing it could then capture the window.
            "on_screen" => {
                let on = body.get("on").and_then(Value::as_bool).unwrap_or(false);
                let until = if on { agentmux_common::time::now_ms() + ON_SCREEN_MS } else { 0 };
                store.vault_on_screen_set(&on_screen_instance(), until).map_err(|e| e.to_string())?;
                Ok((json!({}), false))
            }
            "activity" => {
                let limit = body.get("limit").and_then(Value::as_i64).unwrap_or(200);
                Ok((json!({ "activity": vault::activity(&store, limit)? }), false))
            }
            "kept" => Ok((json!({ "kept": kept(&st) }), false)),
            other => Err(format!("unknown Vault operation {other:?}")),
        }
    })
    .await;
    match result {
        Ok(Ok((data, changed))) => {
            if changed {
                publish(&state);
            }
            Json(json!({ "ok": true, "data": data })).into_response()
        }
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, e),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// This srv's key among the AgentMux instances that share the identity store.
fn on_screen_instance() -> String {
    format!("srv-{}", std::process::id())
}

/// `GET /api/v1/vault/on_screen`: whether any AgentMux on this computer shows
/// a Vault pane now. `UI window` refuses to capture AgentMux windows while one
/// does (SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5.3).
pub(crate) async fn handle_vault_on_screen(State(state): State<AppState>) -> Response {
    let store = state.identity_store.clone();
    let now = agentmux_common::time::now_ms();
    match tokio::task::spawn_blocking(move || store.vault_on_screen(now)).await {
        Ok(Ok(on)) => Json(json!({ "on_screen": on })).into_response(),
        Ok(Err(e)) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// `GET /api/v1/vault`: the entries an agent can name. Reads no value, so it
/// never touches the keychain.
pub(crate) async fn handle_vault_list(State(state): State<AppState>) -> Response {
    let store = state.identity_store.clone();
    match tokio::task::spawn_blocking(move || vault::list_for_agent(&store)).await {
        Ok(Ok(entries)) => Json(json!({ "entries": entries })).into_response(),
        Ok(Err(e)) => error(StatusCode::INTERNAL_SERVER_ERROR, e),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct RequestBody {
    kind: String,
    label: String,
    #[serde(default)]
    why: String,
    /// The asking agent's signed identity: a name in the body proves nothing.
    #[serde(default)]
    auth: Option<agentmux_common::api_types::UiAutomationAuth>,
    #[serde(default)]
    wait_secs: Option<u64>,
}

/// Who the request says is asking, and its pane: the agent whose signature
/// verified, else a plain "not a verified agent".
fn asking_agent(state: &AppState, auth: Option<&agentmux_common::api_types::UiAutomationAuth>) -> (String, Option<String>) {
    const UNVERIFIED: &str = "Something on this computer (not a verified agent)";
    match auth.map(|a| (a, super::ui_handlers::verified_block_id(state, None, a))) {
        Some((a, Ok(pane))) => (a.agent_id.chars().take(64).collect(), Some(pane)),
        Some((a, Err(e))) => {
            tracing::warn!(claimed = %a.agent_id, error = %e, "a Vault request's signed identity didn't verify");
            (UNVERIFIED.to_string(), None)
        }
        None => (UNVERIFIED.to_string(), None),
    }
}

/// `POST /api/v1/vault/request`: ask the person for a new entry; answer when
/// they save or dismiss it, or when the wait runs out (the request stays
/// open in the Vault, and the agent finds the entry with `list` later).
pub(crate) async fn handle_vault_request(State(state): State<AppState>, Json(body): Json<RequestBody>) -> Response {
    if vault::kinds::kind(&body.kind).is_none() {
        let kinds: Vec<&str> = vault::kinds::KINDS.iter().map(|k| k.name).collect();
        return error(StatusCode::BAD_REQUEST, format!("unknown kind {:?}; one of {}", body.kind, kinds.join(", ")));
    }
    let label = body.label.trim().chars().take(200).collect::<String>();
    if label.is_empty() {
        return error(StatusCode::BAD_REQUEST, "a request needs a label: what the entry is, e.g. \"Stripe test key\"");
    }
    let (agent, pane) = asking_agent(&state, body.auth.as_ref());
    let req = requests::VaultRequest {
        id: uuid::Uuid::new_v4().to_string(),
        agent: agent.clone(),
        kind: body.kind.clone(),
        label: label.clone(),
        why: body.why.trim().chars().take(500).collect(),
        requested_ms: agentmux_common::time::now_ms_u64(),
    };
    {
        let store = state.identity_store.clone();
        let (label, agent, kind) = (label.clone(), agent.clone(), body.kind.clone());
        let _ = tokio::task::spawn_blocking(move || vault::record(&store, "", &label, &agent, "requested an entry", &kind, "asked")).await;
    }
    let answer = requests::requests().ask(req);
    publish(&state);
    let _asking = crate::backend::user_attention::Asking::start(
        &state.broker,
        &state.mstore,
        pane.as_deref().unwrap_or(""),
        "vault",
        &format!("{agent} asks for a Vault entry: {label}"),
    );
    let wait = body.wait_secs.map(Duration::from_secs).unwrap_or(DEFAULT_WAIT).min(MAX_WAIT);
    match tokio::time::timeout(wait, answer).await {
        Ok(Ok(requests::Answer::Created(id))) => Json(json!({ "status": "created", "id": id })).into_response(),
        Ok(Ok(requests::Answer::Declined)) | Ok(Err(_)) => {
            Json(json!({ "status": "declined", "message": "the person dismissed the request" })).into_response()
        }
        Err(_) => Json(json!({
            "status": "pending",
            "message": "the person hasn't answered yet; the request stays open in the Vault for an hour, and a saved entry shows up in list",
        }))
        .into_response(),
    }
}

#[cfg(test)]
mod tests;
