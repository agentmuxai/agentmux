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
        // Proxy: agents/ exists only for real prior-usage installs.
        // Runner creates home/shared/ (making home exist) before m0000 runs,
        // so ctx.home.exists() is no longer a reliable proxy.
        if ctx.home.join("agents").exists() {
            stamp_global("0001_legacy_data_dir");
        }

        // ── Channel: agent zone migration ────────────────────────────────────
        // marker = data_dir/migration_agent_zones_v1.flag
        if ctx.data_dir.join("migration_agent_zones_v1.flag").exists() {
            stamp_channel("0002_block_zones_v1");
        }

        // ── Channel: template promotion ──────────────────────────────────────
        // Proxy: agents/ exists only for real prior-usage installs.
        // Runner always creates objects.db before m0000 runs, so
        // objects_db.exists() is no longer a reliable proxy.
        if ctx.home.join("agents").exists() {
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
        //
        // Phase 0a hardening: existence of the marker alone is no longer
        // trusted as proof the backfill actually populated db_agents (a
        // rebuilt/restored objects.db sitting next to a stale flag file
        // would otherwise get permanently stamped "applied" with zero rows
        // written — see docs/specs/SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03.md
        // §1.2/Phase 0a). If the channel store looks incomplete, leave this
        // unstamped so the real migration (m0007) runs and does the work.
        if ctx.data_dir.join("migration_agents_consolidate_v1.flag").exists() {
            let looks_incomplete = channel_store
                .as_ref()
                .map(|cs| cs.agents_consolidate_looks_incomplete())
                .transpose()
                .map_err(|e| MigrationError(format!("bootstrap: verify agents_consolidate: {}", e)))?
                .unwrap_or(false);
            if !looks_incomplete {
                stamp_channel("0007_agents_consolidate");
            }
        }

        // NOTE: 0008_default_bundle is intentionally NOT stamped here. Its
        // body is now a documented no-op (Phase 4c of
        // SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md deleted the
        // identity::migration module it used to call into, since
        // db_identity_bundles/db_identity_bindings were dropped), so leaving
        // it unstamped is harmless either way — the runner just marks it
        // applied via the normal m0008 registry pass instead.

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
