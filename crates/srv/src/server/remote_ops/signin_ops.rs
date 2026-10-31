// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `identity.signin.*` (spec §4.4, D6): signing in an account on this machine for an agent
//! somewhere else. The provider's own login runs here, under an account directory srv mints; the
//! caller gets the link and the session, and later the account's id and email, and never a token
//! or a path. The caller shows the link where the person is reading, and the credentials stay on
//! this machine. A session belongs to the caller that started it, as the transport proved it
//! (`slug@machine`): another caller can't read it or finish it.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};

use super::{AppState, RemoteCaller, RemoteRefusal, RemoteResult};
use crate::identity::auth_session::AuthSessionStatus;
use crate::server::identity_handlers::{start_provider_auth, StartProviderAuthReq};

/// How long `start` waits for the CLI to print its link, and `finish` for the sign-in to end.
const LINK_WAIT: Duration = Duration::from_secs(15);
const FINISH_WAIT: Duration = Duration::from_secs(20);
const POLL_EVERY: Duration = Duration::from_millis(400);
/// Sessions this machine remembers a caller for; the oldest go first.
const MAX_OWNED: usize = 64;

/// Who started each session (and in which session manager), so only that caller reads or finishes it.
static OWNERS: Mutex<Vec<(usize, String, String)>> = Mutex::new(Vec::new());

/// Identifies the session manager that holds a session: one per srv.
fn manager_key(state: &AppState) -> usize {
    std::sync::Arc::as_ptr(&state.auth_session_manager) as usize
}

/// What this op can sign in today: the provider's login command is a frontend catalog entry, so
/// only the one whose command is known here.
const CLAUDE: &str = "claude";

#[derive(Debug, Deserialize)]
struct StartParams {
    #[serde(default)]
    provider: Option<String>,
    /// Sign an account in again (refresh its tokens in place) instead of adding a new one.
    #[serde(default)]
    account: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SessionParams {
    session: String,
    #[serde(default)]
    code: Option<String>,
}

fn bad(message: impl Into<String>) -> RemoteRefusal {
    RemoteRefusal::new("bad_params", message)
}

fn failed(message: impl Into<String>) -> RemoteRefusal {
    RemoteRefusal::failed(message.into())
}

fn remember(state: &AppState, session: &str, actor: &str) {
    let mut owners = OWNERS.lock().unwrap_or_else(|e| e.into_inner());
    // Sessions srv has dropped (ended and swept) go first; only a full list of live ones evicts the oldest.
    let key = manager_key(state);
    owners.retain(|(k, s, _)| *k != key || state.auth_session_manager.poll_session(s).is_some());
    if owners.iter().filter(|(k, ..)| *k == key).count() >= MAX_OWNED {
        if let Some(oldest) = owners.iter().position(|(k, ..)| *k == key) {
            owners.remove(oldest);
        }
    }
    owners.push((key, session.to_string(), actor.to_string()));
}

fn owned_by(state: &AppState, session: &str, actor: &str) -> bool {
    let key = manager_key(state);
    let owners = OWNERS.lock().unwrap_or_else(|e| e.into_inner());
    owners.iter().any(|(k, s, a)| *k == key && s == session && a == actor)
}

/// The session's state as the caller sees it. An account's id and email appear once it is signed in.
fn view(session: &str, status: &AuthSessionStatus) -> Value {
    match status {
        AuthSessionStatus::Pending => json!({ "session": session, "state": "starting" }),
        AuthSessionStatus::UrlAvailable { auth_url } => json!({ "session": session, "state": "waiting_for_person", "url": auth_url }),
        AuthSessionStatus::CodeEmitted { device_code, verification_url } => {
            json!({ "session": session, "state": "waiting_for_person", "url": verification_url, "device_code": device_code })
        }
        AuthSessionStatus::Success { email, account_id, .. } => json!({ "session": session, "state": "signed_in", "account_id": account_id, "email": email }),
        AuthSessionStatus::Failed { error } => json!({ "session": session, "state": "failed", "error": error }),
    }
}

/// Poll until `done` says stop or `wait` passes; the last status seen.
async fn poll_until(state: &AppState, session: &str, wait: Duration, done: impl Fn(&AuthSessionStatus) -> bool) -> Result<AuthSessionStatus, RemoteRefusal> {
    let deadline = Instant::now() + wait;
    loop {
        let snap = state.auth_session_manager.poll_session(session).ok_or_else(|| failed("that sign-in is gone (it ended or timed out); start another"))?;
        if done(&snap.status) || Instant::now() >= deadline {
            return Ok(snap.status);
        }
        tokio::time::sleep(POLL_EVERY).await;
    }
}

/// Start signing in an account on this machine, and give back the link for the person.
pub(super) async fn start(state: &AppState, caller: &RemoteCaller, params: Value) -> RemoteResult {
    let p: StartParams = serde_json::from_value(params).map_err(|e| bad(format!("identity.signin.start: {e}")))?;
    let provider = p.provider.as_deref().map(str::trim).filter(|s| !s.is_empty()).unwrap_or(CLAUDE);
    if provider != CLAUDE {
        return Err(bad(format!("identity.signin.start: only {CLAUDE} can be signed in from another machine so far, not {provider}")));
    }
    let existing = p.account.unwrap_or_default().trim().to_string();
    if !existing.is_empty() {
        match state.id_store.identity_get(&existing) {
            Ok(Some(a)) if a.provider == provider => {}
            _ => return Err(bad(format!("identity.signin.start: {existing} isn't a {provider} account on this machine; leave account out to add a new one"))),
        }
    }
    let cli = crate::server::cli_handlers::locate_provider_cli(provider)
        .await
        .ok_or_else(|| failed(format!("{provider} isn't installed on this machine; open a {provider} agent here once so AgentMux installs it")))?;
    let req = StartProviderAuthReq {
        provider_id: provider.to_string(),
        into_bundle_id: None,
        direct_account: true,
        existing_account_id: existing,
        cli_path: cli.path,
        auth_login_args: vec!["auth".into(), "login".into()],
        auth_check_args: vec!["auth".into(), "status".into(), "--json".into()],
        auth_env: HashMap::new(),
        requires_tty: true,
    };
    let started = start_provider_auth(state, req);
    let actor = caller.actor();
    remember(state, &started.session_id, &actor);
    tracing::info!(actor = %actor, provider, "remote identity.signin.start: started a sign-in");
    let status = poll_until(state, &started.session_id, LINK_WAIT, |s| !matches!(s, AuthSessionStatus::Pending)).await?;
    Ok(view(&started.session_id, &status))
}

/// Wait for the sign-in to end: the login finishes on its own once the person has authorized in a
/// browser, or takes the code they were shown (`code`) when the browser can't finish it.
pub(super) async fn finish(state: &AppState, caller: &RemoteCaller, params: Value) -> RemoteResult {
    let p: SessionParams = serde_json::from_value(params).map_err(|e| bad(format!("identity.signin.finish: {e}")))?;
    if !owned_by(state, &p.session, &caller.actor()) {
        return Err(failed("no sign-in of yours with that session"));
    }
    if let Some(code) = p.code.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        if !state.auth_session_manager.send_to_stdin(&p.session, code.to_string()).await {
            return Err(failed("that sign-in isn't waiting for a code any more"));
        }
    }
    let status = poll_until(state, &p.session, FINISH_WAIT, AuthSessionStatus::is_terminal).await?;
    Ok(view(&p.session, &status))
}

#[cfg(test)]
#[path = "signin_ops_tests.rs"]
mod tests;
