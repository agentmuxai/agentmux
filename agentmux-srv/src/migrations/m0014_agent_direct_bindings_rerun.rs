// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Re-run the direct agent↔account link backfill (m0013) to catch
//! instances launched in the gap between m0013's original run and the
//! launch flow write-through landing.
//!
//! `m0013` backfills `db_agent_identity_links` from bundle bindings, but
//! every migration runs AT MOST ONCE per install (guarded by
//! `db_migrations`, see `migrations/mod.rs`). Any agent launched after
//! m0013 ran on a given install, but before
//! `SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md` PR B1 (launch flow
//! write-through) shipped, would have a bundle-based `identity_id` and NO
//! direct link — m0013 never sees it because it already ran. PR B3 (next)
//! removes the resolver's bundle-bindings fallback entirely, so every such
//! instance needs its direct link before that lands, or it silently loses
//! its credentials.
//!
//! Reuses `m0013`'s `backfill_direct_links` verbatim rather than
//! reimplementing it — the logic (deterministic collision resolution,
//! sentinel-identity skip, idempotent upsert) is identical; only the
//! *when* differs. Safe to run on an install where m0013 already covered
//! everything: `agent_identity_link` is `ON CONFLICT DO UPDATE`, so
//! re-backfilling already-linked instances is a no-op.

use crate::backend::storage::store::Store;
use crate::registry;
use super::m0013_agent_direct_bindings::backfill_direct_links;
use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0014AgentDirectBindingsRerun;

impl Migration for M0014AgentDirectBindingsRerun {
    fn id(&self) -> &'static str { "0014_agent_direct_bindings_rerun" }
    fn scope(&self) -> MigrationScope { MigrationScope::Global }
    fn description(&self) -> &'static str {
        "Re-run the direct agent<->account link backfill for instances launched \
         after m0013's one-time run but before the launch-flow write-through"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        let shared = Store::open_shared(&ctx.shared_store_path)
            .map_err(|e| MigrationError(format!("agent_direct_bindings_rerun: open shared: {}", e)))?;

        let mut sibling_stores: Vec<Store> = Vec::new();
        if ctx.channel_store_path.exists() {
            match Store::open_source_readonly(&ctx.channel_store_path) {
                Ok(s) => sibling_stores.push(s),
                Err(e) => tracing::debug!(
                    path = %ctx.channel_store_path.display(),
                    error = %e,
                    "agent_direct_bindings_rerun: skip current channel store"
                ),
            }
        }
        for path in registry::enumerate_objects_dbs(&ctx.home) {
            if path == ctx.channel_store_path { continue; }
            match Store::open_source_readonly(&path) {
                Ok(s) => sibling_stores.push(s),
                Err(e) => tracing::debug!(
                    path = %path.display(),
                    error = %e,
                    "agent_direct_bindings_rerun: skip sibling"
                ),
            }
        }

        backfill_direct_links(&shared, &sibling_stores)
    }
}
