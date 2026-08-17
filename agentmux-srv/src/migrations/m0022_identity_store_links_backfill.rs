// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Backfill `db_agent_identity_links` from every reachable store into the new,
//! permanently-global identity store — see
//! `docs/specs/SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md`.
//!
//! Before this migration, an agent's account link lived wherever `id_store`
//! (`registry::resolve_shared_store_path`) happened to resolve at write time:
//! the true global `shared/store.db` on the `"stable"` channel, or a
//! throwaway `<instance_dir>/identity-store.db` on every local/dev/portable
//! channel (isolated by default since PR #2431,
//! `docs/specs/SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06.md`) — the
//! exact fragmentation
//! `docs/specs/REPORT_HISTORY_CONTINUITY_ACROSS_VERSION_UPGRADE_2026_08_17.md`
//! traces. Without this migration, every link written since 2026-08-06 would
//! be invisible to the new always-global lookup — shipping the store split
//! alone would REGRESS every existing non-stable-channel install (a working
//! agent would suddenly fail its credential gate) instead of fixing it.
//!
//! Sources scanned, read-only (never mutates a source):
//! 1. The current channel's `objects.db` (links can live there if `id_store`
//!    ever fell back to `wstore`, e.g. before `0011_shared_store_backfill`
//!    applied).
//! 2. The true global `shared/store.db`, directly — independent of whether
//!    isolation is currently active for THIS process, since a `"stable"`-
//!    channel run (or any run with `AGENTMUX_ISOLATED_AUTH=0`) may have
//!    written real links there.
//! 3. Every sibling per-(channel,version) and per-dev-branch `objects.db`
//!    (`registry::enumerate_objects_dbs`, the same enumerator
//!    `0011_shared_store_backfill` uses).
//! 4. For each sibling `objects.db`, its sibling isolated identity store
//!    (`<instance_dir>/identity-store.db` — same `instance_dir` the
//!    `objects.db` lives under, derived by stripping `data/db/objects.db`)
//!    — this is the one that actually holds the fragmented, post-2026-08-06
//!    links the whole redesign exists to reunify.
//!
//! Deliberately UNCONDITIONAL on `isolated_auth_enabled()` (unlike
//! `0011_shared_store_backfill`'s sibling-scan, which skips other channels
//! when isolated so a disposable Armory test store starts empty): this
//! migration's destination has no isolation concept at all — see the design
//! doc §2.3 — so there is no "keep this run's view empty" case to preserve.

use crate::backend::storage::store::Store;
use crate::registry;
use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0022IdentityStoreLinksBackfill;

impl Migration for M0022IdentityStoreLinksBackfill {
    fn id(&self) -> &'static str { "0022_identity_store_links_backfill" }
    fn scope(&self) -> MigrationScope { MigrationScope::Global }
    fn description(&self) -> &'static str {
        "Backfill agent<->account links into the permanently-global identity store"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        let identity_store_path = registry::resolve_identity_store_path()
            .ok_or_else(|| MigrationError("identity_store_links_backfill: could not resolve identity store path".to_string()))?;
        if let Some(parent) = identity_store_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| MigrationError(format!("identity_store_links_backfill: create dir: {e}")))?;
        }
        let dest = Store::open_identity_store(&identity_store_path)
            .map_err(|e| MigrationError(format!("identity_store_links_backfill: open destination: {e}")))?;

        // Idempotent re-run: once the destination has ANY link, later boots
        // skip the (relatively expensive) multi-source scan. A link written
        // normally after this migration first ran is not lost by skipping —
        // it already went straight to `dest` via the live application code.
        let already_seeded = !dest.agent_identity_list_all().unwrap_or_default().is_empty();
        if already_seeded {
            return Ok(());
        }

        let mut sources: Vec<Store> = Vec::new();

        // 1. Current channel's objects.db.
        if ctx.channel_store_path.exists() {
            match Store::open_source_readonly(&ctx.channel_store_path) {
                Ok(s) => sources.push(s),
                Err(e) => tracing::debug!(path = %ctx.channel_store_path.display(), error = %e, "identity_store_links_backfill: skip current channel store"),
            }
        }

        // 2. The true global shared/store.db, regardless of this process's
        // current isolation state.
        if ctx.shared_store_path.exists() {
            match Store::open_source_readonly(&ctx.shared_store_path) {
                Ok(s) => sources.push(s),
                Err(e) => tracing::debug!(path = %ctx.shared_store_path.display(), error = %e, "identity_store_links_backfill: skip global shared store"),
            }
        }

        // 3 + 4. Every sibling objects.db, plus each one's sibling isolated
        // identity-store.db (same instance_dir — objects.db lives at
        // <instance_dir>/data/db/objects.db, identity-store.db lives
        // directly at <instance_dir>/identity-store.db).
        for objects_db in registry::enumerate_objects_dbs(&ctx.home) {
            if objects_db == ctx.channel_store_path {
                continue; // already added in step 1
            }
            match Store::open_source_readonly(&objects_db) {
                Ok(s) => sources.push(s),
                Err(e) => tracing::debug!(path = %objects_db.display(), error = %e, "identity_store_links_backfill: skip sibling objects.db"),
            }

            let instance_dir = objects_db
                .parent() // .../data/db
                .and_then(|p| p.parent()) // .../data
                .and_then(|p| p.parent()); // instance_dir
            if let Some(instance_dir) = instance_dir {
                let isolated_store = instance_dir.join("identity-store.db");
                if isolated_store.is_file() && isolated_store != ctx.shared_store_path {
                    match Store::open_source_readonly(&isolated_store) {
                        Ok(s) => sources.push(s),
                        Err(e) => tracing::debug!(path = %isolated_store.display(), error = %e, "identity_store_links_backfill: skip sibling isolated identity store"),
                    }
                }
            }
        }

        let mut linked = 0usize;
        for src in &sources {
            for link in src.agent_identity_list_all().unwrap_or_default() {
                if dest.agent_identity_link(&link.agent_id, &link.account_id, &link.provider).is_ok() {
                    linked += 1;
                }
            }
        }
        tracing::info!(
            sources = sources.len(),
            links_written = linked,
            "identity_store_links_backfill: complete"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_LOCK;

    fn clear() {
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        std::env::remove_var("AGENTMUX_SHARED_DIR");
        std::env::remove_var("AGENTMUX_ISOLATED_AUTH");
        std::env::remove_var("AGENTMUX_CHANNEL");
        std::env::remove_var("AGENTMUX_INSTANCE_DIR");
    }

    /// Sets up the exact real-world broken state: an EMPTY global
    /// `shared/store.db`, an empty current-channel `objects.db`, and one
    /// sibling dev-branch instance dir carrying a link ONLY in its
    /// isolated, per-instance `identity-store.db` (never in an
    /// `objects.db` at all) — precisely how a link written under
    /// `SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06.md`'s default
    /// looks on disk today.
    fn setup() -> (tempfile::TempDir, MigrationContext) {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path().to_path_buf();

        let sibling_instance_dir = home.join("dev").join("other-branch");
        let sibling_objects_db = sibling_instance_dir.join("data").join("db").join("objects.db");
        std::fs::create_dir_all(sibling_objects_db.parent().unwrap()).unwrap();
        Store::open(&sibling_objects_db).unwrap(); // exists, but carries no link — it never does in this scenario

        let sibling_isolated_store = sibling_instance_dir.join("identity-store.db");
        let isolated = Store::open_identity_store(&sibling_isolated_store).unwrap();
        isolated.agent_identity_link("agent-continuing", "acct-real", "claude").unwrap();
        drop(isolated);

        let channel_store_path = home.join("this-channel").join("data").join("db").join("objects.db");
        std::fs::create_dir_all(channel_store_path.parent().unwrap()).unwrap();
        Store::open(&channel_store_path).unwrap();

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
    fn backfills_a_link_from_a_sibling_channels_isolated_identity_store() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        let (_tmp, ctx) = setup();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", &ctx.home);

        M0022IdentityStoreLinksBackfill.up(&ctx).unwrap();

        let dest = Store::open_identity_store(&registry::resolve_identity_store_path().unwrap()).unwrap();
        let links = dest.agent_identity_list_for_agent("agent-continuing").unwrap();
        assert_eq!(
            links.len(), 1,
            "the link written to the sibling channel's ISOLATED identity-store.db \
             must be found — this is the exact real-world broken state, not a \
             hypothetical"
        );
        assert_eq!(links[0].account_id, "acct-real");
        clear();
    }

    #[test]
    fn is_idempotent_on_a_second_run() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        let (_tmp, ctx) = setup();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", &ctx.home);

        M0022IdentityStoreLinksBackfill.up(&ctx).unwrap();
        // Second run must not error, and must not duplicate/lose the row —
        // the already_seeded fast-path should skip the whole scan.
        M0022IdentityStoreLinksBackfill.up(&ctx).unwrap();

        let dest = Store::open_identity_store(&registry::resolve_identity_store_path().unwrap()).unwrap();
        let links = dest.agent_identity_list_for_agent("agent-continuing").unwrap();
        assert_eq!(links.len(), 1);
        clear();
    }

    #[test]
    fn does_not_touch_source_stores() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        let (_tmp, ctx) = setup();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", &ctx.home);

        M0022IdentityStoreLinksBackfill.up(&ctx).unwrap();

        // The sibling's own isolated store must still have its original
        // link — read-only means read-only.
        let sibling_isolated_store = ctx.home.join("dev").join("other-branch").join("identity-store.db");
        let sibling = Store::open_identity_store(&sibling_isolated_store).unwrap();
        let links = sibling.agent_identity_list_for_agent("agent-continuing").unwrap();
        assert_eq!(links.len(), 1, "source store must be untouched, not drained");
        clear();
    }
}
