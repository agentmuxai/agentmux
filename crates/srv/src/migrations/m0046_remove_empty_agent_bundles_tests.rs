// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::agents::test_agent_def;
use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_GUARD;

// `up()` attaches the global registry and opens the recorded-bundle store through the
// process-global `AGENTMUX_HOME_OVERRIDE`; each test runs under the crate-wide lock with a
// throwaway home so it never sees the developer's agents.
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
}

impl Fixture {
    fn new() -> Self {
        Fixture { shared: tempfile::NamedTempFile::new().unwrap(), channel: tempfile::NamedTempFile::new().unwrap(), data: tempfile::tempdir().unwrap() }
    }
    fn ctx(&self) -> MigrationContext {
        MigrationContext {
            home: std::env::temp_dir(),
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
    fn ids(&self) -> BTreeSet<String> {
        self.shared().bundle_list().unwrap().into_iter().map(|b| b.id).collect()
    }
    fn removed_lists(&self) -> usize {
        std::fs::read_dir(self.data.path())
            .unwrap()
            .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with("removed-empty-agent-bundles-"))
            .count()
    }
}

fn agent(store: &Store, id: &str) {
    let mut def = test_agent_def(id, id, "claude", "host", 1, "");
    store.agent_def_insert(&mut def).unwrap();
}

#[test]
fn an_abf_name_is_recognised_with_or_without_the_copy_number() {
    assert!(is_abf_name("Lark — ABF"));
    assert!(is_abf_name("Loap #2 — ABF (27)"));
    assert!(!is_abf_name("Workspace Rules"));
    assert!(!is_abf_name("Notes — ABF (x)"));
    assert!(!is_abf_name("Notes — ABF ()"));
    assert!(!is_abf_name("My ABF"));
}

#[test]
fn removes_the_empty_unreferenced_copies_and_their_rows_and_nothing_else() {
    with_isolated_home(|_home| {
        let f = Fixture::new();
        let (shared, channel) = (f.shared(), f.channel());
        agent(&channel, "test-bound");
        agent(&channel, "test-picker");
        agent(&channel, "test-recorded");

        // Empty copies nothing points at: these go.
        shared.bundle_upsert(&bundle("orphan-1", "Ghost — ABF")).unwrap();
        shared.bundle_upsert(&bundle("orphan-2", "Ghost — ABF (2)")).unwrap();
        // Kept: the agent's own bundle, a pick, a recorded one.
        shared.bundle_upsert(&bundle("own", "Bound — ABF")).unwrap();
        channel.agent_def_set_memory_id_if_empty("test-bound", "own").unwrap();
        shared.bundle_upsert(&bundle("picked", "Picked — ABF (3)")).unwrap();
        channel.agent_bundles_set("test-picker", &["picked".to_string()]).unwrap();
        shared.bundle_upsert(&bundle("rec", "Recorded — ABF")).unwrap();
        let global = crate::backend::agent_bundle_sidecar::open_global().unwrap();
        crate::backend::agent_bundle_sidecar::claim(&global, "test-recorded", "rec").unwrap();
        // Kept: not empty, not named like one, global.
        let mut written = bundle("written", "Written — ABF");
        written.instructions = "be brief".to_string();
        shared.bundle_upsert(&written).unwrap();
        let mut described = bundle("described", "Described — ABF");
        described.description = "mine".to_string();
        shared.bundle_upsert(&described).unwrap();
        shared.bundle_upsert(&bundle("named", "Reusable set")).unwrap();
        let mut global_entry = bundle("glob", "Rules — ABF");
        global_entry.is_global = true;
        shared.bundle_upsert(&global_entry).unwrap();
        // Kept: empty now, but a version of it once held instructions.
        shared.bundle_upsert(&bundle("emptied", "Emptied — ABF")).unwrap();
        {
            let conn = shared.conn().lock().unwrap();
            conn.execute(
                "INSERT INTO db_bundle_versions (id, bundle_id, name, instructions, content_hash, created_at) VALUES ('v1', 'emptied', 'Emptied — ABF', 'was here', 'h1', 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO db_bundle_versions (id, bundle_id, name, instructions, content_hash, created_at) VALUES ('v2', 'orphan-1', 'Ghost — ABF', '', 'h2', 1)",
                [],
            )
            .unwrap();
        }
        let before = f.ids();

        M0046RemoveEmptyAgentBundles.up(&f.ctx()).unwrap();

        let after = f.ids();
        let gone: BTreeSet<String> = before.difference(&after).cloned().collect();
        assert_eq!(gone, ["orphan-1", "orphan-2"].iter().map(|s| s.to_string()).collect::<BTreeSet<_>>());
        let conn = shared.conn().lock().unwrap();
        let versions: i64 = conn.query_row("SELECT COUNT(*) FROM db_bundle_versions WHERE bundle_id = 'orphan-1'", [], |r| r.get(0)).unwrap();
        assert_eq!(versions, 0, "its version rows go with it");
        drop(conn);
        assert_eq!(f.removed_lists(), 1, "the removed bundles are listed in a file");
    });
}

#[test]
fn a_second_run_finds_nothing_and_writes_no_list() {
    with_isolated_home(|_home| {
        let f = Fixture::new();
        f.shared().bundle_upsert(&bundle("orphan", "Ghost — ABF")).unwrap();
        M0046RemoveEmptyAgentBundles.up(&f.ctx()).unwrap();
        assert_eq!(f.removed_lists(), 1);

        M0046RemoveEmptyAgentBundles.up(&f.ctx()).unwrap();

        assert_eq!(f.removed_lists(), 1);
        assert!(!f.ids().contains("orphan"));
    });
}

#[test]
fn a_missing_store_is_a_noop() {
    with_isolated_home(|_home| {
        let f = Fixture::new();
        let mut ctx = f.ctx();
        ctx.channel_store_path = std::path::PathBuf::from("Z:/does/not/exist/objects.db");
        M0046RemoveEmptyAgentBundles.up(&ctx).unwrap();
    });
}
