// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The `waveobj:update` frames each reducer event produces, and their order, for every event the
//! bridge handles (the three delete-ordering and layout-queue cases sit in the module's own tests).

use super::*;
use crate::backend::obj::{Block, LayoutState, MetaMapType, Window, Workspace};

const WS: &str = "10000000-0000-0000-0000-000000000001";
const WS2: &str = "10000000-0000-0000-0000-000000000002";
const TAB: &str = "20000000-0000-0000-0000-000000000001";
const TAB2: &str = "20000000-0000-0000-0000-000000000002";
const WIN: &str = "30000000-0000-0000-0000-000000000001";
const BLK: &str = "40000000-0000-0000-0000-000000000001";
const LAY: &str = "50000000-0000-0000-0000-000000000001";
const CLIENT: &str = "60000000-0000-0000-0000-000000000001";

struct Harness {
    _tmp: tempfile::TempDir,
    mstore: Arc<Store>,
    event_bus: Arc<EventBus>,
    rx: crate::backend::eventbus::WsReceivers,
}

/// A store holding a client, two workspaces, two tabs (the first with a layout), a window and a
/// block, so every arm has something to fetch.
fn harness() -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let mstore = Arc::new(Store::open(&tmp.path().join("objects.db")).unwrap());
    for oid in [WS, WS2] {
        let mut ws = Workspace { oid: oid.into(), version: 1, name: "ws".into(), meta: MetaMapType::new(), ..Default::default() };
        mstore.insert(&mut ws).unwrap();
    }
    let mut layout = LayoutState { oid: LAY.into(), version: 1, ..Default::default() };
    mstore.insert(&mut layout).unwrap();
    for (oid, layoutstate) in [(TAB, LAY), (TAB2, "")] {
        let mut tab = Tab { oid: oid.into(), version: 1, name: "tab".into(), layoutstate: layoutstate.into(), blockids: vec![], meta: MetaMapType::new() };
        mstore.insert(&mut tab).unwrap();
    }
    let mut window = Window { oid: WIN.into(), version: 1, ..Default::default() };
    mstore.insert(&mut window).unwrap();
    let mut block = Block { oid: BLK.into(), version: 1, ..Default::default() };
    mstore.insert(&mut block).unwrap();
    let mut client = Client { oid: CLIENT.into(), version: 1, ..Default::default() };
    mstore.insert(&mut client).unwrap();
    let event_bus = Arc::new(EventBus::new());
    let rx = event_bus.register_ws("test-conn", "test-tab");
    Harness { _tmp: tmp, mstore, event_bus, rx }
}

/// Dispatch `event` and return the frames it broadcast, in order, as `("otype:oid", updatetype)`.
async fn frames(h: &mut Harness, event: Event) -> Vec<(String, String)> {
    dispatch_event(event, Arc::clone(&h.mstore), Arc::clone(&h.event_bus)).await;
    let mut out = Vec::new();
    while let Ok(frame) = h.rx.priority.try_recv() {
        assert_eq!(frame["eventtype"], "waveobj:update");
        out.push((frame["oref"].as_str().unwrap().to_string(), frame["data"]["updatetype"].as_str().unwrap().to_string()));
    }
    out
}

fn f(oref: &str, kind: &str) -> (String, String) {
    (oref.to_string(), kind.to_string())
}

fn s(v: &str) -> String {
    v.to_string()
}

#[tokio::test]
async fn workspace_create_rename_and_meta_events_broadcast_the_workspace() {
    let mut h = harness();
    let events = [
        Event::WorkspaceCreated { workspace_id: s(WS), name: s("n"), version: 2 },
        Event::WorkspaceRenamed { workspace_id: s(WS), name: s("n"), version: 2 },
        Event::WorkspaceMetaUpdated { workspace_id: s(WS), meta_patch: serde_json::json!({}), version: 2 },
    ];
    for event in events {
        assert_eq!(frames(&mut h, event).await, vec![f(&format!("workspace:{WS}"), "update")]);
    }
}

#[tokio::test]
async fn an_event_for_a_missing_object_broadcasts_nothing() {
    let mut h = harness();
    let gone = "10000000-0000-0000-0000-0000000000ff";
    assert!(frames(&mut h, Event::WorkspaceRenamed { workspace_id: s(gone), name: s("n"), version: 2 }).await.is_empty());
}

#[tokio::test]
async fn workspace_deleted_broadcasts_a_delete_without_reading_the_store() {
    let mut h = harness();
    let gone = "10000000-0000-0000-0000-0000000000ff";
    let out = frames(&mut h, Event::WorkspaceDeleted { workspace_id: s(gone), block_ids: vec![], version: 2 }).await;
    assert_eq!(out, vec![f(&format!("workspace:{gone}"), "delete")]);
}

#[tokio::test]
async fn window_meta_and_workspace_change_broadcast_the_window() {
    let mut h = harness();
    let meta = Event::WindowMetaUpdated { window_id: s(WIN), meta_patch: serde_json::json!({}), version: 2 };
    assert_eq!(frames(&mut h, meta).await, vec![f(&format!("window:{WIN}"), "update")]);
    let moved = Event::SrvWindowWorkspaceChanged { window_id: s(WIN), workspace_id: s(WS), version: 2 };
    assert_eq!(frames(&mut h, moved).await, vec![f(&format!("window:{WIN}"), "update")]);
}

#[tokio::test]
async fn a_window_opening_broadcasts_the_window_then_the_client_and_closing_reverses_it() {
    let mut h = harness();
    let opened = Event::SrvWindowOpened { window_id: s(WIN), workspace_id: s(WS), version: 2 };
    assert_eq!(frames(&mut h, opened).await, vec![f(&format!("window:{WIN}"), "update"), f(&format!("client:{CLIENT}"), "update")]);
    let closed = Event::SrvWindowClosed { window_id: s(WIN), version: 3 };
    assert_eq!(frames(&mut h, closed).await, vec![f(&format!("client:{CLIENT}"), "update"), f(&format!("window:{WIN}"), "delete")]);
}

#[tokio::test]
async fn a_window_event_with_no_client_row_still_broadcasts_the_window() {
    let mut h = harness();
    h.mstore.delete::<Client>(CLIENT).unwrap();
    let closed = Event::SrvWindowClosed { window_id: s(WIN), version: 3 };
    assert_eq!(frames(&mut h, closed).await, vec![f(&format!("window:{WIN}"), "delete")]);
}

#[tokio::test]
async fn tab_created_broadcasts_the_tab_then_its_workspace() {
    let mut h = harness();
    let created = Event::TabCreated { workspace_id: s(WS), tab_id: s(TAB), name: s("t"), version: 2 };
    assert_eq!(frames(&mut h, created).await, vec![f(&format!("tab:{TAB}"), "update"), f(&format!("workspace:{WS}"), "update")]);
}

#[tokio::test]
async fn tab_rename_and_meta_broadcast_the_tab() {
    let mut h = harness();
    let renamed = Event::TabRenamed { tab_id: s(TAB), name: s("t"), version: 2 };
    assert_eq!(frames(&mut h, renamed).await, vec![f(&format!("tab:{TAB}"), "update")]);
    let meta = Event::TabMetaUpdated { tab_id: s(TAB), meta_patch: serde_json::json!({}), version: 2 };
    assert_eq!(frames(&mut h, meta).await, vec![f(&format!("tab:{TAB}"), "update")]);
}

#[tokio::test]
async fn tab_order_events_broadcast_the_workspace() {
    let mut h = harness();
    let events = [
        Event::ActiveTabChanged { workspace_id: s(WS), tab_id: Some(s(TAB)), version: 2 },
        Event::TabReordered { workspace_id: s(WS), tab_id: s(TAB), new_index: 0, version: 2 },
        Event::TabsReorderedBulk { workspace_id: s(WS), tab_ids: vec![s(TAB)], version: 2 },
    ];
    for event in events {
        assert_eq!(frames(&mut h, event).await, vec![f(&format!("workspace:{WS}"), "update")]);
    }
}

#[tokio::test]
async fn tab_moved_broadcasts_the_tab_then_both_workspaces() {
    let mut h = harness();
    let moved = Event::TabMoved {
        tab_id: s(TAB),
        src_workspace_id: s(WS),
        dst_workspace_id: s(WS2),
        dst_index: 0,
        new_src_active_tab_id: None,
        new_dst_active_tab_id: None,
        version: 2,
    };
    let expected = vec![f(&format!("tab:{TAB}"), "update"), f(&format!("workspace:{WS}"), "update"), f(&format!("workspace:{WS2}"), "update")];
    assert_eq!(frames(&mut h, moved).await, expected);
}

#[tokio::test]
async fn block_created_broadcasts_the_block_then_its_tab() {
    let mut h = harness();
    let created = Event::BlockCreated { tab_id: s(TAB), block_id: s(BLK), meta: serde_json::json!({}), version: 2 };
    assert_eq!(frames(&mut h, created).await, vec![f(&format!("block:{BLK}"), "update"), f(&format!("tab:{TAB}"), "update")]);
}

#[tokio::test]
async fn deleting_a_parked_block_broadcasts_only_the_block_delete() {
    let mut h = harness();
    let deleted = Event::BlockDeleted { tab_id: s(""), block_id: s(BLK), version: 2 };
    assert_eq!(frames(&mut h, deleted).await, vec![f(&format!("block:{BLK}"), "delete")]);
}

#[tokio::test]
async fn block_meta_broadcasts_the_block() {
    let mut h = harness();
    let meta = Event::BlockMetaUpdated { block_id: s(BLK), meta_patch: serde_json::json!({}), version: 2 };
    assert_eq!(frames(&mut h, meta).await, vec![f(&format!("block:{BLK}"), "update")]);
}

#[tokio::test]
async fn parking_updates_the_tab_then_the_block_and_attaching_the_block_then_the_tab() {
    let mut h = harness();
    let parked = Event::BlockParked { tab_id: s(TAB), block_id: s(BLK), version: 2 };
    assert_eq!(frames(&mut h, parked).await, vec![f(&format!("tab:{TAB}"), "update"), f(&format!("block:{BLK}"), "update")]);
    let attached = Event::BlockAttached { block_id: s(BLK), dst_tab_id: s(TAB2), dst_index: 0, version: 3 };
    assert_eq!(frames(&mut h, attached).await, vec![f(&format!("block:{BLK}"), "update"), f(&format!("tab:{TAB2}"), "update")]);
}

#[tokio::test]
async fn block_moved_broadcasts_the_block_then_both_tabs() {
    let mut h = harness();
    let moved = Event::BlockMoved { block_id: s(BLK), src_tab_id: s(TAB), dst_tab_id: s(TAB2), dst_index: 0, version: 2 };
    let expected = vec![f(&format!("block:{BLK}"), "update"), f(&format!("tab:{TAB}"), "update"), f(&format!("tab:{TAB2}"), "update")];
    assert_eq!(frames(&mut h, moved).await, expected);
}

#[tokio::test]
async fn layout_events_broadcast_the_tabs_layout_and_nothing_for_a_tab_without_one() {
    let mut h = harness();
    let layout = f(&format!("layout:{LAY}"), "update");
    let events = [
        Event::FocusedNodeChanged { tab_id: s(TAB), node_id: s("n"), version: 2 },
        Event::MagnifiedNodeChanged { tab_id: s(TAB), node_id: s("n"), version: 2 },
        Event::LayoutTreeReplaced { tab_id: s(TAB), new_tree: None, correlation_id: s(""), slices: None, version: 2 },
    ];
    for event in events {
        assert_eq!(frames(&mut h, event).await, vec![layout.clone()]);
    }
    let no_layout = Event::FocusedNodeChanged { tab_id: s(TAB2), node_id: s("n"), version: 2 };
    assert!(frames(&mut h, no_layout).await.is_empty());
}

#[tokio::test]
async fn a_deleted_layout_node_is_deliberately_not_bridged() {
    let mut h = harness();
    let deleted = Event::LayoutNodeDeleted { tab_id: s(TAB), node_id: s("n"), new_tree: None, tree_cleared: false, correlation_id: s(""), was_focused: false, was_magnified: false, version: 2 };
    assert!(frames(&mut h, deleted).await.is_empty());
}

#[tokio::test]
async fn events_that_are_not_object_changes_broadcast_nothing() {
    let mut h = harness();
    assert!(frames(&mut h, Event::Pong { nonce: 1, version: 1 }).await.is_empty());
}
