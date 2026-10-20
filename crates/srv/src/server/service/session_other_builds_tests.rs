// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use agentmux_common::ipc::{Command, Event};
use serde_json::json;

use super::*;
use crate::backend::service::WebCallType;
use crate::server::service::session_restore::{restore_last_session, save_last_session_snapshot, snapshot_workspace};
use crate::server::service::window::handle_window_service;
use crate::server::tests::test_state;

/// A build `channel`/`version` under `root` whose database holds `record` as
/// its session record (none for `None`), made with the real store schema.
fn build(root: &Path, channel: &str, version: &str, record: Option<Value>) -> PathBuf {
    let db_dir = root.join(channel).join("versions").join(version).join("data").join("db");
    std::fs::create_dir_all(&db_dir).unwrap();
    let path = db_dir.join("objects.db");
    let store = Store::open(&path).unwrap();
    let mut client = Client { oid: format!("client-{channel}-{version}"), ..Default::default() };
    if let Some(record) = record {
        client.meta.insert(SNAPSHOT_META_KEY.to_string(), record);
    }
    store.insert(&mut client).unwrap();
    drop(store);
    path
}

fn tab(name: &str, metas: &[Value]) -> Value {
    json!({ "name": name, "blocks": metas.iter().map(|m| json!({ "meta": m })).collect::<Vec<_>>() })
}

fn agent(id: &str, name: &str) -> Value {
    json!({ "view": "agent", "agentId": id, "agentName": name, "agent:sessionid": format!("s-{id}") })
}

/// Two tabs, five panes, two agents (one of them twice).
fn work_record(saved_at_ms: Option<i64>) -> Value {
    let mut r = json!({
        "tabs": [
            tab("Tab 1", &[agent("a1", "Korp"), json!({ "view": "swarm" }), json!({ "view": "sysinfo" })]),
            tab("review", &[agent("a2", "AgentX"), agent("a1", "Korp")]),
        ],
        "active_tab_index": 1,
    });
    if let Some(at) = saved_at_ms {
        r["saved_at_ms"] = json!(at);
    }
    r
}

async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
    let events = dispatch_to_reducer(state, cmd).await;
    for ev in &events {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
    }
    events
}

fn roots(root: &Path, own: Option<&Path>) -> ScanRoots {
    ScanRoots { channels_root: root.to_path_buf(), own_db_dir: own.map(Path::to_path_buf) }
}

#[test]
fn summarize_counts_tabs_panes_and_each_agent_once() {
    let s = summarize(&work_record(Some(1_000)), "ch", "0.59.32", 5).unwrap();
    assert_eq!((s.windows, s.tabs, s.panes), (1, 2, 5));
    assert_eq!(s.agents, ["Korp", "AgentX"]);
    assert_eq!((s.saved_at_ms, s.saved_at_exact), (1_000, true));

    let s = summarize(&work_record(None), "ch", "0.59.32", 5).unwrap();
    assert_eq!((s.saved_at_ms, s.saved_at_exact), (5, false), "no time of its own: the database's");
}

#[test]
fn the_untouched_default_layout_is_not_worth_offering() {
    let default = json!({ "tabs": [tab("Tab 1", &[json!({ "view": "agent" }), json!({ "view": "swarm" }), json!({ "view": "sysinfo" })])] });
    assert!(summarize(&default, "ch", "v", 0).is_none());
    assert!(summarize(&json!({ "tabs": [] }), "ch", "v", 0).is_none());
    let with_agent = json!({ "tabs": [tab("Tab 1", &[agent("a1", "Korp"), json!({ "view": "swarm" })])] });
    assert!(summarize(&with_agent, "ch", "v", 0).is_some());
}

#[test]
fn discovery_skips_its_own_build_and_what_it_cannot_read_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    build(root, "local-old", "0.59.31", Some(work_record(Some(1_000))));
    build(root, "local-new", "0.59.32", Some(work_record(Some(3_000))));
    let own = build(root, "stable", "0.59.35", Some(work_record(Some(9_000))));
    build(root, "no-record", "0.59.30", None);
    // A version folder that never got a database, and one that isn't SQLite.
    std::fs::create_dir_all(root.join("never-ran").join("versions").join("0.1.0").join("data")).unwrap();
    let corrupt = root.join("corrupt").join("versions").join("0.59.33").join("data").join("db");
    std::fs::create_dir_all(&corrupt).unwrap();
    std::fs::write(corrupt.join("objects.db"), b"not a database at all, just bytes").unwrap();

    let found = discover(&roots(root, own.parent()), SCAN_BUDGET);
    let names: Vec<(&str, &str)> = found.iter().map(|l| (l.channel.as_str(), l.version.as_str())).collect();
    assert_eq!(names, [("local-new", "0.59.32"), ("local-old", "0.59.31")]);
    assert_eq!(found[0].tabs, 2);
}

#[test]
fn a_locked_database_is_skipped_without_waiting_on_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let locked = build(root, "busy", "0.59.32", Some(work_record(Some(5_000))));
    build(root, "free", "0.59.31", Some(work_record(Some(1_000))));
    let holder = rusqlite::Connection::open(&locked).unwrap();
    holder
        .execute_batch("PRAGMA locking_mode=EXCLUSIVE; BEGIN EXCLUSIVE; UPDATE db_client SET version = version + 1;")
        .unwrap();

    let started = Instant::now();
    let found = discover(&roots(root, None), SCAN_BUDGET);
    assert!(started.elapsed() < Duration::from_secs(2), "took {:?}", started.elapsed());
    let channels: Vec<&str> = found.iter().map(|l| l.channel.as_str()).collect();
    assert_eq!(channels, ["free"]);
    drop(holder);
}

#[test]
fn reading_a_record_leaves_the_database_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = build(dir.path(), "old", "0.59.32", Some(work_record(Some(1_000))));
    let before = std::fs::read(&path).unwrap();
    assert_eq!(read_record(&path).unwrap(), Some(work_record(Some(1_000))));
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn locate_refuses_paths_out_of_the_channels_and_this_builds_own() {
    let dir = tempfile::tempdir().unwrap();
    let own = build(dir.path(), "stable", "0.59.35", None);
    build(dir.path(), "old", "0.59.32", None);
    let r = roots(dir.path(), own.parent());
    assert!(locate(&r, "old", "0.59.32").is_ok());
    assert!(locate(&r, "stable", "0.59.35").is_err(), "this build's own");
    assert!(locate(&r, "..", "0.59.32").is_err());
    assert!(locate(&r, "old/../stable", "0.59.35").is_err());
    assert!(locate(&r, "old", "missing").is_err());
}

async fn cold_start(state: &AppState) -> Window {
    let mut client = state.mstore.get_all::<Client>().unwrap().into_iter().next().unwrap();
    client.windowids.clear();
    state.mstore.update(&mut client).unwrap();
    let call = WebCallType {
        service: "window".to_string(),
        method: "CreateWindow".to_string(),
        uicontext: None,
        args: vec![Value::Null, json!(""), json!("main"), json!(true)],
    };
    let ret = handle_window_service(state, &call).await;
    assert!(ret.success, "CreateWindow failed: {:?}", ret.error);
    serde_json::from_value(ret.data.unwrap()).unwrap()
}

/// `(tab name, each pane's meta)` for every tab of `ws_id`, in order.
fn tabs_of(state: &AppState, ws_id: &str) -> Vec<(String, Vec<Value>)> {
    let ws = state.mstore.get::<Workspace>(ws_id).unwrap().unwrap();
    ws.pinnedtabids
        .iter()
        .chain(ws.tabids.iter())
        .map(|t| {
            let tab = state.mstore.get::<Tab>(t).unwrap().unwrap();
            let metas = tab
                .blockids
                .iter()
                .map(|b| serde_json::to_value(&state.mstore.get::<Block>(b).unwrap().unwrap().meta).unwrap())
                .collect();
            (tab.name, metas)
        })
        .collect()
}

#[tokio::test]
async fn a_cold_start_with_nothing_to_restore_offers_until_the_default_is_used() {
    let state = test_state();
    let win = cold_start(&state).await;
    let tab_id = untouched_default_tab(&state.mstore, &win.workspaceid).expect("the seeded default is offered over");

    // Choosing an agent in the default agent pane makes it the user's layout.
    let tab = state.mstore.get::<Tab>(&tab_id).unwrap().unwrap();
    let agent_pane = tab
        .blockids
        .iter()
        .find(|b| state.mstore.get::<Block>(b).unwrap().unwrap().meta.get("view") == Some(&json!("agent")))
        .unwrap();
    let mut block = state.mstore.get::<Block>(agent_pane).unwrap().unwrap();
    block.meta.insert("agentId".into(), json!("a1"));
    state.mstore.update(&mut block).unwrap();
    assert!(untouched_default_tab(&state.mstore, &win.workspaceid).is_none());
}

#[tokio::test]
async fn a_cold_start_that_restored_its_own_layout_offers_nothing() {
    let state = test_state();
    save_last_session_snapshot(&state.mstore, work_record(None));
    let win = cold_start(&state).await;
    assert!(untouched_default_tab(&state.mstore, &win.workspaceid).is_none());
}

/// The layout comes back the same whether this build saved it or another did.
#[tokio::test]
async fn restoring_another_builds_layout_matches_a_same_build_restore() {
    // The layout, as a real workspace saves it.
    let source = test_state();
    let ws_id = dispatch_apply(&source, Command::CreateWorkspace { name: "w".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let (_, replayed) = replay_tabs(&source, &ws_id, &replay_tabs_of(&work_record(None)), "test").await;
    let second = replayed[1].as_ref().unwrap().tab_id.clone();
    dispatch_apply(&source, Command::SetActiveTab { workspace_id: ws_id.clone(), tab_id: second }).await;
    let record = snapshot_workspace(&source.mstore, &ws_id).expect("a snapshot of the source");
    assert_eq!(record["active_tab_index"], 1);

    // Same build: the record restored where it was saved.
    let same = test_state();
    save_last_session_snapshot(&same.mstore, record.clone());
    let (same_ws, _) = restore_last_session(&same).await.unwrap().unwrap();

    // Another build: a cold start with nothing of its own, then the offer.
    let dir = tempfile::tempdir().unwrap();
    build(dir.path(), "local-old", "0.59.32", Some(record.clone()));
    let here = test_state();
    let win = cold_start(&here).await;
    let out = restore(&here, &roots(dir.path(), None), &win.oid, "local-old", "0.59.32").await.unwrap();

    assert!(out.replaced_default, "the untouched default tab is replaced");
    assert_eq!(out.workspace_id, win.workspaceid);
    assert_eq!(out.other_windows, 0);
    assert_eq!(tabs_of(&here, &win.workspaceid), tabs_of(&same, &same_ws));
    let ws = here.mstore.get::<Workspace>(&win.workspaceid).unwrap().unwrap();
    assert_eq!(ws.activetabid, out.tab_ids[1], "the saved active tab is in front");
    assert!(untouched_default_tab(&here.mstore, &win.workspaceid).is_none(), "the offer is answered");
}

#[tokio::test]
async fn a_used_window_keeps_its_tabs_and_gets_the_layout_beside_them() {
    let dir = tempfile::tempdir().unwrap();
    build(dir.path(), "local-old", "0.59.32", Some(work_record(Some(1_000))));
    let here = test_state();
    let win = cold_start(&here).await;
    let before = tabs_of(&here, &win.workspaceid);
    // The user opened a tab of their own.
    dispatch_apply(&here, Command::CreateTab { workspace_id: win.workspaceid.clone(), name: "mine".into() }).await;

    let out = restore(&here, &roots(dir.path(), None), &win.oid, "local-old", "0.59.32").await.unwrap();
    assert!(!out.replaced_default);
    let after = tabs_of(&here, &win.workspaceid);
    assert_eq!(after.len(), before.len() + 1 + 2);
    assert_eq!(after[0], before[0], "what was open stays");
    assert_eq!(after[2].0, "Tab 1");
    assert_eq!(after[3].1[0]["agentId"], "a2", "agent panes come back as the same agents");
    assert_eq!(after[3].1[0]["agent:sessionid"], "s-a2");
}

#[tokio::test]
async fn the_other_windows_of_a_quits_record_wait_to_be_opened() {
    let mut record = work_record(Some(1_000));
    let second = json!({ "tabs": [tab("side", &[json!({ "view": "browser" })])], "window": { "x": 10, "y": 20, "width": 800, "height": 600, "name": "Side" } });
    record["windows"] = json!([work_record(Some(1_000)), second]);
    let dir = tempfile::tempdir().unwrap();
    build(dir.path(), "local-old", "0.59.32", Some(record));
    let here = test_state();
    let win = cold_start(&here).await;

    let out = restore(&here, &roots(dir.path(), None), &win.oid, "local-old", "0.59.32").await.unwrap();
    assert_eq!(out.other_windows, 1);
    let ret = super::super::session_windows::handle_take_restored_windows(
        &here,
        &WebCallType { service: "window".into(), method: "TakeRestoredWindows".into(), uicontext: None, args: vec![] },
    );
    let waiting = ret.data.unwrap();
    assert_eq!(waiting.as_array().unwrap().len(), 1);
    assert_eq!(waiting[0]["x"], 10);
    let ws = waiting[0]["workspaceid"].as_str().unwrap();
    assert_eq!(tabs_of(&here, ws)[0].0, "side");
}

#[tokio::test]
async fn restoring_a_build_without_a_record_fails_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    build(dir.path(), "empty", "0.59.32", None);
    let here = test_state();
    let win = cold_start(&here).await;
    let before = tabs_of(&here, &win.workspaceid);
    assert!(restore(&here, &roots(dir.path(), None), &win.oid, "empty", "0.59.32").await.is_err());
    assert_eq!(tabs_of(&here, &win.workspaceid), before);
}
