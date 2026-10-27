// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents set_seat` (`POST /api/v1/agent/seats`): a host agent sets or clears
//! the seats of agents on this machine, several in one call (a whole roster).
//! A seat is an agent's role, who it reports to and what it owns
//! (`backend::agent_seat`); the operator's window sets one with `agent.setSeat`
//! (`agent_handlers/seat.rs`), and an agent does it here.
//!
//! The caller is the agent its token names; a request without one is refused,
//! so a change is never attributed to the operator. The checks are the
//! window's (roles, lengths, leads that are agents here, no cycle), applied to
//! the whole call at once: it is written all together or not at all. Each
//! change, and each refused attempt, is recorded in the audit log under the
//! calling agent's slug and in the `identity` log with its UID.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::backend::rpc_types::SeatInput;
use crate::server::agent_handlers::seat::{plan_seats, write_seats, PlannedSeat, SeatActor, SeatChange};
use crate::server::caller::Caller;
use crate::server::state::AppState;

/// The audit entry's action for a change.
pub(crate) const AUDIT_ACTION: &str = "agent.seat.set";
/// The most seats one call changes: a roster, not a sweep.
pub(crate) const MAX_SEATS_PER_CALL: usize = 64;

#[derive(Debug, Deserialize)]
pub(crate) struct SetSeatsRequest {
    seats: Vec<SeatEntry>,
}

#[derive(Debug, Deserialize)]
struct SeatEntry {
    /// The agent's id or slug.
    agent: String,
    /// The seat to set; `null` or absent clears the agent's seat.
    #[serde(default)]
    seat: Option<SeatInput>,
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
            "no agent token: Agents set_seat acts as the calling agent; the operator sets a seat in My Agents",
        ));
    };
    let def = state
        .mstore
        .agent_def_get(uid)
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("agent seats: {e}")))?
        .ok_or_else(|| refuse(StatusCode::UNAUTHORIZED, "the agent token names no agent on this machine"))?;
    if def.agent_type == "container" {
        return Err(refuse(StatusCode::FORBIDDEN, "CONTAINER_AGENT: a container agent can't set seats"));
    }
    Ok(Actor { uid: uid.to_string(), slug: def.slug, name: def.name })
}

/// The uid of the agent `named` (an id or a slug) on this machine.
fn uid_of(state: &AppState, named: &str, what: &str) -> Result<String, Refusal> {
    let named = named.trim();
    crate::backend::agent_resolve::resolve_agent_id(&state.mstore, named)
        .ok()
        .filter(|id| matches!(state.mstore.agent_def_get(id), Ok(Some(_))))
        .ok_or_else(|| refuse(StatusCode::NOT_FOUND, format!("no agent '{named}' on this machine ({what}); seats are set only for this machine's agents")))
}

/// The changes `req` asks for, with every agent and lead named by id.
fn changes_of(state: &AppState, req: SetSeatsRequest) -> Result<Vec<SeatChange>, Refusal> {
    if req.seats.is_empty() {
        return Err(refuse(StatusCode::BAD_REQUEST, "INVALID_ARG: seats is empty; give [{agent, seat}], seat null to clear"));
    }
    if req.seats.len() > MAX_SEATS_PER_CALL {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            format!("INVALID_ARG: at most {MAX_SEATS_PER_CALL} seats in one call (got {})", req.seats.len()),
        ));
    }
    let mut changes: Vec<SeatChange> = Vec::with_capacity(req.seats.len());
    for entry in req.seats {
        let agent_id = uid_of(state, &entry.agent, "agent")?;
        if changes.iter().any(|c| c.agent_id == agent_id) {
            return Err(refuse(StatusCode::BAD_REQUEST, format!("INVALID_ARG: '{}' appears twice in seats", entry.agent.trim())));
        }
        let mut seat = entry.seat;
        if let Some(input) = seat.as_mut() {
            // A lead may be named by slug, like the agent itself.
            if let Some(lead) = input.lead.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
                input.lead = Some(uid_of(state, lead, "lead")?);
            }
        }
        changes.push(SeatChange { agent_id, seat });
    }
    Ok(changes)
}

fn audit(state: &AppState, actor: &Actor, target_slug: &str, ok: bool, error: Option<&str>, note: &str) -> String {
    let request_id = uuid::Uuid::new_v4().to_string();
    let block_id = state.reactive_handler.block_for_uid(&actor.uid).unwrap_or_default();
    let reason = format!("{} (uid {}): {note}", actor.slug, actor.uid);
    state.reactive_handler.log_fleet_action_audit(
        Some(&actor.slug),
        target_slug,
        &block_id,
        AUDIT_ACTION,
        ok,
        error,
        &request_id,
        Some(&reason),
    );
    request_id
}

/// What a change did, for the audit line and the answer.
fn summary_of(p: &PlannedSeat) -> String {
    match &p.seat {
        Some(s) => format!(
            "set the seat of {} ({}) to {}{}{}",
            p.name,
            p.agent_id,
            s.role,
            s.qualifier.as_deref().map(|q| format!(" / {q}")).unwrap_or_default(),
            s.lead.as_deref().map(|l| format!(", reports to {l}")).unwrap_or_else(|| ", reports to the operator".to_string()),
        ),
        None => format!("cleared the seat of {} ({})", p.name, p.agent_id),
    }
}

/// Plan, then write all of `changes` under the seat lock.
fn apply(state: &AppState, actor: &Actor, changes: Vec<SeatChange>) -> Result<Vec<PlannedSeat>, Refusal> {
    let bad = |e: String| refuse(if e.contains("not found") { StatusCode::NOT_FOUND } else { StatusCode::BAD_REQUEST }, e);
    let mstore = &state.mstore;
    // Held from reading every seat to writing the last, across channels and
    // instances, so two writes can't each pass the cycle check (#5378).
    let _seats = mstore
        .agent_seat_lock()
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("agent seats: {e}")))?;
    let defs = mstore
        .agent_def_list()
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("agent seats: {e}")))?;
    let planned = plan_seats(mstore, &defs, &changes, "").map_err(bad)?;
    write_seats(mstore, &planned, &SeatActor::Agent { uid: &actor.uid, name: &actor.name })
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(planned)
}

async fn run(state: &AppState, caller: Option<&Caller>, req: SetSeatsRequest) -> Result<Value, Refusal> {
    let actor = actor_of(state, caller)?;
    let outcome = {
        let (s, a) = (state.clone(), Actor { uid: actor.uid.clone(), slug: actor.slug.clone(), name: actor.name.clone() });
        tokio::task::spawn_blocking(move || changes_of(&s, req).and_then(|changes| apply(&s, &a, changes)))
            .await
            .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("agent seats: {e}")))?
    };
    let planned = match outcome {
        Ok(planned) => planned,
        Err(refusal) => {
            audit(state, &actor, &actor.slug, false, Some(&refusal.message), "refused");
            return Err(refusal);
        }
    };
    let audit_ids: Vec<String> = planned.iter().map(|p| audit(state, &actor, &p.slug, true, None, &summary_of(p))).collect();
    state.broker.publish_event::<crate::backend::mps_events::AgentsChanged>(&(), vec![], 0);
    crate::server::agent_handlers::seat::refresh_pane_seats(state).await;
    let changed: Vec<Value> = planned
        .iter()
        .map(|p| json!({ "agent": p.name, "id": p.agent_id, "slug": p.slug, "seat": p.seat }))
        .collect();
    Ok(json!({
        "changed": changed,
        "audit_ids": audit_ids,
        "note": "recorded under your name; My Agents, the Swarm and the Kanban show the new seats now",
    }))
}

pub(crate) async fn handle_agent_seats(
    State(state): State<AppState>,
    caller: Option<Extension<Caller>>,
    Json(req): Json<SetSeatsRequest>,
) -> Response {
    let caller = caller.map(|Extension(c)| c);
    match run(&state, caller.as_ref(), req).await {
        Ok(body) => Json(body).into_response(),
        Err(r) => (r.status, Json(json!({ "error": r.message }))).into_response(),
    }
}

#[cfg(test)]
#[path = "tests/agent_seats_tests.rs"]
mod tests;
