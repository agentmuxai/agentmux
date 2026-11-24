// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who may fire a drone at once (`SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md` §6.5), read and set over the websocket.
//!
//! * `getdroneaccess`: the agent that owns the drone and the agents it was given to. Anyone may read it.
//! * `setdroneagents`: replace the agents the drone is given to. The person's window may, and so may the agent that
//!   owns the drone (it gives its own drone to another agent). No other agent: giving oneself a drone would undo
//!   the whole check. The owner never changes.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::backend::rpc::actor::{actor_of, Actor};
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{RpcContext, COMMAND_GET_DRONE_ACCESS, COMMAND_SET_DRONE_AGENTS};
use crate::drone::access::{key, DroneAccess, DroneAccessStore};
use crate::drone::storage::DroneStore;
use crate::server::AppState;
use agentmux_common::time::now_ms;

/// The most agents one drone is given to, and the longest an agent's name.
const MAX_AGENTS: usize = 50;
const MAX_NAME: usize = 64;

#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct GetDroneAccessReq {
    pub drone_id: String,
}

#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct SetDroneAgentsReq {
    pub drone_id: String,
    /// The agents (by id) the drone is given to: this list replaces the old one.
    pub agents: Vec<String>,
}

fn must_exist(state: &AppState, drone_id: &str) -> Result<(), String> {
    match state.id_store.drone_get(drone_id) {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(format!("NOT_FOUND: no drone {drone_id:?}")),
        Err(e) => Err(format!("drone: {e}")),
    }
}

pub(crate) fn get_drone_access(state: &AppState, cmd: GetDroneAccessReq) -> Result<DroneAccess, String> {
    must_exist(state, &cmd.drone_id)?;
    state.id_store.access_get(&cmd.drone_id).map_err(|e| format!("getdroneaccess: {e}"))
}

pub(crate) fn set_drone_agents(state: &AppState, ctx: &RpcContext, cmd: SetDroneAgentsReq) -> Result<DroneAccess, String> {
    must_exist(state, &cmd.drone_id)?;
    let current = state.id_store.access_get(&cmd.drone_id).map_err(|e| format!("setdroneagents: {e}"))?;
    let by = match actor_of(ctx) {
        Actor::Operator | Actor::Person(_) => actor_of(ctx).label(),
        Actor::Agent(id) if !current.owner.is_empty() && current.owner == key(&id) => Actor::Agent(id).label(),
        _ => return Err("FORBIDDEN: only the person, or the agent that owns the drone, gives it to other agents".to_string()),
    };
    let agents: Vec<String> = cmd.agents.iter().map(|a| key(a)).filter(|a| !a.is_empty()).collect();
    if agents.len() > MAX_AGENTS || agents.iter().any(|a| a.chars().count() > MAX_NAME) {
        return Err(format!("INVALID_ARG: at most {MAX_AGENTS} agents, each name at most {MAX_NAME} characters"));
    }
    tracing::info!(drone_id = %cmd.drone_id, by = %by, agents = ?agents, "setdroneagents");
    state.id_store.access_set_agents(&cmd.drone_id, &agents, &by, now_ms()).map_err(|e| format!("setdroneagents: {e}"))?;
    state.broker.publish_event::<crate::backend::mps_events::DronesChanged>(&(), vec![], 0);
    state.id_store.access_get(&cmd.drone_id).map_err(|e| format!("setdroneagents: {e}"))
}

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let s = state.clone();
    engine.register_typed(COMMAND_GET_DRONE_ACCESS, move |cmd: GetDroneAccessReq, _ctx| {
        let s = s.clone();
        async move { get_drone_access(&s, cmd) }
    });
    let s = state.clone();
    engine.register_typed(COMMAND_SET_DRONE_AGENTS, move |cmd: SetDroneAgentsReq, ctx| {
        let s = s.clone();
        async move { set_drone_agents(&s, &ctx, cmd) }
    });
}

#[cfg(test)]
#[path = "drone_access_handlers_tests.rs"]
mod tests;
