// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Data migration framework.
//!
//! Migrations are versioned steps that run once via `agentmux-srv migrate`
//! (invoked by the launcher before the daemon starts). They never run during
//! normal srv startup — main.rs starts with a guaranteed clean, migrated state.
//!
//! Adding a migration: implement [`Migration`], add it to [`REGISTRY`], add a
//! file under this module. The ID must be unique and lexicographically ordered
//! (use the `NNNN_slug` convention). Retiring old migrations: remove from
//! [`REGISTRY`] once the minimum supported upgrade path passes the migration's
//! origin version — the `db_migrations` row stays as a permanent record.

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
mod runner;

pub use runner::run_migrate_command;

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

pub trait Migration: Send + Sync {
    fn id(&self) -> &'static str;
    fn scope(&self) -> MigrationScope;
    fn description(&self) -> &'static str;
    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError>;
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
];
