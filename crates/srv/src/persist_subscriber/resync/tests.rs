// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use agentmux_common::ipc::{Command, Event};
use tokio::sync::{broadcast, Mutex};

use super::super::{apply_event_to_mstore, run_persist_subscriber};
use super::resync_from_reducer;
use crate::backend::obj::{Block, Client, LayoutState as PersistedLayoutState, Tab, Window, Workspace};
use crate::backend::storage::store::Store;
use crate::reducer::{update, Ctx};
use crate::state::State;

fn store() -> Arc<Store> {
    let store = Store::open_in_memory().expect("in-memory mstore");
    let mut client = Client { oid: "client".into(), ..Default::default() };
    store.insert(&mut client).unwrap();
    Arc::new(store)
}

fn leaf(id: &str, block: &str) -> agentmux_common::LayoutNode {
    agentmux_common::LayoutNode {
        id: id.into(),
        data: Some(agentmux_common::LayoutNodeData { block_id: block.into(), ..Default::default() }),
        ..Default::default()
    }
}

/// Dispatch with a fixed id; returns the events.
fn dispatch(state: &mut State, cmd: Command, id: &str) -> Vec<Event> {
    let events = update(state, cmd, &Ctx { registered_pid: None, new_id: id.into() });
    assert!(!events.iter().any(|e| matches!(e, Event::Error { .. })), "{events:?}");
    events
}

/// ws-a (tabs t1, t2; t2 active, holding blk-1 and a tree), ws-b (t3), and
/// window win-1 on ws-b, built through the reducer. Returns every event.
fn build(state: &mut State) -> Vec<Event> {
    let mut events = Vec::new();
    events.extend(dispatch(state, Command::CreateWorkspace { name: "A".into() }, "ws-a"));
    events.extend(dispatch(state, Command::CreateWorkspace { name: "B".into() }, "ws-b"));
    for (ws, tab) in [("ws-a", "t1"), ("ws-a", "t2"), ("ws-b", "t3")] {
        events.extend(dispatch(state, Command::CreateTab { workspace_id: ws.into(), name: tab.into() }, tab));
    }
    events.extend(dispatch(state, Command::SetActiveTab { workspace_id: "ws-a".into(), tab_id: "t2".into() }, "-"));
    events.extend(dispatch(
        state,
        Command::CreateBlock { tab_id: "t2".into(), meta: serde_json::json!({ "view": "term" }) },
        "blk-1",
    ));
    events.extend(dispatch(
        state,
        Command::LayoutSetTree {
            tab_id: "t2".into(),
            new_tree: Some(leaf("n1", "blk-1")),
            correlation_id: String::new(),
            slices: None,
        },
        "-",
    ));
    events.extend(dispatch(state, Command::SetFocusedNode { tab_id: "t2".into(), node_id: "n1".into() }, "-"));
    events.extend(dispatch(state, Command::CreateWindow { window_id: "win-1".into(), workspace_id: "ws-b".into() }, "-"));
    events
}

fn ws(s: &Store, id: &str) -> Workspace {
    s.get::<Workspace>(id).unwrap().expect("workspace row")
}

fn tab(s: &Store, id: &str) -> Tab {
    s.get::<Tab>(id).unwrap().expect("tab row")
}

fn layout(s: &Store, tab_id: &str) -> PersistedLayoutState {
    s.get::<PersistedLayoutState>(&tab(s, tab_id).layoutstate).unwrap().expect("layout row")
}

/// The bus drops every domain event; the subscriber's resync alone must
/// write the reducer's workspaces, tabs, blocks, windows and layout.
#[tokio::test]
async fn lagged_bus_resync_rebuilds_every_row_kind() {
    let mstore = store();
    let mut s = State::default();
    let events = build(&mut s);
    // The block row is written by the RPC handler's own synchronous apply,
    // which a bus lag doesn't affect.
    for e in events.iter().filter(|e| matches!(e, Event::BlockCreated { .. })) {
        apply_event_to_mstore(e, &mstore).unwrap();
    }
    let state = Arc::new(Mutex::new(s));

    // Capacity 1: only the last send survives, and it's not a domain event.
    let (tx, rx) = broadcast::channel(1);
    for e in &events {
        tx.send(e.clone()).unwrap();
    }
    tx.send(Event::Pong { nonce: 0, version: 0 }).unwrap();
    drop(tx);
    run_persist_subscriber(rx, mstore.clone(), state).await;

    assert_eq!(ws(&mstore, "ws-a").name, "A");
    assert_eq!(ws(&mstore, "ws-a").tabids, vec!["t1", "t2"]);
    assert_eq!(ws(&mstore, "ws-a").activetabid, "t2");
    assert_eq!(ws(&mstore, "ws-b").tabids, vec!["t3"]);
    assert_eq!(tab(&mstore, "t2").name, "t2");
    assert_eq!(tab(&mstore, "t2").blockids, vec!["blk-1"]);
    assert_eq!(layout(&mstore, "t2").rootnode, Some(leaf("n1", "blk-1")));
    assert_eq!(layout(&mstore, "t2").focusednodeid, "n1");
    assert_eq!(mstore.get::<Window>("win-1").unwrap().expect("window row").workspaceid, "ws-b");
    let client = mstore.get::<Client>("client").unwrap().unwrap();
    assert_eq!(client.windowids, vec!["win-1"]);
}

/// Disk in step with the reducer, then a burst of changes whose events are
/// all lost: the resync brings every changed row back in step.
#[tokio::test]
async fn resync_repairs_stale_rows_without_deleting_any() {
    let mstore = store();
    let mut s = State::default();
    for e in build(&mut s) {
        apply_event_to_mstore(&e, &mstore).unwrap();
    }
    dispatch(&mut s, Command::RenameWorkspace { workspace_id: "ws-a".into(), name: "A2".into() }, "-");
    dispatch(&mut s, Command::RenameTab { tab_id: "t1".into(), name: "renamed".into() }, "-");
    dispatch(&mut s, Command::ReorderTab { workspace_id: "ws-a".into(), tab_id: "t2".into(), new_index: 0 }, "-");
    dispatch(&mut s, Command::CreateTab { workspace_id: "ws-b".into(), name: "t4".into() }, "t4");
    dispatch(
        &mut s,
        Command::MoveTab {
            tab_id: "t1".into(),
            src_workspace_id: "ws-a".into(),
            dst_workspace_id: "ws-b".into(),
            dst_index: 0,
            allow_empty_source: false,
        },
        "-",
    );
    dispatch(&mut s, Command::DeleteTab { workspace_id: "ws-b".into(), tab_id: "t3".into(), force: false }, "-");
    dispatch(&mut s, Command::SwitchWorkspace { window_id: "win-1".into(), workspace_id: "ws-a".into() }, "-");
    dispatch(
        &mut s,
        Command::LayoutSetTree { tab_id: "t2".into(), new_tree: None, correlation_id: String::new(), slices: None },
        "-",
    );
    // A block the store never got: left out, not invented.
    dispatch(&mut s, Command::CreateBlock { tab_id: "t4".into(), meta: serde_json::Value::Null }, "blk-lost");
    let state = Arc::new(Mutex::new(s));

    resync_from_reducer(&state, &mstore).await.unwrap();

    assert_eq!(ws(&mstore, "ws-a").name, "A2");
    assert_eq!(ws(&mstore, "ws-a").tabids, vec!["t2"]);
    assert_eq!(ws(&mstore, "ws-a").activetabid, "t2");
    assert_eq!(ws(&mstore, "ws-b").tabids, vec!["t1", "t4"]);
    assert_eq!(ws(&mstore, "ws-b").activetabid, "t1");
    assert_eq!(tab(&mstore, "t1").name, "renamed");
    assert!(tab(&mstore, "t4").blockids.is_empty());
    assert!(mstore.get::<Block>("blk-lost").unwrap().is_none());
    let t2_layout = layout(&mstore, "t2");
    assert_eq!(t2_layout.rootnode, None);
    assert_eq!(t2_layout.focusednodeid, "");
    assert_eq!(mstore.get::<Window>("win-1").unwrap().unwrap().workspaceid, "ws-a");
    // The deleted tab is unlinked, not deleted.
    assert!(mstore.get::<Tab>("t3").unwrap().is_some());
}

#[tokio::test]
async fn resync_of_an_in_step_store_writes_nothing() {
    let mstore = store();
    let mut s = State::default();
    for e in build(&mut s) {
        apply_event_to_mstore(&e, &mstore).unwrap();
    }
    let versions = |m: &Store| {
        (ws(m, "ws-a").version, tab(m, "t2").version, layout(m, "t2").version, m.get::<Block>("blk-1").unwrap().unwrap().version)
    };
    let before = versions(&mstore);
    resync_from_reducer(&Arc::new(Mutex::new(s)), &mstore).await.unwrap();
    assert_eq!(versions(&mstore), before);
}

/// A block whose `BlockMoved` was dropped gets its new parent.
#[tokio::test]
async fn resync_moves_a_block_row_to_its_reducer_parent() {
    let mstore = store();
    let mut s = State::default();
    for e in build(&mut s) {
        apply_event_to_mstore(&e, &mstore).unwrap();
    }
    dispatch(
        &mut s,
        Command::MoveBlock { block_id: "blk-1".into(), src_tab_id: "t2".into(), dst_tab_id: "t1".into(), dst_index: 0 },
        "-",
    );
    resync_from_reducer(&Arc::new(Mutex::new(s)), &mstore).await.unwrap();
    assert_eq!(mstore.get::<Block>("blk-1").unwrap().unwrap().parentoref, "tab:t1");
    assert_eq!(tab(&mstore, "t1").blockids, vec!["blk-1"]);
    assert!(tab(&mstore, "t2").blockids.is_empty());
}

/// A dropped queue append leaves the row's layout revision behind the
/// reducer's; the resync raises it, and never lowers one that is ahead.
#[tokio::test]
async fn resync_writes_the_layout_revision_and_never_lowers_it() {
    let mstore = store();
    let mut s = State::default();
    for e in build(&mut s) {
        apply_event_to_mstore(&e, &mstore).unwrap();
    }
    let queue = Command::LayoutQueueBackendActions {
        tab_id: "t2".into(),
        actions: serde_json::json!([{ "actiontype": "delete", "actionid": "a1", "blockid": "blk-1" }]),
        correlation_id: String::new(),
    };
    dispatch(&mut s, queue, "-"); // its event is lost
    assert_eq!(layout(&mstore, "t2").layoutrevision, 0);
    let state = Arc::new(Mutex::new(s));
    resync_from_reducer(&state, &mstore).await.unwrap();
    assert_eq!(layout(&mstore, "t2").layoutrevision, 1);

    let mut row = layout(&mstore, "t2");
    row.layoutrevision = 5;
    mstore.update(&mut row).unwrap();
    resync_from_reducer(&state, &mstore).await.unwrap();
    assert_eq!(layout(&mstore, "t2").layoutrevision, 5);
}
