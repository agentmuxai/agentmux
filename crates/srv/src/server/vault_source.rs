// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/vault/check`: an agent asks whether an entry's machine copy still holds the entry's value
//! (`backend::vault::source_link`). srv reads the other place with the agent's own access (the host's AWS setup for
//! a native agent, a broker session for a container agent's grant), compares inside srv and answers `same`,
//! `different` or `could not read` with why. No value comes back, and a place that refuses the read is the answer.
//!
//! A CI secret is read with the GitHub CLI signed in on this computer, which is the person's login
//! (`backend::vault::github_secret`). A native agent already runs as the person. A container agent does not, so its
//! check starts that read only for an entry it holds the person's standing approval to use; without it the answer is
//! the newest stored result, the reason and what to do. Every read that is started is one line in Audit.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::vault_import::{asker, credentials, Who};
use super::AppState;
use crate::backend::rpc::action_record::{ActionRecord, Target};
use crate::backend::rpc::actor::Actor;
use crate::backend::vault::aws_import::Via;
use crate::backend::vault::github_secret::Place;
use crate::backend::vault::linked::place_text;
use crate::backend::vault::source_link::{self, CheckPlan, Checked, Outcome, AWS_SECRET, COULD_NOT_READ, NOT_STARTED};
use crate::backend::vault::vaults;
use crate::identity::aws_broker::mint::{HostRunner, Runner};

/// The Audit action of a read of a CI secret's listing entry.
pub(crate) const AUDIT_ACTION: &str = "vault.check.ci-secret";

#[derive(Debug, Deserialize)]
pub(crate) struct CheckBody {
    #[serde(default)]
    id: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    auth: Option<agentmux_common::api_types::UiAutomationAuth>,
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

/// Why a check was not started, as a status: an entry the agent can't use is not there, and the hourly limit is a
/// "try later".
fn refusal(why: String) -> Response {
    let status = if why.starts_with("no entry") {
        StatusCode::NOT_FOUND
    } else if why.contains("is the limit") {
        StatusCode::TOO_MANY_REQUESTS
    } else {
        StatusCode::BAD_REQUEST
    };
    error(status, why)
}

/// A check that ended before the place was read, recorded like any other.
async fn could_not_read(state: &AppState, plan: CheckPlan, why: String) -> Checked {
    let store = state.identity_store.clone();
    let stopped = Checked::of(&plan, &Outcome::CouldNotRead(why.clone()));
    tokio::task::spawn_blocking(move || source_link::conclude(&store, &plan, &Outcome::CouldNotRead(why))).await.unwrap_or(stopped)
}

/// Whether the person answered Always to this agent's use of the entry's value, in a Vault the agent may use: the
/// standing approval a container agent's read of a CI secret needs.
async fn approved(state: &AppState, plan: &CheckPlan, who: &Who) -> bool {
    let (store, id, agent) = (state.identity_store.clone(), plan.entry_id.clone(), who.agent.clone());
    let key = super::shell_secrets::grant_key(&who.agent, plan.part);
    let asked = move || store.vault_grant_check(&id, &key).unwrap_or(false) && vaults::entry_usable(&store, &id, Some(&agent)).unwrap_or(false);
    tokio::task::spawn_blocking(asked).await.unwrap_or(false)
}

/// Why a container agent's check starts no read of a CI secret, and what to do. A link holds no value, so nobody
/// can have answered Always for it.
fn held(plan: &CheckPlan) -> String {
    let instead = if plan.only {
        "This entry holds no value, so it has no such answer: ask the person to check it, with the button on the entry in the Vault pane."
    } else {
        "Ask for the entry through the Vault, and check again once the person has answered Always."
    };
    format!("A container agent's check of a CI secret is read with the person's GitHub login on this computer, so it starts only for an entry the person answered Always for to this agent. {instead}")
}

/// What the check found at `place`, one of the plan's CI secrets; `None` when no read of it was started.
fn read_of<'a>(plan: &CheckPlan, checked: &'a Checked, place: &source_link::Link) -> Option<(&'a str, &'a str)> {
    let found = if *place == plan.link {
        Some((checked.outcome, checked.why.as_str()))
    } else {
        checked.copies.iter().find(|copy| copy.place == place_text(place)).map(|copy| (copy.outcome, copy.why.as_str()))
    };
    found.filter(|(outcome, _)| *outcome != NOT_STARTED)
}

/// One Audit line for every CI secret the check read, an action of the agent's or of the person's: who, which
/// entry, which repository and the outcome. A place no read was started for has none.
fn audit_reads(state: &AppState, who: Option<&Who>, plan: &CheckPlan, checked: &Checked) {
    let actor = who.map_or(Actor::Operator, |who| Actor::Agent(who.slug.clone()));
    for place in plan.github_places() {
        let Some((outcome, why)) = read_of(plan, checked, place) else { continue };
        let repository = Place::of(place).map(|p| p.repository()).unwrap_or_else(|_| place.location.clone());
        let note = format!("checked the CI secret {} of {repository} for {} ({}): {outcome}", place.key, plan.label, plan.entry_id);
        state.reactive_handler.audit_action(&ActionRecord {
            actor: actor.clone(),
            command: AUDIT_ACTION.to_string(),
            // The read acts on no agent and no pane: the line names the secret, its repository and the entry.
            target: Target { agent: None, block_id: None, subject: Some(note) },
            request_id: uuid::Uuid::new_v4().to_string(),
            error: (outcome == COULD_NOT_READ).then(|| why.to_string()),
        });
    }
}

/// The person's check, from the Vault pane: read with this computer's own logins, and recorded.
pub(crate) fn person_check(state: &AppState, id: &str) -> Result<Checked, String> {
    person_check_with(state, &HostRunner, id)
}

fn person_check_with<R: Runner>(state: &AppState, runner: &R, id: &str) -> Result<Checked, String> {
    let store = &state.identity_store;
    let plan = source_link::plan_for_person(store, id)?;
    let checked = source_link::check_blocking(store, runner, &plan, Via::Host)?;
    audit_reads(state, None, &plan, &checked);
    Ok(checked)
}

/// Reads the place as `who` and compares. A container agent reads an AWS secret under the grant the link names; when
/// it has none, or the broker refuses, that is what the check found. Its read of a CI secret needs the person's
/// standing approval of the entry ([`approved`]).
async fn run<R: Runner>(state: &AppState, runner: &R, mut plan: CheckPlan, who: &Who) -> Checked {
    if who.container && !plan.github_places().is_empty() && !approved(state, &plan, who).await {
        plan.no_github = Some(held(&plan));
    }
    let minted = if who.container && plan.link.kind == AWS_SECRET {
        if plan.link.profile.is_empty() {
            let why = format!("a container agent reads AWS under a grant, and this entry's {} names none", source_link::PROFILE);
            return could_not_read(state, plan, why).await;
        }
        match credentials(state, who, &plan.link.profile, &plan.reason).await {
            Ok(minted) => minted,
            Err(why) => return could_not_read(state, plan, why).await,
        }
    } else {
        None
    };
    let via = minted.as_ref().map_or(Via::Host, Via::Session);
    let checked = source_link::check(&state.identity_store, runner, &plan, via).await;
    audit_reads(state, Some(who), &plan, &checked);
    checked
}

/// `POST /api/v1/vault/check`
pub(crate) async fn handle_vault_check(State(state): State<AppState>, Json(body): Json<CheckBody>) -> Response {
    let who = match asker(&state, body.auth.as_ref(), "check", "who checked each entry") {
        Ok(who) => who,
        Err(refusal) => return refusal,
    };
    if body.id.trim().is_empty() {
        return error(StatusCode::BAD_REQUEST, "check needs an id: the entry whose machine copy to check (list shows which have one)");
    }
    let (store, agent) = (state.identity_store.clone(), who.agent.clone());
    let planned = tokio::task::spawn_blocking(move || source_link::plan_for_agent(&store, &body.id, &agent, body.reason.as_deref())).await;
    let plan = match planned {
        Ok(Ok(plan)) => plan,
        Ok(Err(why)) => return refusal(why),
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let checked = run(&state, &HostRunner, plan, &who).await;
    super::vault_handlers::publish(&state);
    Json(checked).into_response()
}

#[cfg(test)]
#[path = "vault_source_ci_tests.rs"]
mod ci_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_says_not_found_for_an_entry_out_of_reach_and_try_later_at_the_limit() {
        assert_eq!(refusal("no entry api-key-example".into()).status(), StatusCode::NOT_FOUND);
        assert_eq!(refusal("Example was checked 6 times by agents in the last hour, which is the limit: its history has what they found".into()).status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(refusal("Example has no machine copy recorded".into()).status(), StatusCode::BAD_REQUEST);
    }
}
