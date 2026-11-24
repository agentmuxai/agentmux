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
use crate::sagas::credential_notice::{self, Cause};
use crate::server::caller::Caller;

/// The audit entry's action.
pub(crate) const AUDIT_ACTION: &str = "aws.credentials";

#[derive(Debug, Deserialize)]
pub(crate) struct AwsCredentialsRequest {
    pub(crate) profile: String,
    #[serde(default)]
    pub(crate) reason: Option<String>,
    /// The value of the one tag a grant takes from the caller (`AGENTMUX_AWS_TAG`).
    #[serde(default)]
    pub(crate) tag: Option<String>,
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

/// The Audit row of a session a container agent was granted, when it is a fresh one (a cache hit is not recorded).
pub(crate) fn audit_granted(state: &AppState, slug: &str, uid: &str, granted: &Granted) {
    if granted.source == Source::Fresh {
        tracing::info!(target: "identity", agent = %slug, grant = %granted.grant.name, seconds = granted.seconds, "identity.aws: credentials minted for a container agent");
        audit(state, slug, uid, &granted.grant.name, true, None, &granted_note(granted));
    }
}

fn granted_note(g: &Granted) -> String {
    let renewed = if g.source == Source::Fresh { "minted" } else { "reused" };
    let reason = if g.reason.is_empty() { "no reason given".to_string() } else { format!("reason: {}", g.reason) };
    // The tag's value was checked (letters, digits and _.:/=+-@, at most 256), so it is safe to record.
    let tag = g.tag.as_ref().map(|(k, v)| format!("; tag {k}: {v}")).unwrap_or_default();
    format!("{} tier, {} s, {renewed}; {reason}{tag}", g.grant.tier.as_str(), g.seconds)
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
        in_container: def.runtime().is_container(),
        profile: shown_profile(req.profile.trim()),
        reason: req.reason.as_deref(),
        tag: req.tag.as_deref(),
        seconds: req.duration_seconds,
    };
    match credentials_for(policy, sessions, runner, &ask, chrono::Utc::now()).await {
        Ok(granted) => {
            hold_in_transcripts(uid, ask.profile, &granted.creds);
            audit_granted(state, &def.slug, uid, &granted);
            credential_notice::worked(state, uid, &def.slug, Cause::Aws);
            Json(granted.creds.to_json()).into_response()
        }
        Err(refusal) => {
            if the_persons_to_fix(&refusal) {
                // The sentence the agent reads, in its pane.
                credential_notice::refused(state, uid, &def.slug, Cause::Aws, &refusal.message);
            }
            refused(state, &def.slug, uid, ask.profile, refusal)
        }
    }
}

/// The secret key and session token of the credentials an agent was given are never written into its transcript,
/// until they run out. Credentials an agent's profile is given next do not end these: they still work until their
/// own expiration, so they stay withheld until then.
fn hold_in_transcripts(uid: &str, profile: &str, creds: &crate::identity::aws_broker::mint::Minted) {
    let until = creds.expiration.timestamp_millis();
    let owner = |what: &str| format!("aws-credentials:{uid}:{profile}:{what}");
    crate::backend::transcript::withhold::hold_until(&owner("secret"), &creds.secret_access_key, until);
    crate::backend::transcript::withhold::hold_until(&owner("session"), &creds.session_token, until);
}

/// Whether a refusal is one the person can fix on this computer, so the agent's pane says so: the host's AWS login
/// lapsed or was refused, the grant is switched off, or a new or wider grant waits to be allowed. A wrong profile name, a missing reason, a cap or a slow mint is
/// the agent's own to correct or a bad moment.
fn the_persons_to_fix(refusal: &Refusal) -> bool {
    matches!(refusal.code, "AWS_HOST_LOGIN_NEEDED" | "AWS_HOST_REFUSED" | "GRANT_DISABLED" | "GRANT_HELD")
}

/// What the request's profile is called in the audit row, the log and the refusal. The string is the container's to
/// choose, so one that can't be a grant name (too long, control characters, newlines) is never recorded or echoed.
fn shown_profile(profile: &str) -> &str {
    if policy::is_grant_name(profile) { profile } else { "<not a grant name>" }
}

/// The Audit row of a refusal.
pub(crate) fn audit_refused(state: &AppState, slug: &str, uid: &str, profile: &str, refusal: &Refusal) {
    let profile = shown_profile(profile);
    tracing::info!(target: "identity", agent = %slug, grant = %profile, code = refusal.code, "identity.aws: credentials refused");
    audit(state, slug, uid, profile, false, Some(refusal.code), &format!("refused: {}", refusal.code));
}

fn refused(state: &AppState, slug: &str, uid: &str, profile: &str, refusal: Refusal) -> Response {
    audit_refused(state, slug, uid, profile, &refusal);
    refuse(refusal.status, &refusal.text())
}

#[cfg(test)]
#[path = "tests/aws_credentials_tests.rs"]
mod tests;
