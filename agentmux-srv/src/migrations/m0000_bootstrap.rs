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

        // Helper: stamp a migration as applied with zero duration (it already ran).
        let stamp = |id: &str, scope: &str| {
            let _ = shared.migration_mark_applied(id, scope, 0);
        };

        // ── Global: legacy data dir migration ────────────────────────────────
        // m0001_legacy_data_dir: migrated ~/.waveterm → ~/.agentmux.
        // If ~/.agentmux exists, it either was migrated or was created fresh —
        // either way the migration is complete.
        if ctx.home.exists() {
            stamp("0001_legacy_data_dir", "global");
        }

        // ── Channel: agent zone migration ────────────────────────────────────
        // m0002_block_zones_v1: marker = data_dir/migration_agent_zones_v1.flag
        let zone_marker = ctx.data_dir.join("migration_agent_zones_v1.flag");
        if zone_marker.exists() {
            stamp("0002_block_zones_v1", "channel");
        }

        // ── Channel: template promotion ──────────────────────────────────────
        // m0003_template_sessions_v1: no reliable marker in current code.
        // Use objects.db existence as proxy: if the DB exists the user has
        // launched before and this migration has run.
        let objects_db = ctx.data_dir.join("db").join("objects.db");
        if objects_db.exists() {
            stamp("0003_template_sessions_v1", "channel");
        }

        // ── Global: registry instance migration ──────────────────────────────
        // m0004_registry_from_sqlite: marker = <registry_root>/.migrated_from_sqlite
        if let Some(registry_root) = resolve_shared_registry_dir() {
            if registry_root.join(".migrated_from_sqlite").exists() {
                stamp("0004_registry_from_sqlite", "global");
            }

            // m0005_registry_source_bases: marker = <registry_root>/.backfilled_source_bases
            if registry_root.join(".backfilled_source_bases").exists() {
                stamp("0005_registry_source_bases", "global");
            }
        }

        // ── Global: definition registry migration ────────────────────────────
        // m0006_definitions_global: marker = <def_store_root>/.migrated_definitions
        if let Some(def_root) = resolve_shared_definitions_dir() {
            if def_root.join(".migrated_definitions").exists() {
                stamp("0006_definitions_global", "global");
            }
        }

        // ── Channel: agents consolidate ──────────────────────────────────────
        // m0007_agents_consolidate: marker = data_dir/migration_agents_consolidate_v1.flag
        let consolidate_marker = ctx.data_dir.join("migration_agents_consolidate_v1.flag");
        if consolidate_marker.exists() {
            stamp("0007_agents_consolidate", "channel");
        }

        // ── Channel: default OAuth bundle ────────────────────────────────────
        // m0008_default_bundle: no marker file; uses idempotent upsert internally.
        // Stamp if the data dir has been used (objects.db exists).
        if objects_db.exists() {
            stamp("0008_default_bundle", "channel");
        }

        // ── Global: transcript backfill ──────────────────────────────────────
        // m0009_transcript_backfill: marker = <transcripts_dir>/.transcripts_backfilled
        if let Some(transcripts_dir) = resolve_shared_transcripts_dir() {
            if transcripts_dir.join(".transcripts_backfilled").exists() {
                stamp("0009_transcript_backfill", "global");
            }
        }

        // ── Global: session id backfill ──────────────────────────────────────
        // m0010_session_ids: no marker. Stamp if registry exists (idempotent upsert).
        if let Some(registry_root) = resolve_shared_registry_dir() {
            if registry_root.join(".migrated_from_sqlite").exists() {
                stamp("0010_session_ids", "global");
            }
        }

        // ── Global: shared store backfill ────────────────────────────────────
        // m0011_shared_store_backfill: check if shared store already has identity data.
        let has_accounts = shared
            .identity_list(None)
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        if has_accounts {
            stamp("0011_shared_store_backfill", "global");
        }

        Ok(())
    }
}
