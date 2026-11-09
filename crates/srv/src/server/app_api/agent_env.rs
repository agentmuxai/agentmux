// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents env_get` and `Agents env_set`: an agent, host or container, reads and edits the
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
use crate::backend::endpoint_notice;
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

/// Why this change to `target`'s model endpoint is refused, if it is. Not for what it does (the notice and its
/// undo window answer that) but for where it would not work: an agent on a model route keeps the route's
/// server and keys, a container agent gets the person's account mounted in its image, which a custom
/// server must not receive, and a key must be a Vault entry the person opened to that agent. A key is
/// also one the acting agent could use itself: srv does not put a key in a server's hands for an agent
/// that could not have read it. (An agent on another machine names none: it has no agent here.) The
/// answer is the same for an entry that is missing, of another kind or closed to an agent, so it never
/// says which entries exist.
fn endpoint_refusal(state: &AppState, actor: &Actor, target: &AgentDefinition, current: &str, new: &str) -> Option<Refusal> {
    let bad = |m: String| Some(refuse(StatusCode::BAD_REQUEST, m));
    if crate::backend::model_gateway::launch::choice_of(&state.mstore, &target.id).is_some() {
        return bad(format!(
            "{} is on a model route, which sets its server, key and models: switch its route instead (the model panel, or Agents update)",
            target.name
        ));
    }
    if target.runtime().is_container() {
        return bad(format!(
            "{} is a container agent: its image is given the person's account, which a server of its own must not receive. Use a model route, or ask the operator to set the endpoint in My Agents",
            target.name
        ));
    }
    let before = agent_env::effective(current);
    for (name, value) in agent_env::effective(new) {
        let key = name.eq_ignore_ascii_case("ANTHROPIC_AUTH_TOKEN") || name.eq_ignore_ascii_case("ANTHROPIC_API_KEY");
        // A reference already there was checked when it was written, and again at every launch.
        let (Some(entry), true) = (agent_env::vault_ref(&value), key && before.get(&name) != Some(&value)) else { continue };
        let usable = |agent_id: &str| crate::backend::vault::endpoint_key::problem(&state.identity_store, entry, agent_id).is_none();
        let local_actor = !actor.uid.starts_with("remote:");
        if !(local_actor && usable(&actor.uid) && usable(&target.id)) {
            return bad(format!(
                "{name}: {}; the person opens a Vault to an agent in the Vault's agents setting",
                crate::backend::vault::endpoint_key::NOT_AVAILABLE
            ));
        }
    }
    None
}

/// Held from reading an environment to writing it, so two agents changing the
/// same definition at once don't lose one change. (A save from My Agents goes
/// through `setagentcontent` and isn't serialized with this.)
static ENV_WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn run(state: &AppState, caller: Option<&Caller>, req: AgentEnvRequest) -> Result<Value, Refusal> {
    let actor = actor_of(state, caller)?;
    run_as(state, actor, req)
}

/// `env_get` or `env_set` for an agent on another machine (`remote_ops::agent_ops`): the same rules and
/// write a local call has, recorded under `who` (`slug@machine`). The agent must be named: a remote
/// caller has no definition here to default to.
pub(crate) fn run_remote(state: &AppState, who: &str, req: AgentEnvRequest) -> Result<Value, (StatusCode, String)> {
    if req.agent.as_deref().map(str::trim).filter(|a| !a.is_empty()).is_none() {
        return Err((StatusCode::BAD_REQUEST, "INVALID_ARG: agent is required: the id or slug of an agent on this machine".to_string()));
    }
    let actor = Actor { uid: format!("remote:{who}"), slug: who.to_string(), name: who.to_string() };
    run_as(state, actor, req).map_err(|r| (r.status, r.message))
}

fn run_as(state: &AppState, actor: Actor, req: AgentEnvRequest) -> Result<Value, Refusal> {
    let target = target_of(state, &actor, req.agent.as_deref())?;
    let _writing = (req.action == "set").then(|| ENV_WRITE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()));
    let current = current_text(state, &target.id)?;
    let agent = json!({ "id": target.id, "slug": target.slug, "name": target.name });
    match req.action.as_str() {
        "get" => Ok(json!({
            "agent": agent,
            "lines": agent_env::redacted(&current),
            "note": "values of names an agent may not set are shown as [hidden]; a change applies at the agent's next launch",
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
            let endpoint = change.touches_endpoint();
            if change.sets_endpoint() {
                if let Some(refusal) = endpoint_refusal(state, &actor, &target, &current, &new) {
                    audit(state, &actor, &target, false, Some(&refusal.message), "refused (model endpoint)");
                    return Err(refusal);
                }
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
            let summary = if endpoint { format!("{} [model endpoint]", change.summary()) } else { change.summary() };
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
            let notice = endpoint.then(|| {
                let event = endpoint_notice::open(
                    (&target.id, &target.slug, &target.name),
                    &actor.name,
                    endpoint_notice::pointed(&current, &new),
                    &current,
                    &new,
                    agentmux_common::time::now_ms(),
                );
                state.broker.publish_event::<crate::backend::mps_events::AgentEndpointNotice>(&event, vec![], 0);
                json!({ "id": event.id, "undo_for_ms": event.window_ms })
            });
            Ok(json!({
                "agent": agent,
                "changed": change,
                "lines": agent_env::redacted(&new),
                "audit_id": audit_id,
                "note": if endpoint {
                    "applies at the agent's next launch, on a conversation of its own: its pane and My Agents say who pointed it where, with an Undo for 30 seconds"
                } else {
                    "applies at the agent's next launch"
                },
                "notice": notice,
            }))
        }
        other => Err(refuse(StatusCode::BAD_REQUEST, format!("unknown env action '{other}': get or set"))),
    }
}

/// Undo a model-server change within its window, for the person's Undo on the notice: puts the environment
/// back as it was. `by` is who pressed it, for the record.
pub(crate) fn undo_endpoint(state: &AppState, notice_id: &str, by: &str) -> Result<(), String> {
    let agent_id = endpoint_notice::agent_id_of(notice_id).ok_or_else(|| endpoint_notice::Refusal::Unknown.words().to_string())?;
    let def = state
        .mstore
        .agent_def_get(&agent_id)
        .map_err(|e| format!("agent env: {e}"))?
        .ok_or_else(|| format!("no agent '{agent_id}' on this machine"))?;
    let actor = Actor { uid: format!("ui:{by}"), slug: by.to_string(), name: by.to_string() };
    let _writing = ENV_WRITE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let current = current_text(state, &def.id).map_err(|r| r.message)?;
    let (previous, event) = endpoint_notice::check_for_undo(notice_id, &current, agentmux_common::time::now_ms()).map_err(|r| r.words().to_string())?;
    let content = AgentContent {
        agent_id: def.id.clone(),
        content_type: agent_env::CONTENT_TYPE.to_string(),
        content: previous,
        updated_at: agentmux_common::time::now_ms(),
    };
    if let Err(e) = state.mstore.agent_content_set(&content) {
        let message = format!("agent env: {e}");
        audit(state, &actor, &def, false, Some(&message), "undo of a model endpoint change: not saved");
        return Err(message);
    }
    endpoint_notice::finish_undo(notice_id);
    audit(state, &actor, &def, true, None, &format!("undid the model endpoint change {} made", event.by));
    state.broker.publish_event::<crate::backend::mps_events::AgentContentChanged>(&(), vec![], 0);
    state.broker.publish_event::<crate::backend::mps_events::AgentEndpointNotice>(&event, vec![], 0);
    Ok(())
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
