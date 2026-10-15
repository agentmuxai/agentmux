// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A layout intent from the frontend, through `layout.applyIntent`, to the
//! stored layout row. SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30.md §4.

use agentmux_common::ipc::{Command, Event};
use agentmux_common::{FlexDirection, LayoutIntent, LayoutNode, LayoutNodeData, LayoutResizeOp, SplitPosition};

use crate::backend::obj::{LayoutState, Tab};
use crate::backend::rpc_types::CommandLayoutIntentData;
use crate::server::app_api::layout_intent::apply_intent;
use crate::server::tests::test_state;
use crate::server::AppState;

async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
    let events = super::dispatch_to_reducer(state, cmd).await;
    for ev in &events {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
    }
    events
}

fn leaf(id: &str, block: &str) -> LayoutNode {
    LayoutNode {
        id: id.into(),
        flex_direction: FlexDirection::Row,
        size: 10.0,
        data: Some(LayoutNodeData { block_id: block.into(), ..Default::default() }),
        ..Default::default()
    }
}

/// A tab with one pane of block `a` and a second block `b`. Returns
/// `(tab_id, layout_oid, a, b)`.
async fn seed(state: &AppState) -> (String, String, String, String) {
    let ws = dispatch_apply(state, Command::CreateWorkspace { name: "w".into() }).await;
    let Some(Event::WorkspaceCreated { workspace_id, .. }) = ws.first() else { panic!("{ws:?}") };
    let tab = dispatch_apply(state, Command::CreateTab { workspace_id: workspace_id.clone(), name: "t".into() }).await;
    let tab_id = tab
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();
    let mut blocks = Vec::new();
    for _ in 0..2 {
        let meta = serde_json::json!({ "view": "term" });
        let ev = dispatch_apply(state, Command::CreateBlock { tab_id: tab_id.clone(), meta }).await;
        blocks.push(
            ev.iter()
                .find_map(|e| match e {
                    Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
                    _ => None,
                })
                .unwrap(),
        );
    }
    let seed_tree = Command::LayoutSetTree {
        tab_id: tab_id.clone(),
        new_tree: Some(leaf("na", &blocks[0])),
        correlation_id: String::new(),
        slices: None,
    };
    dispatch_apply(state, seed_tree).await;
    let layout_oid = state.mstore.must_get::<Tab>(&tab_id).unwrap().layoutstate;
    (tab_id, layout_oid, blocks[0].clone(), blocks[1].clone())
}

fn row(state: &AppState, oid: &str) -> LayoutState {
    state.mstore.must_get::<LayoutState>(oid).unwrap()
}

async fn reducer_tree(state: &AppState, tab_id: &str) -> Option<LayoutNode> {
    state.srv_state.lock().await.tabs[tab_id].rootnode.clone()
}

fn request(tab_id: &str, action_id: &str, intent: LayoutIntent, focused: &str) -> CommandLayoutIntentData {
    CommandLayoutIntentData {
        tab_id: tab_id.into(),
        action_id: action_id.into(),
        intent,
        focused_node_id: focused.into(),
        magnified_node_id: String::new(),
    }
}

#[tokio::test]
async fn an_intent_is_stored_as_the_reducer_computed_it() {
    let state = test_state();
    let (tab_id, oid, _a, b) = seed(&state).await;
    let version_before = row(&state, &oid).version as u64;
    let split = LayoutIntent::SplitHorizontal {
        target_node_id: "na".into(),
        new_node: leaf("nb", &b),
        position: SplitPosition::After,
        size_fraction: Some(0.5),
        focused: true,
    };
    let res = apply_intent(&state, request(&tab_id, "act-1", split, "nb")).await.unwrap();
    assert!(res.applied, "{res:?}");
    let stored = row(&state, &oid);
    assert!(res.version > version_before);
    assert_eq!(res.version, stored.version as u64, "the answer names the stored row's version");
    assert_eq!(stored.rootnode, reducer_tree(&state, &tab_id).await, "the row holds the reducer's tree");
    let root = stored.rootnode.unwrap();
    assert_eq!(root.children.iter().map(|c| c.size).collect::<Vec<_>>(), [5.0, 5.0]);
    assert_eq!(stored.focusednodeid, "nb");

    let resize = LayoutIntent::Resize {
        resize_operations: vec![
            LayoutResizeOp { node_id: "na".into(), size: 3.0 },
            LayoutResizeOp { node_id: "nb".into(), size: 7.0 },
        ],
    };
    apply_intent(&state, request(&tab_id, "act-2", resize, "nb")).await.unwrap();
    let stored = row(&state, &oid);
    assert_eq!(stored.rootnode, reducer_tree(&state, &tab_id).await);
    assert_eq!(stored.rootnode.unwrap().children[0].size, 3.0);
}

#[tokio::test]
async fn a_refused_intent_leaves_the_row_alone() {
    let state = test_state();
    let (tab_id, oid, ..) = seed(&state).await;
    let before = row(&state, &oid);
    let delete = LayoutIntent::Delete { node_id: "missing".into() };
    let res = apply_intent(&state, request(&tab_id, "act-1", delete, "")).await.unwrap();
    assert!(!res.applied);
    assert!(res.error.unwrap().contains("missing"));
    let after = row(&state, &oid);
    assert_eq!(after.version, before.version);
    assert_eq!(res.version, after.version as u64);
    assert_eq!(after.rootnode, before.rootnode);
}

#[tokio::test]
async fn a_resent_intent_is_applied_once() {
    let state = test_state();
    let (tab_id, oid, _a, b) = seed(&state).await;
    let insert = || LayoutIntent::Insert { node: leaf("nb", &b), magnified: false, focused: false };
    apply_intent(&state, request(&tab_id, "act-1", insert(), "")).await.unwrap();
    let once = row(&state, &oid);
    let res = apply_intent(&state, request(&tab_id, "act-1", insert(), "")).await.unwrap();
    assert!(res.applied);
    let twice = row(&state, &oid);
    assert_eq!(twice.version, once.version);
    assert_eq!(twice.rootnode, once.rootnode);
}

#[tokio::test]
async fn an_unknown_tab_is_an_error() {
    let state = test_state();
    let err = apply_intent(&state, request("nope", "act-1", LayoutIntent::View, "")).await.unwrap_err();
    assert!(err.contains("nope"), "{err}");
}

/// `object.ApplyLayoutIntent`, the service form the tools/tests harnesses use,
/// stores the same edit `layout.applyIntent` does.
#[tokio::test]
async fn the_service_method_applies_an_intent() {
    use crate::backend::service::WebCallType;
    let state = test_state();
    let (tab_id, oid, _a, b) = seed(&state).await;
    let intent = serde_json::json!({
        "type": "splithorizontal",
        "targetNodeId": "na",
        "newNode": { "id": "nb", "flexDirection": "row", "size": 10, "data": { "blockId": b } },
        "position": "after",
        "focused": false,
    });
    let call = |args: Vec<serde_json::Value>| WebCallType {
        service: "object".into(),
        method: "ApplyLayoutIntent".into(),
        uicontext: None,
        args,
    };
    let ret = super::run_service_call(&state, &call(vec![tab_id.clone().into(), intent, "act-1".into()])).await;
    let data = ret.data.unwrap_or_else(|| panic!("{:?}", ret.error));
    assert_eq!(data["applied"], true, "{data}");
    let stored = row(&state, &oid);
    assert_eq!(data["version"], stored.version);
    assert_eq!(stored.rootnode, reducer_tree(&state, &tab_id).await);
    let kids: Vec<_> = stored.rootnode.unwrap().children.iter().map(|c| c.id.clone()).collect();
    assert_eq!(kids, ["na", "nb"]);

    let bad = serde_json::json!({ "type": "delete", "nodeId": "missing" });
    let ret = super::run_service_call(&state, &call(vec![tab_id.into(), bad, "act-2".into()])).await;
    assert_eq!(ret.data.unwrap()["applied"], false);
}

/// Bootstrap the reducer from the store into a fresh state, as srv startup does.
async fn bootstrap_fresh(state: &AppState) -> std::sync::Arc<tokio::sync::Mutex<crate::state::State>> {
    let fresh = std::sync::Arc::new(tokio::sync::Mutex::new(crate::state::State::default()));
    crate::persist::bootstrap_state_from_mstore(&fresh, &state.mstore).await;
    fresh
}

/// A row stored by an earlier build with a leaf for a block another tab owns
/// and one for a deleted block: bootstrap removes both from the reducer's tree
/// and the stored row, and clears the focus that pointed at one of them.
#[tokio::test]
async fn bootstrap_heals_leaves_for_blocks_the_tab_does_not_own() {
    let state = test_state();
    let (tab_id, oid, a, b) = seed(&state).await;
    let ws_id = state.mstore.must_get::<Tab>(&tab_id).unwrap();
    let ws_id = state.srv_state.lock().await.tabs[&ws_id.oid].workspace_id.clone();
    let other = dispatch_apply(&state, Command::CreateTab { workspace_id: ws_id, name: "o".into() }).await;
    let other_tab = other
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();
    let ev = dispatch_apply(&state, Command::CreateBlock { tab_id: other_tab, meta: serde_json::json!({ "view": "term" }) }).await;
    let elsewhere = ev
        .iter()
        .find_map(|e| match e {
            Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .unwrap();

    let mut stored = row(&state, &oid);
    stored.rootnode = Some(LayoutNode {
        id: "root".into(),
        flex_direction: FlexDirection::Row,
        size: 10.0,
        children: vec![leaf("na", &a), leaf("nb", &b), leaf("nx", &elsewhere), leaf("ng", "deleted-block")],
        ..Default::default()
    });
    stored.focusednodeid = "ng".into();
    state.mstore.update(&mut stored).unwrap();
    let version_before = row(&state, &oid).version;

    let fresh = bootstrap_fresh(&state).await;
    let healed = row(&state, &oid);
    let kids: Vec<_> = healed.rootnode.as_ref().unwrap().children.iter().map(|c| c.id.clone()).collect();
    assert_eq!(kids, ["na", "nb"]);
    assert!(healed.version > version_before, "the healed row is stored with a new version");
    assert_eq!(healed.focusednodeid, "", "the focus on a removed leaf is cleared");
    let tab = fresh.lock().await.tabs[&tab_id].clone();
    assert_eq!(tab.rootnode, healed.rootnode, "the reducer holds the stored tree");
    assert_eq!(tab.focused_node_id, "");
    assert!(crate::backend::layout::validate_layout_invariants(&healed.rootnode).is_empty());
}

/// A row with nothing to heal is left exactly as stored: no write.
#[tokio::test]
async fn bootstrap_leaves_a_clean_layout_row_untouched() {
    let state = test_state();
    let (tab_id, oid, ..) = seed(&state).await;
    let before = serde_json::to_string(&row(&state, &oid)).unwrap();
    let fresh = bootstrap_fresh(&state).await;
    assert_eq!(serde_json::to_string(&row(&state, &oid)).unwrap(), before);
    assert_eq!(fresh.lock().await.tabs[&tab_id].rootnode, row(&state, &oid).rootnode);
}
