// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Backfill direct agent↔account links from bundle bindings — historical.
//!
//! Phase 3 slice 2 PR-A (additive, behavior-preserving). Revived the
//! long-existing `db_agent_identity_links` table (`agent_id` ==
//! `AgentDefinition.id`) as a resolution path and seeded it from the
//! `identity_bundle → binding → account` graph so the resolver's new
//! *dual-read* path resolved to the SAME accounts it did before.
//!
//! **Retired.** `db_identity_bundles`/`db_identity_bindings` (the source
//! this read from) were dropped in Phase 4c of
//! SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md, once this migration had
//! already run on every real install and `db_agent_identity_links` was
//! confirmed the sole credential-resolution path
//! (`identity/resolver/inject.rs::resolve_bindings_for_instance`). The migration
//! id stays registered — already-applied installs must never re-run a
//! Global migration — but its body is now a documented no-op.
//!
//! **Idempotency (historical):** `agent_identity_link` is `INSERT ...
//! ON CONFLICT ... DO UPDATE`, so re-running converged to the same rows.

use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0013AgentDirectBindings;

impl Migration for M0013AgentDirectBindings {
    fn id(&self) -> &'static str { "0013_agent_direct_bindings" }
    fn scope(&self) -> MigrationScope { MigrationScope::Global }
    fn description(&self) -> &'static str {
        "Retired no-op — historically backfilled direct agent↔account links from existing bundle bindings"
    }

    fn up(&self, _ctx: &MigrationContext) -> Result<(), MigrationError> {
        Ok(())
    }
}
