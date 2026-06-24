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
        wstore
            .run_agents_consolidate(Some(&ctx.data_dir))
            .map(|_| ())
            .map_err(|e| MigrationError(format!("agents_consolidate: {}", e)))
    }
}
