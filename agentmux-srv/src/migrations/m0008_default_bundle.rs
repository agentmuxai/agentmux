// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Seed Default OAuth identity bundle from ambient credentials — historical.
//!
//! **Retired.** This seeded a "Default" *Identity bundle* (`db_identity_bundles`)
//! from ambient CLI credentials found on disk. `db_identity_bundles`/
//! `db_identity_bindings` were dropped in Phase 4c of
//! SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md — credential resolution
//! reads only `db_agent_identity_links`/`db_accounts`
//! (`identity/resolver.rs::resolve_bindings_for_instance`), which this
//! migration never populated. The migration id stays registered
//! (already-applied installs must never re-run a Global migration) but
//! the body is now a documented no-op; `identity::migration` (the module
//! this called into) was deleted entirely.

use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0008DefaultBundle;

impl Migration for M0008DefaultBundle {
    fn id(&self) -> &'static str { "0008_default_bundle" }
    // Global scope: tracked in shared store so it runs on fresh install.
    fn scope(&self) -> MigrationScope { MigrationScope::Global }
    fn description(&self) -> &'static str { "Seed Default OAuth identity bundle from ambient credentials" }

    fn up(&self, _ctx: &MigrationContext) -> Result<(), MigrationError> {
        Ok(())
    }
}
