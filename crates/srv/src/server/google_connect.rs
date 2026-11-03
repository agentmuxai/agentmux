// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! `Google` `connect`, `connect_status` and `cancel`: an agent connects a Google account itself
//! (docs/specs/SPEC_GOOGLE_PHASE_B_AGENTMUXS_OWN_CLIENT_2026_11_01.md §7, §9, D3). It runs the
//! same flow as the Accounts panel's Connect (`oauth_client::start`), so the two can't drift,
//! except that the consent page is not opened in the system browser: the agent opens it in a
//! Browser pane it drives (the account picker, the unverified-app screen, the scope boxes), and
//! stops for the person only at a password, a second factor or an administrator's block.
//!
//! - `POST /api/v1/google/connect`: start the flow; answers the consent URL and the origins its
//!   pane may stay on.
//! - `POST /api/v1/google/connect/status`: where the flow is (waiting for the person, connected,
//!   failed); it may wait a few seconds for a change.
//! - `POST /api/v1/google/connect/cancel`: end a flow that waits.
//!
//! The token never reaches the agent: it goes from Google to the keychain inside srv. A session
//! belongs to the agent that started it. Every connect is recorded under that agent, in the
//! account's log once the account exists.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::json;

use super::{calling_agent, error, google_accounts, supported_level, SERVICES};
use crate::backend::google::access;
use crate::backend::google::Service;
use crate::backend::storage::google_grants::{GoogleCall, GrantLevel};
use crate::backend::storage::store::Store;
use crate::identity::oauth_client::{self, FlowTarget, OAuthStatus, StartOptions};
use crate::server::caller::Caller;
use crate::server::AppState;

/// How long `connect` waits for the flow to produce its consent URL.
const URL_WAIT: Duration = Duration::from_secs(5);
/// The most `connect_status` waits for a change in one call.
const MAX_STATUS_WAIT_SECS: u64 = 25;
/// How often a waiting call looks at the session again.
const POLL_EVERY: Duration = Duration::from_millis(100);
/// How often the task that records a connect looks at its flow.
const RECORD_POLL_EVERY: Duration = Duration::from_millis(500);

/// The agent that started each flow, by session id: only it may ask after it or cancel it.
fn owners() -> MutexGuard<'static, HashMap<String, String>> {
    static OWNERS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    OWNERS.get_or_init(Mutex::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// The sites a consent pane stays on, beside the page's own loopback callback.
const GOOGLE_ORIGINS: [&str; 3] = ["*.google.com", "*.gstatic.com", "*.googleusercontent.com"];

#[derive(Debug, Deserialize)]
pub(crate) struct ConnectBody {
    /// `calendar`, `drive` and/or `gmail`: what to turn on.
    #[serde(default)]
    services: Vec<String>,
    /// `read` (default) or `write`, for every service named.
    #[serde(default)]
    level: Option<String>,
    /// The Google account to start on (the account picker's `login_hint`).
    #[serde(default)]
    login_hint: Option<String>,
    /// A connected account to authorize again (more services, or a reconnect), by email.
    #[serde(default)]
    account: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SessionBody {
    session: String,
    /// `connect_status`: wait up to this many seconds for the flow to change.
    #[serde(default)]
    wait_seconds: Option<u64>,
}

/// The scopes a connect asks for: the services named at `level`, plus every scope the account
/// already holds (D3: always the union, whether or not Google carries earlier grants over on an
/// installed app).
pub(crate) fn union_scopes(services: &[Service], level: GrantLevel, held: &[String]) -> Vec<String> {
    let mut out: Vec<String> = held.to_vec();
    for service in services {
        for scope in service.scopes(level) {
            if !out.contains(&scope) {
                out.push(scope);
            }
        }
    }
    out
}

/// The origins a consent pane may stay on: Google's, and the loopback address Google redirects
/// to when the person has consented (read from the URL's `redirect_uri`).
pub(crate) fn pane_origins(auth_url: &str) -> Vec<String> {
    let mut out: Vec<String> = GOOGLE_ORIGINS.iter().map(|s| s.to_string()).collect();
    if let Ok(url) = url::Url::parse(auth_url) {
        let redirect = url.query_pairs().find(|(k, _)| k == "redirect_uri").map(|(_, v)| v.into_owned());
        if let Some(origin) = redirect.and_then(|r| url::Url::parse(&r).ok()).map(|r| r.origin().ascii_serialization()) {
            out.push(origin);
        }
    }
    out
}

fn parse_services(names: &[String]) -> Result<Vec<Service>, String> {
    if names.is_empty() {
        return Err("services is required: calendar, drive and/or gmail".to_string());
    }
    let mut out = Vec::new();
    for name in names {
        let service = SERVICES
            .into_iter()
            .find(|s| s.as_str() == name.trim().to_ascii_lowercase())
            .ok_or_else(|| format!("service \"{}\": calendar, drive or gmail", name.trim()))?;
        if !out.contains(&service) {
            out.push(service);
        }
    }
    Ok(out)
}

/// The agents whose connect is starting (the keychain read and the flow's first answer are awaits),
/// so two overlapping requests from one agent can't both pass the one-waiting-connect check.
fn starting() -> MutexGuard<'static, std::collections::HashSet<String>> {
    static STARTING: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    STARTING.get_or_init(Mutex::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// An agent's place for one connect, held while it starts; the session in `owners` takes over after.
struct StartSlot(String);

impl StartSlot {
    /// Takes the agent's slot, or says why it can't have one (a connect is starting or waiting).
    fn take(agent_id: &str) -> Result<StartSlot, String> {
        let mut starting = starting();
        if starting.contains(agent_id) {
            return Err("a connect of yours is just starting: wait for it, then use connect_status".to_string());
        }
        if let Some(sid) = waiting_session_of(agent_id) {
            return Err(format!(
                "a connect is already waiting for the person (session {sid}): finish it with connect_status, or end it with cancel, before starting another"
            ));
        }
        starting.insert(agent_id.to_string());
        Ok(StartSlot(agent_id.to_string()))
    }
}

impl Drop for StartSlot {
    fn drop(&mut self) {
        starting().remove(&self.0);
    }
}

/// Whether this agent already has a connect that hasn't ended.
fn waiting_session_of(agent_id: &str) -> Option<String> {
    let mine: Vec<String> = owners().iter().filter(|(_, a)| a.as_str() == agent_id).map(|(s, _)| s.clone()).collect();
    mine.into_iter().find(|sid| oauth_client::manager().poll(sid).is_some_and(|s| !s.is_terminal()))
}

/// `POST /api/v1/google/connect`.
pub(crate) async fn handle_google_connect(
    State(state): State<AppState>,
    caller: Option<Extension<Caller>>,
    Json(body): Json<ConnectBody>,
) -> Response {
    let actor = match calling_agent(&state, caller.as_deref()) {
        Ok(d) => d,
        Err(e) => return error(StatusCode::FORBIDDEN, e),
    };
    match start_connect(&state, &actor.id, &actor.name, &body).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

async fn start_connect(state: &AppState, agent_id: &str, agent_name: &str, body: &ConnectBody) -> Result<serde_json::Value, String> {
    let services = parse_services(&body.services)?;
    let level = GrantLevel::parse(body.level.as_deref().unwrap_or("read"))?.ok_or("level: read or write")?;
    let _slot = StartSlot::take(agent_id)?;
    let (target, held) = match body.account.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => (FlowTarget::New { name: String::new() }, Vec::new()),
        Some(email) => {
            let account = google_accounts(state)?
                .into_iter()
                .find(|a| access::email_of(a).eq_ignore_ascii_case(email))
                .ok_or_else(|| format!("no connected Google account {email}: leave account out to connect a new one"))?;
            let held = access::Access { account: account.clone(), level: GrantLevel::Read }.granted_scopes();
            (FlowTarget::Existing(account), held)
        }
    };
    // The client the Accounts panel would use: the one saved here, else AgentMux's own. A
    // keychain that can't be read means none, as there.
    let saved = tokio::task::spawn_blocking(|| crate::identity::oauth_tokens::load_saved_client("google"))
        .await
        .map_err(|e| format!("keychain task: {e}"))
        .and_then(|r| r)
        .unwrap_or_else(|e| {
            tracing::warn!(target: "google", error = %e, "google connect: the saved OAuth client could not be read");
            None
        });
    let opts = StartOptions {
        target,
        byo: None,
        saved,
        extra_scopes: union_scopes(&services, level, &held),
        save_client: false,
        cloud_signed_in: false,
        replace_user: false,
        open_system_browser: false,
        login_hint: body.login_hint.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string),
    };
    let (session, first) = oauth_client::start("google", opts, state.id_store.clone(), state.identity_store.clone())?;
    owners().insert(session.clone(), agent_id.to_string());
    tokio::spawn(record_when_connected(state.identity_store.clone(), session.clone(), agent_id.to_string(), agent_name.to_string()));
    let status = match first {
        OAuthStatus::Pending => wait_for(&session, URL_WAIT, |s| !matches!(s, OAuthStatus::Pending)).await,
        other => other,
    };
    match status {
        OAuthStatus::UrlAvailable { auth_url } => {
            tracing::info!(target: "google", agent = %agent_name, session = %session, "google connect started by an agent");
            let names: Vec<&str> = services.iter().map(|s| s.as_str()).collect();
            Ok(json!({
                "session": session,
                "auth_url": auth_url,
                "allowed_origins": pane_origins(&auth_url),
                "services": names,
                "level": level.as_str(),
            }))
        }
        OAuthStatus::Failed { error } => {
            owners().remove(&session);
            Err(error)
        }
        _ => {
            oauth_client::manager().cancel(&session);
            owners().remove(&session);
            Err("the sign-in did not start: try connect again".to_string())
        }
    }
}

/// Records the connect under the agent that started it once its account is committed, whether or
/// not the agent asks again; the task ends with the flow (success, failure, cancel or timeout).
async fn record_when_connected(store: std::sync::Arc<Store>, session: String, agent_id: String, agent_name: String) {
    loop {
        match oauth_client::manager().poll(&session) {
            Some(OAuthStatus::Success { account_id, granted_scopes, .. }) => {
                record_connect(&store, &agent_id, &agent_name, &account_id, &granted_scopes);
                return;
            }
            Some(status) if !status.is_terminal() => tokio::time::sleep(RECORD_POLL_EVERY).await,
            _ => return,
        }
    }
}

/// The session's status once `done` holds for it, or the last one seen after `wait`.
async fn wait_for(session: &str, wait: Duration, done: impl Fn(&OAuthStatus) -> bool) -> OAuthStatus {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let Some(status) = oauth_client::manager().poll(session) else {
            return OAuthStatus::Failed { error: "that connect is gone (it timed out or was cancelled)".to_string() };
        };
        if done(&status) || tokio::time::Instant::now() >= deadline {
            return status;
        }
        tokio::time::sleep(POLL_EVERY).await;
    }
}

/// `POST /api/v1/google/connect/status`.
pub(crate) async fn handle_google_connect_status(
    State(state): State<AppState>,
    caller: Option<Extension<Caller>>,
    Json(body): Json<SessionBody>,
) -> Response {
    let actor = match calling_agent(&state, caller.as_deref()) {
        Ok(d) => d,
        Err(e) => return error(StatusCode::FORBIDDEN, e),
    };
    if let Err(e) = owns(&actor.id, &body.session) {
        return error(StatusCode::UNPROCESSABLE_ENTITY, e);
    }
    let wait = Duration::from_secs(body.wait_seconds.unwrap_or(0).min(MAX_STATUS_WAIT_SECS));
    let status = wait_for(&body.session, wait, |s| !matches!(s, OAuthStatus::Pending | OAuthStatus::UrlAvailable { .. })).await;
    let answer = status_answer(&state, &body.session, &status);
    if !matches!(status, OAuthStatus::Pending | OAuthStatus::UrlAvailable { .. }) {
        owners().remove(&body.session);
    }
    Json(answer).into_response()
}

fn owns(agent_id: &str, session: &str) -> Result<(), String> {
    match owners().get(session) {
        Some(owner) if owner == agent_id => Ok(()),
        Some(_) => Err("that connect was started by another agent".to_string()),
        None => Err("no connect with that session: it ended, or it was never started here".to_string()),
    }
}

fn status_answer(state: &AppState, session: &str, status: &OAuthStatus) -> serde_json::Value {
    match status {
        OAuthStatus::Pending | OAuthStatus::UrlAvailable { .. } => json!({
            "session": session,
            "state": "waiting",
            "say": "Waiting for the sign-in to finish in the pane. If the page asks for a password, a code or an administrator's approval, \
                    hand the pane to the person (Browser handoff) and say what to do; then ask again."
        }),
        OAuthStatus::Success { account_id, missing_scopes, email, .. } => {
            let services = connected_services(state, account_id);
            json!({
                "session": session,
                "state": "connected",
                "account": email,
                "services": services,
                "missing_scopes": missing_scopes,
                "say": if missing_scopes.is_empty() { "Connected." } else { "Connected, but some permissions were left unticked on the consent screen: they are in missing_scopes. Connect again to ask for them." },
            })
        }
        OAuthStatus::Failed { error } => json!({ "session": session, "state": "failed", "error": error }),
        _ => json!({ "session": session, "state": "failed", "error": "the sign-in ended in a state a Google connect never reaches" }),
    }
}

/// What each service of the new account can be used for, as `Google status` shows it.
fn connected_services(state: &AppState, account_id: &str) -> serde_json::Value {
    let Ok(accounts) = google_accounts(state) else { return json!({}) };
    let Some(account) = accounts.iter().find(|a| a.id == account_id) else { return json!({}) };
    let mut out = serde_json::Map::new();
    for service in SERVICES {
        out.insert(service.as_str().to_string(), json!(supported_level(account, service)));
    }
    serde_json::Value::Object(out)
}

/// Records the connect under the agent that did it, in the account's log the person reads.
fn record_connect(store: &Store, agent_id: &str, agent_name: &str, account_id: &str, granted: &[String]) {
    let line = GoogleCall {
        at: chrono::Utc::now().timestamp_millis(),
        agent_id: agent_id.to_string(),
        agent_name: agent_name.to_string(),
        service: "account".to_string(),
        action: "connect".to_string(),
        ids: format!("{} scopes", granted.len()),
        outcome: "ok".to_string(),
    };
    if let Err(e) = store.google_call_log(account_id, &line) {
        tracing::warn!(target: "google", error = %e, "google: logging a connect failed");
    }
    tracing::info!(target: "google", account_id = %account_id, by = %agent_name, "google: account connected by an agent");
}

/// `POST /api/v1/google/connect/cancel`.
pub(crate) async fn handle_google_connect_cancel(
    State(state): State<AppState>,
    caller: Option<Extension<Caller>>,
    Json(body): Json<SessionBody>,
) -> Response {
    let actor = match calling_agent(&state, caller.as_deref()) {
        Ok(d) => d,
        Err(e) => return error(StatusCode::FORBIDDEN, e),
    };
    if let Err(e) = owns(&actor.id, &body.session) {
        return error(StatusCode::UNPROCESSABLE_ENTITY, e);
    }
    let cancelled = oauth_client::manager().cancel(&body.session);
    // A flow already committing its account can't be cancelled and ends as connected: keep the owner
    // so connect_status can still report it and log the connect under this agent.
    if cancelled {
        owners().remove(&body.session);
    }
    Json(json!({ "session": body.session, "cancelled": cancelled })).into_response()
}

#[cfg(test)]
#[path = "google_connect_tests.rs"]
mod tests;
