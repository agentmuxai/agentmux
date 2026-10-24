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
    let state = state.clone();
    engine.register_typed(COMMAND_AGENT_SET_SEAT, move |cmd: CommandAgentSetSeatData, ctx| {
        let state = state.clone();
        async move {
            operator_only(&ctx)?;
            let mstore = state.mstore.clone();
            let result = tokio::task::spawn_blocking(move || set_seat(&mstore, cmd))
                .await
                .map_err(|e| format!("agent.setSeat: {e}"))??;
            state.broker.publish_event::<crate::backend::mps_events::AgentsChanged>(&(), vec![], 0);
            refresh_pane_seats(&state).await;
            Ok(result)
        }
    });
}

/// Bring the `agent:seat` of every open agent pane up to date, so the pane
/// header's tooltip shows the seat just set (§4). Best-effort: a pane that
/// can't be patched shows the old seat until it's opened again.
async fn refresh_pane_seats(state: &AppState) {
    use crate::backend::agent_seat::{stale_seat_meta, summary_by_id, SEAT_META_KEY};
    let mstore = state.mstore.clone();
    let stale = tokio::task::spawn_blocking(move || {
        let blocks = mstore.get_all::<crate::backend::obj::Block>().unwrap_or_default();
        stale_seat_meta(&blocks, |id| summary_by_id(&mstore, id))
    })
    .await
    .unwrap_or_default();
    for (block_id, value) in stale {
        let patch = serde_json::json!({ SEAT_META_KEY: value });
        let events = crate::server::service::dispatch_to_reducer(
            state,
            agentmux_common::ipc::Command::UpdateBlockMeta { block_id, meta_patch: patch },
        )
        .await;
        for ev in &events {
            if let Err(e) = crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore) {
                tracing::warn!("agent.setSeat: pane seat refresh failed: {e}");
            }
        }
        crate::server::service::publish_events(state, &events);
    }
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
