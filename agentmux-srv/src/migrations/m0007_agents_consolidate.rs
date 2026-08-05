// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;
use crate::backend::storage::store::Store;
use super::{Migration, MigrationContext, MigrationError, MigrationScope};

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
}
