// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Whose work claims an agent holds, and giving them back. The agent teardown (`sagas/agent_teardown.rs`) and
//! the controller watchdog both release an agent's claims, so the rule and the release live here in the core;
//! `server::agent_resources` and the teardown re-export them under their old paths.

use crate::backend::storage::work_queue::{work_state, WorkItem};

/// Same holder rule as the store's release (`HOLDER_BY_UID`): by UID when
/// both sides have one, else by name.
pub fn holds_claim(claimed_by: &str, claimed_by_uid: &str, agent_id: &str, uid: Option<&str>) -> bool {
    match uid.filter(|u| !u.is_empty()) {
        Some(u) if !claimed_by_uid.is_empty() => claimed_by_uid == u,
        _ => !agent_id.is_empty() && claimed_by == agent_id,
    }
}

/// The agent's claimed work items (R7).
pub fn claims_held_by<'a>(items: &'a [WorkItem], agent_id: &str, uid: Option<&str>) -> Vec<&'a WorkItem> {
    items
        .iter()
        .filter(|w| w.state == work_state::CLAIMED && holds_claim(&w.claimed_by, &w.claimed_by_uid, agent_id, uid))
        .collect()
}

/// Release the agent's claimed work items with `reason` (each counts as an
/// attempt), so they go back to the pool now rather than when the 120 s lease
/// expires. How many went. Blocking.
pub fn release_claims(work_store: &crate::backend::storage::store::Store, a: &str, u: Option<&str>, reason: &str, now: i64) -> usize {
    let items = work_store.work_queue_claimed_by(a, u.unwrap_or("")).unwrap_or_default();
    claims_held_by(&items, a, u)
        .into_iter()
        .filter(|w| matches!(work_store.work_queue_release(&w.id, &w.claimed_by, &w.claimed_by_uid, w.attempts, reason, now), Ok(Some(_))))
        .count()
}
