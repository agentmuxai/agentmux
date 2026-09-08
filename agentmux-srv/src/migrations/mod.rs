// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Data migration framework.
//!
//! Migrations are versioned steps that run once, either via `agentmux-srv migrate`
//! or in-process at startup via `run_pending_migrations` (called unconditionally;
//! fast-paths to `Ok(0)` when already current). The in-process path runs before
//! any store is opened for normal operation so that `id_store` always binds to a
//! fully-backfilled shared store.
//!
//! Adding a migration: implement [`Migration`], add it to [`REGISTRY`], add a
//! file under this module. The ID must be unique and lexicographically ordered
//! (use the `NNNN_slug` convention). Retiring old migrations: remove from
//! [`REGISTRY`] once the minimum supported upgrade path passes the migration's
//! origin version — the `db_migrations` row stays as a permanent record.
//!
//! # No private marker files
//!
//! `db_migrations` (plus a [`Migration::verify`] post-condition) is the sole
//! source of truth for "has this run" — SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03
//! Phase 2. A new migration must not introduce its own flag file, and no
//! migration may treat the mere *existence* of a marker as proof of
//! completion: that is the shape that left `0007`'s target table empty
//! while its flag said "done" (§1.2, F1). The three legacy markers that
//! remain are evidence only — `0007`'s and `0002`'s helpers decide from the
//! data (`consolidate_looks_incomplete`, `block_zones_look_incomplete`) and
//! are content-idempotent, `0003`'s helper ignores its marker and re-asserts
//! its invariant every run — and `m0000_bootstrap` consults them only
//! together with those content checks when stamping a pre-framework install.
//!
//! # Migrations freeze copies of live logic on purpose
//!
//! Some migrations carry verbatim copies of helpers that also exist in live
//! code: `m0017_ambient_login_grandfather` mirrors ~35 lines of
//! `backend/blockcontroller/core.rs`; `m0020_agent_color_backfill` and
//! `m0021_backfill_agent_bundles` mirror 25–30 lines each of
//! `backend/storage/mcp_servers.rs` and `def_registry_mirror.rs`. That is
//! deliberate, not accidental duplication. A migration must produce the same
//! rows on every machine it ever runs on, so it cannot call a live helper
//! whose behaviour a later release may change; the copy pins the logic as it
//! was at the migration's origin version. Do not "deduplicate" a migration
//! against live code. When you copy logic into a new migration, say so next
//! to the copy — `// frozen copy of <path>::<fn> as of <version>` — so the
//! next reader can tell a freeze from a drift.

mod m0000_bootstrap;
mod m0001_legacy_data_dir;
mod m0002_block_zones_v1;
mod m0003_template_sessions_v1;
mod m0004_registry_from_sqlite;
mod m0005_registry_source_bases;
mod m0006_definitions_global;
mod m0007_agents_consolidate;
mod m0008_default_bundle;
mod m0009_transcript_backfill;
mod m0010_session_ids;
mod m0011_shared_store_backfill;
mod m0012_dedup_identity_accounts;
mod m0013_agent_direct_bindings;
mod m0014_agent_direct_bindings_rerun;
mod m0015_seed_starter_skills;
mod m0016_seed_starter_mcp_servers;
mod m0017_ambient_login_grandfather;
mod m0018_ambient_login_registry;
mod m0019_repair_malformed_secret_ref;
mod m0020_agent_color_backfill;
mod m0021_backfill_agent_bundles;
mod m0022_identity_store_links_backfill;
mod m0023_native_memory_versions_backfill;
mod m0024_native_memory_backfill_from_fs;
mod m0025_agents_launch_state_backfill;
mod m0026_registry_agent_id_rekey;
mod m0027_agents_workspace_backfill;
mod m0028_agent_child_tables_repoint_fk;
mod m0029_drop_legacy_agent_tables;
mod runner;
#[cfg(test)]
mod phase5_tests;

pub use runner::count_pending_migrations;
pub use runner::doctor_report_for_instance;
pub use runner::run_migrate_command;
pub use runner::run_pending_migrations;
pub use runner::DoctorReport;

use std::path::PathBuf;

// ── Public types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationScope {
    /// Touches `~/.agentmux/shared/` — runs once regardless of channel.
    Global,
    /// Touches the current channel's data dir.
    Channel,
}

impl MigrationScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Channel => "channel",
        }
    }
}

/// Context passed to every migration's `up()` call.
pub struct MigrationContext {
    /// `~/.agentmux` (or wherever AGENTMUX_SHARED_DIR's parent lives).
    pub home: PathBuf,
    /// Current channel's data directory (passed via `--wavedata`).
    pub data_dir: PathBuf,
    /// Path to the shared store (`~/.agentmux/shared/store.db`).
    pub shared_store_path: PathBuf,
    /// Path to the channel store (`<data_dir>/db/objects.db`).
    pub channel_store_path: PathBuf,
}

#[derive(Debug)]
pub struct MigrationError(pub String);

impl std::fmt::Display for MigrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for MigrationError {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for MigrationError {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

/// Result of an applied migration's post-condition check
/// (`agentmux-srv migrate --verify`, SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03
/// Phase 1). `db_migrations` records only that `up()` returned `Ok` — never
/// that it wrote what it was meant to. A migration that can state its own
/// effect as a cheap query implements [`Migration::verify`]; the rest report
/// `NotVerifiable` and the doctor pass says so rather than guessing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyOutcome {
    /// The migration declares no checkable post-condition (the default).
    NotVerifiable,
    /// The post-condition holds. The string is the evidence (counts, paths).
    Ok(String),
    /// Recorded applied, but the effect is not there — the F1/F2 "marker
    /// present, wrote nothing" shape the spec was written about.
    Mismatch(String),
    /// The check itself could not run (store unreadable, etc.). Treated
    /// like a mismatch for exit-code purposes: an unverifiable applied
    /// migration is not a passing one.
    Error(String),
}

impl VerifyOutcome {
    /// `true` for `Mismatch` and `Error` — the two outcomes that fail the pass.
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Mismatch(_) | Self::Error(_))
    }

    /// Short status word for CLI / NDJSON output.
    pub fn label(&self) -> &'static str {
        match self {
            Self::NotVerifiable => "n/a",
            Self::Ok(_) => "ok",
            Self::Mismatch(_) => "MISMATCH",
            Self::Error(_) => "error",
        }
    }

    /// The detail string, if any.
    pub fn detail(&self) -> &str {
        match self {
            Self::NotVerifiable => "",
            Self::Ok(s) | Self::Mismatch(s) | Self::Error(s) => s,
        }
    }
}

pub trait Migration: Send + Sync {
    fn id(&self) -> &'static str;
    fn scope(&self) -> MigrationScope;
    fn description(&self) -> &'static str;
    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError>;

    /// Post-condition check for an APPLIED migration. Must be read-only and
    /// cheap (a handful of queries / `exists()` calls) — it runs from a CLI
    /// doctor pass, never from startup. Default: nothing checkable. Override
    /// where the effect can be stated as a query; see `m0007` for the
    /// row-count shape and `m0002` for the marker-file shape.
    fn verify(&self, _ctx: &MigrationContext) -> VerifyOutcome {
        VerifyOutcome::NotVerifiable
    }
}

// ── Registry ─────────────────────────────────────────────────────────────────
//
// Ordered list of all migrations. Applied in order; each runs at most once
// (guarded by `db_migrations` in the shared store). Add new migrations at the
// end. Do NOT reorder or remove applied migrations — removals belong in a
// separate "retire" pass once the minimum supported upgrade path clears them.

static REGISTRY: &[&(dyn Migration + Sync)] = &[
    &m0000_bootstrap::M0000Bootstrap,
    &m0001_legacy_data_dir::M0001LegacyDataDir,
    &m0002_block_zones_v1::M0002BlockZonesV1,
    &m0003_template_sessions_v1::M0003TemplateSessionsV1,
    &m0004_registry_from_sqlite::M0004RegistryFromSqlite,
    &m0005_registry_source_bases::M0005RegistrySourceBases,
    &m0006_definitions_global::M0006DefinitionsGlobal,
    &m0007_agents_consolidate::M0007AgentsConsolidate,
    &m0008_default_bundle::M0008DefaultBundle,
    &m0009_transcript_backfill::M0009TranscriptBackfill,
    &m0010_session_ids::M0010SessionIds,
    &m0011_shared_store_backfill::M0011SharedStoreBackfill,
    &m0012_dedup_identity_accounts::M0012DedupIdentityAccounts,
    &m0013_agent_direct_bindings::M0013AgentDirectBindings,
    &m0014_agent_direct_bindings_rerun::M0014AgentDirectBindingsRerun,
    &m0015_seed_starter_skills::M0015SeedStarterSkills,
    &m0016_seed_starter_mcp_servers::M0016SeedStarterMcpServers,
    &m0017_ambient_login_grandfather::M0017AmbientLoginGrandfather,
    &m0018_ambient_login_registry::M0018AmbientLoginRegistry,
    &m0019_repair_malformed_secret_ref::M0019RepairMalformedSecretRef,
    &m0020_agent_color_backfill::M0020AgentColorBackfill,
    &m0021_backfill_agent_bundles::M0021BackfillAgentBundles,
    &m0022_identity_store_links_backfill::M0022IdentityStoreLinksBackfill,
    &m0023_native_memory_versions_backfill::M0023NativeMemoryVersionsBackfill,
    &m0024_native_memory_backfill_from_fs::M0024NativeMemoryBackfillFromFs,
    &m0025_agents_launch_state_backfill::M0025AgentsLaunchStateBackfill,
    &m0026_registry_agent_id_rekey::M0026RegistryAgentIdRekey,
    &m0027_agents_workspace_backfill::M0027AgentsWorkspaceBackfill,
    &m0028_agent_child_tables_repoint_fk::M0028AgentChildTablesRepointFk,
    &m0029_drop_legacy_agent_tables::M0029DropLegacyAgentTables,
];
