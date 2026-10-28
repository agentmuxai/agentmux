// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The Google connectors' agent routes
//! (docs/specs/SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md D3, D4):
//!
//! - `GET /api/v1/google/self/services`: the services the calling agent has a
//!   grant on, so its MCP lists only those tools.
//! - `POST /api/v1/google/calendar`: one `Calendar` action.
//!
//! Both need an attributed caller (the agent's own `X-Agent-Token`): a grant
//! belongs to an agent, so a request that names none has none.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::json;

use super::app_api::bundle::SelfOwner;
use super::caller::Caller;
use super::AppState;
use crate::backend::google::calendar::CalendarArgs;
use crate::backend::google::calls::{calendar_read, CallDeps};
use crate::backend::google::{access, LiveGoogleHttp};
use crate::backend::storage::store::AgentDefinition;
use crate::identity::oauth_tokens::{KeychainSecrets, LiveHttp};

fn error(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

/// The calling agent's definition, or why there is none (answered 403).
fn calling_agent(state: &AppState, caller: Option<&Caller>) -> Result<AgentDefinition, String> {
    let Some(uid) = caller.and_then(Caller::uid) else {
        return Err("the Google tools work only for an agent AgentMux started: this request carried no agent token".to_string());
    };
    SelfOwner::caller_row(uid, &state.mstore)
}

/// `GET /api/v1/google/self/services`: `{ account, services: [{ service,
/// level }] }` for the calling agent's bound Google account; no account and
/// no services when it has none.
pub(crate) async fn handle_google_self_services(State(state): State<AppState>, caller: Option<Extension<Caller>>) -> Response {
    let def = match calling_agent(&state, caller.as_deref()) {
        Ok(d) => d,
        // An unattributed caller has no grants; it isn't an error to ask.
        Err(_) => return Json(json!({ "account": null, "services": [] })).into_response(),
    };
    let acct = match access::bound_account(&state.mstore, &state.id_store, &state.identity_store, &def) {
        Ok(Some(a)) => a,
        Ok(None) => return Json(json!({ "account": null, "services": [] })).into_response(),
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    match state.identity_store.google_grants_for_agent(&def.id) {
        Ok(grants) => {
            let services: Vec<_> = grants
                .into_iter()
                .filter(|g| g.account_id == acct.id)
                .map(|g| json!({ "service": g.service, "level": g.level }))
                .collect();
            Json(json!({ "account": access::email_of(&acct), "services": services })).into_response()
        }
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct CalendarBody {
    action: String,
    /// The account's email, when the agent names one.
    #[serde(default)]
    account: Option<String>,
    #[serde(flatten)]
    args: CalendarArgs,
}

/// `POST /api/v1/google/calendar`: `{ account, result }`, or `{ error }`
/// with a sentence the agent can act on.
pub(crate) async fn handle_google_calendar(
    State(state): State<AppState>,
    caller: Option<Extension<Caller>>,
    Json(body): Json<CalendarBody>,
) -> Response {
    let def = match calling_agent(&state, caller.as_deref()) {
        Ok(d) => d,
        Err(e) => return error(StatusCode::FORBIDDEN, e),
    };
    let deps = CallDeps {
        google: &LiveGoogleHttp,
        oauth: &LiveHttp,
        secrets: &KeychainSecrets,
        mstore: &state.mstore,
        id_store: &state.id_store,
        identity_store: &state.identity_store,
    };
    match calendar_read(&deps, &def, body.action.trim(), &body.args, body.account.as_deref(), chrono::Utc::now()).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

/// The services a grant can be set on today; the rest come with their stage.
const BUILT_SERVICES: [&str; 1] = ["calendar"];

/// How many log lines the account panel shows.
const PANEL_CALLS: usize = 100;

/// `POST /api/v1/host/google`: the account panel's operations, from the host
/// only, so an agent holding the instance key can't grant itself anything.
/// - `account` `{ account_id }`: the agents bound to it with their levels,
///   and its call log.
/// - `set` `{ account_id, agent_id, service, level }`: `read`, `write` or
///   `off`.
pub(crate) async fn handle_host_google(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    if !super::ui_handlers::from_host(&state, &headers).await {
        return error(StatusCode::FORBIDDEN, "only the AgentMux host can change who uses a Google account");
    }
    let s = |k: &str| body.get(k).and_then(serde_json::Value::as_str).unwrap_or("").trim().to_string();
    let account_id = s("account_id");
    if account_id.is_empty() {
        return error(StatusCode::BAD_REQUEST, "account_id is required");
    }
    let result = match s("op").as_str() {
        "account" => account_panel(&state, &account_id),
        "set" => set_grant(&state, &account_id, &s("agent_id"), &s("service"), &s("level")),
        other => Err(format!("no Google operation \"{other}\"")),
    };
    match result {
        Ok(data) => Json(json!({ "data": data })).into_response(),
        Err(e) => error(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

fn account_panel(state: &AppState, account_id: &str) -> Result<serde_json::Value, String> {
    let grants = state.identity_store.google_grants_for_account(account_id).map_err(|e| e.to_string())?;
    // The same resolution `bound_account` uses, so an agent that gets this account only from its
    // template parent's links is listed here and can be granted.
    let defs = state.mstore.agent_def_list().map_err(|e| e.to_string())?;
    let mut agents = Vec::new();
    for def in defs {
        let links = crate::identity::resolver::effective_identity_links(&state.mstore, &state.identity_store, &def)
            .map_err(|e| e.to_string())?;
        if links.iter().find(|l| l.provider == "google").map(|l| l.account_id.as_str()) != Some(account_id) {
            continue;
        }
        let levels: serde_json::Map<String, serde_json::Value> =
            grants.iter().filter(|g| g.agent_id == def.id).map(|g| (g.service.clone(), json!(g.level))).collect();
        agents.push(json!({ "agentId": def.id, "name": def.name, "levels": levels }));
    }
    let calls = state.identity_store.google_calls(account_id, PANEL_CALLS).map_err(|e| e.to_string())?;
    Ok(json!({ "agents": agents, "calls": calls, "services": BUILT_SERVICES }))
}

fn set_grant(state: &AppState, account_id: &str, agent_id: &str, service: &str, level: &str) -> Result<serde_json::Value, String> {
    if !BUILT_SERVICES.contains(&service) {
        return Err(format!("agents can't use Google's {service} yet"));
    }
    let level = crate::backend::storage::google_grants::GrantLevel::parse(level)?;
    let def = state.mstore.agent_def_get(agent_id).map_err(|e| e.to_string())?.ok_or("no such agent")?;
    let bound = access::bound_account(&state.mstore, &state.id_store, &state.identity_store, &def)?;
    if level.is_some() && bound.map(|a| a.id) != Some(account_id.to_string()) {
        return Err(format!("{} isn't bound to this account: bind it first", def.name));
    }
    let now = chrono::Utc::now().timestamp_millis();
    state.identity_store.google_grant_set(agent_id, account_id, service, level, now).map_err(|e| e.to_string())?;
    tracing::info!(
        target: "google",
        account_id = %account_id,
        agent_id = %agent_id,
        service = %service,
        level = level.map_or("off", |l| l.as_str()),
        "google: grant set by the person"
    );
    Ok(json!(true))
}

#[cfg(test)]
#[path = "google_handlers_tests.rs"]
mod tests;
