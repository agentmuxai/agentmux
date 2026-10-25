// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agent.setSeat`: the operator sets or clears an agent's seat
//! (`SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md` §3). A connection registered
//! as an agent is refused here; an agent sets seats through MuxAPI's `Agents
//! set_seat` (`app_api/agent_seats.rs`), which shares the checks below
//! ([`plan_seats`]) and records the agent as the one who set them.

use std::sync::Arc;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{AgentSetSeatResult, CommandAgentSetSeatData, RpcContext, SeatInput, COMMAND_AGENT_SET_SEAT};
use crate::backend::storage::store::Store;
use crate::backend::storage::AgentDefinition;
use crate::registry::AgentSeat;

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
/// header's tooltip shows the seat just set, or a lead's new name after a
/// rename (§4). Best-effort: a pane that
/// can't be patched shows the old seat until it's opened again.
pub(crate) async fn refresh_pane_seats(state: &AppState) {
    use crate::backend::agent_seat::{stale_seat_meta, summary_in, SEAT_META_KEY};
    let mstore = state.mstore.clone();
    let stale = tokio::task::spawn_blocking(move || {
        let blocks = mstore.get_all::<crate::backend::obj::Block>().unwrap_or_default();
        // Listed once: the overlaid definitions name each lead (#5382).
        let defs = mstore.agent_def_list().unwrap_or_default();
        stale_seat_meta(&blocks, |id| summary_in(&mstore, &defs, id))
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
    ctx.operator_only("only the operator sets an agent's seat")
}

/// One seat to set (`Some`) or clear (`None`) on the agent whose uid is `agent_id`.
pub(crate) struct SeatChange {
    pub agent_id: String,
    pub seat: Option<SeatInput>,
}

/// A seat change that passed every check, ready to write.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedSeat {
    pub agent_id: String,
    pub name: String,
    pub slug: String,
    pub seat: Option<AgentSeat>,
}

/// Who made a seat change, for the log: the operator's window, or an agent.
pub(crate) enum SeatActor<'a> {
    Operator,
    Agent { uid: &'a str, name: &'a str },
}

/// Check every change as it will stand once all of them are written: a lead
/// may be an agent set in the same call, and a cycle through two changes of one
/// call is a cycle. Nothing is written. `refuse` starts the message of the
/// two "no such agent" refusals, so each caller names itself.
pub(crate) fn plan_seats(
    mstore: &Store,
    defs: &[AgentDefinition],
    changes: &[SeatChange],
    refuse: &str,
) -> Result<Vec<PlannedSeat>, String> {
    use std::collections::HashMap;
    let now = agentmux_common::time::now_ms();
    // The lead each agent will have once the changes before this one are written.
    let mut pending: HashMap<String, Option<String>> = HashMap::new();
    let mut planned = Vec::with_capacity(changes.len());
    for change in changes {
        let agent_id = change.agent_id.trim().to_string();
        let Some(def) = defs.iter().find(|d| d.id == agent_id) else {
            return Err(format!("{refuse}agent {agent_id} not found"));
        };
        if def.is_seeded != 0 {
            return Err(format!("{refuse}{} is a template; only an agent has a seat", def.name));
        }
        let lead_of = |id: &str| match pending.get(id) {
            Some(lead) => lead.clone(),
            None => defs
                .iter()
                .find(|d| d.id == id)
                .and_then(|d| crate::backend::agent_seat::effective_seat(mstore, d))
                .and_then(|s| s.lead),
        };
        let (seat, lead_after) = match &change.seat {
            None => {
                // A spawned worker's leader becomes its lead again (#5378).
                let leader = crate::server::app_api::agent_spawn::worker_of(mstore, &agent_id);
                if let Some(leader) = &leader {
                    crate::backend::agent_seat::no_cycle(&agent_id, leader, lead_of)?;
                }
                (None, leader)
            }
            Some(input) => {
                let seat = crate::backend::agent_seat::validate(
                    &agent_id,
                    input,
                    now,
                    |id| defs.iter().any(|d| d.id == id && d.is_seeded == 0),
                    lead_of,
                )?;
                let lead = seat.lead.clone();
                (Some(seat), lead)
            }
        };
        pending.insert(agent_id.clone(), lead_after);
        planned.push(PlannedSeat { agent_id, name: def.name.clone(), slug: def.slug.clone(), seat });
    }
    Ok(planned)
}

/// Write planned seats and log each as an identity change by `actor`. The
/// caller holds the seat lock from the check to here.
pub(crate) fn write_seats(mstore: &Store, planned: &[PlannedSeat], actor: &SeatActor<'_>) -> Result<(), String> {
    for p in planned {
        mstore.agent_seat_set(&p.agent_id, p.seat.as_ref()).map_err(|e| format!("agent.setSeat: {e}"))?;
        let (by, by_uid) = match actor {
            SeatActor::Operator => ("the operator", ""),
            SeatActor::Agent { uid, name } => (*name, *uid),
        };
        match &p.seat {
            Some(s) => tracing::info!(
                target: "identity",
                agent_id = %p.agent_id,
                agent = %p.name,
                role = %s.role,
                lead = %s.lead.as_deref().unwrap_or("the operator"),
                by_agent_id = %by_uid,
                "agent.setSeat: {by} set the seat"
            ),
            None => tracing::info!(
                target: "identity",
                agent_id = %p.agent_id,
                agent = %p.name,
                by_agent_id = %by_uid,
                "agent.setSeat: {by} cleared the seat"
            ),
        }
    }
    Ok(())
}

/// Validate and store (or clear) the seat, and log it as an identity change.
pub(super) fn set_seat(mstore: &Store, cmd: CommandAgentSetSeatData) -> Result<AgentSetSeatResult, String> {
    // Held from reading every seat to writing this one, across channels and
    // instances, so two writes can't each pass the cycle check (#5378).
    let _seats = mstore.agent_seat_lock().map_err(|e| format!("agent.setSeat: {e}"))?;
    let defs = mstore.agent_def_list().map_err(|e| format!("agent.setSeat: {e}"))?;
    let change = SeatChange { agent_id: cmd.agent_id, seat: cmd.seat };
    let planned = plan_seats(mstore, &defs, std::slice::from_ref(&change), "agent.setSeat: ")?;
    write_seats(mstore, &planned, &SeatActor::Operator)?;
    Ok(AgentSetSeatResult { seat: planned.into_iter().next().and_then(|p| p.seat) })
}

#[cfg(test)]
#[path = "seat_tests.rs"]
mod tests;
