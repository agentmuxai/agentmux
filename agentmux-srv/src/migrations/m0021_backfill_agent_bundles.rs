// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Backfill a dedicated ABF bundle for every existing agent definition that
//! doesn't have one — the data-migration half of
//! `docs/specs/ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md` §3/§7:
//! "every agent must have an ABF," and that ABF carries its own
//! provider/model (readonly once set — enforced separately in
//! `bundle.upsert`, see `check_provider_model_immutable`).
//!
//! For every definition with an empty `memory_id`: create a fresh
//! `db_bundles` row (empty instructions/context — a legitimate starting
//! state, not an error; `bundle.validate` already treats an empty bundle as
//! valid) with `provider='claude'`, `model='anthropic'` (§7.3 — every agent
//! on the machine this was designed against runs Claude Code + OAuth
//! Anthropic, so no per-agent inference is needed), and point the
//! definition's `memory_id` at it.
//!
//! **Local-channel only, deliberately** — same pattern this migration's own
//! module doc for `agent_def_set_memory_id_if_empty` documents: a
//! cross-channel (global-registry-only) definition can't be reached today
//! because `DefinitionRecordV1` doesn't carry `memory_id` yet (same
//! accepted gap as `model_vendor_base_url`). The global registry is still
//! attached (mirrors `m0020`'s pattern) so this migration at least SEES
//! every agent and can log which ones it had to skip, rather than silently
//! processing only whatever happens to be in local SQLite.
//!
//! Idempotent: re-running only ever touches definitions still at
//! `memory_id=''`; already-backfilled or already-bound rows are untouched
//! (both by the `agent_def_list()` filter here and by
//! `agent_def_set_memory_id_if_empty`'s own `WHERE memory_id = ''` guard).

use std::sync::Arc;

use crate::backend::storage::store::{Memory, Store};
use crate::registry::{resolve_shared_definitions_dir, DefinitionStore};

use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0021BackfillAgentBundles;

/// Every agent definition backfilled by this migration gets the same
/// provider/model — see the module doc's §7.3 citation. Named constants
/// (not inlined) so a future per-provider-aware backfill has one place to
/// change, and so the test module can assert against the same values the
/// migration actually writes rather than duplicating string literals.
const BACKFILL_PROVIDER: &str = "claude";
const BACKFILL_MODEL: &str = "anthropic";

impl Migration for M0021BackfillAgentBundles {
    fn id(&self) -> &'static str { "0021_backfill_agent_bundles" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str {
        "Provision a dedicated ABF bundle for every agent definition lacking one"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        let wstore = Arc::new(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("backfill_agent_bundles: open wstore: {}", e)))?,
        );
        // Attach the global registry (see module doc) purely for VISIBILITY
        // — this migration cannot write memory_id for a global-only
        // definition (DefinitionRecordV1 doesn't carry the field), but
        // attaching it means agent_def_list() surfaces those agents so we
        // can at least log which ones were skipped, matching m0020's
        // best-effort handling if the registry can't be resolved/opened.
        if let Some(def_dir) = resolve_shared_definitions_dir() {
            match DefinitionStore::open(def_dir) {
                Ok(def_store) => wstore.set_def_registry(Arc::new(def_store)),
                Err(e) => tracing::warn!(error = %e, "backfill_agent_bundles: failed to open global def registry, backfilling local-only"),
            }
        } else {
            tracing::warn!("backfill_agent_bundles: could not resolve global def registry dir, backfilling local-only");
        }

        let defs = wstore
            .agent_def_list()
            .map_err(|e| MigrationError(format!("backfill_agent_bundles: list defs: {}", e)))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        for def in defs {
            if !def.memory_id.is_empty() {
                continue;
            }
            let bundle_id = uuid::Uuid::new_v4().to_string();
            let bundle = Memory {
                id: bundle_id.clone(),
                name: format!("{} — ABF", def.name),
                description: String::new(),
                is_blank: false,
                is_global: false,
                provider: BACKFILL_PROVIDER.to_string(),
                model: BACKFILL_MODEL.to_string(),
                instructions: String::new(),
                instructions_by_provider: "{}".to_string(),
                context_files: "[]".to_string(),
                mcp_servers: "[]".to_string(),
                skills: "[]".to_string(),
                sort_order: 0,
                created_at: now,
                updated_at: now,
            };
            wstore
                .bundle_memory_upsert(&bundle)
                .map_err(|e| MigrationError(format!("backfill_agent_bundles: create bundle for {}: {}", def.id, e)))?;

            let applied = wstore
                .agent_def_set_memory_id_if_empty(&def.id, &bundle_id)
                .map_err(|e| MigrationError(format!("backfill_agent_bundles: bind {}: {}", def.id, e)))?;
            if !applied {
                // No local row for this id — it only resolved via the
                // global registry overlay. Known gap (see module doc); the
                // freshly-created bundle is orphaned (nothing points at it
                // yet) rather than left dangling silently — log it so it's
                // discoverable, matching this codebase's "no silent gaps"
                // convention elsewhere in the migration set.
                tracing::warn!(
                    agent_id = %def.id,
                    bundle_id = %bundle_id,
                    "backfill_agent_bundles: definition only exists in the global registry — \
                     cannot bind memory_id there yet (DefinitionRecordV1 gap); bundle created but unbound"
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::agents::AgentDefinition;

    // Same isolation discipline as m0020_agent_color_backfill's test module
    // (see its own doc comment for the full rationale): every test here
    // calls `up()`, which always tries to resolve+attach the global
    // registry via the process-global `AGENTMUX_HOME_OVERRIDE` env var.
    // Left unset, that would merge in every real agent on the developer's
    // machine. Uses the CRATE-WIDE `test_support::ISOLATED_AUTH_ENV_LOCK`
    // (see its own doc comment), not a mutex local to this module — see
    // m0020_agent_color_backfill's identical comment for why a per-file
    // mutex isn't sufficient (confirmed live, 2026-08-15: these two
    // modules' tests raced each other, and would still race
    // registry/paths.rs's and friends', under any mutex narrower than the
    // crate-wide one every other consumer of this env var already uses).
    use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_GUARD;

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
    fn backfills_a_definition_with_no_bundle() {
        with_isolated_home(|_home| {
            let tmp = tempfile::NamedTempFile::new().unwrap();
            let wstore = Store::open(tmp.path()).unwrap();
            let id = insert_def(&wstore, "Plain Agent");

            M0021BackfillAgentBundles.up(&ctx_for(tmp.path())).unwrap();

            let def = wstore.agent_def_get(&id).unwrap().unwrap();
            assert!(!def.memory_id.is_empty(), "expected memory_id to be backfilled");
            let bundle = wstore.bundle_memory_get(&def.memory_id).unwrap().unwrap();
            assert_eq!(bundle.provider, BACKFILL_PROVIDER);
            assert_eq!(bundle.model, BACKFILL_MODEL);
            assert!(bundle.name.contains("Plain Agent"));
            assert!(!bundle.is_blank);
        });
    }

    #[test]
    fn leaves_an_already_bound_definition_untouched() {
        with_isolated_home(|_home| {
            let tmp = tempfile::NamedTempFile::new().unwrap();
            let wstore = Store::open(tmp.path()).unwrap();
            let id = insert_def(&wstore, "Already Bound");
            wstore.agent_def_set_memory_id_if_empty(&id, "pre-existing-bundle").unwrap();

            M0021BackfillAgentBundles.up(&ctx_for(tmp.path())).unwrap();

            let def = wstore.agent_def_get(&id).unwrap().unwrap();
            assert_eq!(def.memory_id, "pre-existing-bundle", "must not overwrite an existing binding");
        });
    }

    #[test]
    fn rerun_is_idempotent_no_duplicate_bundles() {
        with_isolated_home(|_home| {
            let tmp = tempfile::NamedTempFile::new().unwrap();
            let wstore = Store::open(tmp.path()).unwrap();
            let id = insert_def(&wstore, "Rerun Agent");

            M0021BackfillAgentBundles.up(&ctx_for(tmp.path())).unwrap();
            let first_bundle_id = wstore.agent_def_get(&id).unwrap().unwrap().memory_id;

            M0021BackfillAgentBundles.up(&ctx_for(tmp.path())).unwrap();
            let second_bundle_id = wstore.agent_def_get(&id).unwrap().unwrap().memory_id;

            assert_eq!(first_bundle_id, second_bundle_id, "rerun must not replace the bound bundle");
            // Store::open always seeds the "blank" singleton (see
            // memory_bundles.rs), so a fresh store already has 1 bundle
            // before this migration ever runs — count non-blank bundles.
            let non_blank_count = wstore
                .bundle_memory_list()
                .unwrap()
                .iter()
                .filter(|b| !b.is_blank)
                .count();
            assert_eq!(non_blank_count, 1, "rerun must not create a second orphaned bundle");
        });
    }

    #[test]
    fn backfills_multiple_definitions_with_distinct_bundles() {
        with_isolated_home(|_home| {
            let tmp = tempfile::NamedTempFile::new().unwrap();
            let wstore = Store::open(tmp.path()).unwrap();
            let a = insert_def(&wstore, "Agent A");
            let b = insert_def(&wstore, "Agent B");

            M0021BackfillAgentBundles.up(&ctx_for(tmp.path())).unwrap();

            let bundle_a = wstore.agent_def_get(&a).unwrap().unwrap().memory_id;
            let bundle_b = wstore.agent_def_get(&b).unwrap().unwrap().memory_id;
            assert!(!bundle_a.is_empty());
            assert!(!bundle_b.is_empty());
            assert_ne!(bundle_a, bundle_b, "each agent must get its OWN dedicated bundle");
        });
    }

    #[test]
    fn missing_channel_store_is_a_noop() {
        with_isolated_home(|_home| {
            let ctx = ctx_for(std::path::Path::new("Z:/does/not/exist/objects.db"));
            M0021BackfillAgentBundles.up(&ctx).unwrap();
        });
    }
}
