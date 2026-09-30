// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Grandfather `use_ambient_login` on the GLOBAL cross-channel definition
//! registry (`~/.agentmux/shared/agents/definitions/`) — the sibling of
//! `m0017_ambient_login_grandfather`, which handles the per-channel SQLite
//! projections.
//!
//! Why a separate, Global-scoped step: an agent created in channel A is
//! surfaced in channel B only via its registry record (`agent_def_get`'s
//! registry fallback), so a record left at the serde-default `0` would
//! block that agent's spawns in every OTHER channel even after m0017
//! grandfathered channel A's SQLite rows. Global scope also means this
//! runs exactly once ever — a Channel-scoped registry pass would re-run
//! when a NEW channel is created later and wrongly flip agents that were
//! deliberately created fail-by-default after the upgrade.
//!
//! Spec §2.4 of SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md.

use std::collections::HashSet;

use crate::backend::storage::store::Store;
use crate::registry;
use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0018AmbientLoginRegistry;

impl Migration for M0018AmbientLoginRegistry {
    fn id(&self) -> &'static str { "0018_ambient_login_registry" }
    fn scope(&self) -> MigrationScope { MigrationScope::Global }
    fn description(&self) -> &'static str {
        "Grandfather use_ambient_login=1 on linkless cross-channel definition records"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        // Isolation boundary, not a bug: this migration reads oauth-linked
        // agent ids from ctx.shared_store_path (isolated to a fresh,
        // per-instance store — see agentmux_common::isolated_auth_enabled)
        // and, unlike m0017's channel-scoped sibling, writes its conclusion
        // into the REAL global cross-channel definitions registry
        // (ctx.home is always the true global root, per resolve_home's doc
        // comment). An isolated store starts with zero links by design, so
        // without this guard, merely booting an isolated dev-test instance
        // for the first time would flip every real agent everywhere to
        // use_ambient_login=1 — reproducing the exact "changes real
        // production auth state" bug this whole feature exists to prevent
        // (reagent/codex on PR #2318). There is no isolated-store
        // equivalent of this write to redirect to instead: it is skipped
        // outright.
        if agentmux_common::isolated_auth_enabled() {
            return Ok(());
        }
        let def_dir = ctx.home.join("shared").join("agents").join("definitions");
        if !def_dir.exists() {
            return Ok(());
        }
        let def_store = registry::DefinitionStore::open(def_dir)
            .map_err(|e| MigrationError(format!("ambient_login_registry: open def store: {}", e)))?;

        // Same oauth-class-only rule as m0017 (shared helper — the two
        // passes must never disagree): api-key links don't forfeit
        // grandfathering.
        let linked: HashSet<String> = if ctx.shared_store_path.exists() {
            let shared = Store::open_shared(&ctx.shared_store_path)
                .map_err(|e| MigrationError(format!("ambient_login_registry: open shared store: {}", e)))?;
            super::m0017_ambient_login_grandfather::oauth_linked_agent_ids(&shared)
                .map_err(|e| MigrationError(format!("ambient_login_registry: read links: {}", e)))?
        } else {
            HashSet::new()
        };

        let records = def_store
            .list_active()
            .map_err(|e| MigrationError(format!("ambient_login_registry: list records: {}", e)))?;
        let mut flipped = 0usize;
        for mut rec in records {
            let id = rec.data.id.clone();
            let want = if linked.contains(&id) { 0 } else { 1 };
            if rec.data.use_ambient_login != want {
                rec.data.use_ambient_login = want;
                def_store
                    .upsert(&rec)
                    .map_err(|e| MigrationError(format!("ambient_login_registry: upsert {}: {}", id, e)))?;
                flipped += 1;
            }
        }
        tracing::info!(
            target: "identity",
            flipped,
            "identity.spawn: ambient-login registry grandfather — {} record(s) updated",
            flipped,
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{DefinitionRecord, DefinitionRecordV1, DEF_MAX_SUPPORTED_SCHEMA};
    // Process-global env access — shared with registry::paths,
    // migrations::runner, and migrations::m0011_shared_store_backfill's
    // tests, which mutate the SAME AGENTMUX_ISOLATED_AUTH var. A
    // module-local lock only serializes tests within this file; Cargo runs
    // a crate's tests in one multi-threaded process, so a local-only lock
    // would still let this module's test race against those (reagent/codex
    // on PR #2318).
    use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_LOCK;

    fn record(id: &str) -> DefinitionRecord {
        DefinitionRecord {
            schema_version: DEF_MAX_SUPPORTED_SCHEMA,
            data: DefinitionRecordV1 {
                id: id.to_string(),
                name: id.to_string(),
                provider: "claude".to_string(),
                is_seeded: 0,
                ..Default::default()
            },
        }
    }

    #[test]
    fn linkless_record_flips_to_ambient_linked_record_stays() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("AGENTMUX_ISOLATED_AUTH");
        // isolated_auth_enabled() now also defaults on AGENTMUX_CHANNEL
        // (SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06.md) — a leaked
        // non-"stable" value from a sibling test file sharing ENV_LOCK
        // would flip this test's "not isolated" precondition without this.
        // reagentx P2 on PR #2431 — the three other files sharing this
        // lock were patched for this exact leak class; this one was missed.
        std::env::remove_var("AGENTMUX_CHANNEL");
        let home = tempfile::tempdir().unwrap();
        let def_dir = home.path().join("shared").join("agents").join("definitions");
        let def_store = registry::DefinitionStore::open(def_dir.clone()).unwrap();
        def_store.upsert(&record("remote-linkless")).unwrap();
        def_store.upsert(&record("remote-linked")).unwrap();

        let shared = home.path().join("shared").join("store.db");
        std::fs::create_dir_all(shared.parent().unwrap()).unwrap();
        let shared_store = Store::open_shared(&shared).unwrap();
        let acct = crate::backend::storage::store::IdentityAccount {
            id: "acct-1".to_string(),
            name: "claude-acct-1".to_string(),
            provider: "claude".to_string(),
            kind: "oauth".to_string(),
            display_name: String::new(),
            secret_ref: crate::backend::storage::store::SecretRef::OAuthConfigDir {
                dir: "/tmp/nowhere".to_string(),
            },
            context: serde_json::json!({}),
            status: "unknown".to_string(),
            created_at: 0,
            updated_at: 0,
        };
        shared_store.identity_upsert(&acct).unwrap();
        shared_store
            .agent_identity_link("remote-linked", "acct-1", "claude")
            .unwrap();
        drop(shared_store);

        let ctx = MigrationContext {
            home: home.path().to_path_buf(),
            data_dir: home.path().to_path_buf(),
            shared_store_path: shared,
            channel_store_path: home.path().join("unused-objects.db"),
        };
        M0018AmbientLoginRegistry.up(&ctx).unwrap();

        let reopened = registry::DefinitionStore::open(def_dir).unwrap();
        assert_eq!(
            reopened.get("remote-linkless").unwrap().unwrap().data.use_ambient_login,
            1,
            "linkless registry record must be grandfathered to ambient"
        );
        assert_eq!(
            reopened.get("remote-linked").unwrap().unwrap().data.use_ambient_login,
            0,
            "linked registry record must keep fail-by-default"
        );
    }

    /// reagent/codex P1 on PR #2318: an isolated boot's shared store is a
    /// fresh, empty, per-instance file — computing `linked` from IT and
    /// then rewriting the REAL global cross-channel definitions registry
    /// would flip every genuinely-linked agent everywhere to
    /// use_ambient_login=1, merely because a disposable test store had no
    /// links yet. `up()` must skip entirely when isolated, leaving the
    /// real registry completely untouched regardless of what the isolated
    /// store contains.
    #[test]
    fn skips_global_registry_rewrite_when_isolated() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("AGENTMUX_ISOLATED_AUTH", "1");
        std::env::remove_var("AGENTMUX_CHANNEL");

        let home = tempfile::tempdir().unwrap();
        let def_dir = home.path().join("shared").join("agents").join("definitions");
        let def_store = registry::DefinitionStore::open(def_dir.clone()).unwrap();
        // Already correctly marked fail-by-default, mirroring a real agent
        // with a real oauth link in the REAL global shared store (which
        // this test deliberately never touches — only ctx.shared_store_path,
        // pointed at a separate, empty "isolated" file, as
        // resolve_shared_store_path() would under AGENTMUX_ISOLATED_AUTH=1).
        let mut linked_rec = record("remote-linked");
        linked_rec.data.use_ambient_login = 0;
        def_store.upsert(&linked_rec).unwrap();

        let isolated_shared = home.path().join("isolated-instance").join("identity-store.db");
        std::fs::create_dir_all(isolated_shared.parent().unwrap()).unwrap();

        let ctx = MigrationContext {
            home: home.path().to_path_buf(),
            data_dir: home.path().to_path_buf(),
            shared_store_path: isolated_shared,
            channel_store_path: home.path().join("unused-objects.db"),
        };
        M0018AmbientLoginRegistry.up(&ctx).unwrap();

        let reopened = registry::DefinitionStore::open(def_dir).unwrap();
        assert_eq!(
            reopened.get("remote-linked").unwrap().unwrap().data.use_ambient_login,
            0,
            "isolated boot must not rewrite the real global registry at all, \
             even though the isolated shared store has zero links"
        );

        std::env::remove_var("AGENTMUX_ISOLATED_AUTH");
    }

    #[test]
    fn missing_registry_dir_is_a_noop() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("AGENTMUX_ISOLATED_AUTH");
        std::env::remove_var("AGENTMUX_CHANNEL");
        let home = tempfile::tempdir().unwrap();
        let ctx = MigrationContext {
            home: home.path().to_path_buf(),
            data_dir: home.path().to_path_buf(),
            shared_store_path: home.path().join("shared").join("store.db"),
            channel_store_path: home.path().join("unused-objects.db"),
        };
        M0018AmbientLoginRegistry.up(&ctx).unwrap();
    }
}
