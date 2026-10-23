// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Several workspaces (SPEC_WORKSPACES_2026_10_23.md §3.2, §6): a new one,
//! switching away and back, the next launch, a workspace whose contents are
//! gone, deleting one.

use serde_json::json;

use super::*;
use crate::backend::obj::{Block, Tab, Workspace};
use crate::server::service::session_windows::windows_of;
use crate::server::service::user_workspace::{self, adopt_current, prepare_at_startup};
use crate::server::service::window::handle_window_service;
use crate::server::service::workspace::handle_workspace_service;
use crate::server::tests::test_state;

pub(crate) async fn window_rpc(state: &AppState, method: &str, args: Vec<Value>) -> WebReturnType {
    handle_window_service(state, &WebCallType { service: "window".into(), method: method.into(), uicontext: None, args })
        .await
}

pub(crate) async fn workspace_rpc(state: &AppState, method: &str, args: Vec<Value>) -> WebReturnType {
    handle_workspace_service(state, &WebCallType { service: "workspace".into(), method: method.into(), uicontext: None, args })
        .await
}

/// A window as the frontend opens one (`CreateWindow`, the default panes).
pub(crate) async fn open_window(state: &AppState, label: &str) -> Window {
    let ret = window_rpc(state, "CreateWindow", vec![Value::Null, json!(""), json!(label)]).await;
    assert!(ret.success, "CreateWindow failed: {:?}", ret.error);
    serde_json::from_value(ret.data.unwrap()).unwrap()
}

/// A pane of the user's in contents `ws_id`, so they are worth keeping.
pub(crate) async fn add_pane(state: &AppState, ws_id: &str, view: &str) {
    let ws = state.mstore.get::<Workspace>(ws_id).unwrap().unwrap();
    let tab_id = ws.tabids.iter().chain(ws.pinnedtabids.iter()).next().unwrap().clone();
    let first = state.mstore.get::<Tab>(&tab_id).unwrap().unwrap().blockids.first().cloned();
    let events = dispatch_to_reducer(state, Command::CreateBlock { tab_id: tab_id.clone(), meta: json!({ "view": view }) }).await;
    for ev in &events {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
    }
    // Into the layout too, as a pane tab of the first pane, so a layout
    // export (which walks the tree) sees it.
    let block_id = events.iter().find_map(|e| match e {
        Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
        _ => None,
    });
    if let (Some(target), Some(block_id)) = (first, block_id) {
        let push = Command::LayoutStackPush { tab_id, target_block_id: target, block_id, activate: false, correlation_id: String::new() };
        for ev in &dispatch_to_reducer(state, push).await {
            crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
        }
    }
}

pub(crate) async fn close(state: &AppState, window_id: &str) {
    let ret = window_rpc(state, "CloseWindow", vec![json!(window_id)]).await;
    assert!(ret.success, "CloseWindow failed: {:?}", ret.error);
}

/// Default Workspace open, the bootstrap window closed and its record
/// forgotten, so the test's windows are the workspace's.
pub(crate) async fn fresh_state() -> AppState {
    let state = test_state();
    let decided = prepare_at_startup(&state.mstore, None);
    adopt_current(&state.mstore, decided);
    for id in client(&state.mstore).unwrap().windowids {
        close(&state, &id).await;
    }
    crate::server::service::session_restore::clear_last_session_snapshot(&state.mstore);
    state
}

pub(crate) fn views(state: &AppState, ws_id: &str) -> Vec<String> {
    let Some(ws) = state.mstore.get::<Workspace>(ws_id).unwrap() else { return Vec::new() };
    let mut out: Vec<String> = ws
        .pinnedtabids
        .iter()
        .chain(ws.tabids.iter())
        .filter_map(|t| state.mstore.get::<Tab>(t).unwrap())
        .flat_map(|t| t.blockids)
        .filter_map(|b| state.mstore.get::<Block>(&b).unwrap())
        .filter_map(|b| b.meta.get("view").and_then(|v| v.as_str()).map(str::to_string))
        .collect();
    out.sort();
    out
}

pub(crate) fn exists(state: &AppState, ws_id: &str) -> bool {
    state.mstore.get::<Workspace>(ws_id).unwrap().is_some()
}

pub(crate) fn shown_by(state: &AppState, window_id: &str) -> String {
    state.mstore.get::<Window>(window_id).unwrap().expect("the window is open").workspaceid
}

pub(crate) fn closed_list(state: &AppState) -> Vec<String> {
    client(&state.mstore)
        .and_then(|c| c.meta.get(super::super::session_closed::CLOSED_META_KEY).cloned())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|e| e["workspaceid"].as_str().map(str::to_string))
        .collect()
}

fn row(state: &AppState, id: &str) -> UserWorkspaceRow {
    state.mstore.user_workspace_get(id).unwrap().unwrap()
}

fn record_states(state: &AppState, id: &str) -> Vec<(String, String)> {
    let record: Value = serde_json::from_str(&row(state, id).record.expect("saved")).unwrap();
    windows_of(&record)
        .iter()
        .map(|w| (w["workspaceid"].as_str().unwrap_or("").to_string(), w["state"].as_str().unwrap_or("").to_string()))
        .collect()
}

/// Default Workspace with three windows: main (a terminal added), window-2 (a
/// sysinfo added) and a closed window-3 (a terminal added). Main is named
/// "Main". Returns the windows and the contents.
struct Three {
    main: Window,
    second: Window,
    closed_ws: String,
    default_id: String,
}

async fn three_windows(state: &AppState) -> Three {
    let main = open_window(state, "main").await;
    add_pane(state, &main.workspaceid, "term").await;
    let mut row = state.mstore.get::<Window>(&main.oid).unwrap().unwrap();
    row.meta.insert("window:displayname".into(), json!("Main"));
    state.mstore.update(&mut row).unwrap();
    let second = open_window(state, "window-2").await;
    add_pane(state, &second.workspaceid, "files").await;
    let third = open_window(state, "window-3").await;
    add_pane(state, &third.workspaceid, "term").await;
    close(state, &third.oid).await;
    let default_id = current_id(&state.mstore).unwrap();
    Three { main, second, closed_ws: third.workspaceid, default_id }
}

async fn create(state: &AppState, name: &str, window_id: &str) -> Value {
    let ret = workspace_rpc(state, "CreateUserWorkspace", vec![json!(name), json!(window_id)]).await;
    assert!(ret.success, "CreateUserWorkspace failed: {:?}", ret.error);
    wait_for_switch().await;
    ret.data.unwrap()
}

async fn switch(state: &AppState, id: &str, window_id: &str) -> Value {
    let ret = workspace_rpc(state, "SwitchUserWorkspace", vec![json!(id), json!(window_id)]).await;
    assert!(ret.success, "SwitchUserWorkspace failed: {:?}", ret.error);
    wait_for_switch().await;
    ret.data.unwrap()
}

#[tokio::test]
async fn a_new_workspace_closes_the_open_one_and_opens_the_default_panes() {
    let state = fresh_state().await;
    let t = three_windows(&state).await;

    let out = create(&state, "  Workspace 2 ", &t.main.oid).await;

    assert_eq!(out["name"], "Workspace 2");
    assert_eq!(out["closed"], json!([{ "window_id": t.second.oid, "label": "window-2" }]), "the host closes the other window");
    assert_eq!(out["beside"], 0);
    let new_id = out["id"].as_str().unwrap().to_string();
    assert_eq!(current_id(&state.mstore).as_deref(), Some(new_id.as_str()));
    assert_eq!(client(&state.mstore).unwrap().meta["workspace:name"], "Workspace 2");
    assert_eq!(client(&state.mstore).unwrap().windowids, vec![t.main.oid.clone()], "one window left");

    // The window that asked shows the default panes, and isn't named Main.
    let shown = shown_by(&state, &t.main.oid);
    assert_ne!(shown, t.main.workspaceid);
    assert_eq!(views(&state, &shown), ["agent", "swarm", "sysinfo"]);
    let main_row = state.mstore.get::<Window>(&t.main.oid).unwrap().unwrap();
    assert!(main_row.meta.get("window:displayname").is_none(), "the new workspace's window has no name");
    assert!(state.mstore.get::<Window>(&t.second.oid).unwrap().is_none(), "the other window's row is gone");
    assert!(closed_list(&state).is_empty(), "the new workspace has no closed windows");

    // Default Workspace keeps every window: open, open, closed, with contents.
    assert_eq!(
        record_states(&state, &t.default_id),
        vec![
            (t.main.workspaceid.clone(), "open".to_string()),
            (t.second.workspaceid.clone(), "open".to_string()),
            (t.closed_ws.clone(), "closed".to_string()),
        ]
    );
    for ws in [&t.main.workspaceid, &t.second.workspaceid, &t.closed_ws] {
        assert!(exists(&state, ws), "kept: {ws}");
    }
    // The new workspace was saved at once.
    assert_eq!(record_states(&state, &new_id), vec![(shown, "open".to_string())]);
}

#[tokio::test]
async fn switching_back_reuses_the_window_and_brings_back_every_window() {
    let state = fresh_state().await;
    let t = three_windows(&state).await;
    let new_id = create(&state, "Workspace 2", &t.main.oid).await["id"].as_str().unwrap().to_string();

    let out = switch(&state, &t.default_id, &t.main.oid).await;

    assert_eq!(out["name"], "Default Workspace");
    assert_eq!(out["beside"], 1, "window-2 opens beside");
    assert_eq!(current_id(&state.mstore).as_deref(), Some(t.default_id.as_str()));
    assert_eq!(shown_by(&state, &t.main.oid), t.main.workspaceid, "the same contents, not a rebuilt copy");
    let main_row = state.mstore.get::<Window>(&t.main.oid).unwrap().unwrap();
    assert_eq!(main_row.meta.get("window:displayname"), Some(&json!("Main")), "and its name");
    let beside = window_rpc(&state, "TakeRestoredWindows", vec![]).await.data.unwrap();
    assert_eq!(beside.as_array().unwrap().len(), 1);
    assert_eq!(beside[0]["workspaceid"], json!(t.second.workspaceid));
    assert_eq!(closed_list(&state), vec![t.closed_ws.clone()], "its closed window is listed again");
    assert_eq!(views(&state, &t.main.workspaceid), ["agent", "swarm", "sysinfo", "term"]);

    // The most recently opened first.
    let listed = workspace_rpc(&state, "ListUserWorkspaces", vec![]).await.data.unwrap();
    let names: Vec<&str> = listed.as_array().unwrap().iter().map(|w| w["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Default Workspace", "Workspace 2"]);
    assert_eq!(listed[0]["current"], true);
    assert_eq!((listed[0]["windows"].as_u64(), listed[0]["closed"].as_u64()), (Some(2), Some(1)));
    assert_eq!(listed[1]["current"], false);
    assert_eq!(listed[1]["windows"], 1);
    assert!(row(&state, &new_id).record.is_some());
}

#[tokio::test]
async fn the_next_launch_opens_the_workspace_open_last() {
    let state = fresh_state().await;
    let t = three_windows(&state).await;
    let new_id = create(&state, "Workspace 2", &t.main.oid).await["id"].as_str().unwrap().to_string();

    // srv's next start: the client still names it.
    adopt_current(&state.mstore, prepare_at_startup(&state.mstore, None));
    assert_eq!(current_id(&state.mstore).as_deref(), Some(new_id.as_str()));
    assert_eq!(state.mstore.user_workspace_list().unwrap()[0].id, new_id);

    // Its last window closes (a quit); the cold start restores it, not Default.
    let shown = shown_by(&state, &t.main.oid);
    add_pane(&state, &shown, "files").await;
    close(&state, &t.main.oid).await;
    let ret = window_rpc(&state, "CreateWindow", vec![Value::Null, json!(""), json!("main"), json!(true)]).await;
    let relaunched: Window = serde_json::from_value(ret.data.unwrap()).unwrap();
    assert_eq!(relaunched.workspaceid, shown);
    assert!(window_rpc(&state, "TakeRestoredWindows", vec![]).await.data.unwrap().as_array().unwrap().is_empty());
}

#[tokio::test]
async fn a_workspace_whose_contents_are_gone_is_rebuilt_from_its_record() {
    let state = fresh_state().await;
    let t = three_windows(&state).await;
    create(&state, "Workspace 2", &t.main.oid).await;
    // As after a new version's first start, which copied only the records.
    for ws in [&t.main.workspaceid, &t.second.workspaceid, &t.closed_ws] {
        crate::backend::wcore::delete_workspace(&state.mstore, ws).unwrap();
        state.srv_state.lock().await.workspaces.remove(ws.as_str());
    }

    let out = switch(&state, &t.default_id, &t.main.oid).await;

    assert_eq!(out["beside"], 1);
    let shown = shown_by(&state, &t.main.oid);
    assert_ne!(shown, t.main.workspaceid, "rebuilt, with new ids");
    assert_eq!(views(&state, &shown), ["agent", "swarm", "sysinfo", "term"]);
    let beside = window_rpc(&state, "TakeRestoredWindows", vec![]).await.data.unwrap();
    let beside_ws = beside[0]["workspaceid"].as_str().unwrap();
    assert_ne!(beside_ws, t.second.workspaceid);
    assert!(views(&state, beside_ws).contains(&"files".to_string()));
    let closed = closed_list(&state);
    assert_eq!(closed.len(), 1);
    assert_ne!(closed[0], t.closed_ws);
    assert!(views(&state, &closed[0]).contains(&"term".to_string()));
}

#[tokio::test]
async fn deleting_a_workspace_deletes_its_row_and_its_kept_contents() {
    let state = fresh_state().await;
    let t = three_windows(&state).await;
    let new_id = create(&state, "Workspace 2", &t.main.oid).await["id"].as_str().unwrap().to_string();
    let shown = shown_by(&state, &t.main.oid);

    let refused = workspace_rpc(&state, "DeleteUserWorkspace", vec![json!(new_id)]).await;
    assert!(!refused.success, "the open one can't be deleted");

    let ret = workspace_rpc(&state, "DeleteUserWorkspace", vec![json!(t.default_id)]).await;
    assert!(ret.success, "{:?}", ret.error);

    assert!(state.mstore.user_workspace_get(&t.default_id).unwrap().is_none());
    for ws in [&t.main.workspaceid, &t.second.workspaceid, &t.closed_ws] {
        assert!(!exists(&state, ws), "deleted: {ws}");
    }
    assert!(exists(&state, &shown), "the open workspace is untouched");
    assert_eq!(state.mstore.user_workspace_list().unwrap().len(), 1);
    let again = workspace_rpc(&state, "DeleteUserWorkspace", vec![json!(t.default_id)]).await;
    assert!(!again.success, "already gone");
}

#[tokio::test]
async fn a_switch_that_cant_happen_changes_nothing() {
    let state = fresh_state().await;
    let t = three_windows(&state).await;

    let same = workspace_rpc(&state, "SwitchUserWorkspace", vec![json!(t.default_id), json!(t.main.oid)]).await;
    assert!(!same.success, "it is open already");
    let unknown = workspace_rpc(&state, "SwitchUserWorkspace", vec![json!("no-such"), json!(t.main.oid)]).await;
    assert!(!unknown.success);
    let blank = workspace_rpc(&state, "CreateUserWorkspace", vec![json!("   "), json!(t.main.oid)]).await;
    assert!(!blank.success, "a workspace needs a name");
    let no_window = workspace_rpc(&state, "CreateUserWorkspace", vec![json!("W"), json!("no-such-window")]).await;
    assert!(!no_window.success);
    wait_for_switch().await;

    assert_eq!(state.mstore.user_workspace_list().unwrap().len(), 1, "a failed create leaves no workspace");
    assert_eq!(current_id(&state.mstore).as_deref(), Some(t.default_id.as_str()));
    assert_eq!(shown_by(&state, &t.main.oid), t.main.workspaceid);
    assert_eq!(shown_by(&state, &t.second.oid), t.second.workspaceid);
    assert!(!state.mstore.layout_writes().saves_held());
}

/// A switch that fails after the other workspace became the open one puts
/// the open workspace back: still current, its windows still open, its closed
/// windows still listed, and nothing left pointing at a deleted workspace
/// (#5360).
#[tokio::test]
async fn a_switch_that_fails_halfway_leaves_the_open_workspace_open() {
    let state = fresh_state().await;
    let t = three_windows(&state).await;
    let closed_before = closed_list(&state);

    let failed = workspace_rpc(&state, "CreateUserWorkspace", vec![json!(super::FAIL_OPEN_NAME), json!(t.main.oid)]).await;
    assert!(!failed.success);
    wait_for_switch().await;

    assert_eq!(state.mstore.user_workspace_list().unwrap().len(), 1, "the new workspace is gone");
    assert_eq!(current_id(&state.mstore).as_deref(), Some(t.default_id.as_str()));
    assert_eq!(client(&state.mstore).unwrap().meta["workspace:name"], "Default Workspace");
    assert_eq!(client(&state.mstore).unwrap().windowids, vec![t.main.oid.clone(), t.second.oid.clone()], "both windows stay open");
    assert_eq!(shown_by(&state, &t.main.oid), t.main.workspaceid);
    assert_eq!(shown_by(&state, &t.second.oid), t.second.workspaceid);
    assert_eq!(closed_list(&state), closed_before);
    assert!(!state.mstore.layout_writes().saves_held());
    assert_eq!(user_workspace::save_now(&state.mstore), user_workspace::Saved::Unchanged, "and it saves as before");
}

#[tokio::test]
async fn saves_wait_while_a_switch_moves_the_windows() {
    let state = fresh_state().await;
    open_window(&state, "main").await;
    state.mstore.layout_writes().hold_saves(true);
    assert_eq!(user_workspace::save_now(&state.mstore), Saved::HeldForSwitch);
    state.mstore.layout_writes().hold_saves(false);
    assert_ne!(user_workspace::save_now(&state.mstore), Saved::HeldForSwitch);
}

#[tokio::test]
async fn the_working_agents_list_is_empty_without_running_agents() {
    let state = fresh_state().await;
    open_window(&state, "main").await;
    let ret = workspace_rpc(&state, "UserWorkspaceAgents", vec![]).await;
    assert_eq!(ret.data.unwrap(), json!([]));
}
