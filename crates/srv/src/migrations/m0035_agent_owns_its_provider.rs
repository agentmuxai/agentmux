// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Give every agent the provider it actually runs with, before the bundle
//! stops deciding it (`docs/specs/SPEC_AGENT_BUNDLE_FORMAT_V0_3_2026_10_05.md`
//! §3.3).
//!
//! Until now a bound bundle's non-empty `provider` won over the agent's own
//! (`resolve_effective_provider_id`). From this release the agent's provider
//! decides, so an agent whose two values differ would switch CLI. They differ
//! for agents changed through `agent.define` updates, and for agents with no
//! provider at all that `m0021` gave a `claude` bundle. For each, this copies
//! the bundle's provider onto the agent, so every agent keeps its CLI.
//!
//! **The bundle rows are left as they are.** Bundles live in the store every
//! channel shares, definitions in each channel's own, and channels migrate
//! separately: a channel migrating later must still find the bundle's value.
//! Kept, the values are a "suggested for" hint, and an older build, which
//! still prefers them, runs the same CLI as this one.
//!
//! Same reach as `m0021`: local definitions with a `memory_id` (a definition
//! known only from the global registry carries none, so the bundle never
//! overrode it). Writes go through `agent_def_update`, which also updates the
//! global registry's copy, the one reads prefer. Idempotent: a second run
//! finds nothing to change.

use std::sync::Arc;

use crate::backend::storage::store::Store;
use crate::registry::{resolve_shared_definitions_dir, DefinitionStore};

use super::m0021_backfill_agent_bundles::resolve_bundle_store;
use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0035AgentOwnsItsProvider;

impl Migration for M0035AgentOwnsItsProvider {
    fn id(&self) -> &'static str { "0035_agent_owns_its_provider" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str {
        "Copy each bound bundle's provider onto its agent, where it differs"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        let mstore = Arc::new(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("agent_owns_its_provider: open mstore: {e}")))?,
        );
        // Attached so the update also reaches the registry's copy of each
        // definition, which reads prefer over the local row.
        if let Some(def_dir) = resolve_shared_definitions_dir() {
            match DefinitionStore::open(def_dir) {
                Ok(def_store) => mstore.set_def_registry(Arc::new(def_store)),
                Err(e) => tracing::warn!(error = %e, "agent_owns_its_provider: global def registry unavailable, updating local rows only"),
            }
        }
        let bundle_store = resolve_bundle_store(ctx, &mstore);

        let defs = mstore
            .agent_def_list()
            .map_err(|e| MigrationError(format!("agent_owns_its_provider: list defs: {e}")))?;
        for def in defs {
            if def.memory_id.is_empty() {
                continue;
            }
            let bundle_provider = match bundle_store.bundle_get(&def.memory_id) {
                Ok(Some(b)) => b.provider,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!(agent_id = %def.id, error = %e, "agent_owns_its_provider: bundle unreadable, agent left as is");
                    continue;
                }
            };
            if bundle_provider.is_empty() || bundle_provider == def.provider {
                continue;
            }
            let mut updated = def.clone();
            updated.provider = bundle_provider.clone();
            let applied = mstore
                .agent_def_update(&mut updated)
                .map_err(|e| MigrationError(format!("agent_owns_its_provider: update {}: {e}", def.id)))?;
            if applied {
                tracing::info!(
                    agent_id = %def.id,
                    from = %def.provider,
                    to = %bundle_provider,
                    "agent_owns_its_provider: agent now carries the provider it runs with"
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
    use crate::backend::storage::store::Bundle;
    // Crate-wide lock on the home override: `up()` resolves the global
    // registry through it (see m0021's tests for why it must be this one).
    use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_GUARD;

    fn with_isolated_home<T>(f: impl FnOnce() -> T) -> T {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let home_tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", home_tmp.path().to_str().unwrap());
        let result = f();
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        result
    }

    fn ctx_for(channel_path: &std::path::Path, shared_path: &std::path::Path) -> MigrationContext {
        MigrationContext {
            home: std::env::temp_dir(),
            data_dir: std::env::temp_dir(),
            shared_store_path: shared_path.to_path_buf(),
            channel_store_path: channel_path.to_path_buf(),
        }
    }

    fn agent(id: &str, provider: &str, memory_id: &str) -> AgentDefinition {
        AgentDefinition {
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
            id: id.to_string(),
            slug: String::new(),
            name: id.to_string(),
            icon: String::new(),
            provider: provider.to_string(),
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
            memory_id: memory_id.to_string(),
        }
    }

    fn bundle(id: &str, provider: &str) -> Bundle {
        Bundle {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            is_blank: false,
            is_global: false,
            provider: provider.to_string(),
            model: String::new(),
            instructions: String::new(),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
            is_system: false,
        }
    }

    #[test]
    fn each_agent_takes_the_provider_it_runs_with_and_bundles_are_untouched() {
        with_isolated_home(|| {
            let dir = tempfile::tempdir().unwrap();
            let channel = dir.path().join("objects.db");
            let shared = dir.path().join("shared.db");
            {
                let mstore = Store::open(&channel).unwrap();
                let shared_store = Store::open_shared(&shared).unwrap();
                for (id, provider) in [("b-drift", "claude"), ("b-empty", "claude"), ("b-same", "codex"), ("b-hint-less", "")] {
                    shared_store.bundle_upsert(&bundle(id, provider)).unwrap();
                }
                // Changed through agent.define after creation; ran as claude.
                mstore.agent_def_insert(&mut agent("drifted", "codex", "b-drift")).unwrap();
                // No provider of its own; m0021 gave its bundle claude.
                mstore.agent_def_insert(&mut agent("empty", "", "b-empty")).unwrap();
                mstore.agent_def_insert(&mut agent("same", "codex", "b-same")).unwrap();
                mstore.agent_def_insert(&mut agent("hint-less", "gemini", "b-hint-less")).unwrap();
                mstore.agent_def_insert(&mut agent("unbound", "gemini", "")).unwrap();
            }

            let ctx = ctx_for(&channel, &shared);
            M0035AgentOwnsItsProvider.up(&ctx).unwrap();
            // Idempotent.
            M0035AgentOwnsItsProvider.up(&ctx).unwrap();

            let mstore = Store::open(&channel).unwrap();
            let provider = |id: &str| mstore.agent_def_get(id).unwrap().unwrap().provider;
            assert_eq!(provider("drifted"), "claude");
            assert_eq!(provider("empty"), "claude");
            assert_eq!(provider("same"), "codex");
            assert_eq!(provider("hint-less"), "gemini");
            assert_eq!(provider("unbound"), "gemini");

            let shared_store = Store::open_shared(&shared).unwrap();
            assert_eq!(shared_store.bundle_get("b-drift").unwrap().unwrap().provider, "claude", "bundle rows are kept");
        });
    }

    #[test]
    fn a_missing_channel_store_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx_for(&dir.path().join("absent.db"), &dir.path().join("shared.db"));
        M0035AgentOwnsItsProvider.up(&ctx).unwrap();
    }
}
