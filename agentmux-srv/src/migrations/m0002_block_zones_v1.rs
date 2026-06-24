// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;
use crate::backend::storage::store::Store;
use crate::backend::storage::filestore::FileStore;
use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0002BlockZonesV1;

impl Migration for M0002BlockZonesV1 {
    fn id(&self) -> &'static str { "0002_block_zones_v1" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str { "Migrate per-block agent session zones to per-agent zones" }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        let wstore = Arc::new(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("block_zones_v1: open wstore: {}", e)))?,
        );
        let filestore_path = ctx.data_dir.join("db").join("filestore.db");
        let filestore = Arc::new(
            FileStore::open(&filestore_path)
                .map_err(|e| MigrationError(format!("block_zones_v1: open filestore: {}", e)))?,
        );
        crate::backend::agent_session::migrate_block_zones_v1(&wstore, &filestore, &ctx.data_dir);
        Ok(())
    }
}
