// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Shared, cross-version named-agent registry. File-per-agent JSON
//! tree at `<shared_home>/agents/registry/`. See
//! `docs/specs/SPEC_SHARED_AGENT_REGISTRY_2026_05_12.md`.
//!
//! PR A — Parallel-write only. The `WaveStore` `instance_*` mutators
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
mod paths;
mod schema;
mod store;

#[cfg(test)]
mod tests;

pub use paths::resolve_shared_registry_dir;
pub use schema::{
    NamedAgentRecord, NamedAgentRecordV1, ValidationError, MAX_SUPPORTED_SCHEMA,
    MIN_SUPPORTED_SCHEMA,
};
pub use store::{Registry, RegistryError};
