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

    /// Doctor check, content-based since Phase 2 of
    /// SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03: the post-condition is
    /// "no agent block still holds a snapshot whose agent `:current` zone is
    /// empty" (`block_zones_look_incomplete`), read through the same
    /// read-only connections `--verify` uses everywhere. The marker file is
    /// reported as evidence only — Phase 1b's version of this check trusted
    /// its presence, which is the F1 shape Phase 2 exists to remove.
    fn verify(&self, ctx: &MigrationContext) -> VerifyOutcome {
        if !ctx.channel_store_path.exists() {
            return VerifyOutcome::Ok("no channel store on this data dir; nothing to migrate".into());
        }
        let filestore_path = ctx.data_dir.join("db").join("filestore.db");
        if !filestore_path.exists() {
            return VerifyOutcome::Ok("no filestore on this data dir; no block snapshots to migrate".into());
        }
        let wstore = match Store::open(&ctx.channel_store_path) {
            Ok(s) => s,
            Err(e) => return VerifyOutcome::Error(format!("open channel store: {}", e)),
        };
        let filestore = match FileStore::open(&filestore_path) {
            Ok(f) => f,
            Err(e) => return VerifyOutcome::Error(format!("open filestore: {}", e)),
        };
        let marker = ctx.data_dir.join(crate::backend::agent_session::MIGRATION_MARKER_V1);
        let marker_note = if marker.exists() { "marker present" } else { "marker absent" };
        if crate::backend::agent_session::block_zones_look_incomplete(&wstore, &filestore) {
            VerifyOutcome::Mismatch(format!(
                "an agent block still holds a snapshot whose agent :current zone is empty — recorded applied, zones not migrated ({})",
                marker_note
            ))
        } else {
            VerifyOutcome::Ok(format!("every block snapshot has a populated agent :current zone ({})", marker_note))
        }
    }
}
