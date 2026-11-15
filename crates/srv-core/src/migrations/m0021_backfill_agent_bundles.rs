// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Bind an agent definition to the bundle another channel already recorded for it
//! (`docs/specs/SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md` §2.1.6).
//!
//! This migration used to mint a dedicated, empty "<Name> — ABF" bundle for every definition
//! without one (`docs/specs/ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md`). It no longer does:
//! a new agent has no bundle of its own, and a fresh channel must not fill the shared store with an
//! empty one per agent (`docs/reports/REPORT_AGENT_BUNDLES_REFINE_OR_DROP_2026_11_03.md`: 881 of
//! 926 bundles on one machine were empty and unreferenced). The record other channels wrote
//! (`agent_bundle_sidecar`) is still bound, so an agent that already has a bundle keeps it in every
//! channel; `m0046` removes the empty ones nothing points at.
//!
//! Local-channel only: `DefinitionRecordV1` doesn't carry `memory_id`, so a definition that exists
//! only in the global registry has no row here to bind to. The registry is still attached so the
//! agents it lists are seen.
//!
//! Idempotent: only a definition still at `memory_id=''` is touched, and
//! `agent_def_set_memory_id_if_empty` refuses to overwrite a binding.

use std::sync::Arc;

use crate::backend::storage::store::Store;
use crate::registry::{resolve_shared_definitions_dir, DefinitionStore};

use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0021BackfillAgentBundles;

/// The store bundles live in: the shared store when it opens, else the channel store, as
/// `AppState.id_store` does. Never a hard failure: an unusable shared store means "not today".
pub(super) fn resolve_bundle_store(ctx: &MigrationContext, mstore: &Arc<Store>) -> Arc<Store> {
    match Store::open_shared(&ctx.shared_store_path) {
        Ok(shared) => Arc::new(shared),
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %ctx.shared_store_path.display(),
                "backfill_agent_bundles: shared store unavailable, falling back to channel store for bundle reads"
            );
            mstore.clone()
        }
    }
}

impl Migration for M0021BackfillAgentBundles {
    fn id(&self) -> &'static str { "0021_backfill_agent_bundles" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str {
        "Bind each agent definition to the bundle another channel recorded for it"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        let mstore = Arc::new(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("backfill_agent_bundles: open mstore: {}", e)))?,
        );
        if let Some(def_dir) = resolve_shared_definitions_dir() {
            match DefinitionStore::open(def_dir) {
                Ok(def_store) => mstore.set_def_registry(Arc::new(def_store)),
                Err(e) => tracing::warn!(error = %e, "backfill_agent_bundles: failed to open global def registry, using local definitions only"),
            }
        } else {
            tracing::warn!("backfill_agent_bundles: could not resolve global def registry dir, using local definitions only");
        }
        let bundle_store = resolve_bundle_store(ctx, &mstore);
        let Some(sidecar) = crate::backend::agent_bundle_sidecar::open_global() else { return Ok(()) };

        let defs = mstore
            .agent_def_list()
            .map_err(|e| MigrationError(format!("backfill_agent_bundles: list defs: {}", e)))?;
        for def in defs.into_iter().filter(|d| d.memory_id.is_empty()) {
            let local = mstore
                .agent_def_has_local_row(&def.id)
                .map_err(|e| MigrationError(format!("backfill_agent_bundles: look up {}: {}", def.id, e)))?;
            if !local {
                tracing::debug!(agent_id = %def.id, "backfill_agent_bundles: definition only in the global registry; nothing here to bind a bundle to");
                continue;
            }
            let recorded = crate::backend::agent_bundle_sidecar::get(&sidecar, &def.id).ok().flatten();
            if let Some(existing) = recorded.filter(|id| matches!(bundle_store.bundle_get(id), Ok(Some(_)))) {
                mstore
                    .agent_def_set_memory_id_if_empty(&def.id, &existing)
                    .map_err(|e| MigrationError(format!("backfill_agent_bundles: bind {}: {}", def.id, e)))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "m0021_backfill_agent_bundles_tests.rs"]
mod tests;
