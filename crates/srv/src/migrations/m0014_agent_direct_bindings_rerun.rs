// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Re-run the direct agent↔account link backfill to catch instances
//! launched in the gap between `m0013`'s original run and the launch flow
//! write-through landing (PR B1, #1950) — historical.
//!
//! `m0013` backfills `db_agent_identity_links` from bundle bindings, but
//! every migration runs AT MOST ONCE per install (guarded by
//! `db_migrations`, see `migrations/mod.rs`). Any agent launched after
//! `m0013` ran on a given install, but before PR B1 shipped, would have a
//! bundle-based `identity_id` and NO direct link — `m0013` never sees it
//! because it already ran. This migration only ever looked at the SINGLE
//! latest instance per definition — the one whose bundle pick (or
//! "blank") was the actual current intent — and only ADDED/UPDATED links
//! implied by it (never unlinked, since `identity.account.upsert` also
//! writes `db_agent_identity_links` independent of any bundle or launch).
//!
//! **Retired.** `db_identity_bundles`/`db_identity_bindings` (the source
//! this read from) were dropped in Phase 4c of
//! SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md, once this migration had
//! already run on every real install. The migration id stays registered
//! (already-applied installs must never re-run a Global migration) but its
//! body is now a documented no-op.

use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0014AgentDirectBindingsRerun;

impl Migration for M0014AgentDirectBindingsRerun {
    fn id(&self) -> &'static str { "0014_agent_direct_bindings_rerun" }
    fn scope(&self) -> MigrationScope { MigrationScope::Global }
    fn description(&self) -> &'static str {
        "Retired no-op — historically re-ran the direct agent<->account link \
         backfill (latest instance per definition only) for instances launched \
         after m0013's one-time run but before the launch-flow write-through"
    }

    fn up(&self, _ctx: &MigrationContext) -> Result<(), MigrationError> {
        Ok(())
    }
}
