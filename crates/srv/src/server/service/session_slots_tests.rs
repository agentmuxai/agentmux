// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Window slots (`session_slots`): every window keeps its place in the panel's list. Closing keeps it,
//! reopening returns to it, a new window goes at the end, only a deleted one gives its slot up, and the
//! order survives a restart and a new version's first start.

use serde_json::json;

use super::super::session_slots::{adopt, forget, into_record, place, renamed, slots, SLOTS_META_KEY};
use super::super::session_windows::{set_client_meta, windows_of};
use super::super::user_workspace;
use super::tests::{add_user_pane, close, closed_ids, fresh_state, open_window, rpc};
use super::*;

fn keys(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// What the panel sends: the open windows' workspaces, then the closed ones.
async fn panel_places(state: &AppState, open: &[&Window]) -> Vec<String> {
    let mut shown: Vec<String> = open.iter().map(|w| w.workspaceid.clone()).collect();
    shown.extend(closed_ids(state));
    let ret = rpc(state, "PlaceWindowSlots", vec![json!(shown)]).await;
    assert!(ret.success, "PlaceWindowSlots failed: {:?}", ret.error);
    serde_json::from_value(ret.data.unwrap()).unwrap()
}

#[tokio::test]
async fn a_window_without_a_slot_takes_the_next_one_at_the_end() {
    let state = fresh_state().await;
    assert_eq!(place(&state.mstore, &keys(&["a", "b"])), keys(&["a", "b"]));
    // known ones keep their places whatever order they are asked in; a new one goes last
    assert_eq!(place(&state.mstore, &keys(&["b", "c", "a"])), keys(&["a", "b", "c"]));
    assert_eq!(place(&state.mstore, &keys(&["", "c", "a"])), keys(&["a", "b", "c"]), "an empty key is no window");
}

#[tokio::test]
async fn a_window_takes_the_next_slot_when_it_is_created_whether_or_not_a_panel_is_open() {
    let state = fresh_state().await;
    let first = open_window(&state, "main", false).await;
    let second = open_window(&state, "window-2", false).await;
    let third = open_window(&state, "window-3", false).await;
    assert_eq!(slots(&state.mstore), keys(&[&first.workspaceid, &second.workspaceid, &third.workspaceid]));
}

#[tokio::test]
async fn the_list_is_capped_and_the_oldest_slots_go_first() {
    let state = fresh_state().await;
    let many: Vec<String> = (0..205).map(|i| format!("w{i}")).collect();
    let list = place(&state.mstore, &many);
    assert_eq!(list.len(), 200);
    assert_eq!(list.first().map(String::as_str), Some("w5"));
    assert_eq!(list.last().map(String::as_str), Some("w204"));
}

#[tokio::test]
async fn closing_a_window_in_the_middle_keeps_its_slot_and_reopening_returns_to_it() {
    let state = fresh_state().await;
    let first = open_window(&state, "main", false).await;
    add_user_pane(&state, &first.workspaceid, "term").await;
    let second = open_window(&state, "window-2", false).await;
    add_user_pane(&state, &second.workspaceid, "term").await;
    let third = open_window(&state, "window-3", false).await;
    add_user_pane(&state, &third.workspaceid, "term").await;
    let order = panel_places(&state, &[&first, &second, &third]).await;
    assert_eq!(order, keys(&[&first.workspaceid, &second.workspaceid, &third.workspaceid]));

    // the middle one closes: the panel now lists the open ones, then the closed one
    close(&state, &second.oid).await;
    assert_eq!(closed_ids(&state), keys(&[&second.workspaceid]));
    assert_eq!(panel_places(&state, &[&first, &third]).await, order, "it keeps its slot");

    // reopened: the same contents, the same slot
    let taken = rpc(&state, "TakeClosedWindow", vec![json!(second.workspaceid)]).await;
    assert!(taken.success, "{:?}", taken.error);
    assert_eq!(panel_places(&state, &[&first, &third, &second]).await, order, "it returns to its slot");
}

#[tokio::test]
async fn a_new_window_goes_at_the_end() {
    let state = fresh_state().await;
    let first = open_window(&state, "main", false).await;
    add_user_pane(&state, &first.workspaceid, "term").await;
    let second = open_window(&state, "window-2", false).await;
    add_user_pane(&state, &second.workspaceid, "term").await;
    panel_places(&state, &[&first, &second]).await;
    close(&state, &first.oid).await;
    let third = open_window(&state, "window-3", false).await;
    let order = panel_places(&state, &[&second, &third]).await;
    assert_eq!(order, keys(&[&first.workspaceid, &second.workspaceid, &third.workspaceid]), "after the closed one too");
}

#[tokio::test]
async fn deleting_a_window_closes_the_gap() {
    let state = fresh_state().await;
    let first = open_window(&state, "main", false).await;
    add_user_pane(&state, &first.workspaceid, "term").await;
    let second = open_window(&state, "window-2", false).await;
    add_user_pane(&state, &second.workspaceid, "term").await;
    let third = open_window(&state, "window-3", false).await;
    add_user_pane(&state, &third.workspaceid, "term").await;
    let fourth = open_window(&state, "window-4", false).await;
    add_user_pane(&state, &fourth.workspaceid, "term").await;
    panel_places(&state, &[&first, &second, &third, &fourth]).await;

    // a closed window removed from the list
    close(&state, &second.oid).await;
    let gone = rpc(&state, "DeleteClosedWindow", vec![json!(second.workspaceid)]).await;
    assert!(gone.success, "{:?}", gone.error);
    assert_eq!(slots(&state.mstore), keys(&[&first.workspaceid, &third.workspaceid, &fourth.workspaceid]));

    // an open window removed from the workspace
    let marked = rpc(&state, "DiscardWorkspaceOnClose", vec![json!(third.oid)]).await;
    assert!(marked.success, "{:?}", marked.error);
    close(&state, &third.oid).await;
    assert_eq!(slots(&state.mstore), keys(&[&first.workspaceid, &fourth.workspaceid]));
}

#[tokio::test]
async fn the_order_is_saved_with_the_workspace_and_adopted_when_it_loads() {
    let state = fresh_state().await;
    place(&state.mstore, &keys(&["x", "y", "z"]));
    let mut record = json!({ "tabs": [], "windows": [] });
    into_record(&state.mstore, &mut record);
    assert_eq!(record["window_slots"], json!(["x", "y", "z"]));
    // a record without an order (an older version's) leaves the order alone
    adopt(&state.mstore, &json!({ "windows": [] }));
    assert_eq!(slots(&state.mstore), keys(&["x", "y", "z"]));
    // another workspace's record brings its own
    adopt(&state.mstore, &json!({ "window_slots": ["q", "r"] }));
    assert_eq!(slots(&state.mstore), keys(&["q", "r"]));
    forget(&state.mstore, "q");
    assert_eq!(slots(&state.mstore), keys(&["r"]));
}

#[tokio::test]
async fn a_rebuilt_window_keeps_the_slot_of_the_one_it_was_rebuilt_from() {
    let state = fresh_state().await;
    place(&state.mstore, &keys(&["a", "b", "c"]));
    renamed(&state.mstore, "b", "b2");
    assert_eq!(slots(&state.mstore), keys(&["a", "b2", "c"]));
    renamed(&state.mstore, "zzz", "b2");
    renamed(&state.mstore, "c", "a");
    assert_eq!(slots(&state.mstore), keys(&["a", "b2"]), "never two slots for one window");
}

#[tokio::test]
async fn the_order_survives_a_restart() {
    let state = fresh_state().await;
    let main = open_window(&state, "main", false).await;
    add_user_pane(&state, &main.workspaceid, "term").await;
    let other = open_window(&state, "window-2", false).await;
    add_user_pane(&state, &other.workspaceid, "term").await;
    // the second window holds the first slot, though it opened later
    adopt(&state.mstore, &json!({ "window_slots": [other.workspaceid, main.workspaceid] }));
    close(&state, &other.oid).await;
    close(&state, &main.oid).await;

    let _relaunched = open_window(&state, "main", true).await;
    assert_eq!(slots(&state.mstore), keys(&[&other.workspaceid, &main.workspaceid]));
    assert_eq!(closed_ids(&state), keys(&[&other.workspaceid]), "still closed, still first");
}

#[tokio::test]
async fn the_order_survives_a_new_versions_first_start_that_rebuilds_the_windows() {
    let state = fresh_state().await;
    let main = open_window(&state, "main", false).await;
    add_user_pane(&state, &main.workspaceid, "term").await;
    let other = open_window(&state, "window-2", false).await;
    add_user_pane(&state, &other.workspaceid, "term").await;
    adopt(&state.mstore, &json!({ "window_slots": [other.workspaceid, main.workspaceid] }));
    close(&state, &other.oid).await;
    close(&state, &main.oid).await;
    let id = user_workspace::current_id(&state.mstore).unwrap();
    let saved: serde_json::Value =
        serde_json::from_str(&state.mstore.user_workspace_get(&id).unwrap().unwrap().record.unwrap()).unwrap();
    assert_eq!(saved["window_slots"], json!([other.workspaceid, main.workspaceid]), "saved with the record");
    assert!(windows_of(&saved).len() >= 2);

    // As in a new version's first start, which copied only the record: the closed window's contents
    // are gone, and so is the order the old database held.
    crate::backend::wcore::delete_workspace(&state.mstore, &other.workspaceid).unwrap();
    state.srv_state.lock().await.workspaces.remove(&other.workspaceid);
    set_client_meta(&state.mstore, super::super::session_closed::CLOSED_META_KEY, None);
    set_client_meta(&state.mstore, SLOTS_META_KEY, None);
    crate::server::service::session_restore::clear_last_session_snapshot(&state.mstore);

    let relaunched = open_window(&state, "main", true).await;
    let closed = closed_ids(&state);
    assert_eq!(closed.len(), 1, "listed as closed again");
    assert_ne!(closed[0], other.workspaceid, "rebuilt, with new ids");
    assert_eq!(slots(&state.mstore), keys(&[&closed[0], &relaunched.workspaceid]), "still the first slot");
}
