// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A frontend layout push built before a queued backend action is refused, and
//! the pane-tab RPCs queue their action under the caller's id.
//! SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.3, §3.5.

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

/// The frontend's `persistToBackend` push, built on `revision` (`None`: an
/// older client that sends none).
async fn push(state: &AppState, layout_oid: &str, tree: LayoutNode, revision: Option<u64>) -> crate::backend::service::WebReturnType {
    let mut obj = serde_json::json!({
        "otype": "layout",
        "oid": layout_oid,
        "version": row(state, layout_oid).version,
        "rootnode": tree,
    });
    if let Some(r) = revision {
        obj["layoutrevision"] = r.into();
    }
    let call = WebCallType { service: "object".into(), method: "UpdateObject".into(), uicontext: None, args: vec![obj] };
    super::run_service_call(state, &call).await
}

/// The backend half of `pane.open { stack_onto_block_id }`: `b` becomes a tab
/// of `a`'s pane in the reducer, and a `stackpush` is queued for the frontend.
async fn stack_b_onto_a(state: &AppState, tab_id: &str, a: &str, b: &str) -> u64 {
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
    state.srv_state.lock().await.tabs[tab_id].layout_revision
}

fn members(tree: &Option<LayoutNode>) -> Vec<String> {
    crate::backend::layout::leaf_members(tree.as_ref().unwrap().data.as_ref().unwrap())
}

#[tokio::test]
async fn queued_actions_carry_the_revision_to_the_row_and_back_through_bootstrap() {
    let state = test_state();
    let (tab_id, layout_oid, a, b) = seed(&state).await;
    assert_eq!(stack_b_onto_a(&state, &tab_id, &a, &b).await, 1);
    let rev = super::reducer_helpers::queue_layout_actions_at_revision(
        &state,
        &tab_id,
        vec![super::layout_helpers::source_delete_action("x")],
    )
    .await
    .unwrap();
    assert_eq!(rev, 2);
    assert_eq!(row(&state, &layout_oid).layoutrevision, 2);
    assert_eq!(row(&state, &layout_oid).pendingbackendactions.unwrap().len(), 2);

    let fresh = std::sync::Arc::new(tokio::sync::Mutex::new(crate::state::State::default()));
    crate::persist::bootstrap_state_from_mstore(&fresh, &state.mstore).await;
    assert_eq!(fresh.lock().await.tabs[&tab_id].layout_revision, 2, "a restart keeps refusing pushes built before it");
}

#[tokio::test]
async fn a_push_built_before_a_backend_stack_push_is_refused_and_the_tab_survives() {
    let state = test_state();
    let (tab_id, layout_oid, a, b) = seed(&state).await;
    let rev = stack_b_onto_a(&state, &tab_id, &a, &b).await;

    // Built on revision 0: the frontend never saw `b` join the pane.
    let mut resized = leaf("n-a", &a, &[]);
    resized.size = 2.0;
    let res = push(&state, &layout_oid, resized, Some(0)).await;
    assert!(res.success && res.error.is_none(), "{:?}", res.error);
    assert_eq!(res.data, Some(serde_json::json!({ "stale": true, "layoutrevision": rev })));
    let echoed = &res.updates.as_ref().expect("the row as stored")[0];
    assert_eq!(echoed.obj.as_ref().unwrap()["layoutrevision"], rev);

    let stored = row(&state, &layout_oid);
    assert_eq!(members(&stored.rootnode), vec![a.clone(), b.clone()], "the stale push must not drop the tab");
    assert_eq!(stored.pendingbackendactions.as_ref().map(Vec::len), Some(1), "nor erase its queued stackpush");
    assert_eq!(members(&state.srv_state.lock().await.tabs[&tab_id].rootnode), vec![a.clone(), b.clone()]);

    // Resynced: the frontend applied the stackpush and pushes on `rev`.
    let res = push(&state, &layout_oid, leaf("n-a", &b, &[&a, &b]), Some(rev)).await;
    assert!(res.data.is_none(), "{:?}", res.data);
    let stored = row(&state, &layout_oid);
    assert_eq!(members(&stored.rootnode), vec![a, b]);
    assert!(stored.pendingbackendactions.is_none(), "the accepted push acks the queue");
}

#[tokio::test]
async fn a_push_without_a_revision_is_applied_as_before() {
    let state = test_state();
    let (tab_id, layout_oid, a, b) = seed(&state).await;
    stack_b_onto_a(&state, &tab_id, &a, &b).await;
    let res = push(&state, &layout_oid, leaf("n-a", &a, &[]), None).await;
    assert!(res.data.is_none());
    assert_eq!(members(&row(&state, &layout_oid).rootnode), vec![a]);
}

#[tokio::test]
async fn activate_tab_switches_the_stored_pane_and_queues_under_the_callers_id() {
    let state = test_state();
    let (tab_id, layout_oid, a, b) = seed(&state).await;
    let before = stack_b_onto_a(&state, &tab_id, &a, &b).await;
    let res = crate::server::app_api::pane::activate_tab(
        &state,
        CommandPaneTabData { block_id: a.clone(), action_id: Some("fe-1".into()) },
    )
    .await
    .unwrap();
    assert_eq!(res.layout_revision, before + 1);
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
    let before = stack_b_onto_a(&state, &tab_id, &a, &b).await;
    let res = crate::server::app_api::pane::remove_tab(
        &state,
        CommandPaneTabData { block_id: b.clone(), action_id: Some("fe-2".into()) },
    )
    .await
    .unwrap();
    assert_eq!(res.layout_revision, before + 1);
    let stored = row(&state, &layout_oid);
    assert_eq!(members(&stored.rootnode), vec![a.clone()]);
    let last = stored.pendingbackendactions.unwrap().pop().unwrap();
    assert_eq!((last.actiontype.as_str(), last.actionid.as_str()), ("delete", "fe-2"));

    // Already out of every pane: nothing changes and nothing is queued.
    let again = crate::server::app_api::pane::remove_tab(&state, CommandPaneTabData { block_id: b, action_id: None })
        .await
        .unwrap();
    assert_eq!(again.layout_revision, 0);
    assert_eq!(again.version, row(&state, &layout_oid).version as u64);
    assert_eq!(state.srv_state.lock().await.tabs[&tab_id].layout_revision, before + 1);
}
