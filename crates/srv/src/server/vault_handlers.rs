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
//! - `POST /api/v1/vault/generate`: an agent creates an entry whose value the
//!   Vault makes; it gets the id back, never the value.

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

/// The file the host chose, which must be an absolute path.
fn file_path(body: &Value) -> Result<std::path::PathBuf, String> {
    let p = std::path::PathBuf::from(body.get("path").and_then(Value::as_str).unwrap_or(""));
    if !p.is_absolute() {
        return Err("a file path is needed".into());
    }
    Ok(p)
}

/// A malformed entry or change, said without serde's own message, which can
/// quote the value it couldn't read.
fn shape_error(what: &str) -> String {
    format!("{what} for the Vault isn't in the expected shape")
}

/// Tells every UI the Vault or its requests changed. Carries no entry.
pub(crate) fn publish(state: &AppState) {
    let event = crate::backend::mps_events::VaultEvent {
        requests: requests::requests().list(),
        uses: crate::backend::vault::uses::uses().list(),
        changed_ms: agentmux_common::time::now_ms_u64(),
    };
    state.broker.publish_event::<crate::backend::mps_events::Vault>(&event, vec![], 0);
    // A sign-in page already open lists what the Vault holds now.
    super::browser_signin_fill::refresh_all(state);
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
    // Sign-ins a browser pane saved in the keychain before the Vault held them, that haven't
    // moved in yet (the keychain couldn't be read at the last start).
    if let Ok(signins) = state.identity_store.browser_signins() {
        for (origin, realm, is_proxy, username) in signins {
            let site = if realm.is_empty() { origin } else { format!("{origin} ({realm})") };
            let who = if username.is_empty() { String::new() } else { format!(" for {username}") };
            let what = if is_proxy { format!("proxy sign-in{who} saved by a browser pane") } else { format!("sign-in{who} saved by a browser pane") };
            out.push(json!({ "label": site, "what": what, "managed_in": "browser" }));
        }
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
                        "uses": vault::uses::uses().list(),
                        "backend": crate::identity::secret_store::backend_name(),
                    }),
                    false,
                ))
            }
            "create" => {
                let request_id = body.get("request_id").and_then(Value::as_str).unwrap_or("").to_string();
                let entry: NewEntry = serde_json::from_value(body).map_err(|_| shape_error("an entry"))?;
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
                let change: EntryChange = serde_json::from_value(body).map_err(|_| shape_error("a change"))?;
                vault::update(&store, change)?;
                Ok((json!({}), true))
            }
            // One part's value, for Reveal and Copy. The host asks only after
            // the OS has confirmed the person.
            "reveal" => {
                let s = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                let value = vault::reveal(&store, &s("id"), &s("part"), &s("action"))?;
                Ok((json!({ "value": value.as_str() }), true))
            }
            // The host's report on a copy, after it tried the clipboard.
            "record_copy" => {
                let s = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                vault::record_copy(&store, &s("id"), &s("part"), body.get("done").and_then(Value::as_bool).unwrap_or(false))?;
                Ok((json!({}), true))
            }
            "delete" => {
                let id = body.get("id").and_then(Value::as_str).unwrap_or("");
                vault::delete(&store, id)?;
                Ok((json!({}), true))
            }
            // An entry's earlier values, and putting one back (spec §4.4 of the generate spec).
            "versions" => {
                let id = body.get("id").and_then(Value::as_str).unwrap_or("");
                Ok((json!({ "versions": vault::versions::list(&store, id)? }), false))
            }
            "restore" => {
                let id = body.get("id").and_then(Value::as_str).unwrap_or("");
                let seq = body.get("seq").and_then(Value::as_i64).ok_or("restore needs the version's seq")?;
                vault::versions::restore(&store, id, seq)?;
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
            "requests" => Ok((json!({ "requests": requests::requests().list(), "uses": vault::uses::uses().list() }), false)),
            // The person's answer to an agent's use of an entry (spec section 6.4).
            "answer_use" => {
                let s = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                let (id, word) = (s("id"), s("decision"));
                let decision = vault::uses::Decision::parse(&word).ok_or_else(|| "decision is once, always or deny".to_string())?;
                let request = vault::uses::uses().get(&id).ok_or_else(|| "that request is no longer waiting".to_string())?;
                if decision == vault::uses::Decision::Always && !request.offer_always {
                    return Err("Always isn't offered here: the field doesn't look like what the entry is".into());
                }
                // A value the person typed because no entry exists: required to allow, and
                // never kept unless they ticked the box.
                let value = body.get("value").and_then(Value::as_str).map(|v| zeroize::Zeroizing::new(v.to_string())).filter(|v| !v.is_empty());
                if request.needs_value && decision != vault::uses::Decision::Deny {
                    match &value {
                        None => return Err("type the value to fill, or deny".into()),
                        Some(v) if v.len() > 1024 * 1024 => return Err("that value is too long".into()),
                        _ => {}
                    }
                }
                let save = request.needs_value && body.get("save").and_then(Value::as_bool).unwrap_or(false);
                let answer = vault::uses::UseAnswer { decision, value: if request.needs_value { value } else { None }, save };
                if !vault::uses::uses().answer(&id, answer) {
                    return Err("that request is no longer waiting".into());
                }
                Ok((json!({}), true))
            }
            // Who may use which entry without asking, with no value and no key: Warden's
            // view of the standing permissions, which must not make the OS ask for the key.
            "grants" => {
                let mut grants = store.vault_grants().map_err(|e| e.to_string())?;
                let rows = store.vault_list().map_err(|e| e.to_string())?;
                let granted: Vec<Value> = rows
                    .into_iter()
                    .filter_map(|r| {
                        let g = grants.remove(&r.id)?;
                        Some(json!({ "id": r.id, "label": r.label, "kind": r.kind, "grants": g }))
                    })
                    .collect();
                Ok((json!({ "granted": granted }), false))
            }
            "revoke_grant" => {
                let s = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                let (entry, origin) = (s("entry_id"), s("origin"));
                if !store.vault_grant_revoke(&entry, &origin).map_err(|e| e.to_string())? {
                    return Err("there is no such grant".into());
                }
                vault::record(&store, &entry, "", vault::PERSON, "revoked a site", &origin, "done");
                Ok((json!({}), true))
            }
            // A pane says it is showing (or no longer is). Only the host can
            // say so: an agent clearing it could then capture the window. Each
            // pane has its own note, so one pane hiding can't clear another's.
            "on_screen" => {
                let pane = body.get("pane").and_then(Value::as_str).unwrap_or("").trim();
                if pane.is_empty() || pane.len() > 128 {
                    return Err("on_screen needs the pane's id".into());
                }
                let on = body.get("on").and_then(Value::as_bool).unwrap_or(false);
                let now = agentmux_common::time::now_ms();
                let until = if on { now + ON_SCREEN_MS } else { 0 };
                store.vault_on_screen_set(&on_screen_key(pane), until, now).map_err(|e| e.to_string())?;
                Ok((json!({}), false))
            }
            "activity" => {
                let limit = body.get("limit").and_then(Value::as_i64).unwrap_or(200);
                Ok((json!({ "activity": vault::activity(&store, limit)? }), false))
            }
            "kept" => Ok((json!({ "kept": kept(&st) }), false)),
            // Export and import: the host picks the file in its native dialog and
            // passes the path, so no agent chooses one (spec section 8.1).
            "suggest_passphrase" => Ok((json!({ "passphrase": vault::transfer::suggest_passphrase()? }), false)),
            "export" => {
                let path = file_path(&body)?;
                let pass = body.get("passphrase").and_then(Value::as_str).unwrap_or("").to_string();
                let ids: Option<Vec<String>> = body.get("ids").and_then(|v| serde_json::from_value(v.clone()).ok());
                // `.vault` (an age file), or a KeePass database for the person to take to another manager.
                let out = match body.get("format").and_then(Value::as_str).unwrap_or("vault") {
                    "kdbx" => vault::kdbx_export::export(&store, ids.as_deref(), &pass, &path)?,
                    "vault" => vault::transfer::export(&store, ids.as_deref(), &pass, &path)?,
                    // Unencrypted, for another manager. The host asked the person and the OS first.
                    plain @ ("csv" | "json") => vault::plain_export::export(&store, ids.as_deref(), plain, &path)?,
                    other => return Err(format!("unknown export format {other:?}")),
                };
                Ok((json!({ "count": out.count, "file_name": out.file_name }), true))
            }
            "import_inspect" => {
                let path = file_path(&body)?;
                let pass = body.get("passphrase").and_then(Value::as_str);
                // For a CSV no reader knows: the person's choice of columns.
                let mapping: Option<vault::import::Mapping> = match body.get("mapping").filter(|m| !m.is_null()) {
                    Some(m) => Some(serde_json::from_value(m.clone()).map_err(|e| format!("invalid column mapping: {e}"))?),
                    None => None,
                };
                // The key file of a KeePass database: its bytes, read by the host, from the person's own pick. One that is
                // present but empty is still a key file (a refused one), never "no key file".
                let keyfile: Option<zeroize::Zeroizing<Vec<u8>>> = match body.get("keyfile").and_then(Value::as_str) {
                    Some(k) if k.len() > vault::import::MAX_KEY_FILE * 2 => return Err("that key file is too large to be a KeePass key file".into()),
                    Some(k) => {
                        use base64::Engine as _;
                        Some(zeroize::Zeroizing::new(base64::engine::general_purpose::STANDARD.decode(k).map_err(|_| "the key file wasn't passed on intact".to_string())?))
                    }
                    None => None,
                };
                match vault::transfer::inspect_keyed(&store, &path, pass, mapping.as_ref(), keyfile.as_deref().map(|k| k.as_slice())) {
                    Ok(preview) => Ok((json!({ "preview": preview }), false)),
                    Err(vault::transfer::InspectError::NeedsPassphrase) => Ok((json!({ "needs_passphrase": true }), false)),
                    Err(vault::transfer::InspectError::NeedsMapping(m)) => Ok((json!({ "needs_mapping": m }), false)),
                    Err(vault::transfer::InspectError::Other(why)) => Err(why),
                }
            }
            "import_commit" => {
                let token = body.get("token").and_then(Value::as_str).unwrap_or("");
                let duplicates: vault::transfer::Duplicates = serde_json::from_value(body.get("duplicates").cloned().unwrap_or(json!("skip")))
                    .map_err(|_| "duplicates is skip, replace or keep_both".to_string())?;
                let result = vault::transfer::commit(&store, token, duplicates)?;
                Ok((json!({ "result": result }), true))
            }
            "import_cancel" => {
                vault::transfer::cancel(body.get("token").and_then(Value::as_str).unwrap_or(""));
                Ok((json!({}), false))
            }
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

/// A pane's key among the AgentMux instances that share the identity store:
/// this srv and the pane.
fn on_screen_key(pane: &str) -> String {
    format!("srv-{}:{pane}", std::process::id())
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
pub(crate) struct GenerateBody {
    #[serde(flatten)]
    request: vault::GenerateRequest,
    /// The asking agent's signed identity: the record names it, so it must be proven.
    #[serde(default)]
    auth: Option<agentmux_common::api_types::UiAutomationAuth>,
}

/// `POST /api/v1/vault/generate`: a new entry whose value the Vault makes
/// (SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §4.1). The
/// person isn't asked: the value is no secret of theirs, nobody has seen it, and
/// using it later asks as every use does. The answer names the entry, never the
/// value.
pub(crate) async fn handle_vault_generate(State(state): State<AppState>, Json(body): Json<GenerateBody>) -> Response {
    let Some(auth) = body.auth.as_ref() else {
        return error(StatusCode::FORBIDDEN, "generate needs the agent's signed identity: the Vault records who made each entry");
    };
    let agent: String = match super::ui_handlers::verified_block_id(&state, None, auth) {
        Ok(_) => auth.agent_id.chars().take(64).collect(),
        Err(e) => return error(StatusCode::FORBIDDEN, format!("generate needs the agent's signed identity: {e}")),
    };
    let store = state.identity_store.clone();
    let request = body.request;
    match tokio::task::spawn_blocking(move || vault::generate_entry(&store, request, &agent)).await {
        Ok(Ok(made)) => {
            publish(&state);
            Json(json!({ "status": "created", "id": made.id, "label": made.label, "kind": made.kind, "made": made.made })).into_response()
        }
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, e),
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
///
/// Asking again for the same kind and label is safe: a verified agent whose
/// session ended mid-wait joins the request already open, or gets the answer
/// the person gave, and no second form opens. Every answer carries `request_id`.
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
    let (request_id, answer) = match requests::requests().ask(req, pane.is_some()) {
        // The person already answered this agent's ask for this entry.
        requests::Asked::Answered { request_id, answer } => return answered(&request_id, Ok(answer)),
        requests::Asked::Waiting { request_id, new, answer } => {
            if new {
                let store = state.identity_store.clone();
                let (label, agent, kind) = (label.clone(), agent.clone(), body.kind.clone());
                let _ = tokio::task::spawn_blocking(move || vault::record(&store, "", &label, &agent, "requested an entry", &kind, "asked")).await;
                publish(&state);
            }
            (request_id, answer)
        }
    };
    let _asking = crate::backend::user_attention::Asking::start(
        &state.broker,
        &state.mstore,
        pane.as_deref().unwrap_or(""),
        "vault",
        &format!("{agent} asks for a Vault entry: {label}"),
    );
    let wait = body.wait_secs.map(Duration::from_secs).unwrap_or(DEFAULT_WAIT).min(MAX_WAIT);
    match tokio::time::timeout(wait, answer).await {
        Ok(a) => answered(&request_id, a.map_err(|_| ())),
        Err(_) => Json(json!({
            "status": "pending",
            "request_id": request_id,
            "message": "the person hasn't answered yet; the request stays open in the Vault for an hour. Ask again with the same kind and label to wait again or get the answer: it opens no second form. A saved entry also shows up in list",
        }))
        .into_response(),
    }
}

/// The answer to a request, with its id.
fn answered(request_id: &str, answer: Result<requests::Answer, ()>) -> Response {
    match answer {
        Ok(requests::Answer::Created(id)) => Json(json!({ "status": "created", "id": id, "request_id": request_id })).into_response(),
        Ok(requests::Answer::Declined) | Err(()) => {
            Json(json!({ "status": "declined", "request_id": request_id, "message": "the person dismissed the request" })).into_response()
        }
    }
}

#[cfg(test)]
mod tests;
