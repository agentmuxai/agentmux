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
        let skip_mem_bundles = !shared.bundle_memory_list().map_err(|e| e.to_string()).map_err(MigrationError)?.iter().all(|b| b.id == "blank");
        let skip_drones      = !shared.drone_list().map_err(|e| e.to_string()).map_err(MigrationError)?.is_empty();
        let skip_links       = !shared.agent_identity_list_all().map_err(|e| e.to_string()).map_err(MigrationError)?.is_empty();
        let skip_muxbus      = shared.muxbus_load().ok().flatten().is_some();

        // Always include the current channel's objects.db first.
        // enumerate_objects_dbs scans home/channels/<ch>/… and misses paths
        // outside that tree (e.g. custom AGENTMUX_DATA_HOME). Upserts are
        // idempotent, so a duplicate via the sibling scan is harmless.
        let mut sibling_stores: Vec<Store> = Vec::new();
        if ctx.channel_store_path.exists() {
            match Store::open_source_readonly(&ctx.channel_store_path) {
                Ok(s) => sibling_stores.push(s),
                Err(e) => tracing::debug!(path = %ctx.channel_store_path.display(), error = %e, "shared_store_backfill: skip current channel store"),
            }
        }
        // Isolation boundary, not a bug: an isolated shared store (see
        // agentmux_common::isolated_auth_enabled) must start genuinely
        // empty of every OTHER channel's real identity accounts/links —
        // scanning every sibling objects.db on the machine (ctx.home is
        // always the true global root, per resolve_home's doc comment)
        // would defeat the entire point of a disposable test store. This
        // channel's own local data (added above) is fine to carry in.
        if !agentmux_common::isolated_auth_enabled() {
            for path in registry::enumerate_objects_dbs(&ctx.home) {
                if path == ctx.channel_store_path { continue; } // already added above
                match Store::open_source_readonly(&path) {
                    Ok(s) => sibling_stores.push(s),
                    Err(e) => tracing::debug!(path = %path.display(), error = %e, "shared_store_backfill: skip sibling"),
                }
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

        // Pass 2: memory bundles, drones, links. (Identity bundles were
        // dropped in Phase 4c of SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md
        // — db_identity_bundles/db_identity_bindings no longer exist, so
        // this pass no longer backfills them.)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::store::{IdentityAccount, SecretRef};

    // Process-global env access — shared with registry::paths and
    // migrations::runner's tests, which mutate the SAME
    // AGENTMUX_ISOLATED_AUTH/AGENTMUX_INSTANCE_DIR vars. A module-local lock
    // only serializes tests within this file; Cargo runs a crate's tests in
    // one multi-threaded process, so a local-only lock still let this
    // module's tests race against those two (reagent/codex on PR #2318).
    use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_LOCK;

    fn clear() {
        std::env::remove_var("AGENTMUX_ISOLATED_AUTH");
        // isolated_auth_enabled() now also defaults on AGENTMUX_CHANNEL
        // (SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06.md) — a leaked
        // non-"stable" value from a sibling test file sharing ENV_LOCK
        // would flip backfills_sibling_accounts_when_not_isolated's
        // "not isolated" precondition without this.
        std::env::remove_var("AGENTMUX_CHANNEL");
    }

    fn make_account(id: &str) -> IdentityAccount {
        IdentityAccount {
            id: id.to_string(),
            name: format!("acct-{id}"),
            provider: "claude".to_string(),
            kind: "oauth".to_string(),
            display_name: String::new(),
            secret_ref: SecretRef::OAuthConfigDir { dir: format!("/tmp/{id}") },
            context: serde_json::json!({}),
            status: "valid".to_string(),
            created_at: 0,
            updated_at: 0,
        }
    }

    /// Sets up: a fresh empty shared store, this channel's own (empty)
    /// objects.db, and ONE sibling dev-branch's objects.db (under
    /// `<home>/dev/other-branch/data/db/objects.db`, the layout
    /// `enumerate_objects_dbs` scans) seeded with a real identity account —
    /// simulating another, unrelated dev branch's real credentials.
    fn setup() -> (tempfile::TempDir, MigrationContext) {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path().to_path_buf();

        let sibling_db = home.join("dev").join("other-branch").join("data").join("db").join("objects.db");
        std::fs::create_dir_all(sibling_db.parent().unwrap()).unwrap();
        let sibling_store = Store::open(&sibling_db).unwrap();
        sibling_store.identity_upsert(&make_account("sibling-acct")).unwrap();

        let channel_store_path = home.join("this-channel").join("data").join("db").join("objects.db");
        std::fs::create_dir_all(channel_store_path.parent().unwrap()).unwrap();
        Store::open(&channel_store_path).unwrap(); // create, empty — no local accounts

        let shared_store_path = home.join("shared").join("store.db");
        std::fs::create_dir_all(shared_store_path.parent().unwrap()).unwrap();

        let ctx = MigrationContext {
            home,
            data_dir: tmp.path().join("this-channel").join("data"),
            shared_store_path,
            channel_store_path,
        };
        (tmp, ctx)
    }

    #[test]
    fn backfills_sibling_accounts_when_not_isolated() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        let (_tmp, ctx) = setup();

        M0011SharedStoreBackfill.up(&ctx).unwrap();

        let shared = Store::open_shared(&ctx.shared_store_path).unwrap();
        let ids: Vec<String> = shared.identity_list(None).unwrap().into_iter().map(|a| a.id).collect();
        assert!(
            ids.contains(&"sibling-acct".to_string()),
            "default (non-isolated) backfill must pick up the sibling branch's real account, got: {ids:?}"
        );
    }

    #[test]
    fn skips_sibling_accounts_when_isolated() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        std::env::set_var("AGENTMUX_ISOLATED_AUTH", "1");
        let (_tmp, ctx) = setup();

        M0011SharedStoreBackfill.up(&ctx).unwrap();

        let shared = Store::open_shared(&ctx.shared_store_path).unwrap();
        let ids: Vec<String> = shared.identity_list(None).unwrap().into_iter().map(|a| a.id).collect();
        assert!(
            ids.is_empty(),
            "isolated backfill must NOT pick up any other channel's real account, got: {ids:?}"
        );
        clear();
    }
}
