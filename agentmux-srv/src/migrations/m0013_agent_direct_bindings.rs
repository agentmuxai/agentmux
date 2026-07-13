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
//! (`identity/resolver.rs::resolve_bindings_for_instance`). The migration
//! id stays registered — already-applied installs must never re-run a
//! Global migration — but `backfill_direct_links`'s body is now a
//! documented no-op.
//!
//! **Idempotency (historical):** `agent_identity_link` is `INSERT ...
//! ON CONFLICT ... DO UPDATE`, so re-running converged to the same rows.

use crate::backend::storage::store::Store;
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

/// Core backfill — historical. `db_identity_bundles`/`db_identity_bindings`
/// (the source this read from) were dropped in Phase 4c of
/// SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md, once this migration had
/// already run on every real install and `db_agent_identity_links` was
/// confirmed the sole credential-resolution path. This id stays registered
/// (already applied installs must not re-run it) but the body is now a
/// documented no-op rather than a query against a table that no longer
/// exists.
pub(crate) fn backfill_direct_links(
    _shared: &Store,
    _instance_sources: &[Store],
) -> Result<(), MigrationError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared_store() -> Store {
        // A shared-schema store built on a temp file (open_shared needs a
        // real path; :memory: would be re-created per connection).
        let tmp = tempfile::NamedTempFile::new().unwrap();
        Store::open_shared(tmp.path()).unwrap()
    }

    fn channel_store() -> Store {
        Store::open_in_memory().unwrap()
    }

    /// The retired backfill is a documented no-op now — this just pins
    /// that it stays a harmless, non-erroring no-op (already-applied
    /// installs' `db_migrations` tracking means it never runs again in
    /// practice, but the id must stay compilable and callable).
    #[test]
    fn backfill_is_a_harmless_no_op() {
        let shared = shared_store();
        let channel = channel_store();
        backfill_direct_links(&shared, &[channel]).unwrap();
        assert!(shared.agent_identity_list_all().unwrap().is_empty());
    }
}
