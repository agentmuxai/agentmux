// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;

use agentmux_common::ipc::Command;
use serde_json::json;

use super::*;
use crate::backend::obj::{Block, Point, Tab, WinSize, Window, Workspace};
use crate::server::service::reducer_helpers::dispatch_to_reducer;
use crate::server::service::session_restore::{load_last_session_snapshot, save_last_session_snapshot};
use crate::server::service::window::handle_window_service;
use crate::server::tests::test_state;

async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
    let events = dispatch_to_reducer(state, cmd).await;
    for ev in &events {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
    }
    events
}

/// Only the windows a test adds count: the bootstrapped one is dropped.
fn no_windows(state: &AppState) {
    let mut client = state.mstore.get_all::<Client>().unwrap().into_iter().next().unwrap();
    client.windowids.clear();
    state.mstore.update(&mut client).unwrap();
}

/// A window labelled `label`, its workspace holding one tab with one `view`
/// pane. Returns the window's and the workspace's ids.
async fn window(state: &AppState, label: &str, view: &str) -> (String, String) {
    let ws_id = dispatch_apply(state, Command::CreateWorkspace { name: String::new() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let tab_id = dispatch_apply(state, Command::CreateTab { workspace_id: ws_id.clone(), name: format!("{view} tab") })
        .await
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();
    dispatch_apply(state, Command::CreateBlock { tab_id, meta: json!({ "view": view }) }).await;
    let mut w = Window {
        oid: format!("win-{label}"),
        workspaceid: ws_id.clone(),
        pos: Point { x: 0, y: 0 },
        winsize: WinSize { width: 800, height: 600 },
        ..Default::default()
    };
    w.meta.insert("host:label".into(), json!(label));
    state.mstore.insert(&mut w).unwrap();
    let mut client = state.mstore.get_all::<Client>().unwrap().into_iter().next().unwrap();
    client.windowids.push(w.oid.clone());
    state.mstore.update(&mut client).unwrap();
    (w.oid, ws_id)
}

/// The views of the panes in `ws_id`'s tabs, in order.
fn views(state: &AppState, ws_id: &str) -> Vec<String> {
    let ws = state.mstore.get::<Workspace>(ws_id).unwrap().unwrap();
    ws.pinnedtabids
        .iter()
        .chain(ws.tabids.iter())
        .filter_map(|t| state.mstore.get::<Tab>(t).unwrap())
        .flat_map(|t| t.blockids)
        .filter_map(|b| state.mstore.get::<Block>(&b).unwrap())
        .filter_map(|b| b.meta.get("view").and_then(|v| v.as_str()).map(str::to_string))
        .collect()
}

/// A one-window record holding one tab of `view` panes, saved at `at`.
fn record(views: &[&str], at: i64) -> Value {
    let tab = json!({
        "name": "t",
        "blocks": views.iter().map(|v| json!({ "meta": { "view": v } })).collect::<Vec<_>>(),
        "rootnode": null,
    });
    json!({ "tabs": [tab.clone()], "windows": [{ "tabs": [tab] }], "saved_at_ms": at })
}

fn row(state: &AppState) -> UserWorkspaceRow {
    let id = current_id(&state.mstore).expect("a workspace is open");
    state.mstore.user_workspace_get(&id).unwrap().unwrap()
}

/// A state with Default Workspace open, as srv's start leaves it.
fn state_with_workspace() -> AppState {
    let state = test_state();
    let decided = prepare_at_startup(&state.mstore, None);
    adopt_current(&state.mstore, decided);
    state
}

fn cold_start_call() -> WebCallType {
    WebCallType {
        service: "window".into(),
        method: "CreateWindow".into(),
        uicontext: None,
        args: vec![Value::Null, json!(""), json!("main"), json!(true)],
    }
}

/// The cold start a window makes when `Client.windowids` is empty; returns
/// the new window's workspace.
async fn cold_start(state: &AppState) -> String {
    let ret = handle_window_service(state, &cold_start_call()).await;
    assert!(ret.success, "CreateWindow failed: {:?}", ret.error);
    ret.data.unwrap()["workspaceid"].as_str().unwrap().to_string()
}

fn client_meta(state: &AppState, key: &str) -> Option<Value> {
    client(&state.mstore).and_then(|c| c.meta.get(key).cloned())
}

// ---- When to save ----

#[test]
fn a_change_is_saved_a_second_after_the_last_one() {
    let p = Pending::changed(None, 1_000);
    assert!(!p.due(1_999));
    assert!(p.due(2_000));
    // Another change pushes it back.
    let p = Pending::changed(Some(p), 1_800);
    assert_eq!(p, Pending { first_ms: 1_000, last_ms: 1_800 });
    assert!(!p.due(2_500));
    assert!(p.due(2_800));
}

#[test]
fn changes_that_keep_coming_are_saved_every_fifteen_seconds() {
    let mut p = Pending::changed(None, 0);
    let mut now = 0;
    // A change every half second, as a long drag-resize makes.
    while now < CEILING_MS - 500 {
        now += 500;
        p = Pending::changed(Some(p), now);
        assert!(!p.due(now), "not before the ceiling ({now} ms)");
    }
    now += 500;
    p = Pending::changed(Some(p), now);
    assert!(p.due(now), "at the ceiling, even with a change just now");
}

// ---- Startup ----

#[tokio::test]
async fn the_first_start_opens_default_workspace() {
    let state = state_with_workspace();
    let rows = state.mstore.user_workspace_list().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, DEFAULT_NAME);
    assert!(rows[0].opened_at.is_some());
    assert_eq!(client_meta(&state, CURRENT_META_KEY), Some(json!(rows[0].id)));
    assert_eq!(client_meta(&state, NAME_META_KEY), Some(json!(DEFAULT_NAME)));

    // The next start keeps it.
    let decided = prepare_at_startup(&state.mstore, None);
    assert_eq!(decided, None);
    adopt_current(&state.mstore, decided);
    assert_eq!(state.mstore.user_workspace_list().unwrap().len(), 1);
    assert_eq!(client_meta(&state, CURRENT_META_KEY), Some(json!(rows[0].id)));
}

#[test]
fn names_are_trimmed_and_capped() {
    assert_eq!(clean_name("  Reviews "), Some("Reviews".to_string()));
    assert_eq!(clean_name("   "), None);
    assert_eq!(clean_name(&"x".repeat(100)).unwrap().chars().count(), NAME_MAX_CHARS);
}

#[tokio::test]
async fn rename_changes_the_row_and_the_name_every_window_shows() {
    let state = state_with_workspace();
    let call = |name: &str| WebCallType {
        service: "workspace".into(),
        method: "RenameUserWorkspace".into(),
        uicontext: None,
        args: vec![json!(""), json!(name)],
    };
    let ret = handle_rename(&state, &call("  Reviews  "));
    assert!(ret.success, "{:?}", ret.error);
    let updates = ret.updates.expect("the client goes back as an update");
    assert_eq!(updates[0].otype, "client");
    assert_eq!(updates[0].obj.as_ref().unwrap()["meta"][NAME_META_KEY], "Reviews");
    assert_eq!(row(&state).name, "Reviews");
    assert_eq!(client_meta(&state, NAME_META_KEY), Some(json!("Reviews")));

    let blank = handle_rename(&state, &call("   "));
    assert!(!blank.success, "a blank name is refused");
    assert_eq!(row(&state).name, "Reviews");
}

// ---- Saving ----

#[tokio::test]
async fn a_save_stores_every_window_in_the_session_records_shape() {
    let state = state_with_workspace();
    no_windows(&state);
    window(&state, "main", "agent").await;
    window(&state, "window-2", "sysinfo").await;

    assert_eq!(save_now(&state.mstore), Saved::Written);
    let saved: Value = serde_json::from_str(&row(&state).record.unwrap()).unwrap();
    assert_eq!(windows_of(&saved).len(), 2);
    assert!(saved["saved_at_ms"].as_i64().is_some());
    assert!(saved.get("saved_for").is_none());

    // Nothing changed: nothing written, the time it was saved included.
    assert_eq!(save_now(&state.mstore), Saved::Unchanged);
    let again: Value = serde_json::from_str(&row(&state).record.unwrap()).unwrap();
    assert_eq!(again["saved_at_ms"], saved["saved_at_ms"]);
}

#[tokio::test]
async fn with_no_window_open_the_record_is_kept() {
    let state = state_with_workspace();
    no_windows(&state);
    window(&state, "main", "agent").await;
    assert_eq!(save_now(&state.mstore), Saved::Written);
    let before = row(&state).record;

    no_windows(&state);
    assert_eq!(save_now(&state.mstore), Saved::NoWindow);
    assert_eq!(row(&state).record, before);
}

#[tokio::test]
async fn store_writes_mark_the_workspace_unsaved_and_a_save_clears_it() {
    let state = state_with_workspace();
    no_windows(&state);
    let (_, ws) = window(&state, "main", "agent").await;
    assert!(state.mstore.layout_writes().dirty());
    save_now(&state.mstore);
    assert!(!state.mstore.layout_writes().dirty());

    // A tab added through the reducer is a change.
    dispatch_apply(&state, Command::CreateTab { workspace_id: ws, name: "second".into() }).await;
    assert!(state.mstore.layout_writes().dirty());
    // The client's own meta is not.
    save_now(&state.mstore);
    set_client_meta(&state.mstore, "some:key", Some(json!(1)));
    assert!(!state.mstore.layout_writes().dirty());
}

#[tokio::test]
async fn flush_saves_only_when_something_changed() {
    let state = state_with_workspace();
    no_windows(&state);
    window(&state, "main", "agent").await;
    let call = WebCallType { service: "workspace".into(), method: "Flush".into(), uicontext: None, args: vec![] };

    let ret = handle_flush(&state, &call).await;
    assert_eq!(ret.data.unwrap()["saved"], true);
    let ret = handle_flush(&state, &call).await;
    assert_eq!(ret.data.unwrap()["saved"], false, "nothing changed since");
}

#[tokio::test]
async fn a_quit_holds_saves_until_its_windows_have_closed() {
    let state = state_with_workspace();
    no_windows(&state);
    window(&state, "main", "agent").await;
    let (second, _) = window(&state, "window-2", "sysinfo").await;

    // The quit saves both windows, in the session record and the workspace.
    let quit = WebCallType {
        service: "window".into(),
        method: "SaveSessionSnapshot".into(),
        uicontext: None,
        args: vec![json!(true)],
    };
    assert!(handle_window_service(&state, &quit).await.success);
    let saved: Value = serde_json::from_str(&row(&state).record.unwrap()).unwrap();
    assert_eq!(windows_of(&saved).len(), 2);
    assert_eq!(saved["saved_for"], "quit");

    // Its first close leaves one window open; a save now would keep only it.
    let mut client = client(&state.mstore).unwrap();
    client.windowids.retain(|w| w != &second);
    state.mstore.update(&mut client).unwrap();
    assert_eq!(save_now(&state.mstore), Saved::HeldForQuit);
    let still: Value = serde_json::from_str(&row(&state).record.unwrap()).unwrap();
    assert_eq!(windows_of(&still).len(), 2);
}

#[tokio::test]
async fn the_last_close_saves_the_workspace_too() {
    let state = state_with_workspace();
    no_windows(&state);
    let (_, ws) = window(&state, "main", "term").await;
    super::super::session_windows::save_on_last_close(&state.mstore, &ws, Some("Main"), &Default::default());
    let saved: Value = serde_json::from_str(&row(&state).record.unwrap()).unwrap();
    assert_eq!(windows_of(&saved)[0]["window"]["name"], "Main");
    assert_eq!(load_last_session_snapshot(&state.mstore).unwrap()["saved_at_ms"], saved["saved_at_ms"]);
}

// ---- Loading ----

fn ws_row(record: Option<&str>, previous: Option<&str>, pre_upgrade: Option<&str>) -> UserWorkspaceRow {
    UserWorkspaceRow {
        id: "w".into(),
        name: "W".into(),
        record: record.map(str::to_string),
        previous: previous.map(str::to_string),
        pre_upgrade: pre_upgrade.map(str::to_string),
        ..Default::default()
    }
}

fn order(candidates: &[(LoadedFrom, Value)]) -> Vec<LoadedFrom> {
    candidates.iter().map(|(f, _)| *f).collect()
}

#[test]
fn the_newer_of_the_session_record_and_the_workspace_loads_first() {
    let newer = record(&["agent"], 2_000).to_string();
    let w = ws_row(Some(&newer), Some("{}"), Some("{}"));
    let c = launch_candidates(Some(record(&["term"], 1_000)), Some(&w));
    use LoadedFrom::*;
    assert_eq!(order(&c), [Record, Session, Previous, PreUpgrade]);

    let c = launch_candidates(Some(record(&["term"], 3_000)), Some(&w));
    assert_eq!(order(&c), [Session, Record, Previous, PreUpgrade]);

    // A quit writes both with the same time: the session record first.
    let c = launch_candidates(Some(record(&["term"], 2_000)), Some(&w));
    assert_eq!(order(&c)[0], Session);

    // Only one of them.
    assert_eq!(order(&launch_candidates(None, Some(&ws_row(Some(&newer), None, None)))), [Record]);
    assert_eq!(order(&launch_candidates(Some(record(&["term"], 1)), None)), [Session]);
    assert!(launch_candidates(None, None).is_empty());
}

#[test]
fn a_copy_that_doesnt_parse_is_skipped() {
    let good = record(&["agent"], 1).to_string();
    let w = ws_row(Some("{not json"), Some("[1, 2]"), Some(&good));
    assert_eq!(order(&launch_candidates(None, Some(&w))), [LoadedFrom::PreUpgrade]);
}

#[tokio::test]
async fn a_cold_start_restores_the_workspace_record_when_there_is_no_session_record() {
    let state = state_with_workspace();
    let id = current_id(&state.mstore).unwrap();
    let saved = record(&["term", "sysinfo"], 5_000);
    state.mstore.user_workspace_save_record(&id, &saved.to_string(), 5_000).unwrap();
    no_windows(&state);
    assert!(load_last_session_snapshot(&state.mstore).is_none());

    let ws = cold_start(&state).await;
    assert_eq!(views(&state, &ws), ["term", "sysinfo"]);
    // It loaded cleanly: it is the workspace's last clean load now, and no notice.
    assert_eq!(serde_json::from_str::<Value>(&row(&state).previous.unwrap()).unwrap(), saved);
    assert_eq!(client_meta(&state, RESTORED_FROM_META_KEY), None);
}

#[tokio::test]
async fn a_cold_start_prefers_the_newer_record() {
    let state = state_with_workspace();
    let id = current_id(&state.mstore).unwrap();
    // A crash after the last close: the workspace saved since.
    save_last_session_snapshot(&state.mstore, record(&["agent"], 1_000));
    state.mstore.user_workspace_save_record(&id, &record(&["term"], 9_000).to_string(), 9_000).unwrap();
    no_windows(&state);

    let ws = cold_start(&state).await;
    assert_eq!(views(&state, &ws), ["term"]);
}

#[tokio::test]
async fn a_record_that_doesnt_load_falls_back_to_the_last_clean_load_and_says_so() {
    let state = state_with_workspace();
    let id = current_id(&state.mstore).unwrap();
    state.mstore.user_workspace_save_record(&id, "{\"windows\": [trunc", 9_000).unwrap();
    state.mstore.user_workspace_set_previous(&id, &record(&["agent", "swarm"], 1_000).to_string()).unwrap();
    no_windows(&state);

    let ws = cold_start(&state).await;
    assert_eq!(views(&state, &ws), ["agent", "swarm"]);
    assert_eq!(client_meta(&state, RESTORED_FROM_META_KEY), Some(json!("previous")));
}

#[tokio::test]
async fn a_record_with_nothing_to_restore_falls_back_to_the_pre_upgrade_copy() {
    let state = state_with_workspace();
    let id = current_id(&state.mstore).unwrap();
    state.mstore.user_workspace_save_record(&id, "{\"tabs\": []}", 9_000).unwrap();
    let mut w = row(&state);
    w.pre_upgrade = Some(record(&["sysinfo"], 1).to_string());
    // Only an upgrade copy writes `pre_upgrade`: put the row back with it.
    state.mstore.conn().lock().unwrap().execute("DELETE FROM db_workspaces", []).unwrap();
    state.mstore.user_workspace_insert(&w).unwrap();
    no_windows(&state);

    let ws = cold_start(&state).await;
    assert_eq!(views(&state, &ws), ["sysinfo"]);
    assert_eq!(client_meta(&state, RESTORED_FROM_META_KEY), Some(json!("pre_upgrade")));
}

#[tokio::test]
async fn with_nothing_saved_a_cold_start_seeds_the_default() {
    let state = state_with_workspace();
    no_windows(&state);
    let ws = cold_start(&state).await;
    assert_eq!(views(&state, &ws), ["agent", "swarm", "sysinfo"]);
}

// ---- A new version's first start ----

/// Version `version` of channel `channel` under `root`, its database holding
/// `rows` with `current` open.
fn version_db(root: &Path, channel: &str, version: &str, rows: &[UserWorkspaceRow], current: Option<&str>) -> PathBuf {
    let db_dir = root.join(channel).join("versions").join(version).join("data").join("db");
    std::fs::create_dir_all(&db_dir).unwrap();
    let path = db_dir.join("objects.db");
    let store = Store::open(&path).unwrap();
    let mut client = Client { oid: format!("client-{version}"), ..Default::default() };
    if let Some(current) = current {
        client.meta.insert(CURRENT_META_KEY.into(), json!(current));
    }
    store.insert(&mut client).unwrap();
    for w in rows {
        store.user_workspace_insert(w).unwrap();
    }
    drop(store);
    path
}

fn own_dir(root: &Path, channel: &str, version: &str) -> PathBuf {
    let dir = root.join(channel).join("versions").join(version).join("data").join("db");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn saved_row(id: &str, name: &str, rec: Option<&Value>, opened_at: i64) -> UserWorkspaceRow {
    UserWorkspaceRow {
        id: id.into(),
        name: name.into(),
        record: rec.map(Value::to_string),
        created_at: 1,
        updated_at: 1,
        opened_at: Some(opened_at),
        ..Default::default()
    }
}

#[test]
fn the_newest_other_version_of_this_channel_is_the_one_copied_from() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let older = version_db(root, "stable", "0.1.0", &[], None);
    std::thread::sleep(std::time::Duration::from_millis(20));
    let newer = version_db(root, "stable", "0.2.0", &[], None);
    std::thread::sleep(std::time::Duration::from_millis(20));
    // Newer still, but another channel's.
    version_db(root, "beta", "0.9.0", &[], None);
    let own = own_dir(root, "stable", "0.3.0");
    let roots = ScanRoots { channels_root: root.to_path_buf(), own_db_dir: Some(own) };
    assert_eq!(newest_other_version_db(&roots), Some(newer));
    assert_ne!(Some(older), newest_other_version_db(&roots));

    // A database outside the channels (a dev build's) copies nothing.
    let dev = tmp.path().join("dev").join("data").join("db");
    std::fs::create_dir_all(&dev).unwrap();
    let roots = ScanRoots { channels_root: root.to_path_buf(), own_db_dir: Some(dev) };
    assert_eq!(newest_other_version_db(&roots), None);
}

#[test]
fn a_version_without_workspaces_has_nothing_to_copy() {
    let tmp = tempfile::tempdir().unwrap();
    version_db(tmp.path(), "stable", "0.1.0", &[], None);
    let roots = ScanRoots {
        channels_root: tmp.path().to_path_buf(),
        own_db_dir: Some(own_dir(tmp.path(), "stable", "0.2.0")),
    };
    let store = Store::open_in_memory().unwrap();
    assert!(prepare_at_startup(&store, Some(&roots)).is_some(), "Default Workspace instead");
    let rows = store.user_workspace_list().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, DEFAULT_NAME);
}

#[tokio::test]
async fn a_new_versions_first_start_copies_the_workspaces_and_opens_where_the_last_left_off() {
    let tmp = tempfile::tempdir().unwrap();
    let work = record(&["term", "sysinfo"], 7_000);
    let other = record(&["agent"], 6_000);
    let old_db = version_db(
        tmp.path(),
        "stable",
        "0.1.0",
        &[saved_row("ws-work", "Work", Some(&work), 10), saved_row("ws-other", "Other", Some(&other), 20)],
        Some("ws-work"),
    );
    let before = std::fs::read(&old_db).unwrap();
    let roots = ScanRoots {
        channels_root: tmp.path().to_path_buf(),
        own_db_dir: Some(own_dir(tmp.path(), "stable", "0.2.0")),
    };

    // A fresh store: the new version's first start.
    let store = Arc::new(Store::open_in_memory().unwrap());
    let decided = prepare_at_startup(&store, Some(&roots));
    assert_eq!(decided.as_deref(), Some("ws-work"), "the one the previous version had open");
    let copied = store.user_workspace_get("ws-work").unwrap().unwrap();
    assert_eq!((copied.name.as_str(), copied.record.as_deref()), ("Work", Some(work.to_string().as_str())));
    assert_eq!(copied.pre_upgrade, copied.record, "kept as the pre-upgrade copy");
    assert!(store.user_workspace_get("ws-other").unwrap().is_some());
    // Only the client: no default window to seed, and none for another build to offer over.
    assert!(!crate::backend::wcore::ensure_initial_data(&store).unwrap(), "not a first launch any more");
    assert!(client(&store).unwrap().windowids.is_empty());
    adopt_current(&store, decided);
    assert_eq!(client(&store).unwrap().meta.get(NAME_META_KEY), Some(&json!("Work")));
    assert_eq!(std::fs::read(&old_db).unwrap(), before, "the previous version's database is only read");
}

#[tokio::test]
async fn a_copied_workspace_restores_at_the_first_cold_start_and_no_other_build_is_offered() {
    let tmp = tempfile::tempdir().unwrap();
    let work = record(&["term", "sysinfo"], 7_000);
    version_db(tmp.path(), "stable", "0.1.0", &[saved_row("ws-work", "Work", Some(&work), 10)], Some("ws-work"));
    let roots = ScanRoots {
        channels_root: tmp.path().to_path_buf(),
        own_db_dir: Some(own_dir(tmp.path(), "stable", "0.2.0")),
    };
    let state = test_state();
    no_windows(&state);
    let decided = copy_from_previous_version(&state.mstore, &roots);
    adopt_current(&state.mstore, decided);

    let ws = cold_start(&state).await;
    assert_eq!(views(&state, &ws), ["term", "sysinfo"]);
    assert!(
        crate::server::service::session_other_builds::untouched_default_tab(&state.mstore, &ws).is_none(),
        "nothing was seeded, so nothing is offered over it"
    );
    assert_eq!(client_meta(&state, RESTORED_FROM_META_KEY), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_saver_saves_the_start_and_then_each_change() {
    let state = state_with_workspace();
    no_windows(&state);
    let (_, ws) = window(&state, "main", "agent").await;
    spawn_saver(Arc::clone(&state.mstore));

    let saved_tabs = |state: &AppState| -> usize {
        row(state)
            .record
            .and_then(|r| serde_json::from_str::<Value>(&r).ok())
            .map_or(0, |r| windows_of(&r)[0]["tabs"].as_array().map_or(0, Vec::len))
    };
    let wait_for = |want: usize| {
        let state = state.clone();
        async move {
            for _ in 0..40 {
                if saved_tabs(&state) == want {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            false
        }
    };
    assert!(wait_for(1).await, "the state it starts with is saved");
    let tab_id = dispatch_apply(&state, Command::CreateTab { workspace_id: ws, name: "second".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();
    dispatch_apply(&state, Command::CreateBlock { tab_id, meta: json!({ "view": "term" }) }).await;
    assert!(wait_for(2).await, "a change is saved about a second later");
}
