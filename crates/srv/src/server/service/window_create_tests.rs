// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `window.CreateWindow` beyond the default panes: attaching to an existing workspace, the host
//! label crumb, the new-window flag, and a cold start's restore request. The restore tests start
//! from `fresh_state`, where no window is open, as a launch does.

use serde_json::{json, Value};

use super::super::window::handle_window_service;
use crate::backend::obj::{Client, Window, Workspace};
use crate::backend::service::{WebCallType, WebReturnType};
use crate::server::tests::test_state;
use crate::server::AppState;

async fn create(state: &AppState, args: Vec<Value>) -> WebReturnType {
    let call = WebCallType { service: "window".into(), method: "CreateWindow".into(), uicontext: None, args };
    handle_window_service(state, &call).await
}

async fn open(state: &AppState, workspace: &str, label: &str) -> Window {
    let ret = create(state, vec![Value::Null, json!(workspace), json!(label)]).await;
    assert!(ret.success, "{:?}", ret.error);
    serde_json::from_value(ret.data.unwrap()).unwrap()
}

/// The state at a launch: the bootstrap window `test_state` opens is not listed, so no window is open.
async fn fresh_state() -> AppState {
    let state = test_state();
    let mut row = client(&state);
    row.windowids.clear();
    state.mstore.update(&mut row).unwrap();
    state
}

fn client(state: &AppState) -> Client {
    state.mstore.get_all::<Client>().unwrap().into_iter().next().expect("a client row")
}

#[tokio::test]
async fn a_window_for_an_existing_workspace_attaches_to_it_and_makes_nothing_else() {
    let state = test_state();
    let first = open(&state, "", "main").await;
    let (workspaces, blocks) = {
        let s = state.srv_state.lock().await;
        (s.workspaces.len(), s.blocks.len())
    };

    let second = open(&state, &first.workspaceid, "second").await;
    assert_eq!(second.workspaceid, first.workspaceid);
    assert_ne!(second.oid, first.oid);
    let s = state.srv_state.lock().await;
    assert_eq!((s.workspaces.len(), s.blocks.len()), (workspaces, blocks), "no workspace, tab or pane was made");
    drop(s);
    let listed = client(&state).windowids;
    assert!(listed.contains(&first.oid) && listed.contains(&second.oid), "{listed:?}");
}

#[tokio::test]
async fn a_window_for_a_workspace_that_does_not_exist_is_refused() {
    let state = test_state();
    let windows = state.mstore.get_all::<Window>().unwrap().len();
    let ret = create(&state, vec![Value::Null, json!("no-such-workspace")]).await;
    assert!(!ret.success);
    assert_eq!(ret.error.as_deref(), Some("CreateWindow: workspace not found: no-such-workspace"));
    assert_eq!(state.mstore.get_all::<Window>().unwrap().len(), windows, "no window row was made");
}

#[tokio::test]
async fn a_new_window_is_marked_new_and_carries_the_hosts_label_when_given_one() {
    let state = test_state();
    let labelled = open(&state, "", "host-label-1").await;
    assert!(labelled.isnew, "the reply says the window is new");
    let stored = state.mstore.must_get::<Window>(&labelled.oid).unwrap();
    assert!(stored.isnew);
    assert_eq!(stored.meta.get("host:label"), Some(&json!("host-label-1")));

    let unlabelled = create(&state, vec![Value::Null, json!("")]).await;
    let win: Window = serde_json::from_value(unlabelled.data.unwrap()).unwrap();
    assert!(!state.mstore.must_get::<Window>(&win.oid).unwrap().meta.contains_key("host:label"));
}

#[tokio::test]
async fn a_cold_start_with_nothing_to_restore_opens_the_default_panes_and_notes_the_restore() {
    let state = fresh_state().await;
    assert!(!client(&state).meta.contains_key("session:restored_at_ms"));
    let args = vec![Value::Null, json!(""), json!("main"), json!(true)];
    let ret = create(&state, args).await;
    assert!(ret.success, "{:?}", ret.error);
    let win: Window = serde_json::from_value(ret.data.unwrap()).unwrap();
    let ws = state.mstore.must_get::<Workspace>(&win.workspaceid).unwrap();
    let tab = state.mstore.must_get::<crate::backend::obj::Tab>(&ws.tabids[0]).unwrap();
    assert_eq!(tab.blockids.len(), 3, "the default panes");
    assert!(client(&state).meta.get("session:restored_at_ms").is_some_and(|v| v.as_i64().is_some()));
}

#[tokio::test]
async fn a_restore_asked_beside_an_open_window_does_not_restore_and_does_not_wait_when_one_just_did() {
    let state = fresh_state().await;
    let first = create(&state, vec![Value::Null, json!(""), json!("main"), json!(true)]).await;
    let first: Window = serde_json::from_value(first.data.unwrap()).unwrap();

    // Another opener arrives while the first window is open and a restore just happened: it opens
    // the default panes at once, instead of waiting for a close that is not coming.
    let started = std::time::Instant::now();
    let args = vec![Value::Null, json!(""), json!("second"), json!(true), Value::Null, Value::Null, Value::Null, json!(true)];
    let ret = create(&state, args).await;
    assert!(ret.success, "{:?}", ret.error);
    assert!(started.elapsed() < std::time::Duration::from_secs(2), "it did not wait for a close ({:?})", started.elapsed());
    let second: Window = serde_json::from_value(ret.data.unwrap()).unwrap();
    assert_ne!(second.workspaceid, first.workspaceid, "a workspace of its own");
    let ws = state.mstore.must_get::<Workspace>(&second.workspaceid).unwrap();
    let tab = state.mstore.must_get::<crate::backend::obj::Tab>(&ws.tabids[0]).unwrap();
    assert_eq!(tab.blockids.len(), 3);
}

#[tokio::test]
async fn a_restore_asked_beside_an_open_window_waits_for_a_close_that_never_comes_then_opens_the_default_panes() {
    let state = fresh_state().await;
    let first = create(&state, vec![Value::Null, json!(""), json!("main")]).await;
    let first: Window = serde_json::from_value(first.data.unwrap()).unwrap();
    // No restore has been recorded, so this is not "just restored": it waits out the grace period.
    let started = std::time::Instant::now();
    let ret = create(&state, vec![Value::Null, json!(""), json!("second"), json!(true)]).await;
    assert!(ret.success, "{:?}", ret.error);
    assert!(started.elapsed() >= super::super::window_create::RESTORE_WAITS_FOR_CLOSE, "{:?}", started.elapsed());
    let second: Window = serde_json::from_value(ret.data.unwrap()).unwrap();
    assert_ne!(second.workspaceid, first.workspaceid);
    assert!(!client(&state).meta.contains_key("session:restored_at_ms"), "the window list was never empty, so nothing is noted");
}

/// What a window recreated after a crash does, in the order it happens: told the row it replaces,
/// it opens a row on that row's workspace, and the host closes the old row once the new one is up.
#[tokio::test]
async fn a_window_recreated_onto_the_workspace_of_the_row_it_replaces_keeps_it_and_leaves_no_closed_window() {
    let state = test_state();
    let old = open(&state, "", "window-old").await;
    let (workspaces, blocks) = {
        let s = state.srv_state.lock().await;
        (s.workspaces.len(), s.blocks.len())
    };

    let args = vec![Value::Null, json!(old.workspaceid), json!("window-new"), json!(false), Value::Null, Value::Null, json!(old.oid)];
    let ret = create(&state, args).await;
    assert!(ret.success, "{:?}", ret.error);
    let new: Window = serde_json::from_value(ret.data.unwrap()).unwrap();
    assert_eq!(new.workspaceid, old.workspaceid);

    let close = WebCallType { service: "window".into(), method: "CloseWindow".into(), uicontext: None, args: vec![json!(old.oid)] };
    let ret = handle_window_service(&state, &close).await;
    assert!(ret.success, "{:?}", ret.error);

    assert!(state.mstore.get::<Workspace>(&old.workspaceid).unwrap().is_some(), "the workspace is the new window's now");
    let s = state.srv_state.lock().await;
    assert_eq!((s.workspaces.len(), s.blocks.len()), (workspaces, blocks), "nothing was made and nothing was closed");
    drop(s);
    let listed = client(&state).windowids;
    assert!(listed.contains(&new.oid) && !listed.contains(&old.oid), "{listed:?}");
    let closed = super::super::session_closed::closed_entries(&state.mstore);
    assert!(closed.iter().all(|e| e["workspaceid"] != json!(old.workspaceid)), "not a closed window: {closed:?}");
}
