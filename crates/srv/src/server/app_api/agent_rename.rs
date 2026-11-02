// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents rename` (`POST /api/v1/agent/rename`): an agent gives an agent on this machine a new displayed
//! name, by the same path the window's rename takes (`agent_handlers::rename`), so the pane, My Agents, the
//! Swarm, message addressing, seats and the Kanban all follow. The agent's id, slug, signing keys and
//! workspace folder do not change: they are fixed when it launches. The old name keeps reaching it as a former
//! name, and no other agent may take it.
//!
//! The caller is the agent its token names; a request without one is refused, so a change is never attributed
//! to the operator. A name another agent has now is refused. Each rename, and each refusal, is recorded in the
//! audit log under the calling agent.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use super::agent_lifecycle::{refuse, target_of, Refusal};
use crate::server::agent_handlers::rename::RenameError;
use crate::server::caller::Caller;
use crate::server::state::AppState;

/// The audit entry's action for a rename.
pub(crate) const AUDIT_ACTION: &str = "agent.rename";
/// The longest name: what a pane tab and a Swarm row can show.
const MAX_NAME_CHARS: usize = 64;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RenameRequest {
    /// The agent's id or slug.
    agent: String,
    /// Its new displayed name.
    name: String,
}

/// The agent making the request.
struct Actor {
    uid: String,
    slug: String,
}

fn actor_of(state: &AppState, caller: Option<&Caller>) -> Result<Actor, Refusal> {
    let Some(uid) = caller.and_then(Caller::uid) else {
        return Err(refuse(
            StatusCode::UNAUTHORIZED,
            "no agent token: Agents rename acts as the calling agent; the operator renames in My Agents",
        ));
    };
    let def = state
        .mstore
        .agent_def_get(uid)
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("Agents rename: {e}")))?
        .ok_or_else(|| refuse(StatusCode::UNAUTHORIZED, "the agent token names no agent on this machine"))?;
    Ok(Actor { uid: uid.to_string(), slug: def.slug })
}

fn audit(state: &AppState, actor: &Actor, target: &str, ok: bool, error: Option<&str>, note: &str) -> String {
    let request_id = uuid::Uuid::new_v4().to_string();
    let block_id = state.reactive_handler.block_for_uid(&actor.uid).unwrap_or_default();
    let reason = format!("{} (uid {}): {note}", actor.slug, actor.uid);
    state.reactive_handler.log_fleet_action_audit(Some(&actor.slug), target, &block_id, AUDIT_ACTION, ok, error, &request_id, Some(&reason));
    request_id
}

async fn run_as(state: &AppState, actor: &Actor, req: RenameRequest) -> Result<Value, Refusal> {
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(refuse(StatusCode::BAD_REQUEST, "INVALID_ARG: name is empty; give the agent's new name"));
    }
    if name.chars().count() > MAX_NAME_CHARS || name.chars().any(char::is_control) {
        return Err(refuse(StatusCode::BAD_REQUEST, format!("INVALID_ARG: a name is at most {MAX_NAME_CHARS} characters on one line")));
    }
    let blocking = state.clone();
    let named = req.agent.clone();
    let target = tokio::task::spawn_blocking(move || target_of(&blocking, &named))
        .await
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("Agents rename: {e}")))??;
    let (updated, old_name) = crate::server::agent_handlers::rename::rename_title(state, &target.agent_id, &name, true)
        .await
        .map_err(|e| {
            let status = match &e {
                RenameError::NotFound(_) => StatusCode::NOT_FOUND,
                RenameError::Invalid(_) => StatusCode::BAD_REQUEST,
                RenameError::Taken(_) => StatusCode::CONFLICT,
                RenameError::Failed(_) => StatusCode::INTERNAL_SERVER_ERROR,
            };
            refuse(status, format!("Agents rename: {e}"))
        })?;
    let audit_id = audit(state, actor, &updated.slug, true, None, &format!("renamed {old_name} ({}) to {name}", target.agent_id));
    Ok(json!({
        "agent_id": target.agent_id,
        "slug": updated.slug,
        "name": name,
        "was": old_name,
        "audit_id": audit_id,
        "note": "its id, slug and workspace folder are unchanged; the old name still reaches it; if it is running it has been told",
    }))
}

async fn run(state: &AppState, caller: Option<&Caller>, req: RenameRequest) -> Result<Value, Refusal> {
    let actor = actor_of(state, caller)?;
    let named = req.agent.clone();
    match run_as(state, &actor, req).await {
        Ok(body) => Ok(body),
        Err(refusal) => {
            audit(state, &actor, named.trim(), false, Some(&refusal.message), "refused");
            Err(refusal)
        }
    }
}

/// `rename` for an agent on another machine (`remote_ops::agent_ops`): the same checks and change a local call
/// has, recorded under `who` (`slug@machine`).
pub(crate) async fn run_remote(state: &AppState, who: &str, req: RenameRequest) -> Result<Value, (StatusCode, String)> {
    let actor = Actor { uid: format!("remote:{who}"), slug: who.to_string() };
    let named = req.agent.clone();
    match run_as(state, &actor, req).await {
        Ok(body) => Ok(body),
        Err(r) => {
            audit(state, &actor, named.trim(), false, Some(&r.message), "refused");
            Err((r.status, r.message))
        }
    }
}

pub(crate) async fn handle_agent_rename(
    State(state): State<AppState>,
    caller: Option<Extension<Caller>>,
    Json(req): Json<RenameRequest>,
) -> Response {
    let caller = caller.map(|Extension(c)| c);
    match run(&state, caller.as_ref(), req).await {
        Ok(body) => Json(body).into_response(),
        Err(r) => (r.status, Json(json!({ "error": r.message }))).into_response(),
    }
}

#[cfg(test)]
#[path = "tests/agent_rename_tests.rs"]
mod tests;
