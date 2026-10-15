// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::reducer::test_support::*;
use crate::reducer::update;
use agentmux_common::ipc::Command;

// ---------------------------------------------------------------
// Phase E.4 (Option A) — SetFocusedNode / SetMagnifiedNode
// ---------------------------------------------------------------

#[test]
fn set_focused_node_round_trip_emits_event_and_updates_state() {
    let mut state = State::default();
    let ws_id = create_workspace(&mut state, "w");
    let tab_id = create_tab(&mut state, &ws_id, "t1");
    assert_eq!(state.tabs[&tab_id].focused_node_id, "");
    let events = update(
        &mut state,
        Command::SetFocusedNode {
            tab_id: tab_id.clone(),
            node_id: "node-7".into(),
        },
        &ctx(2),
    );
    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::FocusedNodeChanged { tab_id: t, node_id, .. } => {
            assert_eq!(t, &tab_id);
            assert_eq!(node_id, "node-7");
        }
        other => panic!("expected FocusedNodeChanged, got {:?}", other),
    }
    assert_eq!(state.tabs[&tab_id].focused_node_id, "node-7");
}

#[test]
fn set_focused_node_no_op_when_value_unchanged() {
    let mut state = State::default();
    let ws_id = create_workspace(&mut state, "w");
    let tab_id = create_tab(&mut state, &ws_id, "t1");
    let _ = update(
        &mut state,
        Command::SetFocusedNode {
            tab_id: tab_id.clone(),
            node_id: "node-1".into(),
        },
        &ctx(2),
    );
    let version_before = state.event_version;
    let events = update(
        &mut state,
        Command::SetFocusedNode {
            tab_id,
            node_id: "node-1".into(),
        },
        &ctx(3),
    );
    assert!(events.is_empty(), "no-op should emit no events");
    assert_eq!(state.event_version, version_before, "no version bump on no-op");
}

#[test]
fn set_focused_node_unknown_tab_emits_error() {
    let mut state = State::default();
    let events = update(
        &mut state,
        Command::SetFocusedNode {
            tab_id: "ghost-tab".into(),
            node_id: "node-1".into(),
        },
        &ctx(1),
    );
    assert_eq!(events.len(), 1);
    assert!(
        matches!(&events[0], Event::Error { .. }),
        "expected Event::Error, got {:?}",
        events[0]
    );
}

#[test]
fn set_magnified_node_round_trip_emits_event_and_updates_state() {
    let mut state = State::default();
    let ws_id = create_workspace(&mut state, "w");
    let tab_id = create_tab(&mut state, &ws_id, "t1");
    let events = update(
        &mut state,
        Command::SetMagnifiedNode {
            tab_id: tab_id.clone(),
            node_id: "node-9".into(),
        },
        &ctx(2),
    );
    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::MagnifiedNodeChanged { tab_id: t, node_id, .. } => {
            assert_eq!(t, &tab_id);
            assert_eq!(node_id, "node-9");
        }
        other => panic!("expected MagnifiedNodeChanged, got {:?}", other),
    }
    assert_eq!(state.tabs[&tab_id].magnified_node_id, "node-9");
}

#[test]
fn set_magnified_node_no_op_when_value_unchanged() {
    let mut state = State::default();
    let ws_id = create_workspace(&mut state, "w");
    let tab_id = create_tab(&mut state, &ws_id, "t1");
    let _ = update(
        &mut state,
        Command::SetMagnifiedNode {
            tab_id: tab_id.clone(),
            node_id: "node-2".into(),
        },
        &ctx(2),
    );
    let version_before = state.event_version;
    let events = update(
        &mut state,
        Command::SetMagnifiedNode {
            tab_id,
            node_id: "node-2".into(),
        },
        &ctx(3),
    );
    assert!(events.is_empty());
    assert_eq!(state.event_version, version_before);
}

#[test]
fn set_magnified_node_clear_with_empty_node_id() {
    let mut state = State::default();
    let ws_id = create_workspace(&mut state, "w");
    let tab_id = create_tab(&mut state, &ws_id, "t1");
    // Magnify a node first.
    let _ = update(
        &mut state,
        Command::SetMagnifiedNode {
            tab_id: tab_id.clone(),
            node_id: "node-3".into(),
        },
        &ctx(2),
    );
    assert_eq!(state.tabs[&tab_id].magnified_node_id, "node-3");
    // Now clear with empty node_id (toggle-off semantics).
    let events = update(
        &mut state,
        Command::SetMagnifiedNode {
            tab_id: tab_id.clone(),
            node_id: String::new(),
        },
        &ctx(3),
    );
    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::MagnifiedNodeChanged { node_id, .. } => assert_eq!(node_id, ""),
        other => panic!("expected MagnifiedNodeChanged with empty node_id, got {:?}", other),
    }
    assert_eq!(state.tabs[&tab_id].magnified_node_id, "");
}

#[test]
fn set_magnified_node_unknown_tab_emits_error() {
    let mut state = State::default();
    let events = update(
        &mut state,
        Command::SetMagnifiedNode {
            tab_id: "ghost-tab".into(),
            node_id: "node-1".into(),
        },
        &ctx(1),
    );
    assert_eq!(events.len(), 1);
    assert!(matches!(&events[0], Event::Error { .. }));
}

// ---- Phase E.7 — property tests for reducer arm invariants ----
//
// Drives randomized sequences of valid commands through `update`
// and asserts cross-arm invariants the unit tests above only
// touch on per-arm. Catches regressions where an individual arm
// looks correct in isolation but interacts with sibling arms in
// a way that violates the reducer's whole-state contract.
//
// Invariants asserted:
//   1. Version monotonicity: every event's version strictly
//      increases across the sequence (no duplicates, no gaps in
//      the wrong direction).
//   2. Referential integrity: every tab in `state.tabs` has a
//      `workspace_id` that exists in `state.workspaces`; every
//      block in `state.blocks` has a `tab_id` that exists in
//      `state.tabs`; every workspace's `tab_ids` references real
//      tabs; every tab's `block_ids` references real blocks.
//   3. Cascade integrity: after a `DeleteWorkspace`, no tab
//      remains in `state.tabs` with that workspace_id, and no
//      block remains in `state.blocks` whose tab was in that
//      workspace.
//   4. Active-tab validity: every workspace's `active_tab_id`
//      is either `None` or points at a tab present in its own
//      `tab_ids`.

use proptest::prelude::*;

/// Higher-level operations the property tests pick from. Each
/// resolves to one or more `Command` invocations against the
/// current state. We can't generate `Command`s directly because
/// IDs are reducer-generated; pick from existing IDs instead.
#[derive(Debug, Clone)]
enum PropOp {
    CreateWorkspace,
    CreateTab,
    CreateBlock,
    DeleteTab,
    DeleteBlock,
    DeleteWorkspace,
    QueueBackendAction,
}

fn op_strategy() -> impl Strategy<Value = PropOp> {
    // Bias toward "constructive" ops so sequences accumulate
    // state rather than churn empty. Each Just is one variant;
    // proptest weights via `prop_oneof![weight => strat, …]`.
    prop_oneof![
        4 => Just(PropOp::CreateWorkspace),
        4 => Just(PropOp::CreateTab),
        3 => Just(PropOp::CreateBlock),
        1 => Just(PropOp::DeleteTab),
        1 => Just(PropOp::DeleteBlock),
        1 => Just(PropOp::DeleteWorkspace),
        2 => Just(PropOp::QueueBackendAction),
    ]
}

/// Apply one PropOp; returns the events produced (which may be
/// empty if the op was a no-op like "delete from empty pool").
fn apply_prop_op(state: &mut State, op: PropOp, conn_id: u64) -> Vec<Event> {
    match op {
        PropOp::CreateWorkspace => update(
            state,
            Command::CreateWorkspace { name: format!("ws-{}", conn_id) },
            &ctx(conn_id),
        ),
        PropOp::CreateTab => {
            let target_ws = state.workspaces.keys().next().cloned();
            match target_ws {
                Some(workspace_id) => update(
                    state,
                    Command::CreateTab {
                        workspace_id,
                        name: format!("tab-{}", conn_id),
                    },
                    &ctx(conn_id),
                ),
                None => Vec::new(),
            }
        }
        PropOp::CreateBlock => {
            let target_tab = state.tabs.keys().next().cloned();
            match target_tab {
                Some(tab_id) => update(
                    state,
                    Command::CreateBlock { tab_id, meta: serde_json::Value::Null },
                    &ctx(conn_id),
                ),
                None => Vec::new(),
            }
        }
        PropOp::DeleteTab => {
            if let Some((tab_id, tab)) = state.tabs.iter().next() {
                let cmd = Command::DeleteTab {
                    workspace_id: tab.workspace_id.clone(),
                    tab_id: tab_id.clone(),
                    // Proptest exercises both guarded + unguarded
                    // paths; force=true here ensures cascade
                    // invariants are tested without the guard
                    // short-circuiting the operation.
                    force: true,
                };
                update(state, cmd, &ctx(conn_id))
            } else {
                Vec::new()
            }
        }
        PropOp::DeleteBlock => {
            if let Some((block_id, block)) = state.blocks.iter().next() {
                let cmd = Command::DeleteBlock {
                    tab_id: block.tab_id.clone(),
                    block_id: block_id.clone(),
                };
                update(state, cmd, &ctx(conn_id))
            } else {
                Vec::new()
            }
        }
        PropOp::DeleteWorkspace => {
            if let Some(workspace_id) = state.workspaces.keys().next().cloned() {
                update(
                    state,
                    Command::DeleteWorkspace { workspace_id, force: false },
                    &ctx(conn_id),
                )
            } else {
                Vec::new()
            }
        }
        PropOp::QueueBackendAction => match state.tabs.keys().next().cloned() {
            Some(tab_id) => update(
                state,
                Command::LayoutQueueBackendActions {
                    tab_id,
                    actions: serde_json::json!([{ "actiontype": "delete", "actionid": format!("a{conn_id}"), "blockid": "b" }]),
                    correlation_id: String::new(),
                },
                &ctx(conn_id),
            ),
            None => Vec::new(),
        },
    }
}

/// Verify all four reducer-state invariants at once. Panics
/// (proptest catches and shrinks) if any is violated.
fn assert_invariants(state: &State) {
    // (2) Tabs reference real workspaces.
    for (tab_id, tab) in &state.tabs {
        assert!(
            state.workspaces.contains_key(&tab.workspace_id),
            "tab {} references unknown workspace {}",
            tab_id,
            tab.workspace_id
        );
    }
    // (2) Blocks reference real tabs.
    for (block_id, block) in &state.blocks {
        assert!(
            state.tabs.contains_key(&block.tab_id),
            "block {} references unknown tab {}",
            block_id,
            block.tab_id
        );
    }
    // (2) Workspace.tab_ids references real tabs.
    for (workspace_id, ws) in &state.workspaces {
        for tab_id in &ws.tab_ids {
            assert!(
                state.tabs.contains_key(tab_id),
                "workspace {} tab_ids contains unknown tab {}",
                workspace_id,
                tab_id
            );
        }
    }
    // (2) Tab.block_ids references real blocks.
    for (tab_id, tab) in &state.tabs {
        for block_id in &tab.block_ids {
            assert!(
                state.blocks.contains_key(block_id),
                "tab {} block_ids contains unknown block {}",
                tab_id,
                block_id
            );
        }
    }
    // (4) Active-tab validity.
    for (workspace_id, ws) in &state.workspaces {
        if let Some(active) = &ws.active_tab_id {
            assert!(
                ws.tab_ids.iter().any(|t| t == active),
                "workspace {} active_tab_id {} not in its tab_ids",
                workspace_id,
                active
            );
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]

    /// Apply a random sequence of valid ops. After each op,
    /// referential integrity + active-tab validity hold; across
    /// the whole sequence, version is strictly monotonic for any
    /// emitted events.
    #[test]
    fn invariants_hold_across_random_sequences(ops in prop::collection::vec(op_strategy(), 0..40)) {
        let mut state = State::default();
        let mut last_version: u64 = 0;
        for (i, op) in ops.into_iter().enumerate() {
            let events = apply_prop_op(&mut state, op, (i + 1) as u64);
            for ev in &events {
                let v = extract_version(ev);
                prop_assert!(
                    v > last_version,
                    "version {} not strictly greater than previous {} (event {:?})",
                    v,
                    last_version,
                    ev
                );
                last_version = v;
            }
            assert_invariants(&state);
        }
    }

    /// Cascade integrity — explicit setup-then-delete pattern.
    /// Build a non-trivial graph (workspace + tabs + blocks),
    /// delete the workspace, assert NO surviving entities
    /// reference the deleted workspace.
    #[test]
    fn delete_workspace_cascades_cleanly(
        tab_count in 1usize..6,
        blocks_per_tab in 0usize..4,
    ) {
        let mut state = State::default();
        // Create workspace.
        let ws_events = update(
            &mut state,
            Command::CreateWorkspace { name: "ws".into() },
            &ctx(1),
        );
        let ws_id = ws_events
            .iter()
            .find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            })
            .unwrap();
        // Create tabs + blocks under it. We don't keep the tab
        // IDs around — the cascade-after-delete assertions below
        // check the WHOLE-state collections (state.tabs.is_empty()
        // etc.), so per-tab IDs aren't needed. (reagent P2 #627.)
        for _ in 0..tab_count {
            let evs = update(
                &mut state,
                Command::CreateTab {
                    workspace_id: ws_id.clone(),
                    name: "t".into(),
                },
                &ctx(2),
            );
            let tid = evs
                .iter()
                .find_map(|e| match e {
                    Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                    _ => None,
                })
                .unwrap();
            for _ in 0..blocks_per_tab {
                let _ = update(
                    &mut state,
                    Command::CreateBlock {
                        tab_id: tid.clone(),
                        meta: serde_json::Value::Null,
                    },
                    &ctx(3),
                );
            }
        }
        // Sanity: counts match.
        prop_assert_eq!(state.workspaces.len(), 1);
        prop_assert_eq!(state.tabs.len(), tab_count);
        prop_assert_eq!(state.blocks.len(), tab_count * blocks_per_tab);
        // Delete the workspace.
        let _ = update(
            &mut state,
            Command::DeleteWorkspace { workspace_id: ws_id.clone(), force: false },
            &ctx(4),
        );
        // Cascade — workspaces, tabs, blocks should all be empty.
        prop_assert!(state.workspaces.is_empty());
        prop_assert!(state.tabs.is_empty());
        prop_assert!(
            state.blocks.is_empty(),
            "blocks should cascade-delete with their tabs; got {} survivors",
            state.blocks.len()
        );
        // And invariants hold on the empty state.
        assert_invariants(&state);
    }
}

// ── Layout tree arms ──────────────────────────────────────────
//
// State mutation and event shape per arm. The pure helpers have
// their own tests in `crates/srv/src/backend/layout/`.

fn leaf_node(id: &str, block_id: &str) -> agentmux_common::LayoutNode {
    agentmux_common::LayoutNode {
        id: id.to_string(),
        size: 1.0,
        data: Some(agentmux_common::LayoutNodeData {
            block_id: block_id.to_string(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn fresh_tab() -> (State, String) {
    let mut state = State::default();
    let ws = create_workspace(&mut state, "w");
    let tab = create_tab(&mut state, &ws, "t");
    (state, tab)
}

/// Registers a minimal `BlockRecord` for `block_id` in `state.blocks`,
/// so a test-constructed leaf referencing it survives
/// `prune_dangling_block_refs` (added alongside `LayoutSetTree` and the
/// `apply_atomic`-routed arms — see
/// `docs/investigations/INVESTIGATION_LAYOUT_DEAD_SPACE_STALE_TREE_RESURRECTION_2026_07_08.md`).
/// Real callers always have a live `BlockRecord` before a layout leaf
/// can reference it (`CreateBlock` populates `state.blocks` before any
/// layout insert for that block can be dispatched — verified via
/// `server::service::object`'s create-block handler); these
/// lower-level reducer tests construct trees directly and need this
/// explicit seed to match that real precondition.
fn seed_block(state: &mut State, tab_id: &str, block_id: &str) {
    state.blocks.insert(
        block_id.to_string(),
        crate::state::BlockRecord {
            block_id: block_id.to_string(),
            tab_id: tab_id.to_string(),
        },
    );
}

#[test]
fn layout_queue_backend_actions_passes_through_and_emits_event() {
    let (mut state, tab_id) = fresh_tab();
    let actions = serde_json::json!([{
        "actiontype": "insert",
        "actionid": "a1",
        "blockid": "b1",
        "nodesize": null,
        "nodesizefraction": null,
        "indexarr": null,
        "focused": true,
        "magnified": false,
        "ephemeral": false,
        "targetblockid": "",
        "position": "",
    }]);
    let events = update(
        &mut state,
        Command::LayoutQueueBackendActions {
            tab_id: tab_id.clone(),
            actions: actions.clone(),
            correlation_id: "corr-q1".into(),
        },
        &ctx(1),
    );
    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::LayoutBackendActionsQueued {
            tab_id: ev_tab_id,
            actions: ev_actions,
            correlation_id,
            ..
        } => {
            assert_eq!(ev_tab_id, &tab_id);
            assert_eq!(ev_actions, &actions);
            assert_eq!(correlation_id, "corr-q1");
        }
        other => panic!("expected LayoutBackendActionsQueued, got {other:?}"),
    }
}

#[test]
fn layout_queue_backend_actions_unknown_tab_emits_error() {
    let mut state = State::default();
    let events = update(
        &mut state,
        Command::LayoutQueueBackendActions {
            tab_id: "nope".into(),
            actions: serde_json::json!([{}]),
            correlation_id: "corr".into(),
        },
        &ctx(1),
    );
    assert_eq!(events.len(), 1);
    assert!(matches!(&events[0], Event::Error { .. }));
}

#[test]
fn layout_queue_backend_actions_empty_array_emits_error() {
    let (mut state, tab_id) = fresh_tab();
    let events = update(
        &mut state,
        Command::LayoutQueueBackendActions {
            tab_id,
            actions: serde_json::json!([]),
            correlation_id: "corr".into(),
        },
        &ctx(1),
    );
    assert_eq!(events.len(), 1);
    assert!(matches!(&events[0], Event::Error { .. }));
}

#[test]
fn layout_set_tree_replaces_rootnode_wholesale() {
    let (mut state, tab_id) = fresh_tab();
    seed_block(&mut state, &tab_id, "b1");
    seed_block(&mut state, &tab_id, "b2");
    state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(leaf_node("old", "b1"));

    let new_tree = Some(leaf_node("new", "b2"));
    let events = update(
        &mut state,
        Command::LayoutSetTree {
            tab_id: tab_id.clone(),
            new_tree: new_tree.clone(),
            correlation_id: "corr-set".into(),
            slices: None,
        },
        &ctx(1),
    );

    assert!(matches!(&events[0], Event::LayoutTreeReplaced { .. }));
    assert_eq!(state.tabs[&tab_id].rootnode, new_tree);
}

#[test]
fn layout_set_tree_to_none_clears_rootnode() {
    let (mut state, tab_id) = fresh_tab();
    state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(leaf_node("n", "b"));

    let _ = update(
        &mut state,
        Command::LayoutSetTree {
            tab_id: tab_id.clone(),
            new_tree: None,
            correlation_id: "corr".into(),
            slices: None,
        },
        &ctx(1),
    );
    assert!(state.tabs[&tab_id].rootnode.is_none());
}

/// End-to-end regression for
/// docs/investigations/INVESTIGATION_LAYOUT_DEAD_SPACE_STALE_TREE_RESURRECTION_2026_07_08.md:
/// a wholesale tree push — the exact vector a stale frontend copy used
/// to resurrect a deleted block's leaf through — must self-heal
/// instead of persisting the dangling reference. Only "ba" is
/// registered as live; "gone" is not (simulating a block deleted
/// before this push landed, e.g. from a stale frontend LayoutModel).
#[test]
fn layout_set_tree_self_heals_a_dangling_block_ref_in_the_pushed_tree() {
    let (mut state, tab_id) = fresh_tab();
    seed_block(&mut state, &tab_id, "ba");

    let pushed_tree = agentmux_common::LayoutNode {
        id: "root".into(),
        children: vec![leaf_node("a", "ba"), leaf_node("b", "gone")],
        ..Default::default()
    };
    let events = update(
        &mut state,
        Command::LayoutSetTree {
            tab_id: tab_id.clone(),
            new_tree: Some(pushed_tree),
            correlation_id: "corr".into(),
            slices: None,
        },
        &ctx(1),
    );

    assert!(matches!(&events[0], Event::LayoutTreeReplaced { .. }));
    assert!(tree_has(&state, &tab_id, "a"), "live block's leaf survives");
    assert!(!tree_has(&state, &tab_id, "b"), "dangling leaf pruned, not persisted");
    // Event::LayoutTreeReplaced's own new_tree must reflect the healed
    // tree too, not the caller's original — this is what the persist
    // subscriber writes to db_layout, so a pruned reducer state with a
    // stale event would just move the divergence downstream.
    if let Event::LayoutTreeReplaced { new_tree, .. } = &events[0] {
        let root = new_tree.as_ref().unwrap();
        assert!(
            !root.children.iter().any(|c| c.id == "b"),
            "emitted event's new_tree must already reflect the prune"
        );
    }
}

fn tree_with_dangling_leaf() -> agentmux_common::LayoutNode {
    agentmux_common::LayoutNode {
        id: "root".into(),
        children: vec![leaf_node("a", "ba"), leaf_node("b", "gone")],
        ..Default::default()
    }
}

/// A prune that drops the focused/magnified leaf must hand the persist
/// side the cleared ids: in the slices when the push carries them...
#[test]
fn layout_set_tree_prune_clears_focus_in_the_emitted_slices() {
    let (mut state, tab_id) = fresh_tab();
    seed_block(&mut state, &tab_id, "ba");
    let events = update(
        &mut state,
        Command::LayoutSetTree {
            tab_id: tab_id.clone(),
            new_tree: Some(tree_with_dangling_leaf()),
            correlation_id: "corr".into(),
            slices: Some(agentmux_common::LayoutClientSlices {
                focused_node_id: "b".into(),
                magnified_node_id: "b".into(),
                ..Default::default()
            }),
        },
        &ctx(1),
    );
    match &events[..] {
        [Event::LayoutTreeReplaced { slices: Some(s), .. }] => {
            assert_eq!(s.focused_node_id, "");
            assert_eq!(s.magnified_node_id, "");
        }
        other => panic!("expected one LayoutTreeReplaced with slices, got {:?}", other),
    }
    assert_eq!(state.tabs[&tab_id].focused_node_id, "");
    assert_eq!(state.tabs[&tab_id].magnified_node_id, "");
}

/// ...and as their own events when it doesn't.
#[test]
fn layout_set_tree_prune_without_slices_emits_focus_and_magnify_changes() {
    let (mut state, tab_id) = fresh_tab();
    seed_block(&mut state, &tab_id, "ba");
    state.tabs.get_mut(&tab_id).unwrap().focused_node_id = "b".into();
    state.tabs.get_mut(&tab_id).unwrap().magnified_node_id = "b".into();
    let events = update(
        &mut state,
        Command::LayoutSetTree {
            tab_id: tab_id.clone(),
            new_tree: Some(tree_with_dangling_leaf()),
            correlation_id: "corr".into(),
            slices: None,
        },
        &ctx(1),
    );
    match &events[..] {
        [Event::LayoutTreeReplaced { .. }, Event::FocusedNodeChanged { node_id: f, .. }, Event::MagnifiedNodeChanged { node_id: m, .. }] => {
            assert_eq!((f.as_str(), m.as_str()), ("", ""));
        }
        other => panic!("expected tree + focus + magnify events, got {:?}", other),
    }

    // No prune, focus still resolves: just the tree event.
    let events = update(
        &mut state,
        Command::LayoutSetTree {
            tab_id,
            new_tree: Some(leaf_node("a", "ba")),
            correlation_id: "corr".into(),
            slices: None,
        },
        &ctx(2),
    );
    assert!(matches!(&events[..], [Event::LayoutTreeReplaced { .. }]), "got {:?}", events);
}

/// SPEC_864 Phase 2 — a slice-carrying full-row push applies focus/
/// magnify to the TabRecord (empty = clear) and echoes the slices on
/// the emitted event for the persist subscriber.
#[test]
fn layout_set_tree_with_slices_applies_focus_magnify() {
    let (mut state, tab_id) = fresh_tab();
    seed_block(&mut state, &tab_id, "b1");
    state.tabs.get_mut(&tab_id).unwrap().magnified_node_id = "stale".into();

    let events = update(
        &mut state,
        Command::LayoutSetTree {
            tab_id: tab_id.clone(),
            new_tree: Some(leaf_node("n1", "b1")),
            correlation_id: "corr".into(),
            slices: Some(agentmux_common::LayoutClientSlices {
                leaforder: None,
                focused_node_id: "n1".into(),
                magnified_node_id: String::new(),
                pending_backend_actions: None,
            }),
        },
        &ctx(1),
    );

    assert_eq!(state.tabs[&tab_id].focused_node_id, "n1");
    assert!(
        state.tabs[&tab_id].magnified_node_id.is_empty(),
        "empty slice value clears stale magnify"
    );
    assert!(matches!(
        &events[0],
        Event::LayoutTreeReplaced { slices: Some(_), .. }
    ));
}

/// Empty-tree contract wins over slices: wiping the tree clears focus/
/// magnify even if the (skewed) push carried non-empty ids.
#[test]
fn layout_set_tree_none_tree_overrides_slice_focus() {
    let (mut state, tab_id) = fresh_tab();
    let _ = update(
        &mut state,
        Command::LayoutSetTree {
            tab_id: tab_id.clone(),
            new_tree: None,
            correlation_id: "corr".into(),
            slices: Some(agentmux_common::LayoutClientSlices {
                leaforder: None,
                focused_node_id: "dangling".into(),
                magnified_node_id: "dangling".into(),
                pending_backend_actions: None,
            }),
        },
        &ctx(1),
    );
    assert!(state.tabs[&tab_id].focused_node_id.is_empty());
    assert!(state.tabs[&tab_id].magnified_node_id.is_empty());
}


#[test]
fn layout_delete_node_on_empty_tree_is_noop() {
    let (mut state, tab_id) = fresh_tab();
    let events = update(
        &mut state,
        Command::LayoutDeleteNode {
            tab_id: tab_id.clone(),
            node_id: "ghost".into(),
            correlation_id: "corr".into(),
        },
        &ctx(1),
    );
    assert!(events.is_empty(), "no event for delete on empty tree");
    assert!(state.tabs[&tab_id].rootnode.is_none());
}

#[test]
fn layout_set_tree_to_none_also_clears_focused_and_magnified() {
    // An empty-tree set clears focus/magnify; the ids would
    // otherwise dangle past the wipe.
    let (mut state, tab_id) = fresh_tab();
    state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(leaf_node("n", "b"));
    state.tabs.get_mut(&tab_id).unwrap().focused_node_id = "n".into();
    state.tabs.get_mut(&tab_id).unwrap().magnified_node_id = "n".into();
    let _ = update(
        &mut state,
        Command::LayoutSetTree {
            tab_id: tab_id.clone(),
            new_tree: None,
            correlation_id: "corr".into(),
            slices: None,
        },
        &ctx(1),
    );
    let tab = &state.tabs[&tab_id];
    assert!(tab.rootnode.is_none());
    assert_eq!(tab.focused_node_id, "");
    assert_eq!(tab.magnified_node_id, "");
}

#[test]
fn layout_set_tree_with_some_preserves_focused_and_magnified() {
    // Symmetry guard: Some(new_tree) must NOT clear focused/
    // magnified — caller may have set them deliberately.
    let (mut state, tab_id) = fresh_tab();
    seed_block(&mut state, &tab_id, "b");
    state.tabs.get_mut(&tab_id).unwrap().focused_node_id = "n".into();
    state.tabs.get_mut(&tab_id).unwrap().magnified_node_id = "n".into();
    let _ = update(
        &mut state,
        Command::LayoutSetTree {
            tab_id: tab_id.clone(),
            new_tree: Some(leaf_node("n", "b")),
            correlation_id: "corr".into(),
            slices: None,
        },
        &ctx(1),
    );
    let tab = &state.tabs[&tab_id];
    assert_eq!(tab.focused_node_id, "n");
    assert_eq!(tab.magnified_node_id, "n");
}


fn tree_has(state: &State, tab_id: &str, node_id: &str) -> bool {
    state.tabs[tab_id].rootnode.as_ref().is_some_and(|r| {
        crate::backend::layout::find_node_by_id(r, node_id).is_some()
    })
}

#[test]
fn layout_delete_node_on_root_clears_the_tree() {
    // backend::layout::delete_node leaves
    // root deletion to the caller. Without the root-detection
    // branch, we'd emit LayoutNodeDeleted while rootnode still
    // contains the supposedly-deleted tree.
    let (mut state, tab_id) = fresh_tab();
    state.tabs.get_mut(&tab_id).unwrap().rootnode =
        Some(leaf_node("solitary-root", "b1"));
    let events = update(
        &mut state,
        Command::LayoutDeleteNode {
            tab_id: tab_id.clone(),
            node_id: "solitary-root".into(),
            correlation_id: "corr".into(),
        },
        &ctx(1),
    );
    assert!(matches!(&events[0], Event::LayoutNodeDeleted { .. }));
    assert!(
        state.tabs[&tab_id].rootnode.is_none(),
        "root deletion must wipe the tree"
    );
}

// ── SPEC_864 site #6 — LayoutDeleteNodeByBlock + new_tree carriage ──────

#[test]
fn layout_delete_node_by_block_resolves_and_carries_new_tree() {
    let (mut state, tab_id) = fresh_tab();
    let mut root = leaf_node("group", "");
    root.data = None;
    root.children = vec![
        leaf_node("a", "b1"),
        leaf_node("b", "b2"),
        leaf_node("c", "b3"),
    ];
    state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(root);

    let events = update(
        &mut state,
        Command::LayoutDeleteNodeByBlock {
            tab_id: tab_id.clone(),
            block_id: "b2".into(),
            correlation_id: "corr".into(),
        },
        &ctx(1),
    );
    match &events[0] {
        Event::LayoutNodeDeleted {
            node_id,
            new_tree,
            tree_cleared,
            ..
        } => {
            assert_eq!(node_id, "b", "must resolve block b2 to node b");
            assert!(!*tree_cleared, "non-root delete must not claim a clear");
            let tree = new_tree.as_ref().expect("non-root delete keeps a tree");
            assert!(
                crate::backend::layout::find_node_id_by_block(tree, "b2").is_none(),
                "deleted block must be gone from the event's tree"
            );
            assert!(
                crate::backend::layout::find_node_id_by_block(tree, "b1").is_some(),
                "sibling blocks must survive"
            );
            // The event's tree IS the reducer's post-delete tree — the
            // single-writer contract the persist subscriber relies on.
            assert_eq!(Some(tree), state.tabs[&tab_id].rootnode.as_ref());
        }
        other => panic!("expected LayoutNodeDeleted, got {:?}", other),
    }
}

fn stacked_leaf(id: &str, stack: &[&str], visible: &str) -> agentmux_common::LayoutNode {
    let mut leaf = leaf_node(id, visible);
    let data = leaf.data.as_mut().unwrap();
    data.block_stack = stack.iter().map(|s| s.to_string()).collect();
    data.active_block_id = visible.to_string();
    leaf
}

fn only_leaf_data(state: &State, tab_id: &str) -> agentmux_common::LayoutNodeData {
    state.tabs[tab_id].rootnode.as_ref().unwrap().data.clone().unwrap()
}

fn delete_by_block(state: &mut State, tab_id: &str, block_id: &str) -> Vec<Event> {
    update(
        state,
        Command::LayoutDeleteNodeByBlock {
            tab_id: tab_id.to_string(),
            block_id: block_id.to_string(),
            correlation_id: String::new(),
        },
        &ctx(1),
    )
}

// SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §2.2 / §3.4: deleting one
// tab of a stacked pane removes that tab only — never the pane and its
// sibling tabs (which left their blocks, agents included, with no pane).

#[test]
fn layout_delete_node_by_block_removes_only_a_background_stack_member() {
    let (mut state, tab_id) = fresh_tab();
    state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(stacked_leaf("pane", &["agent", "term"], "agent"));

    let events = delete_by_block(&mut state, &tab_id, "term");

    assert!(matches!(events[0], Event::LayoutTreeReplaced { slices: None, .. }), "got {:?}", events);
    let data = only_leaf_data(&state, &tab_id);
    assert_eq!(data.block_stack, vec!["agent".to_string()]);
    assert_eq!(data.block_id, "agent", "the visible tab is untouched");
}

#[test]
fn layout_delete_node_by_block_on_the_visible_member_activates_the_right_neighbour() {
    let (mut state, tab_id) = fresh_tab();
    state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(stacked_leaf("pane", &["a", "b", "c"], "b"));

    delete_by_block(&mut state, &tab_id, "b");

    let data = only_leaf_data(&state, &tab_id);
    assert_eq!(data.block_stack, vec!["a".to_string(), "c".to_string()]);
    assert_eq!(data.block_id, "c");
    assert_eq!(data.active_block_id, "c");
}

#[test]
fn layout_delete_node_by_block_on_the_rightmost_visible_member_activates_the_new_last() {
    let (mut state, tab_id) = fresh_tab();
    state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(stacked_leaf("pane", &["a", "b"], "b"));

    delete_by_block(&mut state, &tab_id, "b");

    let data = only_leaf_data(&state, &tab_id);
    assert_eq!(data.block_stack, vec!["a".to_string()]);
    assert_eq!(data.block_id, "a");
}

#[test]
fn layout_delete_node_by_block_on_the_last_stack_member_removes_the_pane() {
    let (mut state, tab_id) = fresh_tab();
    state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(stacked_leaf("pane", &["a"], "a"));

    let events = delete_by_block(&mut state, &tab_id, "a");

    assert!(matches!(events[0], Event::LayoutNodeDeleted { .. }), "got {:?}", events);
    assert!(state.tabs[&tab_id].rootnode.is_none());
}

// ── Pane tabs (SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.2) ──────

/// §2.1: the block is created AND placed in one reducer step — there is
/// no state in which it exists without a pane.
#[test]
fn create_block_in_stack_creates_and_places_in_one_step() {
    let (mut state, tab_id) = fresh_tab();
    seed_block(&mut state, &tab_id, "anchor");
    state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(leaf_node("pane", "anchor"));

    let events = update(
        &mut state,
        Command::CreateBlockInStack {
            tab_id: tab_id.clone(),
            target_block_id: "anchor".into(),
            meta: serde_json::json!({ "view": "agent" }),
            activate: true,
        },
        &ctx(1),
    );
    let new_id = events
        .iter()
        .find_map(|e| match e {
            Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .expect("block created");
    assert!(
        events.iter().any(|e| matches!(e, Event::LayoutTreeReplaced { slices: None, .. })),
        "placed in the same step: {events:?}"
    );
    assert!(state.blocks.contains_key(&new_id));
    assert!(state.tabs[&tab_id].block_ids.contains(&new_id));
    let data = state.tabs[&tab_id].rootnode.as_ref().unwrap().data.clone().unwrap();
    assert_eq!(data.block_stack, vec!["anchor".to_string(), new_id.clone()]);
    assert_eq!(data.block_id, new_id);
}

#[test]
fn create_block_in_stack_creates_nothing_when_no_pane_holds_the_target() {
    let (mut state, tab_id) = fresh_tab();
    seed_block(&mut state, &tab_id, "anchor");
    state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(leaf_node("pane", "anchor"));
    let blocks_before = state.blocks.len();
    let tab_blocks_before = state.tabs[&tab_id].block_ids.clone();

    let events = update(
        &mut state,
        Command::CreateBlockInStack {
            tab_id: tab_id.clone(),
            target_block_id: "missing".into(),
            meta: serde_json::Value::Null,
            activate: true,
        },
        &ctx(1),
    );
    assert!(matches!(events.as_slice(), [Event::Error { .. }]), "got {events:?}");
    assert_eq!(state.blocks.len(), blocks_before, "no block without a pane");
    assert_eq!(state.tabs[&tab_id].block_ids, tab_blocks_before, "nothing added to the tab");
}

#[test]
fn layout_stack_push_rejects_a_block_from_another_tab() {
    let (mut state, tab_id) = fresh_tab();
    seed_block(&mut state, &tab_id, "anchor");
    state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(leaf_node("pane", "anchor"));
    let events = update(
        &mut state,
        Command::LayoutStackPush {
            tab_id: tab_id.clone(),
            target_block_id: "anchor".into(),
            block_id: "stranger".into(),
            activate: true,
            correlation_id: String::new(),
        },
        &ctx(1),
    );
    assert!(matches!(events.as_slice(), [Event::Error { .. }]), "got {events:?}");
    assert!(state.tabs[&tab_id].rootnode.as_ref().unwrap().data.as_ref().unwrap().block_stack.is_empty());
}

#[test]
fn layout_stack_activate_switches_the_visible_tab() {
    let (mut state, tab_id) = fresh_tab();
    seed_block(&mut state, &tab_id, "a");
    seed_block(&mut state, &tab_id, "b");
    let mut leaf = leaf_node("pane", "a");
    leaf.data.as_mut().unwrap().block_stack = vec!["a".into(), "b".into()];
    leaf.data.as_mut().unwrap().active_block_id = "a".into();
    state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(leaf);

    let events = update(
        &mut state,
        Command::LayoutStackActivate {
            tab_id: tab_id.clone(),
            block_id: "b".into(),
            correlation_id: String::new(),
        },
        &ctx(1),
    );
    assert!(matches!(events.as_slice(), [Event::LayoutTreeReplaced { .. }]), "got {events:?}");
    let data = state.tabs[&tab_id].rootnode.as_ref().unwrap().data.clone().unwrap();
    assert_eq!((data.block_id.as_str(), data.active_block_id.as_str()), ("b", "b"));
}

// ── LayoutStackMove (SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §4.1) ────

#[test]
fn layout_stack_move_reorders_within_the_same_pane() {
    let (mut state, tab_id) = fresh_tab();
    state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(stacked_leaf("pane", &["a", "b", "c"], "a"));

    let events = update(
        &mut state,
        Command::LayoutStackMove {
            tab_id: tab_id.clone(),
            block_id: "c".into(),
            target_block_id: "a".into(),
            position: agentmux_common::StackMovePosition::Before,
            activate: false,
            correlation_id: String::new(),
        },
        &ctx(1),
    );
    assert!(matches!(events.as_slice(), [Event::LayoutTreeReplaced { slices: None, .. }]), "got {events:?}");
    let data = only_leaf_data(&state, &tab_id);
    assert_eq!(data.block_stack, vec!["c".to_string(), "a".to_string(), "b".to_string()]);
    assert_eq!(data.block_id, "a", "not activated, and reorder alone must not switch the visible tab");
}

#[test]
fn layout_stack_move_moves_a_tab_into_a_different_pane() {
    let (mut state, tab_id) = fresh_tab();
    seed_block(&mut state, &tab_id, "a");
    seed_block(&mut state, &tab_id, "b");
    seed_block(&mut state, &tab_id, "c");
    let root = agentmux_common::LayoutNode {
        id: "root".into(),
        children: vec![stacked_leaf("src", &["a", "b"], "a"), leaf_node("dst", "c")],
        ..Default::default()
    };
    state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(root);

    let events = update(
        &mut state,
        Command::LayoutStackMove {
            tab_id: tab_id.clone(),
            block_id: "b".into(),
            target_block_id: "c".into(),
            position: agentmux_common::StackMovePosition::End,
            activate: true,
            correlation_id: String::new(),
        },
        &ctx(1),
    );
    assert!(matches!(events.as_slice(), [Event::LayoutTreeReplaced { slices: None, .. }]), "got {events:?}");
    let tree = state.tabs[&tab_id].rootnode.as_ref().unwrap();
    let src = tree.children[0].data.as_ref().unwrap();
    assert_eq!(src.block_stack, vec!["a".to_string()], "removed from the source pane");
    let dst = tree.children[1].data.as_ref().unwrap();
    assert_eq!(dst.block_stack, vec!["c".to_string(), "b".to_string()]);
    assert_eq!((dst.block_id.as_str(), dst.active_block_id.as_str()), ("b", "b"));
}

// Moving a pane's ONLY tab empties the pane, which is removed. Reported
// like a node delete so focus/magnify on the gone pane are reconciled.
// SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md §4.3.
fn lone_src_and_stacked_dst(state: &mut State, tab_id: &str) {
    for b in ["a", "c", "d", "z"] {
        seed_block(state, tab_id, b);
    }
    state.tabs.get_mut(tab_id).unwrap().rootnode = Some(agentmux_common::LayoutNode {
        id: "root".into(),
        children: vec![leaf_node("src", "a"), stacked_leaf("dst", &["c", "d"], "c"), leaf_node("other", "z")],
        ..Default::default()
    });
}

fn move_lone_a_to_d(state: &mut State, tab_id: &str) -> Vec<Event> {
    update(
        state,
        Command::LayoutStackMove {
            tab_id: tab_id.to_string(),
            block_id: "a".into(),
            target_block_id: "d".into(),
            position: agentmux_common::StackMovePosition::End,
            activate: true,
            correlation_id: String::new(),
        },
        &ctx(1),
    )
}

#[test]
fn layout_stack_move_of_a_panes_only_tab_removes_the_emptied_pane() {
    let (mut state, tab_id) = fresh_tab();
    lone_src_and_stacked_dst(&mut state, &tab_id);
    let blocks_before = state.blocks.len();

    let events = move_lone_a_to_d(&mut state, &tab_id);
    let tree = state.tabs[&tab_id].rootnode.clone().unwrap();
    match events.as_slice() {
        [Event::LayoutNodeDeleted { node_id, new_tree, tree_cleared, .. }] => {
            assert_eq!(node_id, "src", "the emptied source pane");
            assert_eq!(new_tree.as_ref(), Some(&tree), "carries the full post-move tree for persistence");
            assert!(!tree_cleared);
        }
        other => panic!("expected one LayoutNodeDeleted, got {other:?}"),
    }
    let ids: Vec<&str> = tree.children.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, vec!["dst", "other"]);
    let dst = tree.children[0].data.as_ref().unwrap();
    assert_eq!(dst.block_stack, vec!["c".to_string(), "d".to_string(), "a".to_string()]);
    assert_eq!((dst.block_id.as_str(), dst.active_block_id.as_str()), ("a", "a"));
    assert_eq!(state.blocks.len(), blocks_before, "a move: no block is deleted");
}

#[test]
fn layout_stack_move_of_a_panes_only_tab_clears_focus_and_magnify_on_the_gone_pane() {
    let (mut state, tab_id) = fresh_tab();
    lone_src_and_stacked_dst(&mut state, &tab_id);
    {
        let tab = state.tabs.get_mut(&tab_id).unwrap();
        tab.focused_node_id = "src".into();
        tab.magnified_node_id = "src".into();
    }
    let events = move_lone_a_to_d(&mut state, &tab_id);
    assert!(
        matches!(events.as_slice(), [Event::LayoutNodeDeleted { was_focused: true, was_magnified: true, .. }]),
        "got {events:?}"
    );
    let tab = &state.tabs[&tab_id];
    assert!(tab.focused_node_id.is_empty() && tab.magnified_node_id.is_empty());
}

#[test]
fn layout_stack_move_of_a_panes_only_tab_keeps_focus_on_a_pane_that_survives() {
    let (mut state, tab_id) = fresh_tab();
    lone_src_and_stacked_dst(&mut state, &tab_id);
    state.tabs.get_mut(&tab_id).unwrap().focused_node_id = "other".into();
    let events = move_lone_a_to_d(&mut state, &tab_id);
    assert!(
        matches!(events.as_slice(), [Event::LayoutNodeDeleted { was_focused: false, was_magnified: false, .. }]),
        "got {events:?}"
    );
    assert_eq!(state.tabs[&tab_id].focused_node_id, "other");
}

#[test]
fn layout_delete_node_by_block_unknown_block_is_silent_noop() {
    // Every delete_block saga run dispatches this command; a block
    // with no layout node (frontend already pruned, never laid out,
    // empty tab) must be a silent no-op — no Event::Error, no
    // version churn, tree untouched.
    let (mut state, tab_id) = fresh_tab();
    state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(leaf_node("only", "b1"));

    let events = update(
        &mut state,
        Command::LayoutDeleteNodeByBlock {
            tab_id: tab_id.clone(),
            block_id: "not-in-tree".into(),
            correlation_id: String::new(),
        },
        &ctx(1),
    );
    assert!(events.is_empty(), "unresolved block must be a silent no-op");
    assert_eq!(state.tabs[&tab_id].rootnode.as_ref().unwrap().id, "only");
}

#[test]
fn layout_delete_node_by_block_root_orphan_sets_tree_cleared() {
    // Deleting the last block empties the tree — the one structural
    // op where new_tree=None is legitimate. tree_cleared=true is what
    // lets the persist subscriber distinguish this from a
    // version-skewed sender's absent field.
    let (mut state, tab_id) = fresh_tab();
    state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(leaf_node("solo", "b1"));

    let events = update(
        &mut state,
        Command::LayoutDeleteNodeByBlock {
            tab_id: tab_id.clone(),
            block_id: "b1".into(),
            correlation_id: String::new(),
        },
        &ctx(1),
    );
    match &events[0] {
        Event::LayoutNodeDeleted {
            new_tree,
            tree_cleared,
            ..
        } => {
            assert!(new_tree.is_none());
            assert!(*tree_cleared, "root-orphan delete must mark the clear as real");
        }
        other => panic!("expected LayoutNodeDeleted, got {:?}", other),
    }
    assert!(state.tabs[&tab_id].rootnode.is_none());
}

#[test]
fn layout_delete_node_clears_magnified_when_target_was_magnified() {
    // magnified must be cleared
    // alongside focused; same staleness concern.
    let (mut state, tab_id) = fresh_tab();
    let mut root = leaf_node("group", "");
    root.data = None;
    root.children = vec![leaf_node("a", "b1"), leaf_node("b", "b2")];
    state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(root);
    state.tabs.get_mut(&tab_id).unwrap().magnified_node_id = "a".into();
    let _ = update(
        &mut state,
        Command::LayoutDeleteNode {
            tab_id: tab_id.clone(),
            node_id: "a".into(),
            correlation_id: "corr".into(),
        },
        &ctx(1),
    );
    assert_eq!(state.tabs[&tab_id].magnified_node_id, "");
}

#[test]
fn layout_node_deleted_event_carries_was_magnified() {
    // subscribers need
    // the was_magnified field to refresh their UI when the
    // magnified node is deleted.
    let (mut state, tab_id) = fresh_tab();
    let mut root = leaf_node("group", "");
    root.data = None;
    root.children = vec![leaf_node("a", "b1"), leaf_node("b", "b2")];
    state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(root);
    state.tabs.get_mut(&tab_id).unwrap().magnified_node_id = "a".into();

    let events = update(
        &mut state,
        Command::LayoutDeleteNode {
            tab_id: tab_id.clone(),
            node_id: "a".into(),
            correlation_id: "corr".into(),
        },
        &ctx(1),
    );
    match &events[0] {
        Event::LayoutNodeDeleted { was_magnified, was_focused, .. } => {
            assert!(*was_magnified, "was_magnified must be true");
            assert!(!*was_focused, "was_focused stays false");
        }
        other => panic!("expected LayoutNodeDeleted, got {:?}", other),
    }
}

#[test]
fn layout_delete_node_clears_focused_when_collapse_replaces_parent_id() {
    // `backend::layout::delete_node`'s collapse-sole-child path
    // promotes the surviving child and rewrites the parent's id
    // to the child's id. If focused/magnified pointed at the
    // ORIGINAL parent id, that id is gone from the tree even
    // though the same physical layout slot exists. Reducer must
    // clear the dangling reference.
    let (mut state, tab_id) = fresh_tab();
    let mut group = leaf_node("group-id", "");
    group.data = None;
    group.children = vec![leaf_node("only-child", "b1"), leaf_node("sibling", "b2")];
    state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(group);
    // Focus the group (the parent that will get its id rewritten
    // when "sibling" is deleted and "only-child" is the sole
    // survivor of the now-1-child group).
    state.tabs.get_mut(&tab_id).unwrap().focused_node_id = "group-id".into();

    let events = update(
        &mut state,
        Command::LayoutDeleteNode {
            tab_id: tab_id.clone(),
            node_id: "sibling".into(),
            correlation_id: "corr".into(),
        },
        &ctx(1),
    );
    match &events[0] {
        Event::LayoutNodeDeleted { was_focused, .. } => {
            assert!(
                *was_focused,
                "collapse rewrote parent id; reducer must report focus loss"
            );
        }
        other => panic!("expected LayoutNodeDeleted, got {:?}", other),
    }
    assert_eq!(
        state.tabs[&tab_id].focused_node_id, "",
        "stale focus cleared post-collapse"
    );
}

#[test]
fn layout_delete_node_clears_focused_when_target_subtree_contains_focus() {
    // deleting a container
    // wipes its descendants, but a direct-id-match check on
    // focused/magnified misses descendants — they stay dangling.
    let (mut state, tab_id) = fresh_tab();
    // Tree:
    //   root-group (children: group-A, leaf-z)
    //     group-A (children: leaf-x, leaf-y)
    let mut leaf_x = leaf_node("leaf-x", "bx");
    leaf_x.data = Some(agentmux_common::LayoutNodeData {
        block_id: "bx".into(),
        ..Default::default()
    });
    let mut group_a = leaf_node("group-A", "");
    group_a.data = None;
    group_a.children = vec![leaf_x, leaf_node("leaf-y", "by")];
    let mut root = leaf_node("root-group", "");
    root.data = None;
    root.children = vec![group_a, leaf_node("leaf-z", "bz")];
    state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(root);
    // Focus a descendant of group-A, then delete group-A.
    state.tabs.get_mut(&tab_id).unwrap().focused_node_id = "leaf-x".into();
    state.tabs.get_mut(&tab_id).unwrap().magnified_node_id = "leaf-y".into();

    let events = update(
        &mut state,
        Command::LayoutDeleteNode {
            tab_id: tab_id.clone(),
            node_id: "group-A".into(),
            correlation_id: "corr".into(),
        },
        &ctx(1),
    );
    match &events[0] {
        Event::LayoutNodeDeleted { was_focused, was_magnified, .. } => {
            assert!(*was_focused, "descendant focus must be cleared");
            assert!(*was_magnified, "descendant magnify must be cleared");
        }
        other => panic!("expected LayoutNodeDeleted, got {:?}", other),
    }
    assert_eq!(state.tabs[&tab_id].focused_node_id, "");
    assert_eq!(state.tabs[&tab_id].magnified_node_id, "");
}

#[test]
fn layout_delete_node_clears_focused_when_target_was_focused() {
    let (mut state, tab_id) = fresh_tab();
    // Tree: group with two leaves.
    let mut root = leaf_node("root-group", "");
    root.data = None;
    root.children = vec![leaf_node("a", "b1"), leaf_node("b", "b2")];
    state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(root);
    state.tabs.get_mut(&tab_id).unwrap().focused_node_id = "a".into();

    let events = update(
        &mut state,
        Command::LayoutDeleteNode {
            tab_id: tab_id.clone(),
            node_id: "a".into(),
            correlation_id: "corr-del".into(),
        },
        &ctx(1),
    );
    assert!(matches!(
        &events[0],
        Event::LayoutNodeDeleted { was_focused: true, .. }
    ));
    assert_eq!(state.tabs[&tab_id].focused_node_id, "");
}

/// SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §5: the stack commands keep
/// I1 (each block in exactly one pane) and I2 (a consistent stack).
#[test]
fn stack_push_activate_and_remove_keep_every_block_in_exactly_one_pane() {
    let (mut state, tab_id) = fresh_tab();
    let (a, b, c) = (create_block(&mut state, &tab_id), create_block(&mut state, &tab_id), create_block(&mut state, &tab_id));
    let tree = agentmux_common::LayoutNode {
        id: "root".into(),
        size: 1.0,
        children: vec![leaf_node("pa", &a), leaf_node("pc", &c)],
        ..Default::default()
    };
    state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(tree);
    let check = |state: &State, unplaced: &[&String]| {
        let tab = &state.tabs[&tab_id];
        assert!(crate::backend::layout::validate_layout_invariants(&tab.rootnode).is_empty());
        let expected: Vec<String> = unplaced.iter().map(|s| s.to_string()).collect();
        assert_eq!(crate::backend::layout::unplaced_blocks(&tab.rootnode, &tab.block_ids), expected);
    };
    check(&state, &[&b]);

    let push = Command::LayoutStackPush {
        tab_id: tab_id.clone(),
        target_block_id: a.clone(),
        block_id: b.clone(),
        activate: true,
        correlation_id: String::new(),
    };
    assert!(matches!(update(&mut state, push, &ctx(1))[0], Event::LayoutTreeReplaced { .. }));
    check(&state, &[]);

    let activate = Command::LayoutStackActivate { tab_id: tab_id.clone(), block_id: a.clone(), correlation_id: String::new() };
    assert!(matches!(update(&mut state, activate, &ctx(1))[0], Event::LayoutTreeReplaced { .. }));
    check(&state, &[]);

    // The tab leaves its pane first; its block goes with the ClosePane after.
    let remove = Command::LayoutDeleteNodeByBlock { tab_id: tab_id.clone(), block_id: b.clone(), correlation_id: String::new() };
    assert!(matches!(update(&mut state, remove, &ctx(1))[0], Event::LayoutTreeReplaced { .. }));
    check(&state, &[&b]);
}
