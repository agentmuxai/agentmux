// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;
use crate::backend::storage::store::Store;
use crate::backend::storage::filestore::FileStore;
use super::{Migration, MigrationContext, MigrationError, MigrationScope, VerifyOutcome};

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

    /// Phase 1 doctor check — the legacy-marker shape. `migrate_block_zones_v1`
    /// writes `migration_agent_zones_v1.flag` under the data dir when it
    /// completes (even on partial failure — see its doc comment) and
    /// deliberately does NOT write it when it bails before doing any work
    /// (`get_all::<Block>` failed, "let the next start retry"). So: channel
    /// store present + marker absent means `up()` returned Ok around a
    /// helper that never finished — recorded applied, did nothing.
    fn verify(&self, ctx: &MigrationContext) -> VerifyOutcome {
        if !ctx.channel_store_path.exists() {
            return VerifyOutcome::Ok("no channel store on this data dir; nothing to migrate".into());
        }
        let marker = ctx.data_dir.join(crate::backend::agent_session::MIGRATION_MARKER_V1);
        if marker.exists() {
            VerifyOutcome::Ok(format!("marker present: {}", marker.display()))
        } else {
            VerifyOutcome::Mismatch(format!(
                "channel store exists but {} is missing — the zone migration never completed on this data dir",
                marker.display()
            ))
        }
    }
}
