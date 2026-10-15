// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The pane-tab RPCs queue their action under the caller's id and answer with
//! the row's version; a whole layout row can't be pushed.
//! SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.3,
//! SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30.md §5.

use agentmux_common::ipc::{Command, Event};
use agentmux_common::{LayoutNode, LayoutNodeData};

use crate::backend::obj::{LayoutState, Tab};
use crate::backend::rpc_types::CommandPaneTabData;
use crate::backend::service::WebCallType;
use crate::server::tests::test_state;
use crate::server::AppState;

async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
    let events = super::dispatch_to_reducer(state, cmd).await;
    for ev in &events {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
    }
    events
}

/// A tab holding one pane with block `a`, and a second block `b` of that tab
/// placed in no pane yet. Returns `(tab_id, layout_oid, a, b)`.
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
        let ev = dispatch_apply(state, Command::CreateBlock { tab_id: tab_id.clone(), meta: serde_json::json!({ "view": "term" }) }).await;
        blocks.push(
            ev.iter()
                .find_map(|e| match e {
                    Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
                    _ => None,
                })
                .unwrap(),
        );
    }
    let (a, b) = (blocks[0].clone(), blocks[1].clone());
    dispatch_apply(
        state,
        Command::LayoutSetTree { tab_id: tab_id.clone(), new_tree: Some(leaf("n-a", &a, &[])), correlation_id: String::new(), slices: None },
    )
    .await;
    let layout_oid = state.mstore.must_get::<Tab>(&tab_id).unwrap().layoutstate;
    (tab_id, layout_oid, a, b)
}

fn leaf(id: &str, visible: &str, stack: &[&str]) -> LayoutNode {
    let stack: Vec<String> = stack.iter().map(|s| s.to_string()).collect();
    LayoutNode {
        id: id.into(),
        size: 1.0,
        data: Some(LayoutNodeData {
            block_id: visible.into(),
            active_block_id: if stack.is_empty() { String::new() } else { visible.into() },
            block_stack: stack,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn row(state: &AppState, layout_oid: &str) -> LayoutState {
    state.mstore.must_get::<LayoutState>(layout_oid).unwrap()
}

/// A whole-row `UpdateObject` of the layout row, as the frontend used to push.
async fn push(state: &AppState, layout_oid: &str, tree: LayoutNode) -> crate::backend::service::WebReturnType {
    let obj = serde_json::json!({
        "otype": "layout",
        "oid": layout_oid,
        "version": row(state, layout_oid).version,
        "rootnode": tree,
    });
    let call = WebCallType { service: "object".into(), method: "UpdateObject".into(), uicontext: None, args: vec![obj] };
    super::run_service_call(state, &call).await
}

/// The backend half of `pane.open { stack_onto_block_id }`: `b` becomes a tab
/// of `a`'s pane in the reducer, and a `stackpush` is queued for the frontend.
async fn stack_b_onto_a(state: &AppState, tab_id: &str, a: &str, b: &str) {
    dispatch_apply(
        state,
        Command::LayoutStackPush {
            tab_id: tab_id.into(),
            target_block_id: a.into(),
            block_id: b.into(),
            activate: true,
            correlation_id: String::new(),
        },
    )
    .await;
    super::layout_helpers::queue_target_stack_push(state, tab_id, b, a).await.unwrap();
}

fn members(tree: &Option<LayoutNode>) -> Vec<String> {
    crate::backend::layout::leaf_members(tree.as_ref().unwrap().data.as_ref().unwrap())
}

#[tokio::test]
async fn a_whole_layout_row_push_is_refused_and_changes_nothing() {
    let state = test_state();
    let (tab_id, layout_oid, a, b) = seed(&state).await;
    stack_b_onto_a(&state, &tab_id, &a, &b).await;
    let before = row(&state, &layout_oid);
    let res = push(&state, &layout_oid, leaf("n-a", &a, &[])).await;
    assert!(res.error.as_deref().is_some_and(|e| e.contains("layout intents")), "{:?}", res.error);
    let after = row(&state, &layout_oid);
    assert_eq!(after.version, before.version);
    assert_eq!(members(&after.rootnode), vec![a.clone(), b.clone()]);
    assert_eq!(members(&state.srv_state.lock().await.tabs[&tab_id].rootnode), vec![a, b]);
}

#[tokio::test]
async fn activate_tab_switches_the_stored_pane_and_queues_under_the_callers_id() {
    let state = test_state();
    let (tab_id, layout_oid, a, b) = seed(&state).await;
    stack_b_onto_a(&state, &tab_id, &a, &b).await;
    let res = crate::server::app_api::pane::activate_tab(
        &state,
        CommandPaneTabData { block_id: a.clone(), action_id: Some("fe-1".into()) },
    )
    .await
    .unwrap();
    let stored = row(&state, &layout_oid);
    assert_eq!(res.version, stored.version as u64, "the answer names the stored row's version");
    assert_eq!(stored.rootnode.as_ref().unwrap().data.as_ref().unwrap().block_id, a);
    let last = stored.pendingbackendactions.unwrap().pop().unwrap();
    assert_eq!((last.actiontype.as_str(), last.actionid.as_str(), last.blockid.as_str()), ("stackactivate", "fe-1", a.as_str()));
}

#[tokio::test]
async fn remove_tab_takes_one_tab_out_and_queues_a_delete_under_the_callers_id() {
    let state = test_state();
    let (tab_id, layout_oid, a, b) = seed(&state).await;
    stack_b_onto_a(&state, &tab_id, &a, &b).await;
    let res = crate::server::app_api::pane::remove_tab(
        &state,
        CommandPaneTabData { block_id: b.clone(), action_id: Some("fe-2".into()) },
    )
    .await
    .unwrap();
    let stored = row(&state, &layout_oid);
    assert_eq!(res.version, stored.version as u64);
    assert_eq!(members(&stored.rootnode), vec![a.clone()]);
    let last = stored.pendingbackendactions.unwrap().pop().unwrap();
    assert_eq!((last.actiontype.as_str(), last.actionid.as_str()), ("delete", "fe-2"));

    // Already out of every pane: nothing changes and nothing is queued.
    let again = crate::server::app_api::pane::remove_tab(&state, CommandPaneTabData { block_id: b, action_id: None })
        .await
        .unwrap();
    assert_eq!(again.version, row(&state, &layout_oid).version as u64);
    assert_eq!(row(&state, &layout_oid).pendingbackendactions.unwrap().len(), 2, "nothing more queued");
}
