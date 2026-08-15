// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Backfill a display color (`agent_content` `ui:color`) for every existing
//! agent definition that doesn't have one — the "generate a random color
//! for all the existing agents" one-time script from
//! `docs/specs/SPEC_AGENT_COLOR_2026_08_08.md`.
//!
//! Color choice is a deterministic hash of the agent id over the shared
//! 14-hue palette (`backend::agent_color`) — effectively random across
//! agents, stable across machines/re-runs, no `rand` dependency. Idempotent
//! beyond the framework's once-per-channel tracking: defs that already
//! carry a `ui:color` (e.g. assigned by a newer `createagent` before this
//! migration ran on a second channel) are left untouched.
//!
//! **Must attach the global def registry itself.** `run_pending_migrations`
//! runs before `bootstrap.rs` calls `Store::set_def_registry` on the
//! server's real store (`bootstrap.rs`, global registry attached ~60 lines
//! after the migration call) — a migration's own bare `Store::open()` has
//! no registry wired up unless it attaches one itself. Without this,
//! `agent_def_list()` silently falls back to LOCAL-ONLY
//! (`shared_def_registry() == None`), so this migration would only ever
//! see whatever happens to be in the current channel's own SQLite at
//! migration time — never the cross-channel user agents that live solely
//! in the global registry, which is the entire point of a one-time
//! backfill run from a single channel. Confirmed live: without this fix,
//! a fresh channel's migration ran against 0 local agents (templates seed
//! *after* migrations, in a separate non-migration step) and colored
//! nothing.

use std::sync::Arc;

use crate::backend::agent_color::pick_agent_color;
use crate::backend::storage::store::{AgentContent, Store};
use crate::registry::{resolve_shared_definitions_dir, DefinitionStore};

use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0020AgentColorBackfill;

impl Migration for M0020AgentColorBackfill {
    fn id(&self) -> &'static str { "0020_agent_color_backfill" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str {
        "Backfill a ui:color for every agent definition lacking one"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        let wstore = Arc::new(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("agent_color_backfill: open wstore: {}", e)))?,
        );
        // Attach the global registry (see module doc) so agent_def_list()
        // sees cross-channel agents, not just this channel's local rows.
        // Best-effort, matching bootstrap.rs's own handling: if the shared
        // dir can't be resolved or opened, proceed local-only rather than
        // failing the whole migration — a later channel boot (which DOES
        // wire the registry before any agent.open) still backfills any
        // agent this run couldn't see.
        if let Some(def_dir) = resolve_shared_definitions_dir() {
            match DefinitionStore::open(def_dir) {
                Ok(def_store) => wstore.set_def_registry(Arc::new(def_store)),
                Err(e) => tracing::warn!(error = %e, "agent_color_backfill: failed to open global def registry, backfilling local-only"),
            }
        } else {
            tracing::warn!("agent_color_backfill: could not resolve global def registry dir, backfilling local-only");
        }
        let defs = wstore
            .agent_def_list()
            .map_err(|e| MigrationError(format!("agent_color_backfill: list defs: {}", e)))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        for def in defs {
            let existing = wstore
                .agent_content_get(&def.id, "ui:color")
                .map_err(|e| MigrationError(format!("agent_color_backfill: get {}: {}", def.id, e)))?;
            if existing.map_or(false, |c| !c.content.trim().is_empty()) {
                continue;
            }
            wstore
                .agent_content_set(&AgentContent {
                    agent_id: def.id.clone(),
                    content_type: "ui:color".to_string(),
                    content: pick_agent_color(&def.id).to_string(),
                    updated_at: now,
                })
                .map_err(|e| MigrationError(format!("agent_color_backfill: set {}: {}", def.id, e)))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::agent_color::is_valid_agent_color;
    use crate::backend::storage::agents::AgentDefinition;

    // Every test in this module calls `up()`, which now (see the module doc)
    // always tries to resolve+attach the global registry via the
    // process-global `AGENTMUX_HOME_OVERRIDE` env var. Two hazards follow
    // from that, both closed by this helper:
    //
    // 1. **Real-data leakage.** Left unset, `resolve_shared_definitions_dir()`
    //    falls through to the machine's REAL `~/.agentmux/shared` — so an
    //    "isolated" unit test using a throwaway local channel store would
    //    still merge in every real agent on the developer's machine and
    //    potentially write `ui:color` to any of them lacking one. Every
    //    test here MUST set an override, even the ones with no interest in
    //    global-registry behavior.
    // 2. **Cross-test races.** `cargo test` runs tests as threads within
    //    ONE process, so the env var is genuinely shared: two tests each
    //    setting/using their own override concurrently can observe each
    //    other's value mid-run and attach to the wrong registry. Uses the
    //    CRATE-WIDE `test_support::ISOLATED_AUTH_ENV_LOCK` — see its own
    //    doc comment — not a mutex local to this module; a per-file mutex
    //    only serializes within that one file, and several OTHER files
    //    (registry/paths.rs, server/app_api/mod.rs,
    //    server/identity_auth_dirs.rs) already correctly use this same
    //    shared lock for the identical class of env-var race. Confirmed
    //    live, 2026-08-15: this module's tests raced
    //    m0021_backfill_agent_bundles's before switching both to the
    //    shared lock (an earlier fix that invented a second,
    //    migrations-only mutex was itself insufficient for exactly this
    //    reason).
    use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_GUARD;

    /// Run `f` with `AGENTMUX_HOME_OVERRIDE` pointed at a fresh, empty temp
    /// dir — guarantees `resolve_shared_definitions_dir()` never resolves to
    /// the real `~/.agentmux/shared`, and (with no `shared/agents/definitions`
    /// subdir created) `DefinitionStore::open` fails harmlessly, so `up()`
    /// falls back to local-only exactly like "no registry available" — the
    /// right behavior for tests that don't care about cross-channel agents.
    /// Returns the temp dir so registry-aware tests can populate it first.
    fn with_isolated_home<T>(f: impl FnOnce(&std::path::Path) -> T) -> T {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let home_tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", home_tmp.path().to_str().unwrap());
        let result = f(home_tmp.path());
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        result
    }

    fn ctx_for(path: &std::path::Path) -> MigrationContext {
        MigrationContext {
            home: std::env::temp_dir(),
            data_dir: std::env::temp_dir(),
            shared_store_path: std::env::temp_dir().join("unused-store.db"),
            channel_store_path: path.to_path_buf(),
        }
    }

    fn insert_def(wstore: &Store, name: &str) -> String {
        let mut def = AgentDefinition {
            id: format!("test-{name}"),
            slug: String::new(),
            name: name.to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
        };
        wstore.agent_def_insert(&mut def).unwrap();
        def.id
    }

    #[test]
    fn backfills_only_defs_without_a_color() {
        with_isolated_home(|_home| {
            let tmp = tempfile::NamedTempFile::new().unwrap();
            let wstore = Store::open(tmp.path()).unwrap();
            let plain = insert_def(&wstore, "plain");
            let colored = insert_def(&wstore, "colored");
            wstore
                .agent_content_set(&AgentContent {
                    agent_id: colored.clone(),
                    content_type: "ui:color".to_string(),
                    content: "#123abc".to_string(),
                    updated_at: 42,
                })
                .unwrap();

            M0020AgentColorBackfill.up(&ctx_for(tmp.path())).unwrap();

            let filled = wstore.agent_content_get(&plain, "ui:color").unwrap().unwrap();
            assert!(is_valid_agent_color(&filled.content), "{}", filled.content);
            // Pre-existing color untouched.
            let kept = wstore.agent_content_get(&colored, "ui:color").unwrap().unwrap();
            assert_eq!(kept.content, "#123abc");
            assert_eq!(kept.updated_at, 42);
        });
    }

    #[test]
    fn rerun_is_idempotent() {
        with_isolated_home(|_home| {
            let tmp = tempfile::NamedTempFile::new().unwrap();
            let wstore = Store::open(tmp.path()).unwrap();
            let id = insert_def(&wstore, "one");

            M0020AgentColorBackfill.up(&ctx_for(tmp.path())).unwrap();
            let first = wstore.agent_content_get(&id, "ui:color").unwrap().unwrap();
            M0020AgentColorBackfill.up(&ctx_for(tmp.path())).unwrap();
            let second = wstore.agent_content_get(&id, "ui:color").unwrap().unwrap();
            assert_eq!(first.content, second.content);
            assert_eq!(first.updated_at, second.updated_at);
        });
    }

    #[test]
    fn missing_channel_store_is_a_noop() {
        with_isolated_home(|_home| {
            let ctx = ctx_for(std::path::Path::new("Z:/does/not/exist/objects.db"));
            M0020AgentColorBackfill.up(&ctx).unwrap();
        });
    }

    /// Regression test for the bug this migration's module doc describes:
    /// an agent that exists ONLY in the global cross-channel registry (not
    /// this channel's local SQLite — the common case for a real user agent
    /// opened from a different channel) must still get backfilled. Before
    /// the `set_def_registry` fix, `agent_def_list()` silently fell back
    /// to local-only and this agent was invisible to the migration.
    #[test]
    fn backfills_an_agent_that_exists_only_in_the_global_registry() {
        with_isolated_home(|home| {
            let def_dir = home.join("shared").join("agents").join("definitions");
            std::fs::create_dir_all(&def_dir).unwrap();
            let def_store = DefinitionStore::open(def_dir).unwrap();
            let record = crate::registry::DefinitionRecord {
                schema_version: 1,
                data: crate::registry::DefinitionRecordV1 {
                    id: "global-only-agent".to_string(),
                    slug: String::new(),
                    name: "Global Only".to_string(),
                    icon: String::new(),
                    provider: "claude".to_string(),
                    description: String::new(),
                    working_directory: String::new(),
                    shell: String::new(),
                    provider_flags: String::new(),
                    auto_start: 0,
                    restart_on_crash: 0,
                    idle_timeout_minutes: 0,
                    created_at: 1,
                    agent_type: "host".to_string(),
                    environment: String::new(),
                    agent_bus_id: String::new(),
                    is_seeded: 0,
                    accounts: String::new(),
                    parent_id: String::new(),
                    branch_label: String::new(),
                    updated_at: 1,
                    user_hidden: 0,
                    container_image: String::new(),
                    container_volumes: "[]".to_string(),
                    container_name: String::new(),
                    use_ambient_login: 0,
                    auto_continue_enabled: 0,
                    content: Vec::new(),
                    skills: Vec::new(),
                },
            };
            def_store.upsert(&record).unwrap();

            // Empty LOCAL channel store — this agent has no local row at all.
            let channel_tmp = tempfile::NamedTempFile::new().unwrap();
            Store::open(channel_tmp.path()).unwrap();

            M0020AgentColorBackfill.up(&ctx_for(channel_tmp.path())).unwrap();

            let rec = def_store.get("global-only-agent").unwrap().unwrap();
            let color = rec
                .data
                .content
                .iter()
                .find(|c| c.content_type == "ui:color")
                .map(|c| c.content.clone());
            assert!(
                color.as_deref().is_some_and(is_valid_agent_color),
                "expected a valid color written to the GLOBAL registry, got {color:?}"
            );
        });
    }
}
