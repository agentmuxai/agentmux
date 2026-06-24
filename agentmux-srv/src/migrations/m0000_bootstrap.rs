// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Bootstrap: stamp all pre-framework migrations as applied for existing users.
//!
//! Any user who has launched AgentMux before the migration framework existed
//! will have the old startup-embedded migrations already applied (via the
//! marker-file system). This step reads those marker files and stamps the
//! corresponding migration IDs into `db_migrations` so the runner skips them.
//!
//! For migrations without a deterministic marker file, we check for the
//! presence of the data they produce. If we cannot determine whether a
//! migration ran, we leave it unstamped so it runs (all ported migrations are
//! idempotent and safe to re-run).
//!
//! This migration itself is idempotent: on second run the `db_migrations` rows
//! are already present and `migration_is_applied` returns true before we even
//! get here.

use crate::backend::storage::store::Store;
use crate::registry::{resolve_shared_definitions_dir, resolve_shared_registry_dir, resolve_shared_transcripts_dir};

use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0000Bootstrap;

impl Migration for M0000Bootstrap {
    fn id(&self) -> &'static str { "0000_bootstrap_migration_state" }
    fn scope(&self) -> MigrationScope { MigrationScope::Global }
    fn description(&self) -> &'static str {
        "Stamp pre-framework migrations as applied for existing installs"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        let shared = Store::open_shared(&ctx.shared_store_path)
            .map_err(|e| MigrationError(format!("bootstrap: open shared store: {}", e)))?;

        // Channel store for channel-scoped migration stamps. May not exist on a
        // fresh install — in that case all channel migrations are no-ops anyway.
        let channel_store = if ctx.channel_store_path.exists() {
            Some(
                Store::open(&ctx.channel_store_path)
                    .map_err(|e| MigrationError(format!("bootstrap: open channel store: {}", e)))?,
            )
        } else {
            None
        };

        // Stamp helpers — global into shared, channel into channel store.
        let stamp_global = |id: &str| {
            let _ = shared.migration_mark_applied(id, "global", 0);
        };
        let stamp_channel = |id: &str| {
            if let Some(ref cs) = channel_store {
                let _ = cs.migration_mark_applied(id, "channel", 0);
            }
        };

        // ── Global: legacy data dir migration ────────────────────────────────
        if ctx.home.exists() {
            stamp_global("0001_legacy_data_dir");
        }

        // ── Channel: agent zone migration ────────────────────────────────────
        // marker = data_dir/migration_agent_zones_v1.flag
        if ctx.data_dir.join("migration_agent_zones_v1.flag").exists() {
            stamp_channel("0002_block_zones_v1");
        }

        // ── Channel: template promotion ──────────────────────────────────────
        // No reliable marker; use objects.db existence as proxy.
        let objects_db = &ctx.channel_store_path;
        if objects_db.exists() {
            stamp_channel("0003_template_sessions_v1");
        }

        // ── Global: registry instance migration ──────────────────────────────
        if let Some(registry_root) = resolve_shared_registry_dir() {
            if registry_root.join(".migrated_from_sqlite").exists() {
                stamp_global("0004_registry_from_sqlite");
            }
            if registry_root.join(".backfilled_source_bases").exists() {
                stamp_global("0005_registry_source_bases");
            }
        }

        // ── Global: definition registry migration ────────────────────────────
        if let Some(def_root) = resolve_shared_definitions_dir() {
            if def_root.join(".migrated_definitions").exists() {
                stamp_global("0006_definitions_global");
            }
        }

        // ── Channel: agents consolidate ──────────────────────────────────────
        // marker = data_dir/migration_agents_consolidate_v1.flag
        if ctx.data_dir.join("migration_agents_consolidate_v1.flag").exists() {
            stamp_channel("0007_agents_consolidate");
        }

        // ── Channel: default OAuth bundle ────────────────────────────────────
        // No marker; stamp if objects.db exists (migration already ran).
        if objects_db.exists() {
            stamp_channel("0008_default_bundle");
        }

        // ── Global: transcript backfill ──────────────────────────────────────
        if let Some(transcripts_dir) = resolve_shared_transcripts_dir() {
            if transcripts_dir.join(".transcripts_backfilled").exists() {
                stamp_global("0009_transcript_backfill");
            }
        }

        // ── Global: session id backfill ──────────────────────────────────────
        // No marker; stamp if registry migration already ran (its marker confirms
        // session_ids backfill also ran — both ran in the same startup sequence).
        if let Some(registry_root) = resolve_shared_registry_dir() {
            if registry_root.join(".migrated_from_sqlite").exists() {
                stamp_global("0010_session_ids");
            }
        }

        // ── Global: shared store backfill ────────────────────────────────────
        let has_accounts = shared
            .identity_list(None)
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        if has_accounts {
            stamp_global("0011_shared_store_backfill");
        }

        Ok(())
    }
}
