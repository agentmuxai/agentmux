// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use agentmux_common::ipc::Command;
use serde_json::json;

use super::*;
use crate::backend::obj::{Block, Point, Tab, WinSize};
use crate::server::service::reducer_helpers::dispatch_to_reducer;
use crate::server::service::session_restore::{
    handle_save_session_snapshot, load_last_session_snapshot, restore_last_session, save_last_session_snapshot,
};
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

/// A window labelled `label` at `(x, y)`, its workspace holding one tab with
/// one `view` pane. Returns the window's and the workspace's ids.
async fn window(state: &AppState, label: &str, view: &str, x: i64, y: i64) -> (String, String) {
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
        pos: Point { x, y },
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

fn entry_views(entry: &Value) -> Vec<String> {
    entry["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["blocks"].as_array().unwrap().clone())
        .map(|b| b["meta"]["view"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn a_quit_saves_every_window_the_main_one_first() {
    let state = test_state();
    no_windows(&state);
    window(&state, "window-2", "sysinfo", 10, 20).await;
    window(&state, "main", "agent", 0, 0).await;

    let record = snapshot_session(&state.mstore, true).unwrap();
    let windows = windows_of(&record);
    assert_eq!(windows.len(), 2);
    assert_eq!(entry_views(&windows[0]), ["agent"], "the main window first");
    assert_eq!(entry_views(&windows[1]), ["sysinfo"]);
    assert_eq!(windows[1]["window"], json!({ "x": 10, "y": 20, "width": 800, "height": 600 }));
    // The main window's tabs stay at the top level, where an older AgentMux reads them.
    assert_eq!(entry_views(&record), ["agent"]);
    assert!(record.get("window").is_none());
    assert_eq!(record["saved_for"], "quit");
}

#[tokio::test]
async fn subwindows_and_windows_sharing_a_workspace_are_not_saved_twice() {
    let state = test_state();
    no_windows(&state);
    let (_, ws) = window(&state, "main", "agent", 0, 0).await;
    let (sub_id, _) = window(&state, "sub", "term", 0, 0).await;
    let mut sub = state.mstore.get::<Window>(&sub_id).unwrap().unwrap();
    sub.kind = Some("subwindow".into());
    state.mstore.update(&mut sub).unwrap();
    // A second window onto the main window's workspace.
    let mut twin = Window { oid: "win-twin".into(), workspaceid: ws, ..Default::default() };
    state.mstore.insert(&mut twin).unwrap();
    let mut client = state.mstore.get_all::<Client>().unwrap().into_iter().next().unwrap();
    client.windowids.push(twin.oid.clone());
    client.windowids.push("no-such-window".into());
    state.mstore.update(&mut client).unwrap();

    let record = snapshot_session(&state.mstore, false).unwrap();
    assert_eq!(windows_of(&record).len(), 1);
    assert!(record.get("saved_for").is_none(), "only a quit's record is marked");
}

#[tokio::test]
async fn a_record_from_before_reads_as_one_window() {
    let old = json!({ "tabs": [{ "name": "t", "blocks": [] }], "active_tab_index": 0 });
    assert_eq!(windows_of(&old), vec![old.clone()]);
}

#[tokio::test]
async fn the_last_close_of_a_quit_keeps_the_quits_record() {
    let state = test_state();
    no_windows(&state);
    window(&state, "main", "agent", 0, 0).await;
    let (_, last_ws) = window(&state, "window-2", "sysinfo", 10, 20).await;
    save_last_session_snapshot(&state.mstore, snapshot_session(&state.mstore, true).unwrap());

    save_on_last_close(&state.mstore, &last_ws);
    let kept = load_last_session_snapshot(&state.mstore).unwrap();
    assert_eq!(windows_of(&kept).len(), 2, "the quit's record, not the last window alone");

    // A quit that stalled for minutes was abandoned: the close saves as usual.
    let mut stale = kept;
    stale["saved_at_ms"] = json!(agentmux_common::time::now_ms() - QUIT_RECORD_KEPT_FOR_MS - 1);
    save_last_session_snapshot(&state.mstore, stale);
    save_on_last_close(&state.mstore, &last_ws);
    let replaced = load_last_session_snapshot(&state.mstore).unwrap();
    assert_eq!(entry_views(&replaced), ["sysinfo"]);
    assert!(replaced.get("windows").is_none());

    // Nor does a record that wasn't a quit's hold the close back.
    save_last_session_snapshot(&state.mstore, snapshot_session(&state.mstore, false).unwrap());
    save_on_last_close(&state.mstore, &last_ws);
    assert!(load_last_session_snapshot(&state.mstore).unwrap().get("windows").is_none());
}

#[tokio::test]
async fn the_quit_flush_marks_its_record() {
    let state = test_state();
    no_windows(&state);
    window(&state, "main", "agent", 0, 0).await;
    let call = |args| WebCallType { service: "window".into(), method: "SaveSessionSnapshot".into(), args, uicontext: None };

    assert!(handle_save_session_snapshot(&state, &call(vec![])).await.error.is_none());
    assert!(load_last_session_snapshot(&state.mstore).unwrap().get("saved_for").is_none());
    assert!(handle_save_session_snapshot(&state, &call(vec![json!(true)])).await.error.is_none());
    assert_eq!(load_last_session_snapshot(&state.mstore).unwrap()["saved_for"], "quit");
}

#[tokio::test]
async fn restore_brings_back_every_window_and_hands_out_the_others_once() {
    let state = test_state();
    no_windows(&state);
    window(&state, "main", "agent", 0, 0).await;
    window(&state, "window-2", "sysinfo", 10, 20).await;
    save_last_session_snapshot(&state.mstore, snapshot_session(&state.mstore, true).unwrap());
    no_windows(&state);

    let (main_ws, _) = restore_last_session(&state).await.unwrap().unwrap();
    assert_eq!(views(&state, &main_ws), ["agent"]);

    let take = WebCallType { service: "window".into(), method: "TakeRestoredWindows".into(), args: vec![], uicontext: None };
    let others = handle_take_restored_windows(&state, &take).data.unwrap();
    let others = others.as_array().unwrap();
    assert_eq!(others.len(), 1);
    let ws = others[0]["workspaceid"].as_str().unwrap();
    assert_ne!(ws, main_ws);
    assert_eq!(views(&state, ws), ["sysinfo"]);
    assert_eq!((others[0]["x"].as_i64(), others[0]["width"].as_i64()), (Some(10), Some(800)));

    let again = handle_take_restored_windows(&state, &take).data.unwrap();
    assert_eq!(again, json!([]), "each window is handed out once");
}

#[tokio::test]
async fn a_restored_window_whose_workspace_is_gone_is_not_handed_out() {
    let state = test_state();
    no_windows(&state);
    window(&state, "main", "agent", 0, 0).await;
    window(&state, "window-2", "sysinfo", 10, 20).await;
    save_last_session_snapshot(&state.mstore, snapshot_session(&state.mstore, true).unwrap());
    no_windows(&state);
    restore_last_session(&state).await.unwrap().unwrap();
    let mut client = state.mstore.get_all::<Client>().unwrap().into_iter().next().unwrap();
    let waiting = client.meta[WAITING_META_KEY].clone();
    let gone = waiting[0]["workspaceid"].as_str().unwrap().to_string();
    state.mstore.delete::<Workspace>(&gone).unwrap();
    client = state.mstore.get_all::<Client>().unwrap().into_iter().next().unwrap();
    assert!(client.meta.contains_key(WAITING_META_KEY));

    let take = WebCallType { service: "window".into(), method: "TakeRestoredWindows".into(), args: vec![], uicontext: None };
    assert_eq!(handle_take_restored_windows(&state, &take).data.unwrap(), json!([]));
}

#[tokio::test]
async fn a_window_a_quit_left_behind_is_forgotten_at_startup_and_restored() {
    let state = test_state();
    no_windows(&state);
    window(&state, "main", "agent", 0, 0).await;
    let (left_id, left_ws) = window(&state, "window-2", "memory", 10, 20).await;
    save_last_session_snapshot(&state.mstore, snapshot_session(&state.mstore, true).unwrap());
    // The quit closed the main window; the second one's close never arrived.
    let mut client = state.mstore.get_all::<Client>().unwrap().into_iter().next().unwrap();
    client.windowids.retain(|id| *id == left_id);
    state.mstore.update(&mut client).unwrap();

    assert_eq!(forget_windows_left_by_quit(&state.mstore), 1);
    let client = state.mstore.get_all::<Client>().unwrap().into_iter().next().unwrap();
    assert!(client.windowids.is_empty());
    assert!(state.mstore.get::<Window>(&left_id).unwrap().is_none());
    assert!(state.mstore.get::<Workspace>(&left_ws).unwrap().is_none(), "its workspace isn't left behind");
    // Both windows come back from the record.
    let (main_ws, _) = restore_last_session(&state).await.unwrap().unwrap();
    assert_eq!(views(&state, &main_ws), ["agent"]);
    let take = WebCallType { service: "window".into(), method: "TakeRestoredWindows".into(), args: vec![], uicontext: None };
    let others = handle_take_restored_windows(&state, &take).data.unwrap();
    assert_eq!(views(&state, others[0]["workspaceid"].as_str().unwrap()), ["memory"]);
}

#[tokio::test]
async fn windows_are_kept_at_startup_unless_a_quit_saved_them() {
    let state = test_state();
    no_windows(&state);
    let (id, _) = window(&state, "main", "agent", 0, 0).await;
    // No record (a crash): the windows are what the crash recovery reopens.
    assert_eq!(forget_windows_left_by_quit(&state.mstore), 0);
    // A record the last close saved (not a quit's) doesn't count either.
    save_last_session_snapshot(&state.mstore, snapshot_session(&state.mstore, false).unwrap());
    assert_eq!(forget_windows_left_by_quit(&state.mstore), 0);
    let client = state.mstore.get_all::<Client>().unwrap().into_iter().next().unwrap();
    assert_eq!(client.windowids, vec![id]);
}

#[tokio::test]
async fn an_older_quits_record_never_costs_a_later_run_its_windows() {
    let state = test_state();
    no_windows(&state);
    window(&state, "main", "agent", 0, 0).await;
    save_last_session_snapshot(&state.mstore, snapshot_session(&state.mstore, true).unwrap());
    // The next run's restore never finished, so the record stayed; that run
    // has its own windows when it crashes.
    no_windows(&state);
    let (later, later_ws) = window(&state, "main-2", "memory", 0, 0).await;
    assert_eq!(forget_windows_left_by_quit(&state.mstore), 0);
    let client = state.mstore.get_all::<Client>().unwrap().into_iter().next().unwrap();
    assert_eq!(client.windowids, vec![later]);
    assert!(state.mstore.get::<Workspace>(&later_ws).unwrap().is_some());
}

/// Name `ws_id`'s workspace and window `win_id` as the user would have.
async fn name(state: &AppState, win_id: &str, ws_id: &str, window_name: &str, workspace_name: &str) {
    dispatch_apply(state, Command::RenameWorkspace { workspace_id: ws_id.into(), name: workspace_name.into() }).await;
    let mut w = state.mstore.get::<Window>(win_id).unwrap().unwrap();
    w.meta.insert("window:displayname".into(), json!(window_name));
    state.mstore.update(&mut w).unwrap();
}

#[tokio::test]
async fn windows_and_their_workspaces_come_back_with_their_names() {
    let state = test_state();
    no_windows(&state);
    let (main_id, main_ws) = window(&state, "main", "agent", 0, 0).await;
    let (other_id, other_ws) = window(&state, "window-2", "sysinfo", 10, 20).await;
    name(&state, &main_id, &main_ws, "Release work", "Upgrades").await;
    name(&state, &other_id, &other_ws, "Monitoring", "Ops").await;
    let record = snapshot_session(&state.mstore, true).unwrap();
    assert_eq!(record["windows"][0]["window"]["name"], "Release work");
    assert_eq!(record["windows"][1]["name"], "Ops");
    save_last_session_snapshot(&state.mstore, record);
    no_windows(&state);

    let (restored_main, _) = restore_last_session(&state).await.unwrap().unwrap();
    let take = WebCallType { service: "window".into(), method: "TakeRestoredWindows".into(), args: vec![], uicontext: None };
    let others = handle_take_restored_windows(&state, &take).data.unwrap();
    let restored_other = others[0]["workspaceid"].as_str().unwrap().to_string();
    let ws_name = |ws: &str| state.mstore.get::<Workspace>(ws).unwrap().unwrap().name;
    assert_eq!((ws_name(&restored_main), ws_name(&restored_other)), ("Upgrades".into(), "Ops".into()));

    // The window that opens onto each restored workspace takes its name, once.
    let create = |ws: &str| WebCallType {
        service: "window".into(),
        method: "CreateWindow".into(),
        args: vec![Value::Null, json!(ws), json!("window-x")],
        uicontext: None,
    };
    for (ws, expected) in [(&restored_other, "Monitoring"), (&restored_main, "Release work")] {
        let ret = crate::server::service::window::handle_window_service(&state, &create(ws)).await;
        assert!(ret.error.is_none(), "{:?}", ret.error);
        let win_id = ret.data.unwrap()["oid"].as_str().unwrap().to_string();
        let win = state.mstore.get::<Window>(&win_id).unwrap().unwrap();
        assert_eq!(win.meta.get("window:displayname"), Some(&json!(expected)));
    }
    assert_eq!(take_restored_window_name(&state.mstore, &restored_main), None, "handed out once");
    let client = state.mstore.get_all::<Client>().unwrap().into_iter().next().unwrap();
    assert!(!client.meta.contains_key(NAMES_META_KEY), "nothing left waiting");
}

#[tokio::test]
async fn an_unnamed_window_and_an_older_record_restore_without_names() {
    let state = test_state();
    no_windows(&state);
    window(&state, "main", "agent", 0, 0).await;
    let record = snapshot_session(&state.mstore, true).unwrap();
    assert!(record["windows"][0]["window"].get("name").is_none());
    assert!(record["windows"][0].get("name").is_none());
    save_last_session_snapshot(&state.mstore, record);
    no_windows(&state);
    let (ws, _) = restore_last_session(&state).await.unwrap().unwrap();
    assert_eq!(state.mstore.get::<Workspace>(&ws).unwrap().unwrap().name, "");
    assert_eq!(take_restored_window_name(&state.mstore, &ws), None);
}
