// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Shared, cross-version named-agent registry. File-per-agent JSON
//! tree at `<shared_home>/agents/registry/`. See
//! `docs/specs/SPEC_SHARED_AGENT_REGISTRY_2026_05_12.md`.
//!
//! PR A — Parallel-write only. The `Store` `instance_*` mutators
//! call `Registry::{upsert,retire,unretire,hard_delete}` after their
//! SQL execute() succeeds. Frontend RPCs still source rows from
//! SQLite; the registry files are populated but not yet read.
//!
//! `MIN_SUPPORTED_SCHEMA`, `ValidationError`, `RegistryError`, and
//! `Registry::list_active` are part of the registry's stable surface
//! but are only consumed by tests + by PR B's read-path swap. The
//! `allow(dead_code)` on the re-exports keeps that intent explicit.

#![allow(dead_code, unused_imports)]

mod atomic;
mod def_migrate;
mod def_schema;
mod def_store;
mod migrate;
mod paths;
mod schema;
mod store;

#[cfg(test)]
mod tests;

pub use migrate::{
    backfill_source_bases_once, enumerate_objects_dbs, migrate_from_sqlite_once, MigrateStats,
    SourceBackfillStats,
};
pub use paths::{
    resolve_shared_definitions_dir, resolve_shared_reactive_dir, resolve_shared_registry_dir,
    resolve_shared_store_path, resolve_shared_transcripts_dir,
};
// crate-internal only — see resolve_global_shared_root's doc comment for why
// migrations/runner.rs must call this directly instead of deriving `home`
// from resolve_shared_store_path()'s (isolation-aware) return value.
pub(crate) use paths::resolve_global_shared_root;
pub use def_migrate::{migrate_definitions_global_once, DefMigrateStats};
pub use def_schema::{
    DefContentBlob, DefSkillBlob, DefValidationError, DefinitionRecord, DefinitionRecordV1,
    DEF_MAX_SUPPORTED_SCHEMA, DEF_MIN_SUPPORTED_SCHEMA,
};
pub use def_store::{DefStoreError, DefinitionStore};
pub use schema::{
    NamedAgentRecord, NamedAgentRecordV1, ValidationError, MAX_SUPPORTED_SCHEMA,
    MIN_SUPPORTED_SCHEMA,
};
pub use store::{Registry, RegistryError};
