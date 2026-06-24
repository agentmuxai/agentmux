// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use crate::backend::storage::store::Store;
use crate::backend::storage::muxbus::MuxBusCredentials;
use crate::drone::storage::DroneStore;
use crate::registry;
use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0011SharedStoreBackfill;

impl Migration for M0011SharedStoreBackfill {
    fn id(&self) -> &'static str { "0011_shared_store_backfill" }
    fn scope(&self) -> MigrationScope { MigrationScope::Global }
    fn description(&self) -> &'static str { "Seed shared store from per-channel objects.db files" }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        let shared = Store::open_shared(&ctx.shared_store_path)
            .map_err(|e| MigrationError(format!("shared_store_backfill: open shared: {}", e)))?;

        let skip_accts       = !shared.identity_list(None).map_err(|e| e.to_string()).map_err(MigrationError)?.is_empty();
        let skip_id_bundles  = !shared.bundle_identity_list().map_err(|e| e.to_string()).map_err(MigrationError)?.iter().all(|b| b.id == "blank");
        let skip_mem_bundles = !shared.bundle_memory_list().map_err(|e| e.to_string()).map_err(MigrationError)?.iter().all(|b| b.id == "blank");
        let skip_drones      = !shared.drone_list().map_err(|e| e.to_string()).map_err(MigrationError)?.is_empty();
        let skip_links       = !shared.agent_identity_list_all().map_err(|e| e.to_string()).map_err(MigrationError)?.is_empty();
        let skip_muxbus      = shared.muxbus_load().ok().flatten().is_some();

        let mut sibling_stores: Vec<Store> = Vec::new();
        for path in registry::enumerate_objects_dbs(&ctx.home) {
            match Store::open_source_readonly(&path) {
                Ok(s) => sibling_stores.push(s),
                Err(e) => tracing::debug!(path = %path.display(), error = %e, "shared_store_backfill: skip sibling"),
            }
        }

        // Pass 1: accounts from all sources (before bindings/links for FK safety)
        if !skip_accts {
            for src in &sibling_stores {
                for acct in src.identity_list(None).unwrap_or_default() {
                    let _ = shared.identity_upsert(&acct);
                }
            }
        }

        // Pass 2: bundles, presets, drones, links
        if !skip_id_bundles {
            for src in &sibling_stores {
                for bundle in src.bundle_identity_list().unwrap_or_default().iter().filter(|b| b.id != "blank") {
                    if shared.bundle_identity_upsert(bundle).is_err() { continue; }
                    for b in src.bundle_identity_bindings(&bundle.id).unwrap_or_default() {
                        let _ = shared.bundle_identity_bind(&b.identity_id, &b.provider, &b.account_id);
                    }
                }
            }
        }
        if !skip_mem_bundles {
            for src in &sibling_stores {
                for mem in src.bundle_memory_list().unwrap_or_default().iter().filter(|b| b.id != "blank") {
                    let _ = shared.bundle_memory_upsert(mem);
                }
            }
        }
        if !skip_drones {
            for src in &sibling_stores {
                for drone in src.drone_list().unwrap_or_default() {
                    let _ = shared.drone_upsert(&drone);
                }
            }
        }
        if !skip_links {
            for src in &sibling_stores {
                for link in src.agent_identity_list_all().unwrap_or_default() {
                    let _ = shared.agent_identity_link(&link.agent_id, &link.account_id, &link.provider);
                }
            }
        }
        if !skip_muxbus {
            let best: Option<MuxBusCredentials> = sibling_stores.iter()
                .filter_map(|src| src.muxbus_load().ok().flatten())
                .max_by_key(|c| c.expires_at);
            if let Some(creds) = best {
                let _ = shared.muxbus_save(&creds);
            }
        }

        Ok(())
    }
}
