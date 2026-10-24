// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agent.setSeat`: the operator sets or clears an agent's seat
//! (`SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md` §3). Agents read seats
//! (`listagents`, MuxAPI's `Agents list`) but never write one: a connection
//! registered as an agent is refused here, and MuxAPI has no HTTP route for it.

use std::sync::Arc;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{AgentSetSeatResult, CommandAgentSetSeatData, RpcContext, COMMAND_AGENT_SET_SEAT};
use crate::backend::storage::store::Store;

use super::super::AppState;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let broker = state.broker.clone();
    engine.register_typed(COMMAND_AGENT_SET_SEAT, move |cmd: CommandAgentSetSeatData, ctx| {
        let mstore = mstore.clone();
        let broker = broker.clone();
        async move {
            operator_only(&ctx)?;
            let result = tokio::task::spawn_blocking(move || set_seat(&mstore, cmd))
                .await
                .map_err(|e| format!("agent.setSeat: {e}"))??;
            broker.publish_event::<crate::backend::mps_events::AgentsChanged>(&(), vec![], 0);
            Ok(result)
        }
    });
}

/// Refuse a connection registered as an agent (`bus:register`), the same
/// rule as Tower's and the browser profiles' RPCs: only the operator's own
/// client sets a seat, an agent's own included.
fn operator_only(ctx: &RpcContext) -> Result<(), String> {
    if ctx.agent_id.is_empty() {
        Ok(())
    } else {
        Err("FORBIDDEN: only the operator sets an agent's seat".to_string())
    }
}

/// Validate and store (or clear) the seat, and log it as an identity change.
pub(super) fn set_seat(mstore: &Store, cmd: CommandAgentSetSeatData) -> Result<AgentSetSeatResult, String> {
    let agent_id = cmd.agent_id.trim().to_string();
    // Held from reading every seat to writing this one, across channels and
    // instances, so two writes can't each pass the cycle check (#5378).
    let _seats = mstore.agent_seat_lock().map_err(|e| format!("agent.setSeat: {e}"))?;
    let defs = mstore.agent_def_list().map_err(|e| format!("agent.setSeat: {e}"))?;
    let Some(def) = defs.iter().find(|d| d.id == agent_id) else {
        return Err(format!("agent.setSeat: agent {agent_id} not found"));
    };
    if def.is_seeded != 0 {
        return Err(format!("agent.setSeat: {} is a template; only an agent has a seat", def.name));
    }
    let lead_of = |id: &str| {
        defs.iter()
            .find(|d| d.id == id)
            .and_then(|d| crate::backend::agent_seat::effective_seat(mstore, d))
            .and_then(|s| s.lead)
    };
    let seat = match &cmd.seat {
        None => {
            // A spawned worker's leader becomes its lead again (#5378).
            if let Some(leader) = crate::server::app_api::agent_spawn::worker_of(mstore, &agent_id) {
                crate::backend::agent_seat::no_cycle(&agent_id, &leader, lead_of)?;
            }
            None
        }
        Some(input) => Some(crate::backend::agent_seat::validate(
            &agent_id,
            input,
            agentmux_common::time::now_ms(),
            |id| defs.iter().any(|d| d.id == id && d.is_seeded == 0),
            lead_of,
        )?),
    };
    mstore
        .agent_seat_set(&agent_id, seat.as_ref())
        .map_err(|e| format!("agent.setSeat: {e}"))?;
    match &seat {
        Some(s) => tracing::info!(
            target: "identity",
            agent_id = %agent_id,
            agent = %def.name,
            role = %s.role,
            lead = %s.lead.as_deref().unwrap_or("the operator"),
            "agent.setSeat: the operator set the seat"
        ),
        None => tracing::info!(
            target: "identity",
            agent_id = %agent_id,
            agent = %def.name,
            "agent.setSeat: the operator cleared the seat"
        ),
    }
    Ok(AgentSetSeatResult { seat })
}

#[cfg(test)]
#[path = "seat_tests.rs"]
mod tests;
