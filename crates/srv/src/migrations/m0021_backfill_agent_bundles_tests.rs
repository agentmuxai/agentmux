// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::agents::test_agent_def;
use crate::backend::storage::store::Bundle;

// Every test calls `up()`, which resolves and attaches the global registry through the
// process-global `AGENTMUX_HOME_OVERRIDE`. Left unset it would merge in every real agent on the
// developer's machine, so each runs under the crate-wide lock with a throwaway home.
use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_GUARD;

fn with_isolated_home<T>(f: impl FnOnce(&std::path::Path) -> T) -> T {
    let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let home_tmp = tempfile::tempdir().unwrap();
    std::env::set_var("AGENTMUX_HOME_OVERRIDE", home_tmp.path().to_str().unwrap());
    let result = f(home_tmp.path());
    std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
    result
}

/// `shared_path` must be a fresh per-test file: the tests write bundles there.
fn ctx_for(channel_path: &std::path::Path, shared_path: &std::path::Path) -> MigrationContext {
    MigrationContext {
        home: std::env::temp_dir(),
        data_dir: std::env::temp_dir(),
        shared_store_path: shared_path.to_path_buf(),
        channel_store_path: channel_path.to_path_buf(),
    }
}

fn insert_def(mstore: &Store, name: &str) -> String {
    let mut def = test_agent_def(&format!("test-{name}"), name, "claude", "host", 1, "");
    mstore.agent_def_insert(&mut def).unwrap();
    def.id
}

fn put_bundle(store: &Store, id: &str) {
    store
        .bundle_upsert(&Bundle {
            id: id.to_string(),
            name: format!("{id} — ABF"),
            description: String::new(),
            is_blank: false,
            is_global: false,
            provider: String::new(),
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
        })
        .unwrap();
}

fn non_blank_bundles(shared_path: &std::path::Path) -> usize {
    Store::open_shared(shared_path).unwrap().bundle_list().unwrap().iter().filter(|b| !b.is_blank).count()
}

/// A definition with no recorded bundle stays unbound: nothing is minted for it.
#[test]
fn an_agent_with_no_recorded_bundle_stays_unbound_and_none_is_minted() {
    with_isolated_home(|_home| {
        let shared_tmp = tempfile::NamedTempFile::new().unwrap();
        let ch = tempfile::NamedTempFile::new().unwrap();
        let s = Store::open(ch.path()).unwrap();
        let id = insert_def(&s, "Plain Agent");

        M0021BackfillAgentBundles.up(&ctx_for(ch.path(), shared_tmp.path())).unwrap();

        assert_eq!(s.agent_def_get(&id).unwrap().unwrap().memory_id, "");
        assert_eq!(non_blank_bundles(shared_tmp.path()), 0);
    });
}

/// The agent's second channel binds the bundle another channel recorded.
#[test]
fn a_channel_binds_the_bundle_another_recorded() {
    with_isolated_home(|_home| {
        let shared_tmp = tempfile::NamedTempFile::new().unwrap();
        let (ch1, ch2) = (tempfile::NamedTempFile::new().unwrap(), tempfile::NamedTempFile::new().unwrap());
        let (s1, s2) = (Store::open(ch1.path()).unwrap(), Store::open(ch2.path()).unwrap());
        let id = insert_def(&s1, "RoamingAgent");
        assert_eq!(insert_def(&s2, "RoamingAgent"), id, "same agent in both channels");
        put_bundle(&Store::open_shared(shared_tmp.path()).unwrap(), "recorded-bundle");
        let global = crate::backend::agent_bundle_sidecar::open_global().unwrap();
        crate::backend::agent_bundle_sidecar::claim(&global, &id, "recorded-bundle").unwrap();

        M0021BackfillAgentBundles.up(&ctx_for(ch1.path(), shared_tmp.path())).unwrap();
        M0021BackfillAgentBundles.up(&ctx_for(ch2.path(), shared_tmp.path())).unwrap();

        assert_eq!(s1.agent_def_get(&id).unwrap().unwrap().memory_id, "recorded-bundle");
        assert_eq!(s2.agent_def_get(&id).unwrap().unwrap().memory_id, "recorded-bundle");
        assert_eq!(non_blank_bundles(shared_tmp.path()), 1, "no second bundle minted");
    });
}

/// A recorded bundle this channel can't find is not bound, and nothing replaces it.
#[test]
fn a_recorded_bundle_missing_here_is_not_bound() {
    with_isolated_home(|_home| {
        let shared_tmp = tempfile::NamedTempFile::new().unwrap();
        let ch = tempfile::NamedTempFile::new().unwrap();
        let s = Store::open(ch.path()).unwrap();
        let id = insert_def(&s, "PinnedAgent");
        let global = crate::backend::agent_bundle_sidecar::open_global().unwrap();
        crate::backend::agent_bundle_sidecar::claim(&global, &id, "bundle-in-an-isolated-store").unwrap();

        M0021BackfillAgentBundles.up(&ctx_for(ch.path(), shared_tmp.path())).unwrap();

        assert_eq!(s.agent_def_get(&id).unwrap().unwrap().memory_id, "");
        assert_eq!(non_blank_bundles(shared_tmp.path()), 0);
    });
}

/// A definition that only resolves through the global registry gets nothing here.
#[test]
fn a_global_only_definition_gets_no_bundle() {
    with_isolated_home(|home| {
        use crate::registry::{DefinitionRecord, DefinitionRecordV1, DEF_MAX_SUPPORTED_SCHEMA};
        let def_dir = home.join("shared").join("agents").join("definitions");
        crate::registry::DefinitionStore::open(def_dir)
            .unwrap()
            .upsert(&DefinitionRecord {
                schema_version: DEF_MAX_SUPPORTED_SCHEMA,
                data: DefinitionRecordV1 {
                    id: "test-GlobalOnly".to_string(),
                    name: "GlobalOnly".to_string(),
                    provider: "claude".to_string(),
                    ..Default::default()
                },
            })
            .unwrap();
        let shared_tmp = tempfile::NamedTempFile::new().unwrap();
        let ch = tempfile::NamedTempFile::new().unwrap();
        let s = Store::open(ch.path()).unwrap();
        assert!(!s.agent_def_has_local_row("test-GlobalOnly").unwrap());

        M0021BackfillAgentBundles.up(&ctx_for(ch.path(), shared_tmp.path())).unwrap();

        assert_eq!(non_blank_bundles(shared_tmp.path()), 0);
    });
}

#[test]
fn leaves_an_already_bound_definition_untouched() {
    with_isolated_home(|_home| {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let shared_tmp = tempfile::NamedTempFile::new().unwrap();
        let mstore = Store::open(tmp.path()).unwrap();
        let id = insert_def(&mstore, "Already Bound");
        mstore.agent_def_set_memory_id_if_empty(&id, "pre-existing-bundle").unwrap();

        M0021BackfillAgentBundles.up(&ctx_for(tmp.path(), shared_tmp.path())).unwrap();

        assert_eq!(mstore.agent_def_get(&id).unwrap().unwrap().memory_id, "pre-existing-bundle");
    });
}

#[test]
fn missing_channel_store_is_a_noop() {
    with_isolated_home(|_home| {
        let shared_tmp = tempfile::NamedTempFile::new().unwrap();
        let ctx = ctx_for(std::path::Path::new("Z:/does/not/exist/objects.db"), shared_tmp.path());
        M0021BackfillAgentBundles.up(&ctx).unwrap();
    });
}
