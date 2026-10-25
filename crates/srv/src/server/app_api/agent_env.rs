// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents env_get` and `Agents env_set`: a host agent reads and edits the
//! extra environment variables a definition launches with (its `env` content,
//! `backend::agent_env`), its own or another's. The change applies at that
//! agent's next launch.
//!
//! The caller is the agent its token names; a request without one (the UI has
//! `setagentcontent` for this) is refused, so the change is never attributed
//! to the operator. Each change, and each refused attempt, is recorded in the
//! audit log under the calling agent's slug and in the `identity` log with
//! its UID.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

use crate::backend::agent_env;
use crate::backend::storage::store::{AgentContent, AgentDefinition};
use crate::server::caller::Caller;
use crate::server::state::AppState;

/// The audit entry's action for a change.
pub(crate) const AUDIT_ACTION: &str = "agent.env.set";

#[derive(Debug, Deserialize)]
pub(crate) struct AgentEnvRequest {
    /// `get` or `set`.
    action: String,
    /// The definition's id or slug; the caller's own when absent.
    #[serde(default)]
    agent: Option<String>,
    /// `set`: the whole new text, comments and blank lines as written.
    #[serde(default)]
    lines: Option<String>,
    /// `set`: variables to add or change, the rest left alone.
    #[serde(default)]
    set: Option<BTreeMap<String, String>>,
    /// `set`: variables to remove.
    #[serde(default)]
    unset: Option<Vec<String>>,
}

struct Refusal {
    status: StatusCode,
    message: String,
}

fn refuse(status: StatusCode, message: impl Into<String>) -> Refusal {
    Refusal { status, message: message.into() }
}

/// The agent making the request.
struct Actor {
    uid: String,
    slug: String,
    name: String,
}

fn actor_of(state: &AppState, caller: Option<&Caller>) -> Result<Actor, Refusal> {
    let Some(uid) = caller.and_then(Caller::uid) else {
        return Err(refuse(
            StatusCode::UNAUTHORIZED,
            "no agent token: Agents env_get and env_set act as the calling agent; the operator edits an environment in My Agents",
        ));
    };
    let def = state
        .mstore
        .agent_def_get(uid)
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("agent env: {e}")))?
        .ok_or_else(|| refuse(StatusCode::UNAUTHORIZED, "the agent token names no agent on this machine"))?;
    if def.agent_type == "container" {
        return Err(refuse(StatusCode::FORBIDDEN, "CONTAINER_AGENT: a container agent can't change environments"));
    }
    Ok(Actor { uid: uid.to_string(), slug: def.slug, name: def.name })
}

/// The definition `agent` names, the caller's own when it names none.
fn target_of(state: &AppState, actor: &Actor, agent: Option<&str>) -> Result<AgentDefinition, Refusal> {
    let id = match agent.map(str::trim).filter(|a| !a.is_empty()) {
        None => actor.uid.clone(),
        Some(named) => crate::backend::agent_resolve::resolve_agent_id(&state.mstore, named)
            .map_err(|_| refuse(StatusCode::NOT_FOUND, format!("no agent '{named}' on this machine")))?,
    };
    let def = state
        .mstore
        .agent_def_get(&id)
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("agent env: {e}")))?
        .ok_or_else(|| refuse(StatusCode::NOT_FOUND, format!("no agent '{id}' on this machine")))?;
    if def.agent_type == "container" {
        return Err(refuse(StatusCode::FORBIDDEN, "CONTAINER_AGENT: a container agent's environment can't be changed from here"));
    }
    if def.is_seeded != 0 {
        return Err(refuse(StatusCode::BAD_REQUEST, format!("{} is a template; only an agent has an environment", def.name)));
    }
    Ok(def)
}

fn current_text(state: &AppState, id: &str) -> Result<String, Refusal> {
    state
        .mstore
        .agent_content_get(id, agent_env::CONTENT_TYPE)
        .map(|c| c.map(|c| c.content).unwrap_or_default())
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("agent env: {e}")))
}

/// The text `req` would leave, refused when it breaks a rule.
fn proposed(current: &str, req: &AgentEnvRequest) -> Result<String, Refusal> {
    let bad = |m: String| refuse(StatusCode::BAD_REQUEST, m);
    let text = match (&req.lines, &req.set, &req.unset) {
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
            return Err(bad("give lines (the whole new text) or set and unset, not both".to_string()))
        }
        (Some(lines), None, None) => {
            agent_env::resolve_lines(current, lines).map_err(&bad)?
        }
        (None, set, unset) => {
            if set.is_none() && unset.is_none() {
                return Err(bad("env_set needs lines, or set and/or unset".to_string()));
            }
            agent_env::edit(current, set.as_ref().unwrap_or(&BTreeMap::new()), unset.as_deref().unwrap_or(&[])).map_err(&bad)?
        }
    };
    agent_env::check_size(&text).map_err(&bad)?;
    Ok(text)
}

fn audit(state: &AppState, actor: &Actor, target: &AgentDefinition, ok: bool, error: Option<&str>, note: &str) -> String {
    let request_id = uuid::Uuid::new_v4().to_string();
    let block_id = state.reactive_handler.block_for_uid(&actor.uid).unwrap_or_default();
    let reason = format!("{} (uid {}) on {} ({}): {note}", actor.slug, actor.uid, target.name, target.id);
    state.reactive_handler.log_fleet_action_audit(
        Some(&actor.slug),
        &target.slug,
        &block_id,
        AUDIT_ACTION,
        ok,
        error,
        &request_id,
        Some(&reason),
    );
    request_id
}

/// Held from reading an environment to writing it, so two agents changing the
/// same definition at once don't lose one change. (A save from My Agents goes
/// through `setagentcontent` and isn't serialized with this.)
static ENV_WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn run(state: &AppState, caller: Option<&Caller>, req: AgentEnvRequest) -> Result<Value, Refusal> {
    let actor = actor_of(state, caller)?;
    let target = target_of(state, &actor, req.agent.as_deref())?;
    let _writing = (req.action == "set").then(|| ENV_WRITE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()));
    let current = current_text(state, &target.id)?;
    let agent = json!({ "id": target.id, "slug": target.slug, "name": target.name });
    match req.action.as_str() {
        "get" => Ok(json!({
            "agent": agent,
            "lines": agent_env::redacted(&current),
            "note": "values of credential-looking names are shown as [hidden]; a change applies at the agent's next launch",
        })),
        "set" => {
            let new = match proposed(&current, &req) {
                Ok(new) => new,
                Err(refusal) => {
                    audit(state, &actor, &target, false, Some(&refusal.message), "refused");
                    return Err(refusal);
                }
            };
            let change = agent_env::diff(&current, &new);
            if new == current {
                return Ok(json!({ "agent": agent, "changed": change, "note": "nothing changed" }));
            }
            let content = AgentContent {
                agent_id: target.id.clone(),
                content_type: agent_env::CONTENT_TYPE.to_string(),
                content: new.clone(),
                updated_at: agentmux_common::time::now_ms(),
            };
            if let Err(e) = state.mstore.agent_content_set(&content) {
                let message = format!("agent env: {e}");
                audit(state, &actor, &target, false, Some(&message), "not saved");
                return Err(refuse(StatusCode::INTERNAL_SERVER_ERROR, message));
            }
            let summary = change.summary();
            let audit_id = audit(state, &actor, &target, true, None, &summary);
            tracing::info!(
                target: "identity",
                actor_uid = %actor.uid,
                actor = %actor.name,
                agent_id = %target.id,
                agent = %target.name,
                "agent.env.set: {summary}"
            );
            state.broker.publish_event::<crate::backend::mps_events::AgentContentChanged>(&(), vec![], 0);
            Ok(json!({
                "agent": agent,
                "changed": change,
                "lines": agent_env::redacted(&new),
                "audit_id": audit_id,
                "note": "applies at the agent's next launch",
            }))
        }
        other => Err(refuse(StatusCode::BAD_REQUEST, format!("unknown env action '{other}': get or set"))),
    }
}

/// `POST /api/v1/agent/env`
pub(crate) async fn handle_agent_env(
    State(state): State<AppState>,
    caller: Option<Extension<Caller>>,
    Json(req): Json<AgentEnvRequest>,
) -> Response {
    let caller = caller.map(|Extension(c)| c);
    match tokio::task::spawn_blocking(move || run(&state, caller.as_ref(), req)).await {
        Ok(Ok(body)) => Json(body).into_response(),
        Ok(Err(r)) => (r.status, Json(json!({ "error": r.message }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("agent env: {e}") }))).into_response(),
    }
}

#[cfg(test)]
#[path = "tests/agent_env_tests.rs"]
mod tests;
