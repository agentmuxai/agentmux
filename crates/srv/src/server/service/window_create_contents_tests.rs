// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `create_default_contents` called directly: the workspace, tab, seed panes and layout a new window
//! opens onto, and the events it hands back unpublished.

use agentmux_common::ipc::Event;
use serde_json::json;

use super::create_default_contents;
use crate::backend::obj::{Block, Client, LayoutNode, LayoutState, Tab, Workspace};
use crate::server::tests::test_state;
use crate::server::AppState;

/// The block ids of a layout's leaves, in tree order.
fn leaf_block_ids(node: &LayoutNode, out: &mut Vec<String>) {
    if let Some(data) = &node.data {
        out.push(data.block_id.clone());
    }
    for child in &node.children {
        leaf_block_ids(child, out);
    }
}

fn the_tab_of(state: &AppState, ws_id: &str) -> Tab {
    let ws = state.mstore.must_get::<Workspace>(ws_id).unwrap();
    assert_eq!(ws.tabids.len(), 1, "one tab");
    state.mstore.must_get::<Tab>(&ws.tabids[0]).unwrap()
}

fn view_of(state: &AppState, block_id: &str) -> String {
    let block = state.mstore.must_get::<Block>(block_id).unwrap();
    block.meta.get("view").and_then(|v| v.as_str()).unwrap_or_default().to_string()
}

fn count(events: &[Event], is: impl Fn(&Event) -> bool) -> usize {
    events.iter().filter(|e| is(e)).count()
}

/// With no seed view the tab holds agent, swarm and sysinfo (in the order the tree is built from),
/// laid out as the shared three-pane tree, and the events cover the workspace, the tab and each pane.
#[tokio::test]
async fn the_default_contents_are_a_workspace_with_one_tab_of_three_panes() {
    let state = test_state();
    let (ws_id, events) = create_default_contents(&state, None, None, false).await.unwrap();

    assert_eq!(count(&events, |e| matches!(e, Event::WorkspaceCreated { workspace_id, .. } if *workspace_id == ws_id)), 1);
    assert_eq!(count(&events, |e| matches!(e, Event::TabCreated { .. })), 1);
    assert_eq!(count(&events, |e| matches!(e, Event::BlockCreated { .. })), 3);

    let tab = the_tab_of(&state, &ws_id);
    let views: Vec<String> = tab.blockids.iter().map(|id| view_of(&state, id)).collect();
    assert_eq!(views, ["agent", "swarm", "sysinfo"]);

    let layout = state.mstore.must_get::<LayoutState>(&tab.layoutstate).unwrap();
    let mut actual = Vec::new();
    leaf_block_ids(layout.rootnode.as_ref().expect("a layout"), &mut actual);
    let mut expected = Vec::new();
    leaf_block_ids(&crate::backend::wcore::default_three_pane_tree("agent", "swarm", "sysinfo").0, &mut expected);
    let actual_views: Vec<String> = actual.iter().map(|id| view_of(&state, id)).collect();
    assert_eq!(actual_views, expected, "the shared three-pane tree's order");
}

/// A seed view alone opens one pane of that view, its meta a bare `{ view }` when none is given, in a
/// single-leaf layout.
#[tokio::test]
async fn a_seed_view_opens_one_pane_of_that_view() {
    let state = test_state();
    let (ws_id, events) = create_default_contents(&state, Some("browser".into()), None, false).await.unwrap();

    assert_eq!(count(&events, |e| matches!(e, Event::BlockCreated { .. })), 1);
    let tab = the_tab_of(&state, &ws_id);
    assert_eq!(tab.blockids.len(), 1);
    let block = state.mstore.must_get::<Block>(&tab.blockids[0]).unwrap();
    assert_eq!(serde_json::to_value(&block.meta).unwrap(), json!({ "view": "browser" }));

    let layout = state.mstore.must_get::<LayoutState>(&tab.layoutstate).unwrap();
    let root = layout.rootnode.as_ref().expect("a layout");
    assert!(root.children.is_empty());
    assert_eq!(root.data.as_ref().unwrap().block_id, tab.blockids[0]);
}

/// The meta a widget asks for is used whole for its seed pane.
#[tokio::test]
async fn a_seed_meta_is_used_whole_for_the_seed_pane() {
    let state = test_state();
    let meta = json!({ "view": "browser", "url": "https://example.com" });
    let (ws_id, _) = create_default_contents(&state, Some("browser".into()), Some(meta.clone()), false).await.unwrap();

    let tab = the_tab_of(&state, &ws_id);
    let block = state.mstore.must_get::<Block>(&tab.blockids[0]).unwrap();
    assert_eq!(serde_json::to_value(&block.meta).unwrap(), meta);
}

fn seeded_note(state: &AppState) -> Option<serde_json::Value> {
    let client = state.mstore.get_all::<Client>().unwrap().into_iter().next().expect("a client row");
    client.meta.get("session:seeded_default").cloned().filter(|v| !v.is_null())
}

/// A cold start that offers another build's layout notes which workspace, tab and panes it seeded; a
/// seed view, or a caller that doesn't offer, notes nothing for its workspace.
#[tokio::test]
async fn only_a_default_cold_start_notes_what_it_seeded() {
    let state = test_state();
    let (quiet_ws, _) = create_default_contents(&state, None, None, false).await.unwrap();
    assert_ne!(seeded_note(&state).and_then(|n| n["workspace_id"].as_str().map(String::from)), Some(quiet_ws));
    let (seeded_view_ws, _) = create_default_contents(&state, Some("browser".into()), None, true).await.unwrap();
    assert_ne!(seeded_note(&state).and_then(|n| n["workspace_id"].as_str().map(String::from)), Some(seeded_view_ws));

    let (ws_id, _) = create_default_contents(&state, None, None, true).await.unwrap();
    let tab = the_tab_of(&state, &ws_id);
    let note = seeded_note(&state).expect("the seeded default is noted");
    assert_eq!(note["workspace_id"], json!(ws_id));
    assert_eq!(note["tab_id"], json!(tab.oid));
    assert_eq!(note["block_ids"], json!(tab.blockids));
}
