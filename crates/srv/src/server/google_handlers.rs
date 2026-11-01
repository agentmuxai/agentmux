// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The Google connectors' agent routes
//! (docs/specs/SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md D3, D4):
//!
//! - `GET /api/v1/google/self/services`: the services the calling agent has a
//!   grant on, so its MCP lists only those tools.
//! - `POST /api/v1/google/calendar`: one `Calendar` action.
//! - `POST /api/v1/google/drive`: one `Drive` action (`pick_files` opens Google's file picker
//!   for the person).
//! - `POST /api/v1/google/gmail`: one `Gmail` action.
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
use crate::backend::google::calls::{calendar_read, drive_read, gmail_read, log_call, CallDeps};
use crate::backend::google::drive::DriveArgs;
use crate::backend::google::gmail_flow::{gmail_write, Answer, Approver};
use crate::backend::google::gmail_write::WRITE_ACTIONS;
use crate::backend::google::gmail::GmailArgs;
use crate::backend::google::{access, LiveGoogleHttp, Service};
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

/// `GET /api/v1/google/self/services`: `{ account, accounts, services: [{ service,
/// level, account }] }` for the Google accounts the calling agent holds a grant on
/// (`account` is the first); none when it has none.
pub(crate) async fn handle_google_self_services(State(state): State<AppState>, caller: Option<Extension<Caller>>) -> Response {
    let def = match calling_agent(&state, caller.as_deref()) {
        Ok(d) => d,
        // An unattributed caller has no grants; it isn't an error to ask.
        Err(_) => return Json(json!({ "account": null, "services": [] })).into_response(),
    };
    match state.identity_store.google_grants_for_agent(&def.id) {
        Ok(grants) => {
            // One entry per grant, on whichever account it is (an agent may hold several, D7).
            let mut accounts: Vec<String> = Vec::new();
            let mut services = Vec::new();
            for g in grants {
                let Ok(Some((acct, _))) =
                    crate::identity::resolver::resolve_account(&state.id_store, &state.identity_store, &g.account_id)
                else {
                    continue;
                };
                if acct.provider != "google" {
                    continue;
                }
                let email = access::email_of(&acct);
                if !accounts.contains(&email) {
                    accounts.push(email.clone());
                }
                services.push(json!({ "service": g.service, "level": g.level, "account": email }));
            }
            Json(json!({ "account": accounts.first(), "accounts": accounts, "services": services })).into_response()
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

#[derive(Debug, Deserialize)]
pub(crate) struct DriveBody {
    action: String,
    /// The account's email, when the agent names one.
    #[serde(default)]
    account: Option<String>,
    #[serde(flatten)]
    args: DriveArgs,
}

/// `POST /api/v1/google/drive`: `{ account, result }`, or `{ error }` with a sentence the agent
/// can act on.
pub(crate) async fn handle_google_drive(
    State(state): State<AppState>,
    caller: Option<Extension<Caller>>,
    Json(body): Json<DriveBody>,
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
    let action = body.action.trim();
    let result = if action == "pick_files" {
        pick_files(&state, &deps, &def, body.account.as_deref()).await
    } else {
        drive_read(&deps, &def, action, &body.args, body.account.as_deref(), chrono::Utc::now()).await
    };
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct GmailBody {
    action: String,
    /// The account's email, when the agent names one.
    #[serde(default)]
    account: Option<String>,
    #[serde(flatten)]
    args: GmailArgs,
}

/// `POST /api/v1/google/gmail`: `{ account, result }`, or `{ error }` with a sentence the agent
/// can act on.
pub(crate) async fn handle_google_gmail(
    State(state): State<AppState>,
    caller: Option<Extension<Caller>>,
    Json(body): Json<GmailBody>,
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
    let action = body.action.trim();
    let result = if WRITE_ACTIONS.contains(&action) {
        gmail_write(&deps, &HostApprover { state: &state }, &def, action, &body.args, body.account.as_deref(), chrono::Utc::now()).await
    } else {
        gmail_read(&deps, &def, action, &body.args, body.account.as_deref(), chrono::Utc::now()).await
    };
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

/// The person's yes or no, asked in the host's approval window over the agent's own pane: a window
/// no agent can click (`connections::ask_user`). With no window connected, or no pane for the agent,
/// there is no one to ask and nothing is done.
struct HostApprover<'a> {
    state: &'a AppState,
}

impl Approver for HostApprover<'_> {
    async fn ask(&self, def: &AgentDefinition, title: String, message: String, yes: &str, no: &str) -> Result<Answer, String> {
        let block = super::app_api::agent_block::find_agent_block_by_uid(&self.state.mstore, &def.id)?
            .ok_or("you have no pane open to ask the person in, so this can't be approved")?
            .block
            .oid;
        let question = json!({ "kind": "consent", "title": title, "message": message, "checkbox": "", "ok_label": yes, "cancel_label": no });
        let answer = super::app_api::connections::ask_user(self.state, &block, question).await?;
        Ok(match (answer.answered, answer.approve) {
            (true, true) => Answer::Yes,
            (true, false) => Answer::No,
            _ => Answer::Unanswered,
        })
    }
}

/// The least time between two pickers opened for one account by agents, so a loop of
/// `pick_files` calls can't fill the person's browser with tabs.
const PICK_GAP: std::time::Duration = std::time::Duration::from_secs(60);

fn last_picks() -> std::sync::MutexGuard<'static, std::collections::HashMap<String, std::time::Instant>> {
    use std::sync::{Mutex, OnceLock};
    static LAST: OnceLock<Mutex<std::collections::HashMap<String, std::time::Instant>>> = OnceLock::new();
    LAST.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// Take the account's picker slot, or say it was taken a moment ago. The slot is held
/// from the check, so two calls at once can't both open a picker; [`pick_release`] gives it
/// back when the picker didn't open.
fn pick_throttle(account_id: &str, now: std::time::Instant) -> Result<(), String> {
    let mut last = last_picks();
    if let Some(at) = last.get(account_id) {
        if now.saturating_duration_since(*at) < PICK_GAP {
            return Err("a file picker was opened for this account a moment ago: wait for the person to pick, then search_files".to_string());
        }
    }
    last.insert(account_id.to_string(), now);
    Ok(())
}

/// A picker that failed to open doesn't count against the account.
fn pick_release(account_id: &str) {
    last_picks().remove(account_id);
}

/// `pick_files`: open Google's file picker in the person's browser. The agent gets no ids
/// back: it searches once the person has picked.
async fn pick_files<G, O, S>(
    state: &AppState,
    deps: &CallDeps<'_, G, O, S>,
    def: &AgentDefinition,
    account: Option<&str>,
) -> Result<serde_json::Value, String>
where
    G: crate::backend::google::GoogleHttp,
    O: crate::identity::oauth_tokens::OAuthHttp,
    S: crate::identity::oauth_tokens::TokenSecrets,
{
    let acc = access::access_for(&state.mstore, &state.id_store, &state.identity_store, def, Service::Drive, account)?;
    let result = match pick_throttle(&acc.account.id, std::time::Instant::now()) {
        Ok(()) => {
            let started = start_picker_for(state, &acc.account).await;
            if started.is_err() {
                pick_release(&acc.account.id);
            }
            started.map(|_| {
                json!({
                    "started": true,
                    "note": "A Google file picker is open in the person's browser. Nothing to wait for here: once they have picked, search_files lists what they gave you."
                })
            })
        }
        Err(e) => Err(e),
    };
    log_call(deps, &acc, def, Service::Drive, "pick_files", String::new(), &result, chrono::Utc::now());
    result.map(|r| json!({ "account": acc.email(), "result": r }))
}

/// Start the picker for `account` with the client it was made with; the session id is
/// polled with `account.oauth.poll`.
async fn start_picker_for(state: &AppState, account: &crate::backend::storage::store::IdentityAccount) -> Result<String, String> {
    let client = crate::identity::oauth_client::picker_client(&KeychainSecrets, account).await?;
    let (session_id, _) = crate::identity::oauth_client::start_picker(
        account.clone(),
        client,
        state.id_store.clone(),
        state.identity_store.clone(),
    )?;
    Ok(session_id)
}

/// The services a grant can be set on today; the rest come with their stage.
const BUILT_SERVICES: [&str; 3] = ["calendar", "drive", "gmail"];

/// How many log lines the account panel shows.
const PANEL_CALLS: usize = 100;

/// `POST /api/v1/host/google`: the account panel's operations, from the host
/// only. (Agents turn services on and off through `POST /api/v1/google/grant`.)
/// - `account` `{ account_id }`: the agents bound to it with their levels,
///   and its call log.
/// - `set` `{ account_id, agent_id, service, level }`: `read`, `write` or
///   `off`.
/// - `pick` `{ account_id }`: open Google's file picker for the person; answers
///   `{ session_id }` to poll with `account.oauth.poll`.
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
        "pick" => pick_for_person(&state, &account_id).await,
        other => Err(format!("no Google operation \"{other}\"")),
    };
    match result {
        Ok(data) => Json(json!({ "data": data })).into_response(),
        Err(e) => error(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

async fn pick_for_person(state: &AppState, account_id: &str) -> Result<serde_json::Value, String> {
    let account = state.id_store.identity_get(account_id).map_err(|e| e.to_string())?.ok_or("no such account")?;
    let session_id = start_picker_for(state, &account).await?;
    Ok(json!({ "session_id": session_id }))
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
        let bound = links.iter().find(|l| l.provider == "google").map(|l| l.account_id.as_str()) == Some(account_id);
        // An agent granted without being bound (an agent granted it, or it holds several accounts) is listed too.
        if !bound && !grants.iter().any(|g| g.agent_id == def.id) {
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
    // A grant is all an agent needs; it need not be bound to the account first (spec D7).
    state.mstore.agent_def_get(agent_id).map_err(|e| e.to_string())?.ok_or("no such agent")?;
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

// ── The `Google` tool: status, grant, ungrant ───────────────────────────────

/// Every Google account on this computer, by email.
fn google_accounts(state: &AppState) -> Result<Vec<crate::backend::storage::store::IdentityAccount>, String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for store in [&state.id_store, &state.identity_store] {
        for a in store.identity_list(Some("google")).map_err(|e| e.to_string())? {
            if seen.insert(a.id.clone()) {
                out.push(a);
            }
        }
    }
    out.sort_by_key(|a| access::email_of(a).to_ascii_lowercase());
    Ok(out)
}

/// The highest level the account's token supports for `service`, from the scopes it was
/// granted: `write`, `read` or `none`.
fn supported_level(account: &crate::backend::storage::store::IdentityAccount, service: Service) -> &'static str {
    let granted = access::Access { account: account.clone(), level: crate::backend::storage::google_grants::GrantLevel::Read }
        .granted_scopes();
    let has_all = |level| service.scopes(level).iter().all(|s| granted.contains(s));
    if has_all(crate::backend::storage::google_grants::GrantLevel::Write) {
        "write"
    } else if has_all(crate::backend::storage::google_grants::GrantLevel::Read) {
        "read"
    } else {
        "none"
    }
}

const SERVICES: [Service; 3] = [Service::Calendar, Service::Drive, Service::Gmail];

/// `GET /api/v1/google/status`: the Google accounts connected here, what each can be used for
/// and who holds what, so an agent knows what to grant or to ask the person for.
pub(crate) async fn handle_google_status(State(state): State<AppState>, caller: Option<Extension<Caller>>) -> Response {
    let def = match calling_agent(&state, caller.as_deref()) {
        Ok(d) => d,
        Err(e) => return error(StatusCode::FORBIDDEN, e),
    };
    let result = (|| -> Result<serde_json::Value, String> {
        let names: std::collections::HashMap<String, String> = state
            .mstore
            .agent_def_list()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|d| (d.id, d.name))
            .collect();
        let mut accounts = Vec::new();
        for acct in google_accounts(&state)? {
            let grants = state.identity_store.google_grants_for_account(&acct.id).map_err(|e| e.to_string())?;
            let mut services = serde_json::Map::new();
            for service in SERVICES {
                let holders: Vec<_> = grants
                    .iter()
                    .filter(|g| g.service == service.as_str())
                    .map(|g| json!({ "agent": names.get(&g.agent_id).cloned().unwrap_or_else(|| g.agent_id.clone()), "level": g.level }))
                    .collect();
                let mine = grants.iter().find(|g| g.service == service.as_str() && g.agent_id == def.id).map(|g| g.level.as_str());
                services.insert(
                    service.as_str().to_string(),
                    json!({ "connected_for": supported_level(&acct, service), "yours": mine.unwrap_or("off"), "holders": holders }),
                );
            }
            accounts.push(json!({ "account": access::email_of(&acct), "status": acct.status, "services": services }));
        }
        Ok(json!({ "you": def.name, "accounts": accounts }))
    })();
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct GrantBody {
    /// The Google account's email.
    account: String,
    /// The agent to grant, by name or id; the caller itself when absent.
    #[serde(default)]
    agent: Option<String>,
    /// `calendar`, `drive` or `gmail`.
    service: String,
    /// `read`, `write` or `off`.
    level: String,
}

/// `POST /api/v1/google/grant`: turn a service of a connected Google account on for an agent, or
/// off. Any agent may do it (decided: "yes, agents can grant"); it is recorded as the agent that
/// did it, in the account's log the person reads. Sends, shares, invites and deletes still wait
/// for the person in the host window, which no agent can answer.
pub(crate) async fn handle_google_grant(
    State(state): State<AppState>,
    caller: Option<Extension<Caller>>,
    Json(body): Json<GrantBody>,
) -> Response {
    let actor = match calling_agent(&state, caller.as_deref()) {
        Ok(d) => d,
        Err(e) => return error(StatusCode::FORBIDDEN, e),
    };
    match grant_by_agent(&state, &actor, &body) {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

fn grant_by_agent(state: &AppState, actor: &AgentDefinition, body: &GrantBody) -> Result<serde_json::Value, String> {
    use crate::backend::storage::google_grants::GoogleCall;
    let service = SERVICES
        .into_iter()
        .find(|s| s.as_str() == body.service.trim())
        .ok_or_else(|| format!("service \"{}\": calendar, drive or gmail", body.service.trim()))?;
    let level = crate::backend::storage::google_grants::GrantLevel::parse(&body.level)?;
    let asked = body.account.trim();
    let accounts = google_accounts(state)?;
    let account = accounts.iter().find(|a| access::email_of(a).eq_ignore_ascii_case(asked)).ok_or_else(|| {
        let have: Vec<_> = accounts.iter().map(access::email_of).collect();
        if have.is_empty() {
            "no Google account is connected: the person connects one in Accounts (they sign in and consent)".to_string()
        } else {
            format!("no connected Google account {asked}. Connected: {}", have.join(", "))
        }
    })?;
    let target = body.agent.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let defs = state.mstore.agent_def_list().map_err(|e| e.to_string())?;
    let target_def = match target {
        None => actor.clone(),
        Some(t) => defs
            .into_iter()
            .find(|d| d.id == t || d.name.eq_ignore_ascii_case(t))
            .ok_or_else(|| format!("no agent \"{t}\""))?,
    };
    let email = access::email_of(account);
    if let Some(l) = level {
        // The token must carry the scopes the level needs, or every call would fail later.
        let supported = supported_level(account, service);
        let enough = matches!((l, supported), (_, "write") | (crate::backend::storage::google_grants::GrantLevel::Read, "read"));
        if !enough {
            return Err(format!(
                "{email} was connected without {} {} access (it supports: {supported}): the person reconnects it with {} turned on",
                service.as_str(),
                l.as_str(),
                service.as_str()
            ));
        }
    }
    let now = chrono::Utc::now().timestamp_millis();
    state
        .identity_store
        .google_grant_set(&target_def.id, &account.id, service.as_str(), level, now)
        .map_err(|e| e.to_string())?;
    let level_name = level.map_or("off", |l| l.as_str());
    let line = GoogleCall {
        at: now,
        agent_id: actor.id.clone(),
        agent_name: actor.name.clone(),
        service: service.as_str().to_string(),
        action: if level.is_some() { "grant".to_string() } else { "ungrant".to_string() },
        ids: format!("{}:{level_name}", target_def.name),
        outcome: "ok".to_string(),
    };
    if let Err(e) = state.identity_store.google_call_log(&account.id, &line) {
        tracing::warn!(target: "google", error = %e, "google: logging a grant failed");
    }
    tracing::info!(
        target: "google",
        account_id = %account.id,
        by = %actor.name,
        agent = %target_def.name,
        service = %service.as_str(),
        level = level_name,
        "google: grant set by an agent"
    );
    Ok(json!({ "account": email, "agent": target_def.name, "service": service.as_str(), "level": level_name }))
}

#[cfg(test)]
#[path = "google_handlers_tests.rs"]
mod tests;
