// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeSet;

use super::*;
use crate::backend::storage::agents::test_agent_def;
use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_GUARD;

// `up()` removes bundles through the process-global `AGENTMUX_HOME_OVERRIDE`; each test runs under
// the crate-wide lock with a throwaway home so it never sees the developer's agents.
fn with_isolated_home<T>(f: impl FnOnce(&std::path::Path) -> T) -> T {
    let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let home_tmp = tempfile::tempdir().unwrap();
    std::env::set_var("AGENTMUX_HOME_OVERRIDE", home_tmp.path().to_str().unwrap());
    let result = f(home_tmp.path());
    std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
    result
}

fn bundle(id: &str, name: &str) -> Bundle {
    Bundle {
        id: id.to_string(),
        name: name.to_string(),
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
        created_at: 7,
        updated_at: 7,
        is_system: false,
    }
}

struct Fixture {
    shared: tempfile::NamedTempFile,
    channel: tempfile::NamedTempFile,
    data: tempfile::TempDir,
    home: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        Fixture { shared: tempfile::NamedTempFile::new().unwrap(), channel: tempfile::NamedTempFile::new().unwrap(), data: tempfile::tempdir().unwrap(), home: tempfile::tempdir().unwrap() }
    }
    fn ctx(&self) -> MigrationContext {
        MigrationContext {
            home: self.home.path().to_path_buf(),
            data_dir: self.data.path().to_path_buf(),
            shared_store_path: self.shared.path().to_path_buf(),
            channel_store_path: self.channel.path().to_path_buf(),
        }
    }
    fn shared(&self) -> Store {
        Store::open_shared(self.shared.path()).unwrap()
    }
    fn channel(&self) -> Store {
        Store::open(self.channel.path()).unwrap()
    }
    fn bundle_ids(&self) -> BTreeSet<String> {
        self.shared().bundle_list().unwrap().into_iter().map(|b| b.id).collect()
    }
}

/// An agent bound to `bundle_id`; `provider` is the agent's own ("" for the degenerate case).
fn bound_agent(store: &Store, id: &str, provider: &str, bundle_id: &str) {
    let mut def = test_agent_def(id, id, "claude", "host", 1, "");
    def.provider = provider.to_string();
    store.agent_def_insert(&mut def).unwrap();
    assert!(store.agent_def_set_memory_id_if_empty(id, bundle_id).unwrap());
}

fn binding(store: &Store, id: &str) -> String {
    store.conn().lock().unwrap().query_row("SELECT default_memory_id FROM db_agents WHERE id = ?1", [id], |r| r.get(0)).unwrap()
}

fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_bundle_with_content_becomes_the_first_pick_and_the_binding_is_cleared() {
    with_isolated_home(|_home| {
        let f = Fixture::new();
        let (shared, channel) = (f.shared(), f.channel());
        let mut own = bundle("own", "Mary — ABF");
        own.instructions = "be brief".to_string();
        shared.bundle_upsert(&own).unwrap();
        shared.bundle_upsert(&bundle("b1", "Reviewer")).unwrap();
        bound_agent(&channel, "test-mary", "claude", "own");
        channel.agent_bundles_set("test-mary", &ids(&["b1", "own"])).unwrap();

        M0047AgentsDropOwnBundle.up(&f.ctx()).unwrap();

        assert_eq!(binding(&channel, "test-mary"), "");
        assert_eq!(channel.agent_bundle_ids("test-mary").unwrap(), ids(&["own", "b1"]), "first, and listed once");
        assert!(f.bundle_ids().contains("own"));
    });
}

#[test]
fn the_empty_copy_is_cleared_not_picked_and_then_removed() {
    with_isolated_home(|_home| {
        let f = Fixture::new();
        let (shared, channel) = (f.shared(), f.channel());
        shared.bundle_upsert(&bundle("empty", "Ghost — ABF")).unwrap();
        shared.bundle_upsert(&bundle("keep", "Reviewer")).unwrap();
        bound_agent(&channel, "test-ghost", "claude", "empty");
        channel.agent_bundles_set("test-ghost", &ids(&["keep"])).unwrap();

        M0047AgentsDropOwnBundle.up(&f.ctx()).unwrap();

        assert_eq!(binding(&channel, "test-ghost"), "");
        assert_eq!(channel.agent_bundle_ids("test-ghost").unwrap(), ids(&["keep"]));
        assert!(!f.bundle_ids().contains("empty"), "nothing points at it now");
        assert!(f.bundle_ids().contains("keep"));
    });
}

#[test]
fn an_empty_copy_that_skills_are_bound_to_is_picked() {
    with_isolated_home(|_home| {
        let f = Fixture::new();
        let (shared, channel) = (f.shared(), f.channel());
        shared.bundle_upsert(&bundle("skilled", "Sam — ABF")).unwrap();
        bound_agent(&channel, "test-sam", "claude", "skilled");
        channel.conn().lock().unwrap().execute("INSERT INTO db_bundle_skills_ref (bundle_id, skill_id) VALUES ('skilled', 's1')", []).unwrap();

        M0047AgentsDropOwnBundle.up(&f.ctx()).unwrap();

        assert_eq!(channel.agent_bundle_ids("test-sam").unwrap(), ids(&["skilled"]));
        assert!(f.bundle_ids().contains("skilled"));
    });
}

#[test]
fn a_blank_global_or_missing_bundle_is_cleared_and_not_picked() {
    with_isolated_home(|_home| {
        let f = Fixture::new();
        let (shared, channel) = (f.shared(), f.channel());
        let mut blank = bundle("blank", "Vanilla CLI");
        blank.is_blank = true;
        shared.bundle_upsert(&blank).unwrap();
        let mut global = bundle("glob", "Rules");
        global.is_global = true;
        global.instructions = "all agents".to_string();
        shared.bundle_upsert(&global).unwrap();
        bound_agent(&channel, "test-blank", "claude", "blank");
        bound_agent(&channel, "test-global", "claude", "glob");
        bound_agent(&channel, "test-gone", "claude", "no-such-bundle");

        M0047AgentsDropOwnBundle.up(&f.ctx()).unwrap();

        for agent in ["test-blank", "test-global", "test-gone"] {
            assert_eq!(binding(&channel, agent), "", "{agent}");
            assert!(channel.agent_bundle_ids(agent).unwrap().is_empty(), "{agent}");
        }
        assert!(f.bundle_ids().contains("glob"));
    });
}

/// An agent with no provider of its own reads one from its bundle: the binding stays.
#[test]
fn an_agent_with_no_provider_keeps_its_binding_and_is_still_picked() {
    with_isolated_home(|_home| {
        let f = Fixture::new();
        let (shared, channel) = (f.shared(), f.channel());
        let mut own = bundle("own", "Old — ABF");
        own.provider = "codex".to_string();
        own.instructions = "kept".to_string();
        shared.bundle_upsert(&own).unwrap();
        bound_agent(&channel, "test-old", "", "own");

        M0047AgentsDropOwnBundle.up(&f.ctx()).unwrap();

        assert_eq!(binding(&channel, "test-old"), "own");
        assert_eq!(channel.agent_bundle_ids("test-old").unwrap(), ids(&["own"]));
        let def = channel.agent_def_get("test-old").unwrap().unwrap();
        assert_eq!(shared.resolve_effective_provider_id(&def), "codex", "the bundle store answers, as in AppState.id_store");
    });
}

/// The empty copy another channel's agent row still names stays until that channel runs this too,
/// and a recorded bundle is no longer a reason to keep an empty one.
#[test]
fn another_channels_binding_keeps_the_copy_and_a_recorded_one_does_not() {
    with_isolated_home(|_home| {
        let f = Fixture::new();
        let shared = f.shared();
        shared.bundle_upsert(&bundle("elsewhere", "Far — ABF")).unwrap();
        shared.bundle_upsert(&bundle("recorded", "Rec — ABF")).unwrap();
        let global = crate::backend::agent_bundle_sidecar::open_global().unwrap();
        let mut def = test_agent_def("test-rec", "test-rec", "claude", "host", 1, "");
        f.channel().agent_def_insert(&mut def).unwrap();
        crate::backend::agent_bundle_sidecar::claim(&global, "test-rec", "recorded").unwrap();
        let other_db = f.home.path().join("channels").join("other").join("versions").join("0.1.0").join("data").join("db");
        std::fs::create_dir_all(&other_db).unwrap();
        let other = Store::open(&other_db.join("objects.db")).unwrap();
        bound_agent(&other, "test-far", "claude", "elsewhere");
        drop(other);

        M0047AgentsDropOwnBundle.up(&f.ctx()).unwrap();

        let all = f.bundle_ids();
        assert!(all.contains("elsewhere"));
        assert!(!all.contains("recorded"));
    });
}

#[test]
fn a_second_run_changes_nothing() {
    with_isolated_home(|_home| {
        let f = Fixture::new();
        let (shared, channel) = (f.shared(), f.channel());
        let mut own = bundle("own", "Mary — ABF");
        own.instructions = "be brief".to_string();
        shared.bundle_upsert(&own).unwrap();
        bound_agent(&channel, "test-mary", "claude", "own");

        M0047AgentsDropOwnBundle.up(&f.ctx()).unwrap();
        M0047AgentsDropOwnBundle.up(&f.ctx()).unwrap();

        assert_eq!(channel.agent_bundle_ids("test-mary").unwrap(), ids(&["own"]));
        assert_eq!(binding(&channel, "test-mary"), "");
    });
}

#[test]
fn a_missing_store_is_a_noop() {
    with_isolated_home(|_home| {
        let f = Fixture::new();
        let mut ctx = f.ctx();
        ctx.channel_store_path = std::path::PathBuf::from("Z:/does/not/exist/objects.db");
        M0047AgentsDropOwnBundle.up(&ctx).unwrap();
    });
}
