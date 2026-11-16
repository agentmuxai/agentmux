// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/vault/{show,update,verify,health}`: what an agent can ask the Vault about an entry and about itself
//! (docs/specs/SPEC_VAULT_PARITY_ENTRY_HISTORY_REASONS_AND_IMPORT_2026_11_06.md §8). Each answers ids, names, states
//! and counts, for the Vaults the proven agent may use; none returns a value. `update` changes details only and needs
//! the agent's signed identity and a reason, as the record names who changed what. An update that changes where the
//! entry's machine copy is asks the person first, showing the old place and the new one, and silence is a no.

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::vault_handlers::{asking_agent, publish};
use super::AppState;
use crate::backend::vault::inspect::{self, KeyResult, LinkChange, UpdateRequest};
use crate::backend::vault::seal;
use crate::backend::vault::source_link::Link;
use crate::backend::vault::uses::{Ended, UseLink, UseRequest};

/// How long the person has to answer a change to an entry's machine copy by default, and at most.
const ANSWER_WAIT: Duration = Duration::from_secs(120);
const ANSWER_WAIT_MAX: Duration = Duration::from_secs(600);

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
    /// How long to wait for the person, when the update changes where the entry's machine copy is.
    #[serde(default)]
    wait_secs: Option<u64>,
}

/// The question for the person: the entry, the place its machine copy is in now and the place asked for, and what
/// the agent says it is for.
fn link_card(change: &LinkChange, agent: &str, pane: &str) -> UseRequest {
    let place = |link: &Option<Link>| link.as_ref().map(Link::describe).unwrap_or_default();
    UseRequest {
        id: uuid::Uuid::new_v4().to_string(),
        agent: agent.to_string(),
        block_id: pane.to_string(),
        entry_id: change.entry_id.clone(),
        entry_label: change.label.clone(),
        what: "machine copy".into(),
        offer_always: false,
        reason: "an agent asked to change where this entry's machine copy is".into(),
        agent_reason: change.reason.clone(),
        kind: change.kind.clone(),
        requested_ms: agentmux_common::time::now_ms_u64(),
        link: Some(UseLink { from: place(&change.from), to: place(&change.to), about_from: change.about_from.clone(), about_to: change.about_to.clone(), only: change.only }),
        ..Default::default()
    }
}

/// What the agent is told when the card didn't end in a yes: silence is a no for a new place, whatever the kind.
fn link_not_allowed(ended: Ended) -> String {
    match ended {
        Ended::Expired => "nobody answered, and a change to where an entry's machine copy is needs a yes: nothing was changed; ask the person".to_string(),
        other => other.refusal("nothing was changed"),
    }
}

/// Asks the person to allow `change`; a refusal is recorded in the entry's history and answered.
async fn allow_link_change(state: &AppState, change: &LinkChange, agent: &str, pane: &str, wait: Duration) -> Result<(), Response> {
    let headline = match change.only {
        true => format!("{agent} wants to change what the Vault records about the key {}", change.label),
        false => format!("{agent} wants to change where the machine copy of {} is", change.label),
    };
    let (ended, _) = super::vault_ask::ask(state, pane, link_card(change, agent, pane), wait, &headline).await;
    if ended.allowed() {
        return Ok(());
    }
    let (store, change, agent) = (state.identity_store.clone(), change.clone(), agent.to_string());
    let _ = tokio::task::spawn_blocking(move || inspect::note_link_refused(&store, &change, &agent, ended.outcome())).await;
    publish(state);
    Err(error(StatusCode::FORBIDDEN, link_not_allowed(ended)))
}

/// `POST /api/v1/vault/update`
pub(crate) async fn handle_vault_update(State(state): State<AppState>, Json(body): Json<UpdateBody>) -> Response {
    let (agent, pane) = asking_agent(&state, body.auth.as_ref());
    let Some(pane) = pane else {
        return error(StatusCode::FORBIDDEN, "update needs the agent's signed identity: the Vault records who changed each entry");
    };
    let request = body.request;
    let (store, asked, by) = (state.identity_store.clone(), request.clone(), agent.clone());
    let change = match blocking(move || inspect::link_change(&store, &asked, &by)).await {
        Ok(change) => change,
        Err(refusal) => return refusal,
    };
    if let Some(change) = &change {
        let wait = body.wait_secs.map(Duration::from_secs).unwrap_or(ANSWER_WAIT).min(ANSWER_WAIT_MAX);
        if let Err(refusal) = allow_link_change(&state, change, &agent, &pane, wait).await {
            return refusal;
        }
    }
    let store = state.identity_store.clone();
    let response = answer(move || match &change {
        Some(allowed) => inspect::update_details_allowed(&store, &request, &agent, allowed),
        None => inspect::update_details(&store, &request, &agent),
    })
    .await;
    if response.status().is_success() {
        publish(&state);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change() -> LinkChange {
        let to = Link { kind: "aws-secret".into(), location: "staging/app".into(), key: "db.password".into(), region: "eu-west-1".into(), profile: String::new() };
        LinkChange {
            entry_id: "api-key-example".into(),
            label: "Example".into(),
            kind: "api-key".into(),
            from: None,
            to: Some(to),
            about_from: String::new(),
            about_to: "Read by: invoice-sync (lambda): at run time.".into(),
            only: false,
            reason: "record where the staging copy is for task 28071".into(),
        }
    }

    #[test]
    fn the_card_shows_the_old_place_and_the_new_one_and_never_goes_ahead_on_silence() {
        let card = link_card(&change(), "builder", "block-1");
        let link = card.link.as_ref().unwrap();
        assert_eq!((link.from.as_str(), link.to.as_str()), ("", "the AWS secret staging/app, key db.password, region eu-west-1"));
        assert_eq!((card.entry_id.as_str(), card.entry_label.as_str(), card.block_id.as_str()), ("api-key-example", "Example", "block-1"));
        assert_eq!(card.agent_reason, "record where the staging copy is for task 28071");
        // An api-key use goes ahead on silence; a new place for its machine copy never does.
        assert!(crate::backend::vault::uses::goes_ahead_on_silence("api-key", false));
        assert!(!card.offer_always && !crate::backend::vault::uses::goes_ahead(&card));
    }

    #[test]
    fn silence_on_a_new_place_says_it_needed_a_yes() {
        let said = link_not_allowed(Ended::Expired);
        assert!(said.contains("needs a yes") && said.contains("nothing was changed") && !said.contains("ID or card"), "{said}");
        assert_eq!(link_not_allowed(Ended::Denied), "the person said no: nothing was changed");
    }
}
