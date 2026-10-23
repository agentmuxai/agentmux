// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Closed windows (SPEC_WORKSPACES_2026_10_23.md §2, §5): a window's close
//! keeps it in the workspace as closed, the record says which windows are
//! open and which closed, a cold start keeps the closed ones closed, and only
//! the panel's delete removes one.

use agentmux_common::ipc::{Command, Event};
use serde_json::json;

use super::*;
use crate::backend::obj::{Block, Tab};
use crate::server::service::reducer_helpers::dispatch_to_reducer;
use crate::server::service::session_restore::{handle_save_session_snapshot, load_last_session_snapshot};
use crate::server::service::session_windows::{set_client_meta, windows_of};
use crate::server::service::user_workspace;
use crate::server::service::window::handle_window_service;
use crate::server::tests::test_state;

fn call(method: &str, args: Vec<Value>) -> WebCallType {
    WebCallType { service: "window".into(), method: method.into(), uicontext: None, args }
}

async fn rpc(state: &AppState, method: &str, args: Vec<Value>) -> WebReturnType {
    handle_window_service(state, &call(method, args)).await
}

/// A window as the frontend opens one: `CreateWindow`, seeded with the
/// default layout. `cold_start` is the launch's own call, which may restore.
async fn open_window(state: &AppState, label: &str, cold_start: bool) -> Window {
    let ret = rpc(state, "CreateWindow", vec![Value::Null, json!(""), json!(label), json!(cold_start)]).await;
    assert!(ret.success, "CreateWindow failed: {:?}", ret.error);
    serde_json::from_value(ret.data.unwrap()).unwrap()
}

/// A pane of the user's, so the window holds something to keep.
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
/// that close saved, so the test's own windows are the session. Default
/// Workspace is open, as srv's start leaves it.
async fn fresh_state() -> AppState {
    let state = test_state();
    let decided = user_workspace::prepare_at_startup(&state.mstore, None);
    user_workspace::adopt_current(&state.mstore, decided);
    let ids = client(&state.mstore).unwrap().windowids;
    for id in ids {
        close(&state, &id).await;
    }
    crate::server::service::session_restore::clear_last_session_snapshot(&state.mstore);
    state
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

fn views(state: &AppState, ws_id: &str) -> Vec<String> {
    block_ids(state, ws_id)
        .iter()
        .filter_map(|b| state.mstore.get::<Block>(b).unwrap())
        .filter_map(|b| b.meta.get("view").and_then(|v| v.as_str()).map(str::to_string))
        .collect()
}

fn closed_ids(state: &AppState) -> Vec<String> {
    entries(&state.mstore).iter().filter_map(|e| workspace_of(e).map(str::to_string)).collect()
}

fn exists(state: &AppState, ws_id: &str) -> bool {
    state.mstore.get::<Workspace>(ws_id).unwrap().is_some()
}

/// The open workspace's saved record.
fn saved_record(state: &AppState) -> Value {
    let id = user_workspace::current_id(&state.mstore).unwrap();
    let row = state.mstore.user_workspace_get(&id).unwrap().unwrap();
    serde_json::from_str(&row.record.expect("the workspace was saved")).unwrap()
}

/// `(workspaceid, state)` of each window in the saved record.
fn record_states(state: &AppState) -> Vec<(String, String)> {
    windows_of(&saved_record(state))
        .iter()
        .map(|w| (w["workspaceid"].as_str().unwrap_or("").to_string(), w["state"].as_str().unwrap_or("").to_string()))
        .collect()
}

#[tokio::test]
async fn closing_one_of_two_windows_keeps_it_as_a_closed_window() {
    let state = fresh_state().await;
    let main = open_window(&state, "main", false).await;
    let other = open_window(&state, "window-2", false).await;
    add_user_pane(&state, &other.workspaceid, "term").await;
    let mut row = state.mstore.get::<Window>(&other.oid).unwrap().unwrap();
    row.meta.insert("window:displayname".into(), json!("Ops"));
    state.mstore.update(&mut row).unwrap();
    let before = block_ids(&state, &other.workspaceid);

    close(&state, &other.oid).await;

    assert!(exists(&state, &other.workspaceid), "its contents are kept");
    assert_eq!(block_ids(&state, &other.workspaceid), before, "with every pane, under the same ids");
    assert_eq!(closed_ids(&state), vec![other.workspaceid.clone()]);
    assert!(load_last_session_snapshot(&state.mstore).is_none(), "the session hasn't ended");
    let listed = rpc(&state, "ListClosedWindows", vec![]).await.data.unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["workspaceid"], json!(other.workspaceid));
    assert_eq!(listed[0]["name"], "Ops");
    assert!(listed[0]["closed_at_ms"].as_i64().unwrap() > 0);
    assert!(exists(&state, &main.workspaceid));
}

#[tokio::test]
async fn the_record_says_which_windows_are_open_and_which_closed() {
    let state = fresh_state().await;
    let main = open_window(&state, "main", false).await;
    let other = open_window(&state, "window-2", false).await;
    add_user_pane(&state, &other.workspaceid, "term").await;
    close(&state, &other.oid).await;

    assert_eq!(user_workspace::save_now(&state.mstore), user_workspace::Saved::Written);

    assert_eq!(
        record_states(&state),
        vec![(main.workspaceid.clone(), "open".to_string()), (other.workspaceid.clone(), "closed".to_string())]
    );
    let closed = windows_of(&saved_record(&state))[1].clone();
    assert!(closed["closed_at_ms"].as_i64().unwrap() > 0);
    assert!(closed["window"]["width"].is_number(), "where its window was");
    assert!(!closed["tabs"].as_array().unwrap().is_empty(), "and its contents");
    // The session record (which an older build reads) holds the open window only.
    let session = crate::server::service::session_windows::snapshot_session(&state.mstore, false).unwrap();
    assert_eq!(windows_of(&session).len(), 1);
}

#[tokio::test]
async fn with_closed_adds_nothing_twice() {
    let state = test_state();
    let record = json!({ "tabs": [], "windows": [
        { "workspaceid": "a", "tabs": [] },
        { "workspaceid": "b", "tabs": [], "state": "closed" },
    ]});
    let once = with_closed(&state.mstore, record);
    assert_eq!(once["windows"][0]["state"], "open");
    assert_eq!(once["windows"][1]["state"], "closed");
    assert_eq!(with_closed(&state.mstore, once.clone()), once);
}

#[tokio::test]
async fn closing_the_last_window_leaves_it_open_for_the_next_launch() {
    let state = fresh_state().await;
    let win = open_window(&state, "main", false).await;
    add_user_pane(&state, &win.workspaceid, "term").await;
    let before = block_ids(&state, &win.workspaceid);

    close(&state, &win.oid).await;

    assert!(client(&state.mstore).unwrap().windowids.is_empty(), "the window is closed");
    assert!(exists(&state, &win.workspaceid), "its contents are kept");
    assert_eq!(block_ids(&state, &win.workspaceid), before);
    assert!(closed_ids(&state).is_empty(), "closing the last window quits: it isn't a closed window");
    assert_eq!(load_last_session_snapshot(&state.mstore).unwrap()["workspaceid"], json!(win.workspaceid));
    assert_eq!(record_states(&state), vec![(win.workspaceid.clone(), "open".to_string())]);

    let relaunched = open_window(&state, "main", true).await;
    assert_eq!(relaunched.workspaceid, win.workspaceid, "the same contents, not a rebuilt copy");
    assert_eq!(block_ids(&state, &relaunched.workspaceid), before);
    assert_eq!(state.mstore.get_all::<Workspace>().unwrap().len(), 1, "nothing left over");
}

#[tokio::test]
async fn quit_and_relaunch_reopens_every_window_and_lists_none_closed() {
    let state = fresh_state().await;
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
    assert!(closed_ids(&state).is_empty(), "a quit's closes don't close windows in the workspace");

    let relaunched = open_window(&state, "main", true).await;
    assert_eq!(relaunched.workspaceid, main.workspaceid);
    let others = rpc(&state, "TakeRestoredWindows", vec![]).await.data.unwrap();
    assert_eq!(others.as_array().unwrap().len(), 1);
    assert_eq!(others[0]["workspaceid"], json!(second.workspaceid));
    assert_eq!(block_ids(&state, &second.workspaceid), second_blocks);
    assert_eq!(state.mstore.get_all::<Workspace>().unwrap().len(), 2, "no rebuilt copies");
}

#[tokio::test]
async fn a_cold_start_keeps_closed_windows_closed() {
    let state = fresh_state().await;
    let main = open_window(&state, "main", false).await;
    add_user_pane(&state, &main.workspaceid, "term").await;
    let other = open_window(&state, "window-2", false).await;
    add_user_pane(&state, &other.workspaceid, "term").await;
    let other_blocks = block_ids(&state, &other.workspaceid);
    close(&state, &other.oid).await;
    close(&state, &main.oid).await;

    let relaunched = open_window(&state, "main", true).await;

    assert_eq!(relaunched.workspaceid, main.workspaceid);
    let others = rpc(&state, "TakeRestoredWindows", vec![]).await.data.unwrap();
    assert!(others.as_array().unwrap().is_empty(), "no window opens for a closed one");
    assert_eq!(closed_ids(&state), vec![other.workspaceid.clone()], "still listed");
    assert_eq!(block_ids(&state, &other.workspaceid), other_blocks, "as it was");
    assert_eq!(state.mstore.get_all::<Workspace>().unwrap().len(), 2, "no rebuilt copies");
    // The clean load's backup holds the closed window too, though the record
    // that won was the last close's, which holds the open ones only (#5359).
    let id = user_workspace::current_id(&state.mstore).unwrap();
    let previous: Value = serde_json::from_str(&state.mstore.user_workspace_get(&id).unwrap().unwrap().previous.unwrap()).unwrap();
    assert!(
        windows_of(&previous).iter().any(|w| w["workspaceid"] == json!(other.workspaceid) && w["state"] == json!("closed")),
        "{previous}"
    );
}

#[tokio::test]
async fn a_cold_start_rebuilds_a_closed_window_whose_contents_are_gone() {
    let state = fresh_state().await;
    let main = open_window(&state, "main", false).await;
    add_user_pane(&state, &main.workspaceid, "term").await;
    let other = open_window(&state, "window-2", false).await;
    add_user_pane(&state, &other.workspaceid, "term").await;
    close(&state, &other.oid).await;
    close(&state, &main.oid).await;
    // As in a new version's first start, which copied only the record.
    crate::backend::wcore::delete_workspace(&state.mstore, &other.workspaceid).unwrap();
    state.srv_state.lock().await.workspaces.remove(&other.workspaceid);
    set_client_meta(&state.mstore, CLOSED_META_KEY, None);
    crate::server::service::session_restore::clear_last_session_snapshot(&state.mstore);

    let relaunched = open_window(&state, "main", true).await;

    assert_eq!(relaunched.workspaceid, main.workspaceid);
    let others = rpc(&state, "TakeRestoredWindows", vec![]).await.data.unwrap();
    assert!(others.as_array().unwrap().is_empty());
    let closed = closed_ids(&state);
    assert_eq!(closed.len(), 1, "listed as closed again");
    assert_ne!(closed[0], other.workspaceid, "rebuilt, with new ids");
    assert!(views(&state, &closed[0]).contains(&"term".to_string()));
}

#[tokio::test]
async fn reopening_a_closed_window_brings_back_the_same_panes() {
    let state = fresh_state().await;
    let _main = open_window(&state, "main", false).await;
    let other = open_window(&state, "window-2", false).await;
    add_user_pane(&state, &other.workspaceid, "term").await;
    let mut row = state.mstore.get::<Window>(&other.oid).unwrap().unwrap();
    row.meta.insert("window:displayname".into(), json!("Ops"));
    state.mstore.update(&mut row).unwrap();
    let before = block_ids(&state, &other.workspaceid);
    close(&state, &other.oid).await;

    let taken = rpc(&state, "TakeClosedWindow", vec![json!(other.workspaceid)]).await;
    assert!(taken.success, "{:?}", taken.error);
    assert_eq!(taken.data.unwrap()["workspaceid"], json!(other.workspaceid));
    // Listed, and saved, until its window registers: one that fails to open
    // can be taken again (#5359).
    assert_eq!(closed_ids(&state), [other.workspaceid.clone()]);
    user_workspace::save_now(&state.mstore);
    assert!(record_states(&state).contains(&(other.workspaceid.clone(), "closed".to_string())));
    let again = rpc(&state, "TakeClosedWindow", vec![json!(other.workspaceid)]).await;
    assert!(again.success, "{:?}", again.error);

    // Its window reopens onto its contents and gets its name back.
    let ret = rpc(&state, "CreateWindow", vec![Value::Null, json!(other.workspaceid)]).await;
    let reopened: Window = serde_json::from_value(ret.data.unwrap()).unwrap();
    assert_eq!(reopened.meta.get("window:displayname"), Some(&json!("Ops")));
    assert_eq!(block_ids(&state, &other.workspaceid), before);
    assert!(closed_ids(&state).is_empty(), "open now");
    let late = rpc(&state, "TakeClosedWindow", vec![json!(other.workspaceid)]).await;
    assert!(!late.success, "not closed any more");
}

#[tokio::test]
async fn deleting_an_open_window_deletes_its_contents() {
    let state = fresh_state().await;
    let main = open_window(&state, "main", false).await;
    let other = open_window(&state, "window-2", false).await;
    add_user_pane(&state, &other.workspaceid, "term").await;

    let ret = rpc(&state, "DiscardWorkspaceOnClose", vec![json!(other.oid)]).await;
    assert!(ret.success, "{:?}", ret.error);
    close(&state, &other.oid).await;

    assert!(!exists(&state, &other.workspaceid), "removed from the workspace");
    assert!(closed_ids(&state).is_empty(), "and not kept as closed");
    assert!(exists(&state, &main.workspaceid));
}

#[tokio::test]
async fn the_only_open_window_cant_be_deleted() {
    let state = fresh_state().await;
    let main = open_window(&state, "main", false).await;
    let ret = rpc(&state, "DiscardWorkspaceOnClose", vec![json!(main.oid)]).await;
    assert!(!ret.success, "the last window stays");
    assert!(!take_discard(&main.oid), "and its close isn't marked");
}

#[tokio::test]
async fn deleting_a_closed_window_deletes_its_contents() {
    let state = fresh_state().await;
    let _main = open_window(&state, "main", false).await;
    let other = open_window(&state, "window-2", false).await;
    add_user_pane(&state, &other.workspaceid, "term").await;
    close(&state, &other.oid).await;

    let ret = rpc(&state, "DeleteClosedWindow", vec![json!(other.workspaceid)]).await;
    assert!(ret.success, "{:?}", ret.error);

    assert!(!exists(&state, &other.workspaceid));
    assert!(closed_ids(&state).is_empty());
    let again = rpc(&state, "DeleteClosedWindow", vec![json!(other.workspaceid)]).await;
    assert!(!again.success, "already gone");
}

#[tokio::test]
async fn an_open_window_isnt_deleted_as_a_closed_one() {
    let state = fresh_state().await;
    let main = open_window(&state, "main", false).await;
    let ret = rpc(&state, "DeleteClosedWindow", vec![json!(main.workspaceid)]).await;
    assert!(!ret.success);
    assert!(exists(&state, &main.workspaceid));
}

#[tokio::test]
async fn closed_windows_stay_until_deleted() {
    let state = fresh_state().await;
    let _main = open_window(&state, "main", false).await;
    let mut closed = Vec::new();
    for i in 0..12 {
        let w = open_window(&state, &format!("window-{i}"), false).await;
        add_user_pane(&state, &w.workspaceid, "term").await;
        close(&state, &w.oid).await;
        closed.push(w.workspaceid);
    }
    assert!(closed.iter().all(|ws| exists(&state, ws)), "no cap: none is deleted");
    closed.reverse();
    assert_eq!(closed_ids(&state), closed, "newest first");
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
    let state = fresh_state().await;
    let _main = open_window(&state, "main", false).await;
    let fresh = open_window(&state, "window-2", false).await;

    close(&state, &fresh.oid).await;

    assert!(!exists(&state, &fresh.workspaceid), "the untouched default layout isn't kept");
    assert!(closed_ids(&state).is_empty());
}

#[tokio::test]
async fn window_agents_lists_none_without_running_agents() {
    let state = test_state();
    let win_id = client(&state.mstore).unwrap().windowids[0].clone();
    let ret = rpc(&state, "WindowAgents", vec![json!(win_id)]).await;
    assert_eq!(ret.data.unwrap(), json!([]));
}
