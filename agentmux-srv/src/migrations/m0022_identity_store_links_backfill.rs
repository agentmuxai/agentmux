// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Backfill `db_agent_identity_links`, plus a `db_accounts` fallback mirror
//! (reagentx P0 review, added after this migration's first version — see
//! `identity::resolver::inject::resolve_account`'s own doc comment for why
//! the mirror is needed even though `db_accounts` itself stays on `id_store`
//! as the authoritative store), from every reachable store into the new,
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
//! 2. The true global `shared/store.db`, resolved via
//!    `registry::resolve_global_shared_root()` — NOT `MigrationContext`'s own
//!    `shared_store_path` field, which is `resolve_shared_store_path()`'s
//!    result and therefore DOES redirect to a per-channel path under
//!    isolation (reagentx P1 review on this PR — the first version of this
//!    migration used `ctx.shared_store_path` here, silently skipping the
//!    real global store on every non-`"stable"` channel, exactly the
//!    isolation-unaware bug this migration exists to route around). A
//!    `"stable"`-channel run (or any run with `AGENTMUX_ISOLATED_AUTH=0`)
//!    may have written real links there.
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
        "Backfill agent<->account links (and an account fallback mirror) into the permanently-global identity store"
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
        // it already goes straight to `dest` via the live application code.
        // The account mirror is a one-time snapshot from THIS migration
        // only — unlike links, nothing in the live application code writes
        // new accounts into `dest` afterwards (identity::resolver::inject's
        // resolve_account only READS the mirror as a fallback; the OAuth
        // status-update write-back only reaches it for an account already
        // found there). An account created entirely after this migration
        // ran, on a channel that's isolated when the agent is later reopened
        // on a DIFFERENT channel, is the one residual gap this PR
        // explicitly discloses (see the PR description) — the full fix is
        // the deferred per-account `disposable_test` scope split
        // (SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md §3.2).
        let already_seeded = !dest.agent_identity_list_all().unwrap_or_default().is_empty();
        if already_seeded {
            return Ok(());
        }

        // Collect source PATHS first (not yet opened), so they can be sorted
        // into a deterministic order before any reads happen — reagentx P1
        // review on this PR: `agent_identity_link`'s upsert is last-write-
        // wins, and `enumerate_objects_dbs`'s `std::fs::read_dir`-based
        // traversal has no ordering guarantee, so without a defined order an
        // agent linked to different accounts in different channels would
        // resolve non-deterministically across runs/machines — contradicting
        // this migration's own stated "first-seen-wins" intent
        // (SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md §6). Sorting by path
        // string plus the explicit `seen` guard below (belt-and-suspenders:
        // the guard alone would already make ordering irrelevant, but a
        // stable order also makes the resulting choice reproducible/
        // explainable, not just deterministic).
        let mut source_paths: Vec<std::path::PathBuf> = Vec::new();

        // 1. Current channel's objects.db.
        if ctx.channel_store_path.exists() {
            source_paths.push(ctx.channel_store_path.clone());
        }

        // 2. The TRUE global shared/store.db, independent of this process's
        // current isolation state. reagentx P1 review: `ctx.shared_store_path`
        // is NOT this — it's `registry::resolve_shared_store_path()`'s
        // result, which itself redirects to a per-channel path under
        // isolation (`registry/paths.rs`'s own doc comment: "only
        // ctx.shared_store_path itself is meant to vary"). Using it here
        // silently skipped the real global store on every non-`"stable"`
        // channel — exactly the isolation-unaware bug this migration exists
        // to route around. `resolve_global_shared_root()` is the
        // unconditional root every other always-global resolver in this
        // crate uses (`identity_store_path` itself, `resolve_shared_registry_dir`,
        // etc.) — never redirected, per its own doc comment.
        if let Some(global_root) = registry::resolve_global_shared_root() {
            let global_store_path = global_root.join("store.db");
            if global_store_path.exists() {
                source_paths.push(global_store_path);
            }
        }

        // 3 + 4. Every sibling objects.db, plus each one's sibling isolated
        // identity-store.db (same instance_dir — objects.db lives at
        // <instance_dir>/data/db/objects.db, identity-store.db lives
        // directly at <instance_dir>/identity-store.db).
        for objects_db in registry::enumerate_objects_dbs(&ctx.home) {
            if objects_db != ctx.channel_store_path {
                source_paths.push(objects_db.clone());
            }
            let instance_dir = objects_db
                .parent() // .../data/db
                .and_then(|p| p.parent()) // .../data
                .and_then(|p| p.parent()); // instance_dir
            if let Some(instance_dir) = instance_dir {
                let isolated_store = instance_dir.join("identity-store.db");
                if isolated_store.is_file() && !source_paths.contains(&isolated_store) {
                    source_paths.push(isolated_store);
                }
            }
        }

        source_paths.sort();
        source_paths.dedup();

        let mut sources: Vec<Store> = Vec::new();
        for path in &source_paths {
            match Store::open_source_readonly(path) {
                Ok(s) => sources.push(s),
                Err(e) => tracing::debug!(path = %path.display(), error = %e, "identity_store_links_backfill: skip unreadable source"),
            }
        }

        // First-seen-wins across the now-deterministically-ordered sources:
        // track which (agent_id, provider) pairs already got a link written
        // this run so a later source can never silently overwrite an
        // earlier one's choice, regardless of what agent_identity_link's own
        // ON CONFLICT clause would otherwise do.
        let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
        let mut linked = 0usize;
        for src in &sources {
            for link in src.agent_identity_list_all().unwrap_or_default() {
                let key = (link.agent_id.clone(), link.provider.clone());
                if !seen.insert(key) {
                    continue;
                }
                if dest.agent_identity_link(&link.agent_id, &link.account_id, &link.provider).is_ok() {
                    linked += 1;
                }
            }
        }

        // Account mirror (reagentx P0 review on this PR): without this, a
        // link resolved via `dest` still dead-ends the very next step —
        // resolving the ACCOUNT the link points at — whenever that account
        // only ever existed in a per-channel-isolated store, which is the
        // common case on a version/channel switch (see
        // identity::resolver::inject::resolve_account). Same deterministic,
        // first-seen-wins treatment as links, keyed by account id.
        let mut seen_accounts: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut mirrored = 0usize;
        for src in &sources {
            for account in src.identity_list(None).unwrap_or_default() {
                if !seen_accounts.insert(account.id.clone()) {
                    continue;
                }
                if dest.identity_upsert(&account).is_ok() {
                    mirrored += 1;
                }
            }
        }

        tracing::info!(
            sources = sources.len(),
            links_written = linked,
            accounts_mirrored = mirrored,
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

    /// reagentx P0 review on PR #2632: a link alone isn't enough — the
    /// account row it points at must also be reachable, or the spawn dead-
    /// ends resolving it. This test seeds an account in the SAME sibling
    /// isolated store the link test above uses, and confirms it's mirrored
    /// into the destination's db_accounts fallback table.
    #[test]
    fn backfills_the_account_row_alongside_its_link() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        let (_tmp, ctx) = setup();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", &ctx.home);

        // Seed the account in the same sibling isolated store setup()
        // already links "agent-continuing" to.
        let sibling_isolated_store = ctx.home.join("dev").join("other-branch").join("identity-store.db");
        let sibling = Store::open_identity_store(&sibling_isolated_store).unwrap();
        sibling.identity_upsert(&crate::backend::storage::store::IdentityAccount {
            id: "acct-real".to_string(),
            name: "Real Account".to_string(),
            provider: "claude".to_string(),
            kind: "oauth".to_string(),
            display_name: String::new(),
            secret_ref: crate::backend::storage::store::SecretRef::OAuthConfigDir { dir: "/tmp/acct-real".to_string() },
            context: serde_json::json!({}),
            status: "valid".to_string(),
            created_at: 0,
            updated_at: 0,
        }).unwrap();
        drop(sibling);

        M0022IdentityStoreLinksBackfill.up(&ctx).unwrap();

        let dest = Store::open_identity_store(&registry::resolve_identity_store_path().unwrap()).unwrap();
        let account = dest.identity_get("acct-real").unwrap();
        assert!(
            account.is_some(),
            "the account row must be mirrored into the destination, not just its link — \
             a link with no resolvable account still fails the spawn"
        );
        assert_eq!(account.unwrap().provider, "claude");
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

    /// reagentx P1 review on this PR: the original version of this
    /// migration used `ctx.shared_store_path` to find "the true global
    /// store," but that field is `resolve_shared_store_path()`'s result —
    /// which itself redirects to a per-channel path whenever THIS process
    /// is currently isolated. The previous test suite never caught this
    /// because `setup()` hand-built `MigrationContext.shared_store_path`
    /// pointing directly at the real global path, bypassing the exact
    /// function whose isolation-awareness caused the bug. This test drives
    /// the migration with isolation genuinely ON (`AGENTMUX_ISOLATED_AUTH=1`,
    /// a non-`"stable"` channel, a real `AGENTMUX_INSTANCE_DIR`) — the same
    /// environment shape `open_stores_and_migrate` actually runs under on
    /// every local/dev/portable build — with a link seeded ONLY in the true
    /// global `shared/store.db`, and confirms the migration still finds it.
    #[test]
    fn finds_a_link_in_the_true_global_store_even_when_this_process_is_currently_isolated() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path().to_path_buf();

        // Seed a link directly in the TRUE global store — e.g. written on
        // the "stable" channel, or with AGENTMUX_ISOLATED_AUTH=0, at some
        // point before this run.
        let global_store_path = home.join("shared").join("store.db");
        std::fs::create_dir_all(global_store_path.parent().unwrap()).unwrap();
        let global = Store::open_shared(&global_store_path).unwrap();
        // open_shared's schema (unlike open_identity_store's) still has a
        // real account_id FK, so the link write needs a matching row first.
        global.identity_upsert(&crate::backend::storage::store::IdentityAccount {
            id: "acct-global".to_string(),
            name: "Global Acct".to_string(),
            provider: "claude".to_string(),
            kind: "oauth".to_string(),
            display_name: String::new(),
            secret_ref: crate::backend::storage::store::SecretRef::OAuthConfigDir { dir: "/tmp/acct-global".to_string() },
            context: serde_json::json!({}),
            status: "valid".to_string(),
            created_at: 0,
            updated_at: 0,
        }).unwrap();
        global.agent_identity_link("agent-from-stable", "acct-global", "claude").unwrap();
        drop(global);

        let instance_dir = home.join("channels").join("local-somebranch-abcd1234").join("versions").join("0.55.0");
        let channel_store_path = instance_dir.join("data").join("db").join("objects.db");
        std::fs::create_dir_all(channel_store_path.parent().unwrap()).unwrap();
        Store::open(&channel_store_path).unwrap();

        let ctx = MigrationContext {
            home: home.clone(),
            data_dir: instance_dir.join("data"),
            // Deliberately WRONG in exactly the way the bug was: this is
            // what resolve_shared_store_path() would actually return under
            // isolation (the per-channel path), NOT the global store. If
            // the migration used this field for "the global store" the way
            // it did before the fix, this test would fail.
            shared_store_path: instance_dir.join("identity-store.db"),
            channel_store_path,
        };

        std::env::set_var("AGENTMUX_HOME_OVERRIDE", &home);
        std::env::set_var("AGENTMUX_ISOLATED_AUTH", "1");
        std::env::set_var("AGENTMUX_CHANNEL", "local-somebranch-abcd1234");
        std::env::set_var("AGENTMUX_INSTANCE_DIR", &instance_dir);

        M0022IdentityStoreLinksBackfill.up(&ctx).unwrap();

        let dest = Store::open_identity_store(&registry::resolve_identity_store_path().unwrap()).unwrap();
        let links = dest.agent_identity_list_for_agent("agent-from-stable").unwrap();
        assert_eq!(
            links.len(), 1,
            "a link that only ever existed in the TRUE global store must still be \
             found even when the CURRENT process is isolated — this is the exact \
             gap reagentx's review caught"
        );
        assert_eq!(links[0].account_id, "acct-global");
        clear();
    }

    /// reagentx P2 review on this PR: two sources disagreeing about which
    /// account an agent should use must resolve deterministically
    /// (first-seen-wins by sorted source path), not by whatever order
    /// `std::fs::read_dir` happens to return.
    #[test]
    fn conflicting_links_across_sources_resolve_deterministically_not_last_write_wins() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path().to_path_buf();

        // Two sibling dev branches, alphabetically "a-branch" then
        // "z-branch", each linking the SAME agent to a DIFFERENT account.
        // Sorted-path order means "a-branch" must win, regardless of
        // whichever happened to be enumerated or opened first.
        for (branch, account) in [("a-branch", "acct-from-a"), ("z-branch", "acct-from-z")] {
            let instance_dir = home.join("dev").join(branch);
            let objects_db = instance_dir.join("data").join("db").join("objects.db");
            std::fs::create_dir_all(objects_db.parent().unwrap()).unwrap();
            Store::open(&objects_db).unwrap();
            let isolated_store = instance_dir.join("identity-store.db");
            let s = Store::open_identity_store(&isolated_store).unwrap();
            s.agent_identity_link("agent-conflicted", account, "claude").unwrap();
        }

        let channel_store_path = home.join("this-channel").join("data").join("db").join("objects.db");
        std::fs::create_dir_all(channel_store_path.parent().unwrap()).unwrap();
        Store::open(&channel_store_path).unwrap();
        let shared_store_path = home.join("shared").join("store.db");
        std::fs::create_dir_all(shared_store_path.parent().unwrap()).unwrap();

        let ctx = MigrationContext {
            home: home.clone(),
            data_dir: tmp.path().join("this-channel").join("data"),
            shared_store_path,
            channel_store_path,
        };
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", &home);

        M0022IdentityStoreLinksBackfill.up(&ctx).unwrap();

        let dest = Store::open_identity_store(&registry::resolve_identity_store_path().unwrap()).unwrap();
        let links = dest.agent_identity_list_for_agent("agent-conflicted").unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(
            links[0].account_id, "acct-from-a",
            "the alphabetically-first source path must win, deterministically, \
             on every run — not whichever source happened to be read last"
        );
        clear();
    }
}
