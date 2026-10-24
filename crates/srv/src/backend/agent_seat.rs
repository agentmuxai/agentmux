// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent seats: an agent's role, the agent it reports to (its lead) and what
//! it owns (its area), `SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md`.
//!
//! A seat is stored on the agent's shared definition record
//! (`DefinitionRecordV1::seat`), set only by the operator through
//! `agent.setSeat`. This module holds what reads and writes have in common:
//! the validation of a new seat (§2), the effective seat of a spawned worker
//! that has none of its own, and the summary shown in MuxAPI's `Agents list`,
//! a LAN peer's agent list and My Agents (§4, §5). Seats inform; nothing is
//! refused because of one.

use std::collections::HashSet;

use crate::backend::rpc_types::{SeatInput, SeatLead, SeatSummary};
use crate::backend::storage::store::Store;
use crate::backend::storage::AgentDefinition;
use crate::registry::AgentSeat;

/// The roles a seat may name, as stored.
pub const ROLES: [&str; 5] = ["leader", "worker", "specialist", "platform_tester", "analyst"];
pub const MAX_QUALIFIER_CHARS: usize = 40;
pub const MAX_AREA_CHARS: usize = 120;
/// The spec gives `lane` no limit; it's a short name, like the qualifier.
pub const MAX_LANE_CHARS: usize = 40;
/// A lead uid longer than this can't be one srv minted; it's cut, not kept,
/// when it arrives from a LAN peer.
const MAX_LEAD_ID_CHARS: usize = 128;
/// How far [`validate`] follows a chain of leads before calling it a cycle.
const MAX_CHAIN: usize = 256;
/// The qualifier of a spawned worker's effective seat.
pub const SPAWNED_QUALIFIER: &str = "spawned";

/// Whether `role` is one a seat may name.
pub fn is_role(role: &str) -> bool {
    ROLES.contains(&role)
}

/// `text` on one line: trimmed, every run of whitespace (newlines included)
/// one space, control characters dropped. `None` when nothing is left.
fn one_line(text: Option<&str>) -> Option<String> {
    let cleaned: String = text?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    (!cleaned.is_empty()).then_some(cleaned)
}

fn within(field: &str, value: Option<String>, max: usize) -> Result<Option<String>, String> {
    match value {
        Some(v) if v.chars().count() > max => Err(format!("INVALID_ARG: {field} is at most {max} characters (got {})", v.chars().count())),
        v => Ok(v),
    }
}

/// Validate the seat the operator asks for on agent `agent_id`, and stamp it
/// with `now_ms` (§2). `is_agent` says whether a uid is an agent on this
/// machine; `lead_of` gives an agent's current lead (its effective seat's),
/// which is how a cycle is found: following leads from the new lead must not
/// come back to `agent_id`.
///
/// Only an agent this machine has a definition for can be a lead. A uid the
/// LAN directory knows isn't accepted yet: LAN peers list agents by name, not
/// by uid, so there is nothing to check it against.
pub fn validate(
    agent_id: &str,
    input: &SeatInput,
    now_ms: i64,
    is_agent: impl Fn(&str) -> bool,
    lead_of: impl Fn(&str) -> Option<String>,
) -> Result<AgentSeat, String> {
    let role = input.role.trim();
    if !is_role(role) {
        return Err(format!("INVALID_ARG: unknown role '{role}'; one of: {}", ROLES.join(", ")));
    }
    let qualifier = within("qualifier", one_line(input.qualifier.as_deref()), MAX_QUALIFIER_CHARS)?;
    let area = within("area", one_line(input.area.as_deref()), MAX_AREA_CHARS)?;
    let lane = within("lane", one_line(input.lane.as_deref()), MAX_LANE_CHARS)?;
    let lead = input.lead.as_deref().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string);
    if let Some(lead) = &lead {
        if lead == agent_id {
            return Err("INVALID_ARG: an agent can't be its own lead".to_string());
        }
        if !is_agent(lead) {
            return Err(format!("INVALID_ARG: lead {lead} is not an agent on this machine"));
        }
        no_cycle(agent_id, lead, lead_of)?;
    }
    Ok(AgentSeat { role: role.to_string(), qualifier, lead, area, lane, set_at_ms: now_ms })
}

/// Refuse `lead` as `agent_id`'s lead when following leads from it comes
/// back to `agent_id`. Also checked when a spawned worker's seat is cleared:
/// its leader becomes its lead again.
pub fn no_cycle(agent_id: &str, lead: &str, lead_of: impl Fn(&str) -> Option<String>) -> Result<(), String> {
    let mut seen = HashSet::new();
    let mut next = Some(lead.to_string());
    while let Some(id) = next {
        if id == agent_id {
            return Err(format!("INVALID_ARG: {lead} already reports to this agent, so it can't be its lead (a cycle)"));
        }
        if !seen.insert(id.clone()) || seen.len() > MAX_CHAIN {
            // A cycle above this agent, already stored: not this seat's.
            break;
        }
        next = lead_of(&id);
    }
    Ok(())
}

/// The seat a definition is shown with: its stored seat, or, for a spawned
/// worker (`agent:worker_of`) with none, role worker, qualifier "spawned"
/// and its leader as lead (§2). Computed when read, never stored; a seat the
/// operator sets wins. `def.seat` must be the stored one, as
/// `agent_def_list` reads it.
pub fn effective_seat(mstore: &Store, def: &AgentDefinition) -> Option<AgentSeat> {
    if def.seat.is_some() {
        return def.seat.clone();
    }
    if def.is_seeded != 0 {
        return None;
    }
    spawned_seat(crate::server::app_api::agent_spawn::worker_of(mstore, &def.id))
}

/// The effective seat of a worker of `leader`, if any.
fn spawned_seat(leader: Option<String>) -> Option<AgentSeat> {
    leader.map(|leader| AgentSeat {
        role: "worker".to_string(),
        qualifier: Some(SPAWNED_QUALIFIER.to_string()),
        lead: Some(leader),
        ..Default::default()
    })
}

/// [`effective_seat`] by uid, for a reader that has no definition list: the
/// stored seat from the shared record, else the spawned one.
pub fn effective_seat_by_id(mstore: &Store, id: &str) -> Option<AgentSeat> {
    mstore
        .agent_seat_get(id)
        .or_else(|| spawned_seat(crate::server::app_api::agent_spawn::worker_of(mstore, id)))
}

/// Give every definition in `defs` (as `agent_def_list` returns them) its
/// effective seat.
pub fn apply_effective_seats(mstore: &Store, defs: &mut [AgentDefinition]) {
    for def in defs.iter_mut() {
        if def.seat.is_none() {
            def.seat = effective_seat(mstore, def);
        }
    }
}

/// A seat as shown, with its lead's name from `name_of` (§4).
pub fn summarize(seat: &AgentSeat, name_of: impl Fn(&str) -> Option<String>) -> SeatSummary {
    SeatSummary {
        role: seat.role.clone(),
        qualifier: seat.qualifier.clone(),
        lead: seat.lead.as_ref().map(|id| SeatLead { id: id.clone(), name: name_of(id) }),
        area: seat.area.clone(),
    }
}

/// The summary of the seat definition `def_id` is shown with (its effective
/// seat), its lead named from `defs`. `defs` is `agent_def_list`'s, so each
/// carries its stored seat. `None` when it has no seat.
pub fn summary_in(mstore: &Store, defs: &[AgentDefinition], def_id: &str) -> Option<SeatSummary> {
    let def = defs.iter().find(|d| d.id == def_id)?;
    let seat = effective_seat(mstore, def)?;
    Some(summarize(&seat, |id| defs.iter().find(|d| d.id == id).map(|d| d.name.clone())))
}

/// [`summary_in`] by uid, for a reader that has no definition list: the
/// lead is named by looking its definition up.
pub fn summary_by_id(mstore: &Store, id: &str) -> Option<SeatSummary> {
    let seat = effective_seat_by_id(mstore, id)?;
    Some(summarize(&seat, |lead| mstore.agent_def_get(lead).ok().flatten().map(|d| d.name)))
}

/// The block meta key an agent pane keeps its agent's seat summary under,
/// for the pane header's tooltip (§4). Seeded at `agent.open`, as `ui:color`
/// is, and refreshed by `agent.setSeat` ([`stale_seat_meta`]).
pub const SEAT_META_KEY: &str = "agent:seat";

/// `agent:seat` as a meta value: the summary, or `null` for none, which a
/// meta patch reads as "remove the key".
pub fn seat_meta(summary: Option<&SeatSummary>) -> serde_json::Value {
    summary.and_then(|s| serde_json::to_value(s).ok()).unwrap_or(serde_json::Value::Null)
}

/// Put agent `id`'s seat into the meta of a pane being opened for it, when
/// it has one.
pub fn seed_meta(mstore: &Store, id: &str, meta: &mut crate::backend::obj::MetaMapType) {
    if let Some(seat) = summary_by_id(mstore, id) {
        meta.insert(SEAT_META_KEY.to_string(), seat_meta(Some(&seat)));
    }
}

/// The agent panes among `blocks` whose `agent:seat` no longer matches the
/// seat `summary_of` gives their agent now, each with the value to patch in.
/// `summary_of` is asked once per agent.
pub fn stale_seat_meta(
    blocks: &[crate::backend::obj::Block],
    mut summary_of: impl FnMut(&str) -> Option<SeatSummary>,
) -> Vec<(String, serde_json::Value)> {
    let mut by_agent: std::collections::HashMap<String, serde_json::Value> = std::collections::HashMap::new();
    let mut stale = Vec::new();
    for block in blocks {
        if crate::backend::obj::meta_get_string(&block.meta, "view", "") != "agent" {
            continue;
        }
        let agent_id = crate::backend::obj::meta_get_string(&block.meta, "agentId", "");
        if agent_id.is_empty() {
            continue;
        }
        let want = by_agent
            .entry(agent_id.clone())
            .or_insert_with(|| seat_meta(summary_of(&agent_id).as_ref()))
            .clone();
        let have = block.meta.get(SEAT_META_KEY).cloned().unwrap_or(serde_json::Value::Null);
        if have != want {
            stale.push((block.oid.clone(), want));
        }
    }
    stale
}

/// A seat summary a LAN peer sent, made safe to show (§5): a known role
/// only, each text on one line and within its limit (cut, not refused: the
/// peer validated it, this only bounds it). `None` when it can't be read.
pub fn sanitize_peer_summary(value: &serde_json::Value, max_name_chars: usize) -> Option<SeatSummary> {
    let summary: SeatSummary = serde_json::from_value(value.clone()).ok()?;
    if !is_role(&summary.role) {
        return None;
    }
    let cut = |text: Option<&str>, max: usize| one_line(text).map(|t| t.chars().take(max).collect::<String>());
    Some(SeatSummary {
        role: summary.role,
        qualifier: cut(summary.qualifier.as_deref(), MAX_QUALIFIER_CHARS),
        lead: summary.lead.and_then(|l| {
            let id = cut(Some(&l.id), MAX_LEAD_ID_CHARS)?;
            Some(SeatLead { id, name: cut(l.name.as_deref(), max_name_chars) })
        }),
        area: cut(summary.area.as_deref(), MAX_AREA_CHARS),
    })
}

#[cfg(test)]
#[path = "agent_seat_tests.rs"]
mod tests;
