// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use agentmux_common::ipc::{Command, ErrorCode, Event};
use agentmux_common::{FlexDirection, LayoutIntent, LayoutNode, LayoutNodeData, LayoutViewState, SplitPosition};

use crate::backend::layout::intent::derive_node_id;
use crate::reducer::test_support::*;
use crate::reducer::update;
use crate::state::State;

const ACTION: &str = "a0000001-0000-4000-8000-0000000000aa";

fn leaf(id: &str, block: &str) -> LayoutNode {
    LayoutNode {
        id: id.into(),
        flex_direction: FlexDirection::Column,
        size: 10.0,
        data: Some(LayoutNodeData { block_id: block.into(), ..Default::default() }),
        ..Default::default()
    }
}

/// A tab whose tree is one pane of block `a`, plus a second block `b` of the
/// tab placed in no pane.
fn one_pane() -> (State, String, String, String) {
    let mut state = State::default();
    let ws = create_workspace(&mut state, "w");
    let tab = create_tab(&mut state, &ws, "t");
    let a = create_block(&mut state, &tab);
    let b = create_block(&mut state, &tab);
    state.tabs.get_mut(&tab).unwrap().rootnode = Some(LayoutNode { flex_direction: FlexDirection::Row, ..leaf("na", &a) });
    (state, tab, a, b)
}

fn intent(state: &mut State, tab: &str, intent: LayoutIntent, action_id: &str, view: Option<LayoutViewState>) -> Vec<Event> {
    update(
        state,
        Command::LayoutApplyIntent { tab_id: tab.into(), intent, action_id: action_id.into(), view },
        &ctx(1),
    )
}

fn split_b(b: &str) -> LayoutIntent {
    LayoutIntent::SplitHorizontal {
        target_node_id: "na".into(),
        new_node: leaf("nb", b),
        position: SplitPosition::After,
        size_fraction: Some(0.5),
        focused: true,
    }
}

#[test]
fn a_split_is_computed_and_reported_as_the_new_tree() {
    let (mut state, tab, _a, b) = one_pane();
    let events = intent(&mut state, &tab, split_b(&b), ACTION, None);
    let root = state.tabs[&tab].rootnode.clone().unwrap();
    assert_eq!(root.id, derive_node_id(ACTION, 1));
    assert_eq!(root.children.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["na", "nb"]);
    assert!(matches!(&events[0], Event::LayoutTreeReplaced { new_tree: Some(t), slices: None, .. } if *t == root));
    // No sender view: the intent's own `focused` decides.
    assert_eq!(state.tabs[&tab].focused_node_id, "nb");
    assert!(matches!(&events[1], Event::FocusedNodeChanged { node_id, .. } if node_id == "nb"));
}

#[test]
fn the_senders_view_wins_and_is_cleared_when_it_names_no_node() {
    let (mut state, tab, _a, b) = one_pane();
    let view = LayoutViewState { focused_node_id: "na".into(), magnified_node_id: "ghost".into() };
    intent(&mut state, &tab, split_b(&b), ACTION, Some(view));
    assert_eq!(state.tabs[&tab].focused_node_id, "na");
    assert_eq!(state.tabs[&tab].magnified_node_id, "", "a magnified id with no node is cleared");
}

#[test]
fn a_view_intent_changes_only_focus_and_magnify() {
    let (mut state, tab, _a, b) = one_pane();
    intent(&mut state, &tab, split_b(&b), ACTION, None);
    let view = LayoutViewState { focused_node_id: "na".into(), magnified_node_id: "na".into() };
    let events = intent(&mut state, &tab, LayoutIntent::View, "a2", Some(view));
    assert_eq!(events.len(), 2, "{events:?}");
    assert!(matches!(&events[0], Event::FocusedNodeChanged { node_id, .. } if node_id == "na"));
    assert!(matches!(&events[1], Event::MagnifiedNodeChanged { node_id, .. } if node_id == "na"));
}

#[test]
fn a_repeated_action_id_is_a_no_op() {
    let (mut state, tab, _a, b) = one_pane();
    intent(&mut state, &tab, split_b(&b), ACTION, None);
    let after_first = state.tabs[&tab].rootnode.clone();
    let events = intent(&mut state, &tab, split_b(&b), ACTION, None);
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(state.tabs[&tab].rootnode, after_first);
}

#[test]
fn only_the_latest_action_ids_are_remembered() {
    let (mut state, tab, ..) = one_pane();
    for i in 0..crate::reducer::layout::RECENT_INTENTS + 5 {
        intent(&mut state, &tab, LayoutIntent::View, &format!("v{i}"), None);
    }
    let ids = &state.tabs[&tab].recent_intent_ids;
    assert_eq!(ids.len(), crate::reducer::layout::RECENT_INTENTS);
    assert_eq!(ids.last().map(String::as_str), Some(format!("v{}", crate::reducer::layout::RECENT_INTENTS + 4).as_str()));
}

#[test]
fn a_refused_intent_changes_nothing_and_answers_an_error() {
    let (mut state, tab, _a, b) = one_pane();
    let before = state.tabs[&tab].clone();
    let refused = LayoutIntent::SplitVertical {
        target_node_id: "missing".into(),
        new_node: leaf("nb", &b),
        position: SplitPosition::Before,
        size_fraction: None,
        focused: true,
    };
    let events = intent(&mut state, &tab, refused, ACTION, None);
    assert!(matches!(&events[0], Event::Error { code: ErrorCode::InvalidCommand, fatal: false, message, .. } if message.contains("splitvertical")));
    assert_eq!(state.tabs[&tab], before, "nothing applied, and the id isn't remembered");
}

#[test]
fn an_unknown_tab_is_an_error() {
    let mut state = State::default();
    let events = intent(&mut state, "nope", LayoutIntent::View, ACTION, None);
    assert!(matches!(&events[0], Event::Error { message, .. } if message.contains("unknown tab")));
}

#[test]
fn a_leaf_for_a_deleted_block_is_pruned_on_the_way() {
    let (mut state, tab, _a, _b) = one_pane();
    let events = intent(
        &mut state,
        &tab,
        LayoutIntent::Insert { node: leaf("ghost", "no-such-block"), magnified: false, focused: false },
        ACTION,
        None,
    );
    let root = state.tabs[&tab].rootnode.clone().unwrap();
    assert!(crate::backend::layout::find_leaf_containing_block(&root, "no-such-block").is_none());
    assert!(events.is_empty() || matches!(events[0], Event::LayoutTreeReplaced { .. }));
}

#[test]
fn legacy_minimize_fields_are_migrated_before_the_intent() {
    let (mut state, tab, a, b) = one_pane();
    let mut squeezed = leaf("nb", &b);
    squeezed.extra.insert("minimizedSize".into(), 30.into());
    squeezed.extra.insert("minimizedLockedSize".into(), 1.into());
    state.tabs.get_mut(&tab).unwrap().rootnode =
        Some(LayoutNode { id: "r".into(), size: 10.0, children: vec![leaf("na", &a), squeezed], ..Default::default() });
    intent(&mut state, &tab, LayoutIntent::View, ACTION, None);
    let root = state.tabs[&tab].rootnode.clone().unwrap();
    let nb = &root.children[1];
    assert_eq!(nb.size, 30.0);
    assert_eq!(nb.extra.get("minimized"), Some(&serde_json::Value::Bool(true)));
    assert_eq!(nb.extra.len(), 1);
}

#[test]
fn delete_clears_focus_on_the_deleted_pane_without_a_sender_view() {
    let (mut state, tab, _a, b) = one_pane();
    intent(&mut state, &tab, split_b(&b), ACTION, None);
    let events = intent(&mut state, &tab, LayoutIntent::Delete { node_id: "nb".into() }, "a2", None);
    assert_eq!(state.tabs[&tab].focused_node_id, "");
    assert_eq!(state.tabs[&tab].rootnode.as_ref().unwrap().id, "na");
    assert!(events.iter().any(|e| matches!(e, Event::FocusedNodeChanged { node_id, .. } if node_id.is_empty())));
}
