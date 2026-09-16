// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Startup reconciliation between the two host-global agent trees:
//! `shared/agents/registry/` (one record per named agent *instance*) and
//! `shared/agents/definitions/` (one record per agent *definition*).
//!
//! The picker's "My Agents" list is built from the instance registry
//! (`listrecentsessions` reads `Registry::list_active`), with each row's
//! display fields — name, provider, model vendor, agent type — resolved
//! from the definition it points at. An active instance record whose
//! definition has been tombstoned therefore renders as a row nothing can
//! fill in: no provider means no icon, and the name falls back to
//! `"(missing definition)"`. Clicking it can only fail. The user sees a
//! delete that half-worked — the icon vanished, the card stayed.
//!
//! Deletion now sweeps every registry record belonging to the agent
//! (`Registry::hard_delete_for_agent`), so no *new* orphan is created.
//! This pass exists for the ones already on disk, from every build that
//! deleted an agent before that fix: the two trees are host-global and
//! shared across channels, so those records outlive any one install and
//! nothing else would ever remove them.
//!
//! Deliberately narrow: a record is only dropped when its definition is
//! **provably deleted** — absent from the active definition tree AND
//! present in that tree's `retired/` tombstones. "Not in the global
//! definition store" alone is not enough, because a definition can
//! legitimately live only in some channel's local SQLite (e.g. it was
//! created while the global store couldn't be opened), and dropping those
//! would delete real agents rather than ghosts.

use crate::registry::{DefinitionStore, Registry};

/// Drop active instance records whose definition is tombstoned. Returns the
/// number of files removed. Best-effort: a tree that can't be read is
/// logged and skipped, never fatal.
pub fn prune_tombstoned_instance_records(reg: &Registry, defs: &DefinitionStore) -> usize {
    let records = match reg.list_active() {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "registry reconcile: list_active failed — skipping prune");
            return 0;
        }
    };
    // One pass per definition, not per record: several records can share a
    // definition (a legacy launch-keyed file alongside the agent-keyed one),
    // and `hard_delete_for_agent` already removes all of them at once.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut removed = 0usize;
    for rec in records {
        let def_id = rec.data.definition_id;
        if def_id.is_empty() || !seen.insert(def_id.clone()) {
            continue;
        }
        // Active wins: a definition that was retired and later unretired is
        // live again, and its instance records must stay.
        if defs.exists(&def_id) || !defs.exists_anywhere(&def_id) {
            continue;
        }
        match reg.hard_delete_for_agent(&def_id) {
            Ok(n) => removed += n,
            Err(e) => tracing::warn!(
                definition_id = %def_id,
                error = %e,
                "registry reconcile: failed to drop records for a tombstoned definition"
            ),
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{
        DefinitionRecord, DefinitionRecordV1, NamedAgentRecord, NamedAgentRecordV1,
        DEF_MAX_SUPPORTED_SCHEMA, MAX_SUPPORTED_SCHEMA,
    };

    fn instance(instance_id: &str, definition_id: &str) -> NamedAgentRecord {
        NamedAgentRecord {
            schema_version: MAX_SUPPORTED_SCHEMA,
            data: NamedAgentRecordV1 {
                instance_id: instance_id.to_string(),
                instance_name: "Maks".to_string(),
                definition_id: definition_id.to_string(),
                identity_id: None,
                memory_id: None,
                session_id: None,
                working_dir: "maks".to_string(),
                source_agents_base: None,
                created_at_ms: 1,
                last_launched_at_ms: 1,
                created_by_version: "0.56.1".to_string(),
                last_launched_by_version: "0.56.1".to_string(),
            },
        }
    }

    fn definition(id: &str) -> DefinitionRecord {
        DefinitionRecord {
            schema_version: DEF_MAX_SUPPORTED_SCHEMA,
            data: DefinitionRecordV1 {
                id: id.to_string(),
                name: "Maks".to_string(),
                ..Default::default()
            },
        }
    }

    fn trees() -> (tempfile::TempDir, Registry, DefinitionStore) {
        let dir = tempfile::tempdir().unwrap();
        let reg = Registry::open(dir.path().join("registry")).unwrap();
        let defs = DefinitionStore::open(dir.path().join("definitions")).unwrap();
        (dir, reg, defs)
    }

    /// The shape actually on disk after a pre-fix delete: the agent-keyed
    /// record went with the definition, the legacy launch-keyed one didn't.
    #[test]
    fn a_record_whose_definition_is_tombstoned_is_dropped() {
        let (_dir, reg, defs) = trees();
        defs.upsert(&definition("agent-1")).unwrap();
        reg.upsert(&instance("legacy-launch-id", "agent-1")).unwrap();
        defs.retire("agent-1").unwrap();

        assert_eq!(prune_tombstoned_instance_records(&reg, &defs), 1);
        assert!(reg.list_active().unwrap().is_empty());
    }

    #[test]
    fn a_record_with_a_live_definition_is_kept() {
        let (_dir, reg, defs) = trees();
        defs.upsert(&definition("agent-1")).unwrap();
        reg.upsert(&instance("legacy-launch-id", "agent-1")).unwrap();

        assert_eq!(prune_tombstoned_instance_records(&reg, &defs), 0);
        assert_eq!(reg.list_active().unwrap().len(), 1);
    }

    /// The guard that keeps this from deleting real agents: a definition
    /// the global store has never heard of is not evidence of deletion —
    /// it may exist only in some channel's local SQLite.
    #[test]
    fn a_record_whose_definition_is_merely_unknown_is_kept() {
        let (_dir, reg, defs) = trees();
        reg.upsert(&instance("some-id", "channel-local-only")).unwrap();

        assert_eq!(prune_tombstoned_instance_records(&reg, &defs), 0);
        assert_eq!(reg.list_active().unwrap().len(), 1);
    }

    #[test]
    fn an_unretired_definition_keeps_its_records() {
        let (_dir, reg, defs) = trees();
        defs.upsert(&definition("agent-1")).unwrap();
        reg.upsert(&instance("some-id", "agent-1")).unwrap();
        defs.retire("agent-1").unwrap();
        defs.unretire("agent-1").unwrap();

        assert_eq!(prune_tombstoned_instance_records(&reg, &defs), 0);
        assert_eq!(reg.list_active().unwrap().len(), 1);
    }

    /// Both files for one agent go in a single pass — the whole point of
    /// deduping by definition id rather than iterating records.
    #[test]
    fn every_record_for_one_tombstoned_definition_goes_at_once() {
        let (_dir, reg, defs) = trees();
        defs.upsert(&definition("agent-1")).unwrap();
        reg.upsert(&instance("agent-1", "agent-1")).unwrap();
        reg.upsert(&instance("legacy-launch-id", "agent-1")).unwrap();
        defs.retire("agent-1").unwrap();

        assert_eq!(prune_tombstoned_instance_records(&reg, &defs), 2);
        assert!(reg.list_active().unwrap().is_empty());
    }

    #[test]
    fn running_twice_changes_nothing_the_second_time() {
        let (_dir, reg, defs) = trees();
        defs.upsert(&definition("agent-1")).unwrap();
        reg.upsert(&instance("legacy-launch-id", "agent-1")).unwrap();
        defs.retire("agent-1").unwrap();

        assert_eq!(prune_tombstoned_instance_records(&reg, &defs), 1);
        assert_eq!(prune_tombstoned_instance_records(&reg, &defs), 0);
    }
}
