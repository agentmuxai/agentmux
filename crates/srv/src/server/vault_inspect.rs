// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/vault/{show,update,verify,health}`: what an agent can ask the Vault about an entry and about itself
//! (docs/specs/SPEC_VAULT_PARITY_ENTRY_HISTORY_REASONS_AND_IMPORT_2026_11_06.md §8). Each answers ids, names, states
//! and counts, for the Vaults the proven agent may use; none returns a value. `update` changes details only and needs
//! the agent's signed identity and a reason, as the record names who changed what. Nobody is asked, a change to where the
//! entry's machine copy is included: the history line is the record
//! (docs/specs/SPEC_VAULT_AGENT_ADDS_AND_EDITS_WITHOUT_ASKING_2026_11_25.md).

use std::time::Duration;

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
use crate::backend::vault::shares::{self, Ask, Asked};
use crate::backend::vault::uses::{Ended, UseHttpPrompt, UseRequest};

/// The longest the person has to answer a share.
const ANSWER_WAIT_MAX: Duration = Duration::from_secs(600);

type Auth = Option<agentmux_common::api_types::UiAutomationAuth>;

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

/// The agent whose signed identity verified, else none (which sees only the Vaults open to every agent).
fn who(state: &AppState, caller: Option<&super::caller::Caller>, auth: &Auth) -> Option<String> {
    let (agent, pane) = asking_agent(state, caller, auth.as_ref());
    pane.map(|_| agent)
}

/// The Vault key as it reads now, or why it can't be.
async fn key() -> KeyResult {
    tokio::task::spawn_blocking(|| seal::key(false)).await.unwrap_or_else(|e| Err(e.to_string()))
}

/// Runs `read` on the blocking pool; a refusal is a 404 (the entry isn't one this agent may use, or isn't there), a
/// 400 or a 500.
async fn blocking<T: Send + 'static>(read: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, Response> {
    match tokio::task::spawn_blocking(read).await {
        Ok(Ok(out)) => Ok(out),
        Ok(Err(e)) if e.starts_with("no entry") => Err(error(StatusCode::NOT_FOUND, e)),
        Ok(Err(e)) => Err(error(StatusCode::BAD_REQUEST, e)),
        Err(e) => Err(error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// [`blocking`], answering its result as JSON.
async fn answer<T: Serialize + Send + 'static>(read: impl FnOnce() -> Result<T, String> + Send + 'static) -> Response {
    match blocking(read).await {
        Ok(out) => Json(out).into_response(),
        Err(refusal) => refusal,
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
pub(crate) async fn handle_vault_show(State(state): State<AppState>, caller: Option<axum::Extension<super::caller::Caller>>, Json(body): Json<IdBody>) -> Response {
    let agent = who(&state, caller.as_deref(), &body.auth);
    let Some(id) = body.id.filter(|i| !i.trim().is_empty()) else { return error(StatusCode::BAD_REQUEST, "show needs an id: the entry to look at (list shows them)") };
    let uid = agent.as_deref().and_then(|a| state.reactive_handler.get_agent(a)).and_then(|a| a.uid);
    let (store, key) = (state.identity_store.clone(), key().await);
    answer(move || inspect::show_as(&store, &id, (agent.as_deref(), uid.as_deref()), &key, crate::backend::vault::expiry::today())).await
}

/// `POST /api/v1/vault/verify`
pub(crate) async fn handle_vault_verify(State(state): State<AppState>, caller: Option<axum::Extension<super::caller::Caller>>, Json(body): Json<IdBody>) -> Response {
    let agent = who(&state, caller.as_deref(), &body.auth);
    let (store, key) = (state.identity_store.clone(), key().await);
    answer(move || inspect::verify(&store, body.id.as_deref(), agent.as_deref(), &key, crate::backend::vault::expiry::today())).await
}

/// `POST /api/v1/vault/health`
pub(crate) async fn handle_vault_health(State(state): State<AppState>, caller: Option<axum::Extension<super::caller::Caller>>, Json(body): Json<IdBody>) -> Response {
    let agent = who(&state, caller.as_deref(), &body.auth);
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
pub(crate) async fn handle_vault_update(State(state): State<AppState>, caller: Option<axum::Extension<super::caller::Caller>>, Json(body): Json<UpdateBody>) -> Response {
    let (agent, pane) = asking_agent(&state, caller.as_deref(), body.auth.as_ref());
    if pane.is_none() {
        return error(StatusCode::FORBIDDEN, "update needs to know which agent asks (its token or its signed identity, neither of which verified): the Vault records who changed each entry; respawn the agent if it has neither");
    }
    let request = body.request;
    if let Err(why) = crate::backend::vault::pace::admit(&agent, &[crate::backend::vault::pace::Act::Edit]) {
        let (store, id, by) = (state.identity_store.clone(), request.id.clone(), agent.clone());
        let reason = request.reason.clone().unwrap_or_default();
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(Some(row)) = store.vault_get(id.trim()) {
                let line = crate::backend::vault::Line { entry_id: &row.id, label: &row.label, actor: &by, action: inspect::CHANGED_DETAILS, target: "", outcome: "refused: pace", reason: &reason };
                crate::backend::vault::record_line(&store, line);
            }
        })
        .await;
        return error(StatusCode::TOO_MANY_REQUESTS, why);
    }
    let store = state.identity_store.clone();
    let response = answer(move || inspect::update_details(&store, &request, &agent)).await;
    if response.status().is_success() {
        publish(&state);
    }
    response
}

/// How long the person has to answer an agent's request to be given a login by default, and at most.
const SHARE_WAIT: Duration = Duration::from_secs(300);

#[derive(Debug, Deserialize)]
pub(crate) struct ShareBody {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    /// An agent may name only itself: a share for another agent, or for every agent, is the person's to make.
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    auth: Auth,
    #[serde(default)]
    wait_secs: Option<u64>,
}

/// The question for the person: the login, its site and who asks to be able to sign in with it.
fn share_card(ask: &Ask, agent: &str, pane: &str) -> UseRequest {
    UseRequest {
        id: uuid::Uuid::new_v4().to_string(),
        agent: agent.to_string(),
        block_id: pane.to_string(),
        entry_id: ask.entry_id.clone(),
        entry_label: ask.label.clone(),
        what: "share".into(),
        origin: ask.site.clone(),
        field: ask.user.clone(),
        // A form login's "Always" is given at its first sign-in. A prompt's is given here: the browser answers it
        // with no call of the agent's to ask about (A11).
        offer_always: ask.prompt,
        reason: if ask.prompt { "an agent asked for its browser to answer this sign-in prompt with this login".into() } else { "an agent asked to be able to sign in with this login".into() },
        agent_reason: ask.reason.clone(),
        kind: "login".into(),
        requested_ms: agentmux_common::time::now_ms_u64(),
        share: true,
        prompt: ask.prompt.then(|| UseHttpPrompt { realm: ask.realm.clone(), proxy: ask.proxy }),
        ..Default::default()
    }
}

/// The card's headline, as the attention list shows it.
fn share_headline(ask: &Ask, agent: &str) -> String {
    if ask.prompt {
        return format!("{agent} wants its browser to answer the sign-in prompt of {} with {}", ask.site, ask.label);
    }
    format!("{agent} wants to sign in to {} with {}", ask.site, ask.label)
}

/// What the agent is told of a share the person made: for a sign-in the browser answers a prompt with, also that it
/// is not typed by `signin` and what to do next.
fn shared_answer(ask: &Ask, always: bool) -> serde_json::Value {
    let mut out = json!({ "status": "shared", "id": ask.entry_id, "label": ask.label, "site": ask.site });
    if ask.prompt {
        out["prompt"] = json!(true);
        out["always"] = json!(always);
        out["next"] = json!("load the page again in a tab of your own browser profile: the browser answers the site's sign-in prompt with it, you never see it, and you are told what became of it");
    }
    out
}

/// What the agent is told when the card didn't end in a yes: silence is a no.
fn share_not_allowed(ended: Ended) -> String {
    match ended {
        Ended::Expired | Ended::NoAnswer => "nobody answered, and a login is shared only on a yes: nothing was shared; ask the person".to_string(),
        other => other.refusal("nothing was shared"),
    }
}

/// `POST /api/v1/vault/share`: an agent asks the person to share a login with its own browser profile, so that it can
/// sign in to the login's site with it (docs/specs/SPEC_VAULT_AGENTS_ENTER_CREDENTIALS_WHEREVER_ASKED_2026_11_16.md
/// A2). Only the person's yes makes the share, and only for the agent that proved who it is. The agent learns the
/// login's label and site, never its user name or its password.
pub(crate) async fn handle_vault_share(State(state): State<AppState>, caller: Option<axum::Extension<super::caller::Caller>>, Json(body): Json<ShareBody>) -> Response {
    let (agent, pane) = asking_agent(&state, caller.as_deref(), body.auth.as_ref());
    let Some(pane) = pane else {
        return error(StatusCode::FORBIDDEN, "share needs to know which agent asks (its token or its signed identity, neither of which verified): a login is shared with the agent that proved who it is; respawn the agent if it has neither");
    };
    let uid = state.reactive_handler.get_agent(&agent).and_then(|a| a.uid).unwrap_or_default();
    if let Some(named) = body.agent.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
        if !named.eq_ignore_ascii_case(&agent) && named != uid {
            return error(StatusCode::FORBIDDEN, "an agent asks for a share for itself only: leave agent out. A login shared with another agent, or with every agent, is the person's to share, from the login's card in the Vault pane");
        }
    }
    let Some(id) = body.id.map(|i| i.trim().to_string()).filter(|i| !i.is_empty()) else {
        return error(StatusCode::BAD_REQUEST, "share needs an id: the login to be given (list shows them)");
    };
    let (store, reason, who, who_uid) = (state.identity_store.clone(), body.reason, agent.clone(), uid.clone());
    let planned = {
        let id = id.clone();
        blocking(move || shares::plan_ask(&store, &id, &who, &who_uid, reason.as_deref())).await
    };
    let ask = match planned {
        Ok(Asked::Already(label)) => return Json(json!({ "status": "shared", "id": id, "label": label, "already": true })).into_response(),
        Ok(Asked::Ask(ask)) => ask,
        Err(refusal) => return refusal,
    };
    let wait = body.wait_secs.map(Duration::from_secs).unwrap_or(SHARE_WAIT).min(ANSWER_WAIT_MAX);
    let headline = share_headline(&ask, &agent);
    let (ended, _) = super::vault_ask::ask(&state, &pane, share_card(&ask, &agent, &pane), wait, &headline).await;
    if !ended.allowed() {
        let (store, ask, who) = (state.identity_store.clone(), ask.clone(), agent.clone());
        let _ = tokio::task::spawn_blocking(move || shares::note_ask(&store, &ask, &who, ended.outcome())).await;
        publish(&state);
        return error(StatusCode::FORBIDDEN, share_not_allowed(ended));
    }
    // "Always" is only on the card of a sign-in the browser answers a prompt with (A11).
    let always = ask.prompt && ended == Ended::Always;
    let (store, made, who, to) = (state.identity_store.clone(), ask.clone(), agent.clone(), uid.clone());
    let granted = blocking(move || {
        if always {
            let profile = format!("profile:{}", crate::backend::browser_profiles_store::agent_profile_id(&to));
            shares::grant_asked_always(&store, &made, &to, &profile)?;
        } else {
            shares::grant_asked(&store, &made, &to)?;
        }
        shares::note_ask(&store, &made, &who, "done");
        Ok(())
    })
    .await;
    publish(&state);
    match granted {
        Ok(()) => {
            // The yes also answers the prompt that waited for it, once.
            if ask.prompt && !always {
                super::vault_prompt_share::yes_given(&ask.entry_id, &uid);
            }
            Json(shared_answer(&ask, always)).into_response()
        }
        Err(refusal) => refusal,
    }
}
