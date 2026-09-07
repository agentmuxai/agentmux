// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;
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
    /// said "done" while `db_agents` was suspected empty. The post-condition
    /// is a pair of counts: if the legacy source tables have rows, the
    /// consolidated target must too.
    fn verify(&self, ctx: &MigrationContext) -> VerifyOutcome {
        if !ctx.channel_store_path.exists() {
            return VerifyOutcome::Ok("no channel store on this data dir; nothing to consolidate".into());
        }
        let store = match Store::open(&ctx.channel_store_path) {
            Ok(s) => s,
            Err(e) => return VerifyOutcome::Error(format!("open channel store: {}", e)),
        };
        let count = |t: &str| super::runner::table_count(&store, t);
        match (count("db_agent_definitions"), count("db_agent_instances"), count("db_agents")) {
            (Ok(defs), Ok(instances), Ok(agents)) => consolidate_verdict(defs, instances, agents),
            (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => VerifyOutcome::Error(e),
        }
    }
}

/// Pure verdict so the rule is unit-testable without a database: legacy rows
/// present + consolidated table empty is the one shape that means "recorded
/// applied, wrote nothing".
pub(super) fn consolidate_verdict(defs: i64, instances: i64, agents: i64) -> VerifyOutcome {
    if defs + instances == 0 {
        return VerifyOutcome::Ok(format!("no legacy rows to consolidate (db_agents={})", agents));
    }
    if agents == 0 {
        return VerifyOutcome::Mismatch(format!(
            "db_agent_definitions={} db_agent_instances={} but db_agents=0 — recorded applied, wrote nothing",
            defs, instances
        ));
    }
    VerifyOutcome::Ok(format!(
        "db_agents={} (legacy: definitions={}, instances={})",
        agents, defs, instances
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_rows_with_empty_target_is_the_incident_shape() {
        match consolidate_verdict(3, 2, 0) {
            VerifyOutcome::Mismatch(m) => {
                assert!(m.contains("db_agent_definitions=3"));
                assert!(m.contains("db_agents=0"));
            }
            other => panic!("expected Mismatch, got {:?}", other),
        }
    }

    #[test]
    fn nothing_to_consolidate_passes_even_with_an_empty_target() {
        // A fresh install: no legacy tables were ever populated.
        assert!(matches!(consolidate_verdict(0, 0, 0), VerifyOutcome::Ok(_)));
        // And a fresh install that then created agents directly in db_agents.
        assert!(matches!(consolidate_verdict(0, 0, 5), VerifyOutcome::Ok(_)));
    }

    #[test]
    fn populated_target_passes_and_reports_counts() {
        match consolidate_verdict(4, 1, 5) {
            VerifyOutcome::Ok(s) => assert!(s.contains("db_agents=5")),
            other => panic!("expected Ok, got {:?}", other),
        }
    }
}
