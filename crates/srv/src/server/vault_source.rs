// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/vault/check`: an agent asks whether an entry's machine copy still holds the entry's value
//! (`backend::vault::source_link`). srv reads the other place with the agent's own access (the host's AWS setup for
//! a native agent, a broker session for a container agent's grant), compares inside srv and answers `same`,
//! `different` or `could not read` with why. No value comes back, and a place that refuses the read is the answer.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::vault_import::{asker, credentials, Who};
use super::AppState;
use crate::backend::vault::aws_import::Via;
use crate::backend::vault::source_link::{self, CheckPlan, Checked, Outcome, AWS_SECRET};
use crate::identity::aws_broker::mint::HostRunner;

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
    let stopped = Checked { id: plan.entry_id.clone(), label: plan.label.clone(), outcome: source_link::COULD_NOT_READ, why: why.clone(), source: plan.link.text(), checked_at: agentmux_common::time::now_ms() };
    tokio::task::spawn_blocking(move || source_link::conclude(&store, &plan, &Outcome::CouldNotRead(why))).await.unwrap_or(stopped)
}

/// Reads the place as `who` and compares. A container agent reads an AWS secret under the grant the link names; when
/// it has none, or the broker refuses, that is what the check found.
async fn run(state: &AppState, plan: CheckPlan, who: &Who) -> Checked {
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
    source_link::check(&state.identity_store, &HostRunner, &plan, via).await
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
    let checked = run(&state, plan, &who).await;
    super::vault_handlers::publish(&state);
    Json(checked).into_response()
}

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
