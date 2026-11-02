// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Browser` `signin`: an agent signs in with a saved sign-in, and the person
//! says yes (SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §5.3).
//!
//! The agent names its pane and, when the page could take more than one, the
//! entry. Only a `login` of the tab's profile saved for exactly the site of a
//! sign-in form the page shows now can be used. The person is asked in the
//! Vault's own prompt, a window modal outside every pane, which no `UI` click
//! reaches: "Allow once", "Always for <agent> on <site>", or "Deny". Under an
//! "Always" the pane shows "<agent> is signing in to <site> [Stop]" for 5 seconds
//! first. The host types it as for the person's own choice (§5.3); the agent is
//! told only which entry was used and on which site, never a value.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::backend::vault::signins::{self, PERSONAL};
use crate::backend::vault::uses::{self, Decision, UseRequest};
use agentmux_common::api_types::UiAutomationAuth;

use super::browser_actions::{note, How, Kind};
use super::browser_vault_fill::Waiting;
use super::ui_handlers::{err_response, target_block_id};
use super::AppState;

/// The block meta key of the "is signing in" notice.
pub(crate) const PENDING_META_KEY: &str = "browser:signin_pending";
/// How long the person has to answer.
const ANSWER_WAIT: Duration = Duration::from_secs(10 * 60);
/// How long the notice shows before a sign-in under a grant is typed.
const NOTICE: Duration = Duration::from_secs(5);
/// What the agent is told when another sign-in's notice shows on the pane.
const BUSY: &str = "another sign-in is showing its notice on this pane: nothing was typed; try again when it is done";

/// The notices showing, by id: the pane and what stops the sign-in.
static PENDING: LazyLock<Mutex<HashMap<String, (String, oneshot::Sender<()>)>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Deserialize)]
pub(crate) struct SigninRequest {
    auth: UiAutomationAuth,
    #[serde(default)]
    pane: Option<String>,
    /// The saved sign-in to use; needed only when the page could take more than one.
    #[serde(default)]
    entry: Option<String>,
}

/// The key "Always for <agent> on this site" is kept under, in the grants table
/// web forms use for sites: `signin:<agent>` can't be a site's origin. The entry
/// is saved for one site in one profile, so this is per agent, site and profile.
pub(crate) fn grant_key(agent: &str) -> String {
    format!("signin:{agent}")
}

/// Which of the sign-ins the page can take, `(entry, origin, user)`, is meant.
pub(crate) fn choose(found: Vec<(String, String, String)>, entry: Option<&str>) -> Result<(String, String, String), String> {
    if let Some(want) = entry {
        return found.into_iter().find(|(e, _, _)| e == want).ok_or_else(|| {
            "that sign-in isn't saved for a sign-in form on this page in this tab's profile, so nothing was typed".to_string()
        });
    }
    match found.len() {
        0 => Err("no saved sign-in in this tab's profile fits a sign-in form on this page now; the person can save one when they sign in".to_string()),
        1 => Ok(found.into_iter().next().unwrap_or_default()),
        _ => {
            let listed: Vec<String> = found.iter().map(|(e, o, u)| format!("{e} ({} on {o})", signins::mask(u))).collect();
            Err(format!("{} saved sign-ins fit this page; name one with entry: {}", found.len(), listed.join(", ")))
        }
    }
}

fn label_of(state: &AppState, entry: &str) -> String {
    state.identity_store.vault_get(entry).ok().flatten().map(|r| r.label).unwrap_or_default()
}

fn noted(state: &AppState, block_id: &str, agent: &str, how: How) {
    note(block_id, agent, Kind::Fill).detail("a saved sign-in from the Vault").how(how).record(state);
}

fn set_notice(state: &AppState, block_id: &str, value: Value) {
    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert(PENDING_META_KEY.to_string(), value);
    if let Err(e) = super::http_shell::broadcast_meta_update(state, block_id, &meta) {
        tracing::warn!(error = %e, "[signin] couldn't show the pane's sign-in notice");
    }
}

/// A notice showing: dropped when the sign-in goes ahead or is stopped, and also
/// when the agent's call is cancelled while it shows, so no notice is left on the
/// pane with a Stop that stops nothing.
struct Showing<'a> {
    state: &'a AppState,
    block_id: &'a str,
    id: String,
}

impl Drop for Showing<'_> {
    fn drop(&mut self) {
        PENDING.lock().remove(&self.id);
        set_notice(self.state, self.block_id, Value::Null);
    }
}

/// Show "<agent> is signing in" for [`NOTICE`]: `Some(true)` when nobody stopped
/// it, `None` when another sign-in's notice already shows on the pane. One at a
/// time per pane, so the sign-in waiting to be typed is always the one the pane
/// shows and its Stop stops.
async fn notice(state: &AppState, block_id: &str, agent: &str, origin: &str, user: &str) -> Option<bool> {
    let id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel();
    {
        let mut pending = PENDING.lock();
        if pending.values().any(|(b, _)| b == block_id) {
            return None;
        }
        pending.insert(id.clone(), (block_id.to_string(), tx));
    }
    let _showing = Showing { state, block_id, id: id.clone() };
    let deadline = agentmux_common::time::now_ms() + NOTICE.as_millis() as i64;
    set_notice(state, block_id, json!({ "id": id, "agent": agent, "origin": origin, "user": signins::mask(user), "deadline_ms": deadline }));
    Some(tokio::select! {
        _ = tokio::time::sleep(NOTICE) => true,
        _ = rx => false,
    })
}

/// Clear the notice saved with every pane that has none showing. Called when
/// the host registers: a notice lives in memory, and one saved with a pane from
/// before srv restarted would show a Stop that stops nothing.
pub(crate) fn reset_notices(state: &AppState) {
    let live: std::collections::BTreeSet<String> = PENDING.lock().values().map(|(b, _)| b.clone()).collect();
    super::browser_signin_offer::clear_stale(state, PENDING_META_KEY, &live, "[signin]");
}

/// The person pressed [Stop] on notice `id` of pane `block_id`.
pub(crate) fn stop(block_id: &str, id: &str) -> Result<(), String> {
    let mut pending = PENDING.lock();
    match pending.get(id) {
        Some((b, _)) if b == block_id => {}
        _ => return Err("that sign-in has already been typed or stopped".to_string()),
    }
    if let Some((_, tx)) = pending.remove(id) {
        let _ = tx.send(());
    }
    Ok(())
}

/// The saved sign-in an agent means: its entry, label, site and user name.
struct Chosen {
    entry: String,
    label: String,
    origin: String,
    user: String,
}

/// Why nothing was typed: the outcome kept in the Vault's activity, how the
/// pane's log shows it, and what the agent is told.
struct Refusal {
    outcome: &'static str,
    how: How,
    code: StatusCode,
    why: &'static str,
}

/// The tab's profile, and whether it is Incognito, which keeps no sign-ins of
/// its own: it may use Personal's, asked each time.
fn profile_of(block: &crate::backend::obj::Block) -> (String, bool) {
    let identity = block.meta.get(super::browser_identity::IDENTITY_META_KEY).and_then(Value::as_str);
    match signins::profile_for_tab(identity) {
        Some(p) => (p, false),
        None => (PERSONAL.to_string(), true),
    }
}

/// The saved sign-in of `profile` the agent means, among those that fit a
/// sign-in form the page shows now.
async fn chosen(state: &AppState, block_id: &str, agent: &str, profile: &str, entry: Option<&str>) -> Result<Chosen, Response> {
    let origins = super::browser_signin_fill::form_origins(block_id);
    let (store, p) = (state.identity_store.clone(), profile.to_string());
    let found = match tokio::task::spawn_blocking(move || signins::offered_for(&store, &p, &origins)).await {
        Ok(Ok(f)) => f,
        Ok(Err(e)) => return Err(err_response(StatusCode::INTERNAL_SERVER_ERROR, e)),
        Err(e) => return Err(err_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };
    match choose(found, entry) {
        Ok((entry, origin, user)) => Ok(Chosen { label: label_of(state, &entry), entry, origin, user }),
        Err(e) => {
            noted(state, block_id, agent, How::Refused(e.clone()));
            Err(err_response(StatusCode::BAD_REQUEST, e))
        }
    }
}

/// The person's yes: under an "Always" the notice shows and nobody pressed
/// Stop; otherwise they answered the Vault's prompt.
async fn approve(state: &AppState, block_id: &str, agent: &str, c: &Chosen, incognito: bool) -> Result<(), Refusal> {
    let key = grant_key(agent);
    let (store, e, k) = (state.identity_store.clone(), c.entry.clone(), key.clone());
    let granted = !incognito && tokio::task::spawn_blocking(move || store.vault_grant_check(&e, &k).unwrap_or(false)).await.unwrap_or(false);
    if !granted {
        return ask(state, block_id, agent, c, incognito, key).await;
    }
    match notice(state, block_id, agent, &c.origin, &c.user).await {
        Some(true) => Ok(()),
        Some(false) => Err(Refusal { outcome: "stopped", how: How::Declined, code: StatusCode::FORBIDDEN, why: "the person stopped the sign-in: nothing was typed" }),
        None => Err(Refusal { outcome: "another was showing", how: How::Refused(BUSY.to_string()), code: StatusCode::CONFLICT, why: BUSY }),
    }
}

/// The agent may still act on the pane after the wait: the person had time to
/// pause it or take it over, and nothing is typed then.
fn still_allowed(state: &AppState, caller: Option<&crate::server::caller::Caller>, req: &SigninRequest, block_id: &str) -> Result<(), (StatusCode, String)> {
    match target_block_id(state, caller, &req.auth, req.pane.as_deref()) {
        Ok(b) if b == block_id => Ok(()),
        Ok(_) => Err((StatusCode::CONFLICT, "the pane changed while the sign-in waited: nothing was typed".to_string())),
        Err((code, e)) => Err((code, format!("{e}; nothing was typed"))),
    }
}

fn use_request(id: &str, agent: &str, c: &Chosen, incognito: bool) -> UseRequest {
    UseRequest {
        id: id.to_string(),
        agent: agent.to_string(),
        entry_id: c.entry.clone(),
        entry_label: c.label.clone(),
        what: "sign-in".to_string(),
        origin: c.origin.clone(),
        field: signins::mask(&c.user),
        offer_always: !incognito,
        reason: if incognito {
            "this is an Incognito tab, which keeps no sign-ins: Personal's is used, and asked for each time".to_string()
        } else {
            format!("{agent} hasn't signed in with this before")
        },
        needs_value: false,
        kind: "login".to_string(),
        command: String::new(),
        cwd: String::new(),
        replaces: false,
        signin: true,
        requested_ms: agentmux_common::time::now_ms_u64(),
    }
}

/// Ask the person in the Vault's prompt; an "Always" is kept under `key`.
async fn ask(state: &AppState, block_id: &str, agent: &str, c: &Chosen, incognito: bool, key: String) -> Result<(), Refusal> {
    let use_id = uuid::Uuid::new_v4().to_string();
    let answer = uses::uses().ask(use_request(&use_id, agent, c, incognito));
    let _waiting = Waiting::new(use_id, state.clone());
    super::vault_handlers::publish(state);
    let _asking = crate::backend::user_attention::Asking::start(
        &state.broker,
        &state.mstore,
        block_id,
        "vault",
        &format!("{agent} wants to sign in to {}", c.origin),
    );
    match tokio::time::timeout(ANSWER_WAIT, answer).await {
        Ok(Ok(a)) if a.decision == Decision::Once => Ok(()),
        Ok(Ok(a)) if a.decision == Decision::Always => {
            let (store, e) = (state.identity_store.clone(), c.entry.clone());
            let _ = tokio::task::spawn_blocking(move || {
                let created = store.vault_get(&e).ok().flatten().map(|r| r.created_at).unwrap_or(0);
                store.vault_grant_add(&e, &key, agentmux_common::time::now_ms(), created)
            })
            .await;
            Ok(())
        }
        Ok(_) => Err(Refusal { outcome: "denied", how: How::Declined, code: StatusCode::FORBIDDEN, why: "the person said no: nothing was typed" }),
        Err(_) => Err(Refusal {
            outcome: "timed out",
            how: How::TimedOut,
            code: StatusCode::REQUEST_TIMEOUT,
            why: "the person didn't answer in time: nothing was typed; ask again when they are there",
        }),
    }
}

/// `POST /api/v1/ui/browser/signin`.
pub(crate) async fn handle_signin(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<SigninRequest>,
) -> Response {
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    let agent = req.auth.agent_id.chars().take(64).collect::<String>();
    let Some(block) = state.mstore.get::<crate::backend::obj::Block>(&block_id).ok().flatten() else {
        return err_response(StatusCode::NOT_FOUND, format!("no pane {block_id}"));
    };
    let (profile, incognito) = profile_of(&block);
    let c = match chosen(&state, &block_id, &agent, &profile, req.entry.as_deref()).await {
        Ok(c) => c,
        Err(response) => return response,
    };
    tracing::info!(agent = %agent, block_id = %block_id, entry = %c.entry, "[signin] an agent asked to sign in");
    if let Err(r) = approve(&state, &block_id, &agent, &c, incognito).await {
        crate::backend::vault::record(&state.identity_store, &c.entry, &c.label, &agent, signins::SIGNED_IN, &c.origin, r.outcome);
        super::vault_handlers::publish(&state);
        noted(&state, &block_id, &agent, r.how);
        return err_response(r.code, r.why.to_string());
    }
    if let Err((code, e)) = still_allowed(&state, caller.as_deref(), &req, &block_id) {
        noted(&state, &block_id, &agent, How::Refused(e.clone()));
        return err_response(code, e);
    }

    match super::browser_signin_fill::fill_as(&state, &block_id, &c.entry, &profile, &agent).await {
        Ok((site, username_filled)) => {
            noted(&state, &block_id, &agent, How::Allowed);
            let out = json!({ "filled": true, "entry": c.entry, "site": site, "username_filled": username_filled, "password": "[secret: hidden]" });
            (StatusCode::OK, Json(json!({ "ok": true, "data": out }))).into_response()
        }
        Err(e) => {
            noted(&state, &block_id, &agent, How::Refused(e.clone()));
            err_response(StatusCode::BAD_GATEWAY, e)
        }
    }
}

/// `browser.signin_stop`, from the window only: the person's [Stop] on the notice.
pub(crate) fn register(engine: &std::sync::Arc<crate::backend::rpc::engine::WshRpcEngine>) {
    use crate::backend::rpc_types::{CommandBrowserSigninStopData, COMMAND_BROWSER_SIGNIN_STOP};
    engine.register_typed(COMMAND_BROWSER_SIGNIN_STOP, move |req: CommandBrowserSigninStopData, ctx| async move {
        ctx.operator_only("only the person stops an agent's sign-in")?;
        stop(&req.block_id, &req.id)
    });
}

#[cfg(test)]
#[path = "browser_signin_tests.rs"]
mod tests;
