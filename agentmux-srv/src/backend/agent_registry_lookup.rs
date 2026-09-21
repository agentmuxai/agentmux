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

/// The active registry record whose `instance_name` normalizes to the same
/// slug as `agent_id`, if any.
///
/// `None` when the registry is unavailable, unreadable, or nothing matches —
/// callers treat all three the same way (fall back to another source), so they
/// are not distinguished.
pub(crate) fn find_active_record_by_slug(agent_id: &str) -> Option<NamedAgentRecord> {
    let registry_dir = crate::registry::resolve_shared_registry_dir()?;
    let registry = crate::registry::Registry::open(registry_dir).ok()?;
    let queried_slug = derive_slug(agent_id);
    registry
        .list_active()
        .ok()?
        .into_iter()
        .find(|r| derive_slug(&r.data.instance_name) == queried_slug)
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
