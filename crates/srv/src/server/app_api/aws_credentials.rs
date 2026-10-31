// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/aws/credentials`: the AWS credentials a container agent's SDK gets through `credential_process`.
//! Spec: docs/specs/SPEC_CONTAINER_AGENT_AWS_CREDENTIALS_2026_10_31.md §5.
//!
//! `agentmux-bashwrap aws-credentials` asks here from the container with the container's token. Who is asking is the
//! agent that token was minted for, never a name in the request. The checks, the cache and the mint are
//! `identity::aws_broker`; this adds the identity and the audit row. A fresh session and every refusal are recorded in
//! the Audit view; a cache hit is not (the layer in `http_action` leaves this route to the handler for that reason).

use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::AppState;
use crate::identity::aws_broker::mint::{HostRunner, Runner};
use crate::identity::aws_broker::request::{credentials_for, Ask, Granted, Refusal};
use crate::identity::aws_broker::sessions::Source;
use crate::identity::aws_broker::{policy, sessions};
use crate::server::caller::Caller;

/// The audit entry's action.
pub(crate) const AUDIT_ACTION: &str = "aws.credentials";

#[derive(Debug, Deserialize)]
pub(crate) struct AwsCredentialsRequest {
    pub(crate) profile: String,
    #[serde(default)]
    pub(crate) reason: Option<String>,
    #[serde(default)]
    pub(crate) duration_seconds: Option<u64>,
}

fn refuse(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

/// One Audit row: the actor is the container agent. Holds the grant, tier, reason, seconds and outcome; never a key,
/// a token or any part of a credential.
fn audit(state: &AppState, slug: &str, uid: &str, profile: &str, ok: bool, error: Option<&str>, note: &str) {
    let block_id = state.reactive_handler.block_for_uid(uid).unwrap_or_default();
    let reason = format!("{slug} (uid {uid}): grant {profile}: {note}");
    state.reactive_handler.log_fleet_action_audit(Some(slug), slug, &block_id, AUDIT_ACTION, ok, error, &uuid::Uuid::new_v4().to_string(), Some(&reason));
}

fn granted_note(g: &Granted) -> String {
    let renewed = if g.source == Source::Fresh { "minted" } else { "reused" };
    let reason = if g.reason.is_empty() { "no reason given".to_string() } else { format!("reason: {}", g.reason) };
    format!("{} tier, {} s, {renewed}; {reason}", g.grant.tier.as_str(), g.seconds)
}

/// `POST /api/v1/aws/credentials`
pub(crate) async fn handle_aws_credentials(
    State(state): State<AppState>,
    caller: Option<Extension<Caller>>,
    Json(req): Json<AwsCredentialsRequest>,
) -> Response {
    let Some(uid) = caller.as_ref().and_then(|Extension(c)| c.uid()).map(str::to_string) else {
        return refuse(StatusCode::UNAUTHORIZED, "no agent token: AWS credentials are for an agent's own commands");
    };
    let mstore = state.mstore.clone();
    let lookup = uid.clone();
    let def = match tokio::task::spawn_blocking(move || mstore.agent_def_get(&lookup)).await {
        Ok(Ok(Some(def))) => def,
        Ok(Ok(None)) => return refuse(StatusCode::UNAUTHORIZED, "the agent token names no agent on this machine"),
        _ => return refuse(StatusCode::INTERNAL_SERVER_ERROR, "couldn't read the agent"),
    };
    let policy = tokio::task::spawn_blocking(policy::current).await.unwrap_or_else(|e| Err(format!("couldn't read the grants file: {e}")));
    serve(&state, &uid, &def, req, policy, sessions::global(), &HostRunner).await
}

/// The request once the agent is known: the checks, the cache or the mint, the audit row, the answer.
pub(crate) async fn serve<R: Runner>(
    state: &AppState,
    uid: &str,
    def: &crate::backend::storage::AgentDefinition,
    req: AwsCredentialsRequest,
    policy: Result<Option<std::sync::Arc<policy::Policy>>, String>,
    sessions: &sessions::Sessions,
    runner: &R,
) -> Response {
    let ask = Ask {
        agent_uid: uid,
        agent_slug: &def.slug,
        in_container: def.agent_type == "container",
        profile: req.profile.trim(),
        reason: req.reason.as_deref(),
        seconds: req.duration_seconds,
    };
    match credentials_for(policy, sessions, runner, &ask, chrono::Utc::now()).await {
        Ok(granted) => {
            if granted.source == Source::Fresh {
                tracing::info!(target: "identity", agent = %def.slug, grant = %granted.grant.name, seconds = granted.seconds, "identity.aws: credentials minted for a container agent");
                audit(state, &def.slug, uid, &granted.grant.name, true, None, &granted_note(&granted));
            }
            Json(granted.creds.to_json()).into_response()
        }
        Err(refusal) => refused(state, &def.slug, uid, ask.profile, refusal),
    }
}

fn refused(state: &AppState, slug: &str, uid: &str, profile: &str, refusal: Refusal) -> Response {
    tracing::info!(target: "identity", agent = %slug, grant = %profile, code = refusal.code, "identity.aws: credentials refused");
    audit(state, slug, uid, profile, false, Some(refusal.code), &format!("refused: {}", refusal.code));
    refuse(refusal.status, &refusal.text())
}

#[cfg(test)]
#[path = "tests/aws_credentials_tests.rs"]
mod tests;
