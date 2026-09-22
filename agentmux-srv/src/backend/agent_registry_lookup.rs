// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Slug-based lookup into the global named-agent registry.
//!
//! **One implementation, deliberately.** This matching is subtle — the caller
//! holds a routing slug (`AGENTMUX_AGENT_ID`, e.g. `"agenty"`) while a record
//! stores the display-cased `instance_name` (e.g. `"AgentY"`), so both sides
//! have to be normalized through [`derive_slug`] rather than compared
//! directly. Raw string equality here was a real bug (reagentx, PR #2428).
//!
//! It lived in `server::native_memory_handlers` and was then re-implemented in
//! `backend::history`; reagentx P2 on PR #3480 flagged the copy, citing this
//! codebase's own history of exactly that drift — *"a future collision/matching
//! fix applied to one copy will silently miss the other."*
//!
//! It lives in `backend/` rather than `registry/` on purpose: `registry/` has
//! no dependency on `crate::backend` at all today, and [`derive_slug`] is a
//! `backend::storage` function — homing this there would have added a
//! registry→backend edge to a module that is currently self-contained.

use crate::backend::storage::store::derive_slug;
use crate::registry::NamedAgentRecord;

/// Every active registry record whose `instance_name` normalizes to the same
/// slug as `agent_id`. Usually zero or one; more than one means a collision.
fn active_records_matching_slug(agent_id: &str) -> Option<Vec<NamedAgentRecord>> {
    let registry_dir = crate::registry::resolve_shared_registry_dir()?;
    let registry = crate::registry::Registry::open(registry_dir).ok()?;
    let queried_slug = derive_slug(agent_id);
    Some(
        registry
            .list_active()
            .ok()?
            .into_iter()
            .filter(|r| derive_slug(&r.data.instance_name) == queried_slug)
            .collect(),
    )
}

/// The active registry record whose `instance_name` normalizes to the same
/// slug as `agent_id` — **only if exactly one does**.
///
/// `None` when the registry is unavailable, unreadable, nothing matches, or
/// **more than one record matches**. Callers treat the first three the same
/// way (fall back to another source), so they are not distinguished.
///
/// **Why ambiguity returns `None` rather than the first match** (reagentx P1,
/// PR #3480): registry records are keyed by `instance_id`, and nothing
/// enforces a unique `instance_name` across them — `derive_slug("AgentY")` and
/// `derive_slug("AGENTY")` are both `agenty`, which two agents on two hosts
/// can easily produce. This function used to return whichever matched first in
/// `list_active()` order, handing that record's `definition_id` and
/// `working_dir` to the caller. Callers use those to resolve an agent's
/// *conversation history* and *memory directory*, so guessing wrong does not
/// degrade gracefully — it discloses one agent's data to another. A slug
/// simply does not identify an agent when it collides, so the honest answer
/// is "unknown". Same collision class as Codex P1 on #2901 and reagentx on
/// #2428.
///
/// If you hold a `definition_id`, prefer
/// [`find_active_record_by_slug_and_definition`] — it resolves the collision
/// instead of refusing it.
pub(crate) fn find_active_record_by_slug(agent_id: &str) -> Option<NamedAgentRecord> {
    let mut matches = active_records_matching_slug(agent_id)?;
    match matches.len() {
        1 => matches.pop(),
        0 => None,
        n => {
            tracing::warn!(
                slug = %derive_slug(agent_id),
                matches = n,
                "registry slug collision: refusing to resolve an agent by slug alone"
            );
            None
        }
    }
}

/// The active registry record matching both `agent_id`'s slug and an exact
/// `definition_id`.
///
/// For callers that already know which agent they mean. Unlike
/// [`find_active_record_by_slug`] this stays correct under a slug collision,
/// because `definition_id` is the disambiguator the slug lacks — so it returns
/// a record in cases where the slug-only lookup must give up.
pub(crate) fn find_active_record_by_slug_and_definition(
    agent_id: &str,
    definition_id: &str,
) -> Option<NamedAgentRecord> {
    active_records_matching_slug(agent_id)?
        .into_iter()
        .find(|r| r.data.definition_id == definition_id)
}

/// Reconstruct an agent's absolute working directory from its registry record.
///
/// `source_agents_base` joined with the relative `working_dir`, with legacy
/// (v1/v2) records lacking a base falling back to the current channel's agents
/// dir — the same reconstruction rule
/// `native_memory_handlers::memory_dir_for_registry_record` applies.
pub(crate) fn working_dir_from_record(rec: &NamedAgentRecord) -> Option<String> {
    let base = rec
        .data
        .source_agents_base
        .clone()
        .or_else(|| std::env::var("AGENTMUX_AGENTS_DIR").ok())?;
    Some(
        std::path::Path::new(&base)
            .join(&rec.data.working_dir)
            .to_string_lossy()
            .to_string(),
    )
}
