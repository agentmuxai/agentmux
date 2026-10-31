// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A closed window's agents keep running: its close parks their blocks (`agents:always_available`),
//! the closed window's record names them, and reopening it puts them back in their tabs.

use agentmux_common::ipc::Command;
use agentmux_common::{LayoutNode, LayoutNodeData};
use serde_json::json;

use super::*;
use crate::backend::agent_state::test_support::register_stub;
use crate::backend::blockcontroller::{self, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING};
use crate::persist_subscriber::parked::{PARKED_FROM_TAB_META_KEY, PARKED_META_KEY};
use crate::sagas::park_agents_tests::{in_tab, seed_agent_block, set_always_available};
use crate::server::service::reducer_helpers::dispatch_to_reducer;
use super::tests::{add_user_pane, block_ids, close, closed_ids, fresh_state, open_window, rpc};

/// The first tab of workspace `ws_id`.
fn first_tab(state: &AppState, ws_id: &str) -> String {
    let ws = state.mstore.get::<Workspace>(ws_id).unwrap().unwrap();
    ws.tabids.iter().chain(ws.pinnedtabids.iter()).next().unwrap().clone()
}

/// A window beside the main one, with a terminal and an agent running in it.
struct TwoWindows {
    state: AppState,
    other: crate::backend::obj::Window,
    tab: String,
    agent: String,
}

async fn window_with_an_agent(id: &str, with_terminal: bool) -> TwoWindows {
    let state = fresh_state().await;
    set_always_available(&state, true);
    let _main = open_window(&state, "main", false).await;
    let other = open_window(&state, "window-2", false).await;
    let tab = first_tab(&state, &other.workspaceid);
    if with_terminal {
        add_user_pane(&state, &other.workspaceid, "term").await;
    }
    let agent = seed_agent_block(&state, &tab, id).await;
    register_stub(&agent, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING, false);
    TwoWindows { state, other, tab, agent }
}

fn parked_meta(state: &AppState, block_id: &str) -> (bool, String) {
    let block = state.mstore.get::<Block>(block_id).unwrap().unwrap();
    (
        crate::backend::obj::meta_get_bool(&block.meta, PARKED_META_KEY, false),
        crate::backend::obj::meta_get_string(&block.meta, PARKED_FROM_TAB_META_KEY, ""),
    )
}

#[tokio::test]
async fn closing_a_window_parks_its_agents_and_reopening_it_puts_them_back() {
    let t = window_with_an_agent("ag-win", true).await;
    let state = &t.state;

    close(state, &t.other.oid).await;

    assert_eq!(closed_ids(state), vec![t.other.workspaceid.clone()]);
    assert_eq!(in_tab(state, &t.agent).await.as_deref(), Some(""), "the agent is parked, in no tab");
    assert_eq!(parked_meta(state, &t.agent), (true, t.tab.clone()), "and remembers its tab");
    assert!(blockcontroller::get_controller(&t.agent).is_some(), "its process is not stopped");
    let entry = entries(&state.mstore).into_iter().next().unwrap();
    assert_eq!(parked_of(&entry), vec![t.agent.clone()], "the closed window names it");

    let ret = rpc(state, "TakeClosedWindow", vec![json!(t.other.workspaceid)]).await;
    assert!(ret.success, "{:?}", ret.error);
    assert_eq!(in_tab(state, &t.agent).await.as_deref(), Some(t.tab.as_str()), "back in its tab");
    assert!(block_ids(state, &t.other.workspaceid).contains(&t.agent));
    assert!(!parked_meta(state, &t.agent).0, "no longer parked");
}

/// Parking takes a window's agents out of its tabs: it is still a closed window with the same tabs and
/// panes, and its agents counted.
#[tokio::test]
async fn a_closed_window_is_listed_with_its_parked_agents_counted() {
    let t = window_with_an_agent("ag-only", false).await;
    let state = &t.state;
    let panes = block_ids(state, &t.other.workspaceid).len();
    assert!(panes > 0);

    close(state, &t.other.oid).await;

    assert_eq!(in_tab(state, &t.agent).await.as_deref(), Some(""));
    let listed = rpc(state, "ListClosedWindows", vec![]).await.data.unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1, "{listed}");
    assert_eq!(listed[0]["workspaceid"], json!(t.other.workspaceid));
    assert_eq!(listed[0]["tabs"], 1);
    assert_eq!(listed[0]["panes"], json!(panes), "the parked agent is still one of its panes");
    assert_eq!(listed[0]["agents"], json!(["ag-only"]));

    assert!(rpc(state, "TakeClosedWindow", vec![json!(t.other.workspaceid)]).await.success);
    assert_eq!(in_tab(state, &t.agent).await.as_deref(), Some(t.tab.as_str()));
}

#[tokio::test]
async fn with_the_setting_off_a_closed_windows_agents_stay_in_its_tabs_as_before() {
    let t = window_with_an_agent("ag-off", true).await;
    set_always_available(&t.state, false);

    close(&t.state, &t.other.oid).await;

    assert_eq!(in_tab(&t.state, &t.agent).await.as_deref(), Some(t.tab.as_str()), "kept in the window, not parked");
    assert!(parked_of(&entries(&t.state.mstore).into_iter().next().unwrap()).is_empty());
}

/// The close that ends the session reopens with its panes, so its agents stop there as before.
#[tokio::test]
async fn closing_the_last_window_does_not_park_its_agents() {
    let state = fresh_state().await;
    set_always_available(&state, true);
    let only = open_window(&state, "main", false).await;
    let tab = first_tab(&state, &only.workspaceid);
    add_user_pane(&state, &only.workspaceid, "term").await;
    let agent = seed_agent_block(&state, &tab, "ag-last").await;

    close(&state, &only.oid).await;

    assert_eq!(in_tab(&state, &agent).await.as_deref(), Some(tab.as_str()));
}

/// An agent opened somewhere else while its window was closed stays where it is; the closed window's
/// layout drops its leaf so the window doesn't show the agent twice.
#[tokio::test]
async fn an_agent_opened_elsewhere_is_not_brought_back_and_its_leaf_goes() {
    let t = window_with_an_agent("ag-moved", true).await;
    let state = &t.state;
    let leaf = LayoutNode {
        id: "leaf-agent".into(),
        data: Some(LayoutNodeData { block_id: t.agent.clone(), ..Default::default() }),
        ..Default::default()
    };
    dispatch_to_reducer(
        state,
        Command::LayoutSetTree { tab_id: t.tab.clone(), new_tree: Some(leaf), correlation_id: String::new(), slices: None },
    )
    .await;
    let has_leaf = |state: &AppState| {
        let tab = t.tab.clone();
        let agent = t.agent.clone();
        let state = state.clone();
        async move {
            state
                .srv_state
                .lock()
                .await
                .tabs
                .get(&tab)
                .and_then(|t| t.rootnode.as_ref())
                .is_some_and(|root| crate::backend::layout::find_leaf_containing_block(root, &agent).is_some())
        }
    };
    assert!(has_leaf(state).await, "the tree holds the agent's leaf");

    close(state, &t.other.oid).await;
    assert!(has_leaf(state).await, "parking leaves the layout alone");

    // The agent is opened in the main window's tab meanwhile.
    let main_tab = first_tab(state, &client(&state.mstore).unwrap().windowids.first().map(|w| state.mstore.get::<Window>(w).unwrap().unwrap().workspaceid).unwrap());
    crate::sagas::park_agents_tests::dispatch_apply(
        state,
        Command::AttachBlock { block_id: t.agent.clone(), dst_tab_id: main_tab.clone(), dst_index: 0 },
    )
    .await;

    assert!(rpc(state, "TakeClosedWindow", vec![json!(t.other.workspaceid)]).await.success);
    assert_eq!(in_tab(state, &t.agent).await.as_deref(), Some(main_tab.as_str()), "it stays where it was opened");
    assert!(!has_leaf(state).await, "and the reopened window doesn't hold its leaf");
}
