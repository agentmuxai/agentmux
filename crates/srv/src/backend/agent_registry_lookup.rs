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
/// slug as `agent_id` — **only if it names one agent**.
///
/// One record matching is one agent. Several records that all name one
/// non-empty `definition_id` are also one agent, recorded more than once (the
/// move to definition-id keys left the old record beside the new): the
/// preferred record answers (see [`preferred`]).
///
/// `None` when the registry is unavailable, unreadable, nothing matches, or
/// **records of different definitions match**. Callers treat the first three
/// the same way (fall back to another source), so they are not distinguished.
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
    // Records that all name one definition are one agent recorded more than once (an upgrade across the move to
    // definition-id keys left the old record beside the new): not a collision, and the live one answers.
    if matches.len() > 1 && same_definition(&matches) {
        return Some(preferred(matches));
    }
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
    let of_definition: Vec<_> = active_records_matching_slug(agent_id)?.into_iter().filter(|r| r.data.definition_id == definition_id).collect();
    (!of_definition.is_empty()).then(|| preferred(of_definition))
}

/// Whether every record names one non-empty definition: they are one agent, recorded more than once.
fn same_definition(records: &[NamedAgentRecord]) -> bool {
    let first = &records[0].data.definition_id;
    !first.is_empty() && records.iter().all(|r| &r.data.definition_id == first)
}

/// The record to go by among several for one agent: the one whose instance holds a live lease, else the one launched
/// last, else the newest made, else the lowest instance id (so the answer is the same on every call). `records` is not
/// empty. One record is the answer as it stands: the lease store is opened (its folder made, its lock taken) only to
/// choose between several, so the lookups that only read, history and memory among them, write nothing in the
/// normal case.
fn preferred(mut records: Vec<NamedAgentRecord>) -> NamedAgentRecord {
    if records.len() == 1 {
        return records.remove(0);
    }
    let leases = crate::registry::resolve_shared_registry_dir().and_then(|dir| crate::registry::LeaseStore::open(&dir).ok());
    // Read once per record: each read takes the instance's lease lock.
    let live = |r: &NamedAgentRecord| {
        leases.as_ref().is_some_and(|l| l.live_holder_other_than(&r.data.instance_id, "").ok().flatten().is_some())
    };
    records
        .into_iter()
        .map(|r| (live(&r), r))
        .max_by(|(live_a, a), (live_b, b)| {
            live_a
                .cmp(live_b)
                .then(a.data.last_launched_at_ms.cmp(&b.data.last_launched_at_ms))
                .then(a.data.created_at_ms.cmp(&b.data.created_at_ms))
                .then(b.data.instance_id.cmp(&a.data.instance_id))
        })
        .map(|(_, r)| r)
        .expect("records is not empty")
}

/// Reconstruct an agent's absolute working directory from its registry record.
///
/// `source_agents_base` joined with the relative `working_dir`, with legacy
/// (v1/v2) records lacking a base falling back to the current channel's agents
/// dir, matching the registry's own pre-P0.4 reconstruction rule.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{NamedAgentRecordV1, Registry, MAX_SUPPORTED_SCHEMA};

    // These tests drive the real registry through `resolve_shared_registry_dir`,
    // which reads the process-global `AGENTMUX_HOME_OVERRIDE`. Left unset it
    // would resolve the developer's own `~/.agentmux` and merge in every real
    // agent on the machine. Uses the CRATE-WIDE lock every other consumer of
    // this env var already takes — a module-local mutex does not stop the
    // races these tests would otherwise have with the migration test modules.
    use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_GUARD;

    fn with_registry<T>(records: &[(&str, &str)], f: impl FnOnce() -> T) -> T {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", home.path().to_str().unwrap());

        let dir = crate::registry::resolve_shared_registry_dir().unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let registry = Registry::open(dir).unwrap();
        for (instance_id, instance_name) in records {
            registry.upsert(&record(instance_id, instance_name)).unwrap();
        }

        let out = f();
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        out
    }

    fn record(instance_id: &str, instance_name: &str) -> NamedAgentRecord {
        NamedAgentRecord {
            schema_version: MAX_SUPPORTED_SCHEMA,
            data: NamedAgentRecordV1 {
                instance_id: instance_id.to_string(),
                instance_name: instance_name.to_string(),
                definition_id: format!("def-{instance_id}"),
                identity_id: None,
                memory_id: None,
                session_id: None,
                working_dir: format!("{instance_id}-dir"),
                source_agents_base: Some("/agents".to_string()),
                created_at_ms: 1,
                last_launched_at_ms: 1,
                created_by_version: "test".to_string(),
                last_launched_by_version: "test".to_string(),
            },
        }
    }

    #[test]
    fn resolves_the_single_record_whose_name_normalizes_to_the_slug() {
        with_registry(&[("i1", "AgentY")], || {
            let found = find_active_record_by_slug("agenty").expect("one match resolves");
            assert_eq!(found.data.instance_id, "i1");
        });
    }

    // The module's whole reason for existing: the caller holds a routing slug
    // while the record stores a display-cased `instance_name`, so both sides
    // must go through `derive_slug`. Raw string equality here was a real bug
    // (reagentx, PR #2428).
    #[test]
    fn matches_through_derive_slug_on_both_sides_not_raw_equality() {
        with_registry(&[("i1", "AgentY")], || {
            assert!(find_active_record_by_slug("AgentY").is_some(), "display casing");
            assert!(find_active_record_by_slug("AGENTY").is_some(), "upper casing");
            assert!(find_active_record_by_slug("agenty").is_some(), "already a slug");
        });
    }

    #[test]
    fn returns_none_when_nothing_matches() {
        with_registry(&[("i1", "AgentY")], || {
            assert!(find_active_record_by_slug("someone-else").is_none());
        });
    }

    // The guard PR #3480 added and #3497 §2.5.3 flagged as never verified.
    // `derive_slug` lowercases, so "AgentY" and "AGENTY" are two agents with
    // one slug — nothing enforces a unique `instance_name` across records.
    // Returning either one hands its `definition_id`/`working_dir` to a caller
    // resolving conversation history or a memory directory, so the wrong pick
    // discloses one agent's data to another.
    #[test]
    fn refuses_to_resolve_a_slug_two_active_records_share() {
        with_registry(&[("i1", "AgentY"), ("i2", "AGENTY")], || {
            assert!(
                find_active_record_by_slug("agenty").is_none(),
                "an ambiguous slug must resolve to nothing, never to an arbitrary winner"
            );
        });
    }

    // The disambiguated sibling stays correct exactly where the slug-only
    // lookup must give up — otherwise refusing the collision would leave
    // callers that DO know which agent they mean with no way through.
    #[test]
    fn definition_scoped_lookup_resolves_a_collision_the_slug_alone_cannot() {
        with_registry(&[("i1", "AgentY"), ("i2", "AGENTY")], || {
            let found = find_active_record_by_slug_and_definition("agenty", "def-i2")
                .expect("definition_id disambiguates");
            assert_eq!(found.data.instance_id, "i2");
            assert!(
                find_active_record_by_slug_and_definition("agenty", "def-nope").is_none(),
                "an unmatched definition must not fall back to a slug-only guess"
            );
        });
    }
    /// Two records of one definition (an agent recorded twice across the move to definition-id keys). `launched` is
    /// each record's `last_launched_at_ms`.
    fn with_pair<T>(launched: [i64; 2], f: impl FnOnce(&Registry) -> T) -> T {
        with_registry(&[], || {
            let registry = Registry::open(crate::registry::resolve_shared_registry_dir().unwrap()).unwrap();
            for (i, at) in launched.into_iter().enumerate() {
                let mut r = record(&format!("i{i}"), "Manpo");
                r.data.definition_id = "def-manpo".to_string();
                r.data.last_launched_at_ms = at;
                registry.upsert(&r).unwrap();
            }
            f(&registry)
        })
    }

    // The upgrade to 0.60.22 logged "registry slug collision ... matches 2" for an agent that was one: its old record
    // and its new one named the same definition.
    #[test]
    fn two_records_of_one_definition_are_one_agent_and_the_one_launched_last_answers() {
        with_pair([100, 200], |_| {
            let found = find_active_record_by_slug("manpo").expect("one agent, not a collision");
            assert_eq!(found.data.instance_id, "i1");
            let scoped = find_active_record_by_slug_and_definition("manpo", "def-manpo").unwrap();
            assert_eq!(scoped.data.instance_id, "i1", "the definition-scoped lookup picks the same record");
        });
        with_pair([200, 100], |_| {
            assert_eq!(find_active_record_by_slug("manpo").unwrap().data.instance_id, "i0");
        });
        with_pair([100, 100], |_| {
            assert_eq!(find_active_record_by_slug("manpo").unwrap().data.instance_id, "i0", "a tie is decided the same way every time");
        });
    }

    // A lookup that finds one record only reads: opening the lease store makes its folder and takes a lock file in the
    // registry shared by every instance, on every history and memory lookup (#46577).
    #[test]
    fn one_matching_record_is_returned_without_opening_the_lease_store() {
        with_registry(&[("i1", "AgentY")], || {
            let leases = crate::registry::resolve_shared_registry_dir().unwrap().join("leases");
            assert!(!leases.exists(), "nothing has opened the lease store yet");
            assert_eq!(find_active_record_by_slug("agenty").unwrap().data.instance_id, "i1");
            assert_eq!(find_active_record_by_slug_and_definition("agenty", "def-i1").unwrap().data.instance_id, "i1");
            assert!(!leases.exists(), "a single match leaves no lease folder or lock file behind");
        });
    }

    #[test]
    fn the_record_whose_instance_holds_a_live_lease_answers_before_the_one_launched_last() {
        with_pair([100, 200], |registry| {
            let leases = crate::registry::LeaseStore::open(registry.root()).unwrap();
            let boot: std::sync::Arc<str> = std::sync::Arc::from("boot-live");
            let _held = leases.claim("i0", &boot, "block-1", None).unwrap();
            assert_eq!(find_active_record_by_slug("manpo").unwrap().data.instance_id, "i0", "the live one, though the other was launched later");
        });
    }

    // Different definitions are different agents: collapsing them would hand one agent's history to another.
    #[test]
    fn records_of_different_definitions_are_still_a_collision() {
        with_registry(&[("i1", "Manpo"), ("i2", "MANPO")], || {
            assert!(find_active_record_by_slug("manpo").is_none());
        });
    }
}
