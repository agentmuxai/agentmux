// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The AWS grants a host shares with its container agents, as the broker has them: `GET /api/v1/aws/grants` lists each
//! grant with whether it is in force, when it was added and by which recorded change, and `POST
//! /api/v1/aws/grants/decide` answers a grant that is new or wider than what was allowed (`identity::aws_broker::record`).
//!
//! Any agent may read the list. Only a native agent may answer: a container agent is the one a grant is for, so it
//! cannot allow its own. The answer needs a reason, is recorded under the agent's name in the Audit view and in the
//! record, and takes effect on the next request for credentials.

use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::AppState;
use crate::identity::aws_broker::{policy, record};
use crate::server::caller::Caller;

#[derive(Debug, Deserialize)]
pub(crate) struct DecideRequest {
    pub(crate) grant: String,
    /// The change the answerer read in the list; an answer to a grant that has changed since is refused.
    pub(crate) change: String,
    /// `allow` or `remove`.
    pub(crate) decision: String,
    pub(crate) reason: String,
}

fn refuse(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

/// The agent the token was minted for, and whether it runs in a container.
async fn agent_of(state: &AppState, caller: Option<&Caller>) -> Result<(String, bool), Response> {
    let Some(uid) = caller.and_then(Caller::uid).map(str::to_string) else {
        return Err(refuse(StatusCode::UNAUTHORIZED, "no agent token: the AWS grants are for an agent to read and answer"));
    };
    let mstore = state.mstore.clone();
    match tokio::task::spawn_blocking(move || mstore.agent_def_get(&uid)).await {
        Ok(Ok(Some(def))) => Ok((def.slug.clone(), def.runtime().is_container())),
        Ok(Ok(None)) => Err(refuse(StatusCode::UNAUTHORIZED, "the agent token names no agent on this machine")),
        _ => Err(refuse(StatusCode::INTERNAL_SERVER_ERROR, "couldn't read the agent")),
    }
}

/// `GET /api/v1/aws/grants`
pub(crate) async fn handle_aws_grants(State(state): State<AppState>, caller: Option<Extension<Caller>>) -> Response {
    if let Err(r) = agent_of(&state, caller.as_ref().map(|Extension(c)| c)).await {
        return r;
    }
    match tokio::task::spawn_blocking(policy::view).await {
        Ok(Ok(view)) => Json(view).into_response(),
        Ok(Err(e)) => refuse(StatusCode::SERVICE_UNAVAILABLE, e),
        Err(e) => refuse(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// `POST /api/v1/aws/grants/decide`
pub(crate) async fn handle_aws_grant_decide(State(state): State<AppState>, caller: Option<Extension<Caller>>, Json(req): Json<DecideRequest>) -> Response {
    let (slug, container) = match agent_of(&state, caller.as_ref().map(|Extension(c)| c)).await {
        Ok(who) => who,
        Err(r) => return r,
    };
    if container {
        return refuse(StatusCode::FORBIDDEN, "A container agent can't answer for the grants it may use: a native agent (a leader or the host's steward) answers, saying why.");
    }
    let allow = match req.decision.trim() {
        "allow" => true,
        "remove" => false,
        other => return refuse(StatusCode::BAD_REQUEST, format!("decision {other:?} is not allow or remove")),
    };
    let result = tokio::task::spawn_blocking(move || {
        policy::decide(&record::Decision { grant: req.grant.trim(), change: req.change.trim(), allow, by: &slug, reason: &req.reason })
    })
    .await;
    match result {
        Ok(Ok(event)) => Json(json!({ "ok": true, "change": event.id, "grant": event.grant, "text": event.text })).into_response(),
        Ok(Err(e)) => refuse(StatusCode::BAD_REQUEST, e),
        Err(e) => refuse(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[cfg(test)]
#[path = "tests/aws_grants_tests.rs"]
mod tests;
