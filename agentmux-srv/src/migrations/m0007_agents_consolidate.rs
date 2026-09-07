// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;
use crate::backend::storage::agents_consolidate::consolidate_looks_incomplete;
use crate::backend::storage::store::Store;
use super::{Migration, MigrationContext, MigrationError, MigrationScope, VerifyOutcome};

pub struct M0007AgentsConsolidate;

impl Migration for M0007AgentsConsolidate {
    fn id(&self) -> &'static str { "0007_agents_consolidate" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str { "Consolidate db_agent_definitions + db_agent_instances into db_agents" }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        let wstore = Arc::new(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("agents_consolidate: open wstore: {}", e)))?,
        );
        let stats = wstore
            .run_agents_consolidate(Some(&ctx.data_dir))
            .map_err(|e| MigrationError(format!("agents_consolidate: {}", e)))?;
        // Phase 0c hardening: log the outcome instead of discarding it —
        // `already_done` in particular used to be silently thrown away,
        // making a "marker present, wrote nothing" run indistinguishable
        // from a real backfill in the logs. See
        // docs/specs/SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03.md Phase 0c.
        tracing::info!(
            already_done = stats.already_done,
            templates_inserted = stats.templates_inserted,
            user_defs_inserted = stats.user_defs_inserted,
            instances_as_clone_inserted = stats.instances_as_clone_inserted,
            instances_folded_into_def = stats.instances_folded_into_def,
            "m0007_agents_consolidate: outcome",
        );
        Ok(())
    }

    /// Phase 1 doctor check — the incident this spec was written about
    /// (`SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03` §1, F1/F2): a marker
    /// said "done" while `db_agents` was suspected empty.
    ///
    /// The decision is NOT re-derived here: it is Phase 0b's
    /// `consolidate_looks_incomplete` (`backend/storage/agents_consolidate.rs`),
    /// the same rule the marker gate and `m0000_bootstrap` already use — one
    /// definition of "incomplete", so the three cannot drift (reagent P2 on
    /// #3058). This only adds the counts as evidence, through a read-only
    /// connection.
    fn verify(&self, ctx: &MigrationContext) -> VerifyOutcome {
        let conn = match super::runner::open_readonly(&ctx.channel_store_path) {
            Ok(Some(c)) => c,
            Ok(None) => return VerifyOutcome::Ok("no channel store on this data dir; nothing to consolidate".into()),
            Err(e) => return VerifyOutcome::Error(e),
        };
        let incomplete = match consolidate_looks_incomplete(&conn) {
            Ok(b) => b,
            // The rule queries all three tables; a store where one is missing
            // is recorded-applied-but-schema-absent, which IS an inconsistency.
            Err(e) => return VerifyOutcome::Error(format!("consolidate check: {}", e)),
        };
        let count = |t: &str| super::runner::table_count(&conn, t);
        match (count("db_agent_definitions"), count("db_agent_instances"), count("db_agents")) {
            (Ok(defs), Ok(instances), Ok(agents)) => consolidate_report(incomplete, defs, instances, agents),
            (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => VerifyOutcome::Error(e),
        }
    }
}

/// Formats the verdict `consolidate_looks_incomplete` reached, with the counts
/// as evidence. Formatting only — the rule lives in one place.
pub(super) fn consolidate_report(incomplete: bool, defs: i64, instances: i64, agents: i64) -> VerifyOutcome {
    if incomplete {
        return VerifyOutcome::Mismatch(format!(
            "db_agent_definitions={} db_agent_instances={} but db_agents={} — recorded applied, wrote nothing",
            defs, instances, agents
        ));
    }
    if defs + instances == 0 {
        return VerifyOutcome::Ok(format!("no legacy rows to consolidate (db_agents={})", agents));
    }
    VerifyOutcome::Ok(format!(
        "db_agents={} (legacy: definitions={}, instances={})",
        agents, defs, instances
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::store::Store;

    #[test]
    fn incomplete_verdict_reports_the_incident_shape_with_counts() {
        match consolidate_report(true, 3, 2, 0) {
            VerifyOutcome::Mismatch(m) => {
                assert!(m.contains("db_agent_definitions=3"));
                assert!(m.contains("db_agents=0"));
            }
            other => panic!("expected Mismatch, got {:?}", other),
        }
    }

    #[test]
    fn complete_verdicts_pass_and_carry_counts() {
        assert!(matches!(consolidate_report(false, 0, 0, 0), VerifyOutcome::Ok(_)));
        match consolidate_report(false, 4, 1, 5) {
            VerifyOutcome::Ok(s) => assert!(s.contains("db_agents=5")),
            other => panic!("expected Ok, got {:?}", other),
        }
    }

    #[test]
    fn verify_on_a_fresh_channel_store_passes_and_on_no_store_passes_without_creating_one() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let path = data_dir.join("db").join("objects.db");
        let ctx = MigrationContext {
            home: dir.path().to_path_buf(),
            data_dir: data_dir.clone(),
            shared_store_path: dir.path().join("shared").join("store.db"),
            channel_store_path: path.clone(),
        };
        // No store: nothing to consolidate, and verify must not create one.
        assert!(matches!(M0007AgentsConsolidate.verify(&ctx), VerifyOutcome::Ok(_)));
        assert!(!path.exists());

        // A fresh schema: no legacy rows, empty target — that is consistent.
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        drop(Store::open(&path).expect("create channel store"));
        match M0007AgentsConsolidate.verify(&ctx) {
            VerifyOutcome::Ok(s) => assert!(s.contains("no legacy rows"), "{}", s),
            other => panic!("expected Ok, got {:?}", other),
        }
    }
}
