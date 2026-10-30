// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agents.start`, `agents.open`, `agents.stop`, `agents.set_seat`, `agents.env_get` and `agents.env_set`: an
//! agent controls another machine's agents as it controls its own (spec §4.3, P3). Each runs the code of
//! the local `Agents` action with the remote agent (`slug@machine`) as the actor, so every rule a local
//! caller meets is met here too: a stop of another agent waits for the person's keep-running window, a
//! start counts against the sender's start limit, an environment refuses the names an agent may not set.
//! The agent a call names is one on this machine; an id or slug, as a local call takes it.

use axum::http::StatusCode;
use serde_json::{json, Value};

use super::{AppState, RemoteCaller, RemoteRefusal, RemoteResult};
use crate::server::app_api::{agent_env, agent_lifecycle, agent_open_remote, agent_seats};

/// A refusal of the local code, as the requester reads it.
fn refused((status, message): (StatusCode, String)) -> RemoteRefusal {
    let code = match status.as_u16() {
        400 => "invalid_arg",
        401 | 403 => "forbidden",
        404 => "not_found",
        409 => "conflict",
        429 => "rate_limited",
        _ => "failed",
    };
    RemoteRefusal::new(code, message)
}

/// The `agent` a call names: an id or slug of an agent on this machine.
fn agent_of(params: &Value) -> Result<&str, RemoteRefusal> {
    params
        .get("agent")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .ok_or_else(|| RemoteRefusal::new("invalid_arg", "agent is required: the id or slug of an agent on this machine"))
}

pub(super) async fn start(state: &AppState, caller: &RemoteCaller, params: Value) -> RemoteResult {
    let agent = agent_of(&params)?;
    agent_lifecycle::run_remote(state, &caller.actor(), "start", agent).await.map_err(refused)
}

/// Opens the agent in the caller's own tab, without focus (`agent_open_remote`).
pub(super) async fn open(state: &AppState, caller: &RemoteCaller, params: Value) -> RemoteResult {
    let agent = agent_of(&params)?;
    agent_open_remote::run_remote(state, &caller.actor(), agent).await.map_err(refused)
}

pub(super) async fn stop(state: &AppState, caller: &RemoteCaller, params: Value) -> RemoteResult {
    let agent = agent_of(&params)?;
    agent_lifecycle::run_remote(state, &caller.actor(), "stop", agent).await.map_err(refused)
}

pub(super) async fn set_seat(state: &AppState, caller: &RemoteCaller, params: Value) -> RemoteResult {
    let req: agent_seats::SetSeatsRequest =
        serde_json::from_value(params).map_err(|e| RemoteRefusal::new("invalid_arg", format!("seats: {e}")))?;
    agent_seats::run_remote(state, &caller.actor(), req).await.map_err(refused)
}

pub(super) async fn env_get(state: &AppState, caller: &RemoteCaller, params: Value) -> RemoteResult {
    env(state, caller, "get", params).await
}

pub(super) async fn env_set(state: &AppState, caller: &RemoteCaller, params: Value) -> RemoteResult {
    env(state, caller, "set", params).await
}

async fn env(state: &AppState, caller: &RemoteCaller, action: &str, params: Value) -> RemoteResult {
    let mut body = params;
    let Some(map) = body.as_object_mut() else {
        return Err(RemoteRefusal::new("invalid_arg", "params must be an object"));
    };
    map.insert("action".to_string(), json!(action));
    let req: agent_env::AgentEnvRequest =
        serde_json::from_value(body).map_err(|e| RemoteRefusal::new("invalid_arg", format!("env: {e}")))?;
    let (state, who) = (state.clone(), caller.actor());
    tokio::task::spawn_blocking(move || agent_env::run_remote(&state, &who, req))
        .await
        .map_err(|e| RemoteRefusal::failed(format!("env: {e}")))?
        .map_err(refused)
}

#[cfg(test)]
#[path = "agent_ops_tests.rs"]
mod tests;
