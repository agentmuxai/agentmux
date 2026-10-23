// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Closing is not deleting (SPEC_OFFLINE_RESILIENCE_2026_10_20.md G8): a
//! window's close keeps its workspace, the next launch reopens it as it was,
//! and only a confirmed Close workspace deletes one.

use agentmux_common::ipc::{Command, Event};
use serde_json::json;

use super::*;
use crate::backend::obj::{Block, Tab};
use crate::server::service::reducer_helpers::dispatch_to_reducer;
use crate::server::service::session_restore::{handle_save_session_snapshot, load_last_session_snapshot};
use crate::server::service::window::handle_window_service;
use crate::server::tests::test_state;

fn call(method: &str, args: Vec<Value>) -> WebCallType {
    WebCallType { service: "window".into(), method: method.into(), uicontext: None, args }
}

async fn rpc(state: &AppState, method: &str, args: Vec<Value>) -> WebReturnType {
    handle_window_service(state, &call(method, args)).await
}

/// A window as the frontend opens one: `CreateWindow`, seeded with the
/// default layout, plus a terminal pane of the user's so it holds something
/// to keep. `cold_start` is the launch's own call, which may restore.
async fn open_window(state: &AppState, label: &str, cold_start: bool) -> Window {
    let ret = rpc(state, "CreateWindow", vec![Value::Null, json!(""), json!(label), json!(cold_start)]).await;
    assert!(ret.success, "CreateWindow failed: {:?}", ret.error);
    serde_json::from_value(ret.data.unwrap()).unwrap()
}

async fn add_user_pane(state: &AppState, ws_id: &str, view: &str) -> String {
    let ws = state.mstore.get::<Workspace>(ws_id).unwrap().unwrap();
    let tab_id = ws.tabids.iter().chain(ws.pinnedtabids.iter()).next().unwrap().clone();
    let events = dispatch_to_reducer(state, Command::CreateBlock { tab_id, meta: json!({ "view": view }) }).await;
    for ev in &events {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
    }
    events
        .iter()
        .find_map(|e| match e {
            Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .unwrap()
}

async fn close(state: &AppState, window_id: &str) {
    let ret = rpc(state, "CloseWindow", vec![json!(window_id)]).await;
    assert!(ret.success, "CloseWindow failed: {:?}", ret.error);
}

/// `test_state()` opens a bootstrap window: close it, and forget the record
/// that close saved, so the test's own windows are the session.
async fn without_bootstrap_window(state: &AppState) {
    let ids = client(&state.mstore).unwrap().windowids;
    for id in ids {
        close(state, &id).await;
    }
    crate::server::service::session_restore::clear_last_session_snapshot(&state.mstore);
}

fn block_ids(state: &AppState, ws_id: &str) -> Vec<String> {
    let ws = state.mstore.get::<Workspace>(ws_id).unwrap().unwrap();
    ws.pinnedtabids
        .iter()
        .chain(ws.tabids.iter())
        .filter_map(|t| state.mstore.get::<Tab>(t).unwrap())
        .flat_map(|t| t.blockids)
        .collect()
}

fn closed_ids(state: &AppState) -> Vec<String> {
    entries(&state.mstore).iter().filter_map(|e| workspace_of(e).map(str::to_string)).collect()
}

fn exists(state: &AppState, ws_id: &str) -> bool {
    state.mstore.get::<Workspace>(ws_id).unwrap().is_some()
}

#[tokio::test]
async fn closing_the_last_window_keeps_the_workspace() {
    let state = test_state();
    without_bootstrap_window(&state).await;
    let win = open_window(&state, "main", false).await;
    let pane = add_user_pane(&state, &win.workspaceid, "term").await;
    let before = block_ids(&state, &win.workspaceid);

    close(&state, &win.oid).await;

    assert!(client(&state.mstore).unwrap().windowids.is_empty(), "the window is closed");
    assert!(exists(&state, &win.workspaceid), "its workspace is kept");
    assert_eq!(block_ids(&state, &win.workspaceid), before, "with every pane, under the same ids");
    assert!(state.mstore.get::<Block>(&pane).unwrap().is_some());
    assert_eq!(closed_ids(&state), vec![win.workspaceid.clone()]);
    let record = load_last_session_snapshot(&state.mstore).unwrap();
    assert_eq!(record["workspaceid"], json!(win.workspaceid), "the session record names it");
}

#[tokio::test]
async fn the_next_launch_reopens_the_kept_workspace_as_it_was() {
    let state = test_state();
    without_bootstrap_window(&state).await;
    let win = open_window(&state, "main", false).await;
    add_user_pane(&state, &win.workspaceid, "term").await;
    let before = block_ids(&state, &win.workspaceid);
    close(&state, &win.oid).await;

    let relaunched = open_window(&state, "main", true).await;

    assert_eq!(relaunched.workspaceid, win.workspaceid, "the same workspace, not a rebuilt copy");
    assert_eq!(block_ids(&state, &relaunched.workspaceid), before);
    assert!(closed_ids(&state).is_empty(), "open again, so no longer a closed window");
    assert!(load_last_session_snapshot(&state.mstore).is_none(), "the record is used up");
    assert_eq!(state.mstore.get_all::<Workspace>().unwrap().len(), 1, "nothing left over");
}

#[tokio::test]
async fn quit_and_relaunch_reopens_every_window() {
    let state = test_state();
    without_bootstrap_window(&state).await;
    let main = open_window(&state, "main", false).await;
    add_user_pane(&state, &main.workspaceid, "term").await;
    let second = open_window(&state, "window-2", false).await;
    add_user_pane(&state, &second.workspaceid, "sysinfo").await;
    add_user_pane(&state, &second.workspaceid, "term").await;
    let second_blocks = block_ids(&state, &second.workspaceid);

    // The host's quit: save every window, then close them one by one.
    let ret = handle_save_session_snapshot(&state, &call("SaveSessionSnapshot", vec![json!(true)])).await;
    assert!(ret.success, "{:?}", ret.error);
    close(&state, &second.oid).await;
    close(&state, &main.oid).await;
    assert!(exists(&state, &main.workspaceid) && exists(&state, &second.workspaceid));

    let relaunched = open_window(&state, "main", true).await;
    assert_eq!(relaunched.workspaceid, main.workspaceid);
    let others = rpc(&state, "TakeRestoredWindows", vec![]).await.data.unwrap();
    assert_eq!(others.as_array().unwrap().len(), 1);
    assert_eq!(others[0]["workspaceid"], json!(second.workspaceid));
    assert_eq!(block_ids(&state, &second.workspaceid), second_blocks);
    assert_eq!(state.mstore.get_all::<Workspace>().unwrap().len(), 2, "no rebuilt copies");
}

#[tokio::test]
async fn a_record_whose_workspace_is_gone_is_rebuilt() {
    let state = test_state();
    without_bootstrap_window(&state).await;
    let win = open_window(&state, "main", false).await;
    add_user_pane(&state, &win.workspaceid, "term").await;
    close(&state, &win.oid).await;
    crate::backend::wcore::delete_workspace(&state.mstore, &win.workspaceid).unwrap();
    state.srv_state.lock().await.workspaces.remove(&win.workspaceid);

    let relaunched = open_window(&state, "main", true).await;

    assert_ne!(relaunched.workspaceid, win.workspaceid);
    let views: Vec<String> = block_ids(&state, &relaunched.workspaceid)
        .iter()
        .filter_map(|b| state.mstore.get::<Block>(b).unwrap())
        .filter_map(|b| b.meta.get("view").and_then(|v| v.as_str()).map(str::to_string))
        .collect();
    assert!(views.contains(&"term".to_string()), "rebuilt from the record: {views:?}");
}

#[tokio::test]
async fn closing_one_of_two_windows_keeps_it_for_reopen() {
    let state = test_state();
    without_bootstrap_window(&state).await;
    let main = open_window(&state, "main", false).await;
    let other = open_window(&state, "window-2", false).await;
    add_user_pane(&state, &other.workspaceid, "term").await;
    let mut row = state.mstore.get::<Window>(&other.oid).unwrap().unwrap();
    row.meta.insert("window:displayname".into(), json!("Ops"));
    state.mstore.update(&mut row).unwrap();

    close(&state, &other.oid).await;

    assert!(exists(&state, &other.workspaceid), "a non-last close keeps it too");
    assert!(load_last_session_snapshot(&state.mstore).is_none(), "the session hasn't ended");
    let listed = rpc(&state, "ListClosedWindows", vec![]).await.data.unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["workspaceid"], json!(other.workspaceid));
    assert_eq!(listed[0]["name"], "Ops");

    let taken = rpc(&state, "TakeClosedWindow", vec![json!(other.workspaceid)]).await;
    assert!(taken.success, "{:?}", taken.error);
    assert_eq!(taken.data.unwrap()["workspaceid"], json!(other.workspaceid));
    assert!(closed_ids(&state).is_empty());
    let again = rpc(&state, "TakeClosedWindow", vec![json!(other.workspaceid)]).await;
    assert!(!again.success, "handed out once");

    // Its window reopens onto it and gets its name back.
    let ret = rpc(&state, "CreateWindow", vec![Value::Null, json!(other.workspaceid)]).await;
    let reopened: Window = serde_json::from_value(ret.data.unwrap()).unwrap();
    assert_eq!(reopened.meta.get("window:displayname"), Some(&json!("Ops")));
    assert!(exists(&state, &main.workspaceid));
}

#[tokio::test]
async fn close_workspace_deletes_only_when_asked() {
    let state = test_state();
    without_bootstrap_window(&state).await;
    let win = open_window(&state, "main", false).await;
    add_user_pane(&state, &win.workspaceid, "term").await;

    let ret = rpc(&state, "DiscardWorkspaceOnClose", vec![json!(win.oid)]).await;
    assert!(ret.success);
    close(&state, &win.oid).await;

    assert!(!exists(&state, &win.workspaceid), "Close workspace deletes it");
    assert!(closed_ids(&state).is_empty());
    assert!(load_last_session_snapshot(&state.mstore).is_none(), "and saves no record to bring it back");
}

#[tokio::test]
async fn a_discard_mark_is_used_once() {
    mark_discard("win-once");
    assert!(take_discard("win-once"));
    assert!(!take_discard("win-once"));
    assert!(!take_discard("win-never-marked"));
}

#[tokio::test]
async fn a_window_with_nothing_of_the_users_is_deleted_at_close() {
    let state = test_state();
    without_bootstrap_window(&state).await;
    let _main = open_window(&state, "main", false).await;
    let fresh = open_window(&state, "window-2", false).await;

    close(&state, &fresh.oid).await;

    assert!(!exists(&state, &fresh.workspaceid), "the untouched default layout isn't kept");
    assert!(closed_ids(&state).is_empty());
}

#[test]
fn at_most_the_kept_number_of_closed_windows_stay() {
    let mut list: Vec<Value> = (0..CLOSED_WINDOWS_KEPT + 4).map(|i| json!({ "workspaceid": format!("ws-{i}") })).collect();
    // The oldest is named by the session record; one is open again; one is gone.
    let protected: HashSet<String> = [format!("ws-{}", CLOSED_WINDOWS_KEPT + 3)].into();
    let open: HashSet<String> = ["ws-1".to_string()].into();
    let evicted = trim(&mut list, &protected, &open, |ws| ws != "ws-2");

    let kept: Vec<&str> = list.iter().filter_map(workspace_of).collect();
    assert_eq!(kept.len(), CLOSED_WINDOWS_KEPT + 1, "the kept number, plus the one the record names");
    assert!(!kept.contains(&"ws-1") && !kept.contains(&"ws-2"));
    assert!(kept.contains(&format!("ws-{}", CLOSED_WINDOWS_KEPT + 3).as_str()));
    assert_eq!(evicted, vec![format!("ws-{}", CLOSED_WINDOWS_KEPT + 2)], "only the oldest past the limit is deleted");
}

#[tokio::test]
async fn closing_past_the_limit_deletes_the_oldest_closed_window() {
    let state = test_state();
    without_bootstrap_window(&state).await;
    let _main = open_window(&state, "main", false).await;
    let mut closed = Vec::new();
    for i in 0..=CLOSED_WINDOWS_KEPT {
        let w = open_window(&state, &format!("window-{i}"), false).await;
        add_user_pane(&state, &w.workspaceid, "term").await;
        close(&state, &w.oid).await;
        closed.push(w.workspaceid);
    }
    assert!(!exists(&state, &closed[0]), "the oldest goes");
    assert!(closed[1..].iter().all(|ws| exists(&state, ws)));
    assert_eq!(closed_ids(&state).len(), CLOSED_WINDOWS_KEPT);
}

#[tokio::test]
async fn window_agents_lists_none_without_running_agents() {
    let state = test_state();
    let win_id = client(&state.mstore).unwrap().windowids[0].clone();
    let ret = rpc(&state, "WindowAgents", vec![json!(win_id)]).await;
    assert_eq!(ret.data.unwrap(), json!([]));
}
