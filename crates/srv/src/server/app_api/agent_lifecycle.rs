// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents start`, `stop` and `detach` (`POST /api/v1/agent/{start,stop,detach}`): an agent runs, stops
//! or leaves its pane without the operator's window
//! (docs/specs/SPEC_ALWAYS_AVAILABLE_AGENTS_2026_10_27.md §5.1). Each acts as the agent its token
//! names; a request without one is refused, so a change is never attributed to the operator, and
//! each one, and each refusal, is recorded in the audit log under the calling agent.
//!
//! - start: `agent_attach::start_agent`, the one start path (admission, the spawn gate and the lease
//!   apply), after the per-sender start limit (`start_rate`). Running already is not a start.
//! - stop: `agent_teardown::stop`, which also records the agent as wanted stopped. Not your own
//!   agent: `Stop` `self` is the way to end yourself.
//! - detach: the agent's pane closes and its block is parked, whatever `agents:always_available`
//!   says (the setting decides what closing a pane does; this is the explicit action).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::server::agent_runtime::{self, AgentRuntime, AgentRuntimeState};
use crate::server::caller::Caller;
use crate::server::state::AppState;

#[derive(Debug, Deserialize)]
pub(crate) struct LifecycleRequest {
    /// The agent's id, slug or name.
    agent: String,
}

pub(super) struct Refusal {
    pub(super) status: StatusCode,
    pub(super) message: String,
}

pub(super) fn refuse(status: StatusCode, message: impl Into<String>) -> Refusal {
    Refusal { status, message: message.into() }
}

/// The agent making the request.
pub(super) struct Actor {
    uid: String,
    slug: String,
}

pub(super) fn actor_of(state: &AppState, caller: Option<&Caller>, action: &str) -> Result<Actor, Refusal> {
    let Some(uid) = caller.and_then(Caller::uid) else {
        return Err(refuse(
            StatusCode::UNAUTHORIZED,
            format!("no agent token: Agents {action} acts as the calling agent; the operator uses the Swarm or My Agents"),
        ));
    };
    let def = state
        .mstore
        .agent_def_get(uid)
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("Agents {action}: {e}")))?
        .ok_or_else(|| refuse(StatusCode::UNAUTHORIZED, "the agent token names no agent on this machine"))?;
    Ok(Actor { uid: uid.to_string(), slug: def.slug })
}

/// The agent named, as its runtime row: an id, slug or name of an agent created on this machine.
/// A template is not an agent.
pub(super) fn target_of(state: &AppState, named: &str) -> Result<AgentRuntime, Refusal> {
    let named = named.trim();
    if named.is_empty() {
        return Err(refuse(StatusCode::BAD_REQUEST, "INVALID_ARG: agent is empty; give an agent's id or slug (Agents list)"));
    }
    let uid = crate::backend::agent_resolve::resolve_agent_id(&state.mstore, named)
        .map_err(|e| refuse(StatusCode::NOT_FOUND, format!("no agent '{named}' on this machine: {e}")))?;
    agent_runtime::list(state, agentmux_common::time::now_ms_u64())
        .into_iter()
        .find(|r| r.agent_id == uid)
        .ok_or_else(|| refuse(StatusCode::NOT_FOUND, format!("TEMPLATE_NOT_OPENABLE: '{named}' is not an agent created on this machine")))
}

/// Running: the agent is in a state with a process and is not wanted stopped. The state alone does
/// not say: a subprocess agent that was stopped reads `idle` (its process exits between turns), and a
/// stored sign-in or limit failure shows over an ended process. An explicit stop is what settles it.
fn is_alive(row: &AgentRuntime) -> bool {
    !matches!(row.state, AgentRuntimeState::Stopped | AgentRuntimeState::Error) && row.desired == agent_runtime::AgentDesiredState::Running
}

fn audit(state: &AppState, actor: &Actor, action: &str, target: &str, block_id: &str, ok: bool, error: Option<&str>, note: &str) -> String {
    let request_id = uuid::Uuid::new_v4().to_string();
    let reason = format!("agent-initiated {action} by {} (uid {}): {note}", actor.slug, actor.uid);
    tracing::info!(action, target, ok, "{reason}");
    state.reactive_handler.log_fleet_action_audit(Some(&actor.slug), target, block_id, action, ok, error, &request_id, Some(&reason));
    request_id
}

/// What the answer says about the agent now: its state (re-read after the change).
fn row_json(state: &AppState, agent_id: &str) -> Value {
    agent_runtime::list(state, agentmux_common::time::now_ms_u64())
        .into_iter()
        .find(|r| r.agent_id == agent_id)
        .map(|r| json!({ "state": r.state, "desired": r.desired, "block_id": r.block_id }))
        .unwrap_or(Value::Null)
}

async fn start(state: &AppState, actor: &Actor, req: &LifecycleRequest) -> Result<Value, Refusal> {
    let blocking = state.clone();
    let named = req.agent.clone();
    let target = tokio::task::spawn_blocking(move || target_of(&blocking, &named))
        .await
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("Agents start: {e}")))??;
    // Running already is not a start: it costs nothing, so it does not use the sender's minute.
    if is_alive(&target) {
        return Ok(json!({
            "agent": target.name, "agent_id": target.agent_id, "started": false,
            "note": "already running", "runtime": { "state": target.state, "desired": target.desired, "block_id": target.block_id },
        }));
    }
    crate::server::start_rate::allow(state, &actor.uid).map_err(|e| refuse(StatusCode::TOO_MANY_REQUESTS, e))?;
    let block_id = crate::server::app_api::agent_attach::start_agent(state, &target.agent_id)
        .await
        .map_err(|e| refuse(StatusCode::CONFLICT, format!("Agents start {}: {e}", target.name)))?;
    Ok(json!({
        "agent": target.name, "agent_id": target.agent_id, "started": true, "block_id": block_id,
        "note": "running with no pane; Agents open shows it, Agents stop ends it",
        "runtime": row_json(state, &target.agent_id),
    }))
}

async fn stop(state: &AppState, actor: &Actor, req: &LifecycleRequest) -> Result<Value, Refusal> {
    let blocking = state.clone();
    let named = req.agent.clone();
    let target = tokio::task::spawn_blocking(move || target_of(&blocking, &named))
        .await
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("Agents stop: {e}")))??;
    if target.agent_id == actor.uid {
        return Err(refuse(
            StatusCode::CONFLICT,
            "SELF_STOP: that is you. Stop with action \"self\" ends your own agent, with the person's window to keep you",
        ));
    }
    let Some(block_id) = target.block_id.clone() else {
        return Ok(json!({ "agent": target.name, "agent_id": target.agent_id, "stopped": false, "note": "already stopped: it has no process" }));
    };
    if !is_alive(&target) {
        crate::server::app_api::agent_attach::set_desired(&state.mstore, &block_id, crate::server::app_api::agent_attach::DESIRED_STOPPED);
        return Ok(json!({ "agent": target.name, "agent_id": target.agent_id, "stopped": false, "block_id": block_id, "note": "already stopped" }));
    }
    crate::sagas::agent_teardown::stop(state, &block_id, true, None)
        .await
        .map_err(|e| refuse(StatusCode::CONFLICT, format!("Agents stop {}: {e}", target.name)))?;
    Ok(json!({
        "agent": target.name, "agent_id": target.agent_id, "stopped": true, "block_id": block_id,
        "note": "stopped by an agent and wanted stopped: a restart does not resume it; Agents start or a message starts it again",
        "runtime": row_json(state, &target.agent_id),
    }))
}

async fn detach(state: &AppState, req: &LifecycleRequest) -> Result<Value, Refusal> {
    let blocking = state.clone();
    let named = req.agent.clone();
    let (target, found) = tokio::task::spawn_blocking(move || {
        let target = target_of(&blocking, &named)?;
        let found = crate::server::app_api::agent_block::find_agent_block_by_uid(&blocking.mstore, &target.agent_id)
            .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        Ok::<_, Refusal>((target, found))
    })
    .await
    .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("Agents detach: {e}")))??;
    let Some(found) = found else {
        return Err(refuse(StatusCode::NOT_FOUND, format!("{} has no pane or parked block to detach: Agents start runs it with none", target.name)));
    };
    let block_id = found.block.oid.clone();
    if found.tab_id.is_none() {
        return Ok(json!({ "agent": target.name, "agent_id": target.agent_id, "detached": false, "block_id": block_id, "note": "already detached: no pane shows it" }));
    }
    // Detach parks or it does nothing: a block that cannot be parked is left running, never closed.
    if !crate::sagas::park_agents::is_parkable(state, &block_id) {
        return Err(refuse(
            StatusCode::CONFLICT,
            format!("Agents detach {}: its block is not an agent pane that can be parked; it is left as it is", target.name),
        ));
    }
    // This agent's block alone: a pane that stacks other tabs keeps them.
    let opts = crate::sagas::close_pane::CloseOpts { frontend_waits: true, park_agents: true };
    crate::sagas::close_pane::run_with(state, vec![block_id.clone()], opts)
        .await
        .map_err(|e| refuse(StatusCode::CONFLICT, format!("Agents detach {}: {e}", target.name)))?;
    // Parked means the agent's block exists, in no tab. Anything else is not a detach, whatever the saga said.
    let parked = crate::server::app_api::agent_block::find_agent_block_by_uid(&state.mstore, &target.agent_id)
        .ok()
        .flatten()
        .is_some_and(|b| b.tab_id.is_none());
    if !parked {
        return Err(refuse(StatusCode::CONFLICT, format!("Agents detach {}: its block is not parked", target.name)));
    }
    Ok(json!({
        "agent": target.name, "agent_id": target.agent_id, "detached": true, "block_id": block_id,
        "note": "the pane is gone and the agent keeps running; Agents open shows it again",
        "runtime": row_json(state, &target.agent_id),
    }))
}

async fn run(state: &AppState, caller: Option<&Caller>, action: &str, req: LifecycleRequest) -> Result<Value, Refusal> {
    let actor = actor_of(state, caller, action)?;
    let outcome = match action {
        "start" => start(state, &actor, &req).await,
        "stop" => stop(state, &actor, &req).await,
        _ => detach(state, &req).await,
    };
    let audit_action = format!("agent.{action}");
    match outcome {
        Ok(mut body) => {
            let block_id = body["block_id"].as_str().unwrap_or_default().to_string();
            let target = body["agent_id"].as_str().unwrap_or_default().to_string();
            let note = body["note"].as_str().unwrap_or_default().to_string();
            body["audit_id"] = json!(audit(state, &actor, &audit_action, &target, &block_id, true, None, &note));
            Ok(body)
        }
        Err(refusal) => {
            audit(state, &actor, &audit_action, req.agent.trim(), "", false, Some(&refusal.message), "refused");
            Err(refusal)
        }
    }
}

async fn answer(state: AppState, caller: Option<Extension<Caller>>, action: &str, req: LifecycleRequest) -> Response {
    let caller = caller.map(|Extension(c)| c);
    match run(&state, caller.as_ref(), action, req).await {
        Ok(body) => Json(body).into_response(),
        Err(r) => (r.status, Json(json!({ "error": r.message }))).into_response(),
    }
}

pub(crate) async fn handle_agent_start(State(state): State<AppState>, caller: Option<Extension<Caller>>, Json(req): Json<LifecycleRequest>) -> Response {
    answer(state, caller, "start", req).await
}

pub(crate) async fn handle_agent_stop(State(state): State<AppState>, caller: Option<Extension<Caller>>, Json(req): Json<LifecycleRequest>) -> Response {
    answer(state, caller, "stop", req).await
}

pub(crate) async fn handle_agent_detach(State(state): State<AppState>, caller: Option<Extension<Caller>>, Json(req): Json<LifecycleRequest>) -> Response {
    answer(state, caller, "detach", req).await
}

#[cfg(test)]
#[path = "tests/agent_lifecycle_tests.rs"]
mod tests;
