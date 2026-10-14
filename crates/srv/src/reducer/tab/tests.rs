// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::reducer::test_support::*;
use crate::reducer::update;
use agentmux_common::ipc::{Command, ErrorCode};

#[test]
fn create_tab_validates_workspace_exists() {
    let mut state = State::default();
    let events = update(
        &mut state,
        Command::CreateTab {
            workspace_id: "no-such-ws".into(),
            name: "t1".into(),
        },
        &ctx(1),
    );
    assert!(matches!(&events[0], Event::Error { .. }));
    assert!(state.tabs.is_empty());
}

#[test]
fn create_tab_first_tab_becomes_active() {
    let mut state = State::default();
    let ws_id = create_workspace(&mut state, "w");
    let events = update(
        &mut state,
        Command::CreateTab {
            workspace_id: ws_id.clone(),
            name: "t1".into(),
        },
        &ctx(1),
    );
    // First event: TabCreated; second: ActiveTabChanged.
    assert!(matches!(&events[0], Event::TabCreated { .. }));
    assert!(matches!(&events[1], Event::ActiveTabChanged { .. }));
    let workspace = &state.workspaces[&ws_id];
    assert_eq!(workspace.tab_ids.len(), 1);
    assert_eq!(workspace.active_tab_id, Some(workspace.tab_ids[0].clone()));
    assert_eq!(state.tabs.len(), 1);
}

#[test]
fn create_tab_second_tab_does_not_steal_active() {
    let mut state = State::default();
    let ws_id = create_workspace(&mut state, "w");
    let _ = update(
        &mut state,
        Command::CreateTab {
            workspace_id: ws_id.clone(),
            name: "t1".into(),
        },
        &ctx(1),
    );
    let first_active = state.workspaces[&ws_id].active_tab_id.clone();
    let events = update(
        &mut state,
        Command::CreateTab {
            workspace_id: ws_id.clone(),
            name: "t2".into(),
        },
        &ctx(2),
    );
    // Only TabCreated — second tab does not become active.
    assert!(matches!(&events[0], Event::TabCreated { .. }));
    assert_eq!(events.len(), 1);
    assert_eq!(state.workspaces[&ws_id].active_tab_id, first_active);
    assert_eq!(state.workspaces[&ws_id].tab_ids.len(), 2);
}

#[test]
fn delete_tab_removes_from_state_and_workspace_list() {
    let mut state = State::default();
    let ws_id = create_workspace(&mut state, "w");
    let _ = update(
        &mut state,
        Command::CreateTab {
            workspace_id: ws_id.clone(),
            name: "t1".into(),
        },
        &ctx(1),
    );
    let _ = update(
        &mut state,
        Command::CreateTab {
            workspace_id: ws_id.clone(),
            name: "t2".into(),
        },
        &ctx(2),
    );
    let tab2_id = state.workspaces[&ws_id].tab_ids[1].clone();
    let events = update(
        &mut state,
        Command::DeleteTab {
            workspace_id: ws_id.clone(),
            tab_id: tab2_id.clone(),
            force: false,
        },
        &ctx(3),
    );
    assert!(matches!(&events[0], Event::TabDeleted { .. }));
    // tab2 wasn't active, so no ActiveTabChanged.
    assert_eq!(events.len(), 1);
    assert!(!state.tabs.contains_key(&tab2_id));
    assert_eq!(state.workspaces[&ws_id].tab_ids.len(), 1);
}

#[test]
fn delete_active_tab_promotes_neighbor() {
    let mut state = State::default();
    let ws_id = create_workspace(&mut state, "w");
    let _ = update(
        &mut state,
        Command::CreateTab {
            workspace_id: ws_id.clone(),
            name: "t1".into(),
        },
        &ctx(1),
    );
    let _ = update(
        &mut state,
        Command::CreateTab {
            workspace_id: ws_id.clone(),
            name: "t2".into(),
        },
        &ctx(2),
    );
    let tab1_id = state.workspaces[&ws_id].tab_ids[0].clone();
    let tab2_id = state.workspaces[&ws_id].tab_ids[1].clone();
    // tab1 was created first → it's active. Delete it.
    let events = update(
        &mut state,
        Command::DeleteTab {
            workspace_id: ws_id.clone(),
            tab_id: tab1_id.clone(),
            force: false,
        },
        &ctx(3),
    );
    assert!(matches!(&events[0], Event::TabDeleted { .. }));
    assert!(matches!(
        &events[1],
        Event::ActiveTabChanged { tab_id: Some(_), .. }
    ));
    // tab2 should now be active.
    assert_eq!(state.workspaces[&ws_id].active_tab_id, Some(tab2_id));
}

#[test]
fn delete_last_tab_clears_active_to_none() {
    // Reducer accepts last-tab delete (round 2 of PR #633 walked
    // back the guard). User-facing flows gate at the call site
    // (close button + keymodel both check `tab_ids.len() <= 1`);
    // internal compensation paths rely on this acceptance to
    // roll back failed CreateTab persists.
    let mut state = State::default();
    let ws_id = create_workspace(&mut state, "w");
    let _ = update(
        &mut state,
        Command::CreateTab {
            workspace_id: ws_id.clone(),
            name: "t1".into(),
        },
        &ctx(1),
    );
    let tab1_id = state.workspaces[&ws_id].tab_ids[0].clone();
    let events = update(
        &mut state,
        Command::DeleteTab {
            workspace_id: ws_id.clone(),
            tab_id: tab1_id,
            // Last-tab delete needs force=true post-round-4
            // (codex P2 #633). Test asserts the
            // ActiveTabChanged-to-None behavior still works
            // when compensation paths force a last-tab delete.
            force: true,
        },
        &ctx(2),
    );
    assert!(matches!(&events[0], Event::TabDeleted { .. }));
    assert!(matches!(
        &events[1],
        Event::ActiveTabChanged { tab_id: None, .. }
    ));
    assert_eq!(state.workspaces[&ws_id].active_tab_id, None);
    assert!(state.tabs.is_empty());
}

#[test]
fn delete_unknown_tab_silent_no_op() {
    let mut state = State::default();
    let ws_id = create_workspace(&mut state, "w");
    let events = update(
        &mut state,
        Command::DeleteTab {
            workspace_id: ws_id,
            tab_id: "ghost".into(),
            force: false,
        },
        &ctx(1),
    );
    assert!(events.is_empty());
}

#[test]
fn set_active_tab_validates_workspace_and_tab() {
    let mut state = State::default();
    let ws_id = create_workspace(&mut state, "w");
    let _ = update(
        &mut state,
        Command::CreateTab {
            workspace_id: ws_id.clone(),
            name: "t1".into(),
        },
        &ctx(1),
    );
    // Wrong workspace.
    let events = update(
        &mut state,
        Command::SetActiveTab {
            workspace_id: "no-such".into(),
            tab_id: "x".into(),
        },
        &ctx(2),
    );
    assert!(matches!(&events[0], Event::Error { .. }));
    // Right workspace, wrong tab.
    let events = update(
        &mut state,
        Command::SetActiveTab {
            workspace_id: ws_id,
            tab_id: "ghost".into(),
        },
        &ctx(3),
    );
    assert!(matches!(&events[0], Event::Error { .. }));
}

#[test]
fn set_active_tab_idempotent_no_event_when_already_active() {
    let mut state = State::default();
    let ws_id = create_workspace(&mut state, "w");
    let _ = update(
        &mut state,
        Command::CreateTab {
            workspace_id: ws_id.clone(),
            name: "t1".into(),
        },
        &ctx(1),
    );
    let tab_id = state.workspaces[&ws_id].tab_ids[0].clone();
    // Already active (auto-activated on first tab create).
    let events = update(
        &mut state,
        Command::SetActiveTab {
            workspace_id: ws_id,
            tab_id,
        },
        &ctx(2),
    );
    assert!(events.is_empty());
}

#[test]
fn reorder_tab_moves_to_new_index() {
    let mut state = State::default();
    let ws_id = create_workspace(&mut state, "w");
    for i in 1..=3 {
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: format!("t{}", i),
            },
            &ctx(i),
        );
    }
    let original = state.workspaces[&ws_id].tab_ids.clone();
    // Move first tab to index 2 (last).
    let events = update(
        &mut state,
        Command::ReorderTab {
            workspace_id: ws_id.clone(),
            tab_id: original[0].clone(),
            new_index: 2,
        },
        &ctx(10),
    );
    assert!(matches!(&events[0], Event::TabReordered { .. }));
    let after = &state.workspaces[&ws_id].tab_ids;
    assert_eq!(after[0], original[1]);
    assert_eq!(after[1], original[2]);
    assert_eq!(after[2], original[0]);
}

#[test]
fn reorder_tab_clamps_to_last_index() {
    let mut state = State::default();
    let ws_id = create_workspace(&mut state, "w");
    let _ = update(
        &mut state,
        Command::CreateTab {
            workspace_id: ws_id.clone(),
            name: "t1".into(),
        },
        &ctx(1),
    );
    let _ = update(
        &mut state,
        Command::CreateTab {
            workspace_id: ws_id.clone(),
            name: "t2".into(),
        },
        &ctx(2),
    );
    let original = state.workspaces[&ws_id].tab_ids.clone();
    // Asking for index 99 should clamp to 1 (len-1).
    let events = update(
        &mut state,
        Command::ReorderTab {
            workspace_id: ws_id.clone(),
            tab_id: original[0].clone(),
            new_index: 99,
        },
        &ctx(3),
    );
    if let Event::TabReordered { new_index, .. } = &events[0] {
        assert_eq!(*new_index, 1);
    } else {
        panic!("expected TabReordered");
    }
}

#[test]
fn reorder_tab_already_at_position_no_op() {
    let mut state = State::default();
    let ws_id = create_workspace(&mut state, "w");
    let _ = update(
        &mut state,
        Command::CreateTab {
            workspace_id: ws_id.clone(),
            name: "t1".into(),
        },
        &ctx(1),
    );
    let tab_id = state.workspaces[&ws_id].tab_ids[0].clone();
    let events = update(
        &mut state,
        Command::ReorderTab {
            workspace_id: ws_id,
            tab_id,
            new_index: 0,
        },
        &ctx(2),
    );
    assert!(events.is_empty());
}

#[test]
fn reorder_tab_validates_workspace_and_tab() {
    let mut state = State::default();
    let events = update(
        &mut state,
        Command::ReorderTab {
            workspace_id: "no-such-ws".into(),
            tab_id: "x".into(),
            new_index: 0,
        },
        &ctx(1),
    );
    assert!(matches!(&events[0], Event::Error { .. }));
    let ws_id = create_workspace(&mut state, "w");
    let events = update(
        &mut state,
        Command::ReorderTab {
            workspace_id: ws_id,
            tab_id: "ghost".into(),
            new_index: 0,
        },
        &ctx(2),
    );
    assert!(matches!(&events[0], Event::Error { .. }));
}

/// SPEC_864 Phase 5 — strict validation reinstated: `MoveTabToWorkspace`,
/// `PromoteBlockToTab`, `TearOffBlock`, and `TearOffTab` are all
/// reducer-routed now, so `workspace.tab_ids` can't legitimately
/// diverge from the caller's view. A `tab_ids` list containing an id
/// the reducer doesn't know about for this workspace must be rejected,
/// not silently accepted as the codex P1 #620 migration-window
/// relaxation used to do.
#[test]
fn reorder_tabs_bulk_rejects_unknown_ids() {
    let mut state = State::default();
    let ws_id = create_workspace(&mut state, "w");
    let known = create_tab(&mut state, &ws_id, "known");
    let unknown = "unknown-tab".to_string();
    let events = update(
        &mut state,
        Command::ReorderTabsBulk {
            workspace_id: ws_id.clone(),
            tab_ids: vec![unknown.clone(), known.clone()],
        },
        &ctx(99),
    );
    match &events[0] {
        Event::Error { code, message, .. } => {
            assert_eq!(*code, ErrorCode::InvalidCommand);
            assert!(
                message.contains("permutation"),
                "error should mention permutation, got: {}",
                message
            );
        }
        other => panic!("expected Error event, got {:?}", other),
    }
    // Rejected — workspace's tab_ids must be untouched.
    let ws = state.workspaces.get(&ws_id).expect("ws still present");
    assert_eq!(ws.tab_ids, vec![known]);
}

/// The permutation itself (same set, new order) must still succeed —
/// that's the whole point of the command.
#[test]
fn reorder_tabs_bulk_accepts_permutation_of_known_ids() {
    let mut state = State::default();
    let ws_id = create_workspace(&mut state, "w");
    let t1 = create_tab(&mut state, &ws_id, "t1");
    let t2 = create_tab(&mut state, &ws_id, "t2");
    let events = update(
        &mut state,
        Command::ReorderTabsBulk {
            workspace_id: ws_id.clone(),
            tab_ids: vec![t2.clone(), t1.clone()],
        },
        &ctx(99),
    );
    assert!(
        matches!(&events[0], Event::TabsReorderedBulk { .. }),
        "expected TabsReorderedBulk, got {:?}",
        events.first()
    );
    let ws = state.workspaces.get(&ws_id).expect("ws still present");
    assert_eq!(ws.tab_ids, vec![t2, t1]);
}

/// A `tab_ids` list missing a tab the reducer knows about (short
/// permutation) must also be rejected, not silently drop the tab.
#[test]
fn reorder_tabs_bulk_rejects_missing_known_id() {
    let mut state = State::default();
    let ws_id = create_workspace(&mut state, "w");
    let t1 = create_tab(&mut state, &ws_id, "t1");
    let _t2 = create_tab(&mut state, &ws_id, "t2");
    let events = update(
        &mut state,
        Command::ReorderTabsBulk {
            workspace_id: ws_id,
            tab_ids: vec![t1],
        },
        &ctx(99),
    );
    assert!(matches!(&events[0], Event::Error { .. }));
}

/// codex P1 #620 carryover: a duplicate tab_id in the new list
/// is still rejected — that would corrupt the persisted ordering
/// in a way the subscriber can't recover from.
#[test]
fn reorder_tabs_bulk_rejects_duplicates() {
    let mut state = State::default();
    let ws_id = create_workspace(&mut state, "w");
    let t1 = create_tab(&mut state, &ws_id, "t1");
    let events = update(
        &mut state,
        Command::ReorderTabsBulk {
            workspace_id: ws_id,
            tab_ids: vec![t1.clone(), t1.clone()],
        },
        &ctx(2),
    );
    match &events[0] {
        Event::Error { code, message, .. } => {
            assert_eq!(*code, ErrorCode::InvalidCommand);
            assert!(
                message.contains("duplicate"),
                "error should mention duplicate, got: {}",
                message
            );
        }
        other => panic!("expected Error event, got {:?}", other),
    }
}

#[test]
fn reorder_tabs_bulk_validates_workspace() {
    let mut state = State::default();
    let events = update(
        &mut state,
        Command::ReorderTabsBulk {
            workspace_id: "no-such-ws".into(),
            tab_ids: vec!["a".into()],
        },
        &ctx(1),
    );
    assert!(matches!(&events[0], Event::Error { .. }));
}

/// issue #2218 (B.4): TabDeleted/WorkspaceDeleted never emitted a
/// per-block BlockDeleted, so the host had no signal to tear down a
/// browser-pane renderer whose tab/workspace was deleted while it was
/// never live/loaded in a window. `block_ids` on these events is that
/// signal — this test locks the cascade actually carries them.
#[test]
fn delete_tab_emits_cascaded_block_ids() {
    let mut state = State::default();
    let ws_id = create_workspace(&mut state, "w");
    let _ = update(
        &mut state,
        Command::CreateTab { workspace_id: ws_id.clone(), name: "t1".into() },
        &ctx(1),
    );
    let tab1_id = state.workspaces[&ws_id].tab_ids[0].clone();
    // A second tab so the delete isn't a last-tab delete (needs force).
    let _ = update(
        &mut state,
        Command::CreateTab { workspace_id: ws_id.clone(), name: "t2".into() },
        &ctx(2),
    );
    let block1 = match &update(
        &mut state,
        Command::CreateBlock { tab_id: tab1_id.clone(), meta: serde_json::Value::Null },
        &ctx(3),
    )[0] {
        Event::BlockCreated { block_id, .. } => block_id.clone(),
        other => panic!("expected BlockCreated, got {:?}", other),
    };
    let block2 = match &update(
        &mut state,
        Command::CreateBlock { tab_id: tab1_id.clone(), meta: serde_json::Value::Null },
        &ctx(4),
    )[0] {
        Event::BlockCreated { block_id, .. } => block_id.clone(),
        other => panic!("expected BlockCreated, got {:?}", other),
    };
    let events = update(
        &mut state,
        Command::DeleteTab { workspace_id: ws_id, tab_id: tab1_id, force: false },
        &ctx(5),
    );
    match &events[0] {
        Event::TabDeleted { block_ids, .. } => {
            assert_eq!(block_ids.len(), 2);
            assert!(block_ids.contains(&block1));
            assert!(block_ids.contains(&block2));
        }
        other => panic!("expected TabDeleted, got {:?}", other),
    }
}

#[test]
fn delete_tab_with_zero_blocks_emits_empty_block_ids() {
    let mut state = State::default();
    let ws_id = create_workspace(&mut state, "w");
    let _ = update(
        &mut state,
        Command::CreateTab { workspace_id: ws_id.clone(), name: "t1".into() },
        &ctx(1),
    );
    let tab1_id = state.workspaces[&ws_id].tab_ids[0].clone();
    let _ = update(
        &mut state,
        Command::CreateTab { workspace_id: ws_id.clone(), name: "t2".into() },
        &ctx(2),
    );
    let events = update(
        &mut state,
        Command::DeleteTab { workspace_id: ws_id, tab_id: tab1_id, force: false },
        &ctx(3),
    );
    match &events[0] {
        Event::TabDeleted { block_ids, .. } => assert!(block_ids.is_empty()),
        other => panic!("expected TabDeleted, got {:?}", other),
    }
}

/// codex P2 #622: empty name auto-generates `Tab N`, mirroring
/// `wcore::create_tab`'s default-naming behaviour. Without this,
/// CreateWindow's "fresh workspace" path + TearOffBlock's new tab
/// would land with blank titles — a user-visible regression.
#[test]
fn create_tab_auto_generates_tab_n_when_name_empty() {
    let mut state = State::default();
    let ws_id = create_workspace(&mut state, "w");
    let events = update(
        &mut state,
        Command::CreateTab {
            workspace_id: ws_id.clone(),
            name: String::new(),
        },
        &ctx(2),
    );
    match &events[0] {
        Event::TabCreated { name, tab_id, .. } => {
            assert_eq!(name, "Tab 1", "first tab in fresh workspace");
            assert_eq!(state.tabs[tab_id].name, "Tab 1");
        }
        other => panic!("expected TabCreated, got {:?}", other),
    }
    // Second empty-name CreateTab → "Tab 2".
    let events = update(
        &mut state,
        Command::CreateTab {
            workspace_id: ws_id.clone(),
            name: String::new(),
        },
        &ctx(3),
    );
    match &events[0] {
        Event::TabCreated { name, .. } => assert_eq!(name, "Tab 2"),
        other => panic!("expected TabCreated, got {:?}", other),
    }
    // Explicit non-empty name passes through verbatim.
    let events = update(
        &mut state,
        Command::CreateTab {
            workspace_id: ws_id,
            name: "my custom tab".into(),
        },
        &ctx(4),
    );
    match &events[0] {
        Event::TabCreated { name, .. } => assert_eq!(name, "my custom tab"),
        other => panic!("expected TabCreated, got {:?}", other),
    }
}

#[test]
fn delete_tab_cascades_blocks() {
    let mut state = State::default();
    let ws_id = create_workspace(&mut state, "w");
    let tab_id = create_tab(&mut state, &ws_id, "t");
    let _ = update(
        &mut state,
        Command::CreateBlock { tab_id: tab_id.clone(), meta: serde_json::Value::Null },
        &ctx(2),
    );
    let _ = update(
        &mut state,
        Command::CreateBlock { tab_id: tab_id.clone(), meta: serde_json::Value::Null },
        &ctx(3),
    );
    assert_eq!(state.blocks.len(), 2);
    let _ = update(
        &mut state,
        Command::DeleteTab {
            workspace_id: ws_id,
            tab_id,
            // Single-tab workspace; force=true bypasses last-tab
            // guard so we can test the block cascade.
            force: true,
        },
        &ctx(4),
    );
    assert!(state.blocks.is_empty());
}

// ---- Phase E.5.5 — MoveTab tests ----

#[test]
fn move_tab_cross_workspace_updates_lists_and_parent() {
    let mut state = State::default();
    let src = create_workspace(&mut state, "src");
    let dst = create_workspace(&mut state, "dst");
    let t1 = create_tab(&mut state, &src, "t1");
    let t2 = create_tab(&mut state, &src, "t2");
    let dst_existing = create_tab(&mut state, &dst, "existing");
    let events = update(
        &mut state,
        Command::MoveTab {
            tab_id: t1.clone(),
            src_workspace_id: src.clone(),
            dst_workspace_id: dst.clone(),
            dst_index: 0,
            allow_empty_source: false,
        },
        &ctx(99),
    );
    match &events[0] {
        Event::TabMoved {
            tab_id,
            src_workspace_id,
            dst_workspace_id,
            dst_index,
            new_src_active_tab_id,
            ..
        } => {
            assert_eq!(tab_id, &t1);
            assert_eq!(src_workspace_id, &src);
            assert_eq!(dst_workspace_id, &dst);
            assert_eq!(*dst_index, 0);
            assert_eq!(new_src_active_tab_id, &Some(t2.clone()));
        }
        other => panic!("expected TabMoved, got {:?}", other),
    }
    assert_eq!(state.workspaces[&src].tab_ids, vec![t2.clone()]);
    assert_eq!(state.workspaces[&dst].tab_ids, vec![t1.clone(), dst_existing]);
    assert_eq!(state.tabs[&t1].workspace_id, dst);
    assert_eq!(state.workspaces[&src].active_tab_id, Some(t2));
}

#[test]
fn move_tab_clamps_dst_index_to_dst_length() {
    let mut state = State::default();
    let src = create_workspace(&mut state, "src");
    let dst = create_workspace(&mut state, "dst");
    let t1 = create_tab(&mut state, &src, "t1");
    let _ = create_tab(&mut state, &src, "filler");
    let events = update(
        &mut state,
        Command::MoveTab {
            tab_id: t1.clone(),
            src_workspace_id: src,
            dst_workspace_id: dst.clone(),
            dst_index: 999,
            allow_empty_source: false,
        },
        &ctx(2),
    );
    match &events[0] {
        Event::TabMoved { dst_index, .. } => assert_eq!(*dst_index, 0),
        other => panic!("expected TabMoved, got {:?}", other),
    }
    assert_eq!(state.workspaces[&dst].tab_ids, vec![t1]);
}

#[test]
fn move_tab_src_active_clears_when_workspace_empties() {
    let mut state = State::default();
    let src = create_workspace(&mut state, "src");
    let dst = create_workspace(&mut state, "dst");
    let only_tab = create_tab(&mut state, &src, "only");
    let events = update(
        &mut state,
        Command::MoveTab {
            tab_id: only_tab,
            src_workspace_id: src.clone(),
            dst_workspace_id: dst,
            dst_index: 0,
            allow_empty_source: true,
        },
        &ctx(2),
    );
    match &events[0] {
        Event::TabMoved {
            new_src_active_tab_id,
            ..
        } => assert_eq!(new_src_active_tab_id, &None),
        other => panic!("expected TabMoved, got {:?}", other),
    }
    assert_eq!(state.workspaces[&src].active_tab_id, None);
    assert!(state.workspaces[&src].tab_ids.is_empty());
}

#[test]
fn move_tab_rejects_same_workspace() {
    let mut state = State::default();
    let ws = create_workspace(&mut state, "w");
    let t1 = create_tab(&mut state, &ws, "t1");
    let events = update(
        &mut state,
        Command::MoveTab {
            tab_id: t1,
            src_workspace_id: ws.clone(),
            dst_workspace_id: ws,
            dst_index: 0,
            allow_empty_source: false,
        },
        &ctx(2),
    );
    assert!(matches!(&events[0], Event::Error { .. }));
}

#[test]
fn move_tab_rejects_unknown_src_or_dst_or_tab() {
    let mut state = State::default();
    let src = create_workspace(&mut state, "src");
    let dst = create_workspace(&mut state, "dst");
    let t1 = create_tab(&mut state, &src, "t1");

    let events = update(
        &mut state,
        Command::MoveTab {
            tab_id: t1.clone(),
            src_workspace_id: "no-such-src".into(),
            dst_workspace_id: dst.clone(),
            dst_index: 0,
            allow_empty_source: false,
        },
        &ctx(2),
    );
    assert!(matches!(&events[0], Event::Error { .. }));

    let events = update(
        &mut state,
        Command::MoveTab {
            tab_id: t1.clone(),
            src_workspace_id: src.clone(),
            dst_workspace_id: "no-such-dst".into(),
            dst_index: 0,
            allow_empty_source: false,
        },
        &ctx(3),
    );
    assert!(matches!(&events[0], Event::Error { .. }));

    // Phase E.4 strict-mode flip: unknown tabs are now REJECTED.
    // The migration-tolerant lazy-import fallback was removed once
    // the soak window closed without `lazy-import` warnings being
    // observed in production. See `move_tab_unknown_tab_rejects`
    // for the dedicated test.
    let events = update(
        &mut state,
        Command::MoveTab {
            tab_id: "ghost-tab".into(),
            src_workspace_id: src,
            dst_workspace_id: dst,
            dst_index: 0,
            allow_empty_source: false,
        },
        &ctx(4),
    );
    assert!(matches!(&events[0], Event::Error { .. }));
}

/// Phase E.4 strict-mode flip: an unknown tab id (not present in
/// `state.tabs`) is rejected with a clear "tab not found" error
/// rather than being lazy-imported. Replaces the migration-window
/// `move_tab_lazy_imports_unknown_tab` test.
#[test]
fn move_tab_unknown_tab_rejects() {
    let mut state = State::default();
    let src = create_workspace(&mut state, "src");
    let dst = create_workspace(&mut state, "dst");
    let unknown_id = "unknown-tab-xyz".to_string();
    assert!(!state.tabs.contains_key(&unknown_id));
    let events = update(
        &mut state,
        Command::MoveTab {
            tab_id: unknown_id.clone(),
            src_workspace_id: src,
            dst_workspace_id: dst,
            dst_index: 0,
            allow_empty_source: false,
        },
        &ctx(2),
    );
    match &events[0] {
        Event::Error { code, message, .. } => {
            assert_eq!(*code, ErrorCode::InvalidCommand);
            assert!(
                message.contains("tab not found"),
                "error should mention `tab not found`, got: {}",
                message
            );
        }
        other => panic!("expected Error event, got {:?}", other),
    }
    // No lazy import side-effect.
    assert!(!state.tabs.contains_key(&unknown_id));
}

/// Phase E.4 strict-mode flip: a known tab whose reducer-state
/// `workspace_id` doesn't match `src_workspace_id` is rejected.
/// Replaces the migration-window
/// `move_tab_tolerates_workspace_id_mismatch_during_migration`
/// test.
#[test]
fn move_tab_wrong_workspace_rejects() {
    let mut state = State::default();
    let real_src = create_workspace(&mut state, "real_src");
    let dst = create_workspace(&mut state, "dst");
    let other = create_workspace(&mut state, "other");
    let t1 = create_tab(&mut state, &real_src, "t1");
    let filler = create_tab(&mut state, &other, "filler");
    // Claim the tab lives in `other` even though it actually
    // belongs to `real_src` per reducer state.
    let events = update(
        &mut state,
        Command::MoveTab {
            tab_id: t1.clone(),
            src_workspace_id: other.clone(),
            dst_workspace_id: dst.clone(),
            dst_index: 0,
            allow_empty_source: false,
        },
        &ctx(2),
    );
    match &events[0] {
        Event::Error { code, message, .. } => {
            assert_eq!(*code, ErrorCode::InvalidCommand);
            assert!(
                message.contains("workspace_id mismatch"),
                "error should mention `workspace_id mismatch`, got: {}",
                message
            );
        }
        other => panic!("expected Error event, got {:?}", other),
    }
    // Reducer state untouched: t1 still in real_src, filler still
    // in other, dst empty.
    assert_eq!(state.tabs[&t1].workspace_id, real_src);
    assert_eq!(state.workspaces[&real_src].tab_ids, vec![t1]);
    assert_eq!(state.workspaces[&other].tab_ids, vec![filler]);
    assert!(state.workspaces[&dst].tab_ids.is_empty());
}

fn move_tab(state: &mut State, tab: &str, src: &str, dst: &str, allow_empty_source: bool) -> Vec<Event> {
    update(
        state,
        Command::MoveTab {
            tab_id: tab.into(),
            src_workspace_id: src.into(),
            dst_workspace_id: dst.into(),
            dst_index: 0,
            allow_empty_source,
        },
        &ctx(2),
    )
}

#[test]
fn move_tab_refuses_last_tab_without_allow_empty_source() {
    let mut state = State::default();
    let src = create_workspace(&mut state, "src");
    let dst = create_workspace(&mut state, "dst");
    let only = create_tab(&mut state, &src, "only");
    let events = move_tab(&mut state, &only, &src, &dst, false);
    match &events[..] {
        [Event::Error { code, message, .. }] => {
            assert_eq!(*code, ErrorCode::InvalidCommand);
            assert!(message.contains(crate::reducer::MOVE_TAB_LAST_TAB_ERROR), "got: {}", message);
        }
        other => panic!("expected one Error, got {:?}", other),
    }
    assert_eq!(state.workspaces[&src].tab_ids, vec![only.clone()]);
    assert_eq!(state.workspaces[&src].active_tab_id, Some(only.clone()));
    assert!(state.workspaces[&dst].tab_ids.is_empty());
    assert_eq!(state.tabs[&only].workspace_id, src);
}

#[test]
fn move_tab_allow_empty_source_drains_workspace() {
    let mut state = State::default();
    let src = create_workspace(&mut state, "src");
    let dst = create_workspace(&mut state, "dst");
    let only = create_tab(&mut state, &src, "only");
    let events = move_tab(&mut state, &only, &src, &dst, true);
    assert!(matches!(&events[..], [Event::TabMoved { .. }]), "got {:?}", events);
    assert!(state.workspaces[&src].tab_ids.is_empty());
    assert_eq!(state.workspaces[&dst].tab_ids, vec![only]);
}

/// Two moves out of a two-tab workspace, each of which passed a
/// caller-side count of 2: the second must be refused.
#[test]
fn move_tab_two_sequential_moves_leave_one_tab() {
    let mut state = State::default();
    let src = create_workspace(&mut state, "src");
    let dst = create_workspace(&mut state, "dst");
    let t1 = create_tab(&mut state, &src, "t1");
    let t2 = create_tab(&mut state, &src, "t2");
    let first = move_tab(&mut state, &t1, &src, &dst, false);
    assert!(matches!(&first[..], [Event::TabMoved { .. }]), "got {:?}", first);
    let second = move_tab(&mut state, &t2, &src, &dst, false);
    assert!(matches!(&second[..], [Event::Error { .. }]), "got {:?}", second);
    assert_eq!(state.workspaces[&src].tab_ids, vec![t2.clone()]);
    assert_eq!(state.workspaces[&src].active_tab_id, Some(t2));
    assert_eq!(state.workspaces[&dst].tab_ids, vec![t1]);
}

fn set_active(state: &mut State, ws: &str, tab: &str) {
    let events = update(
        state,
        Command::SetActiveTab { workspace_id: ws.into(), tab_id: tab.into() },
        &ctx(1),
    );
    assert!(!matches!(events.first(), Some(Event::Error { .. })), "got {:?}", events);
}

/// Moving the active tab out picks its neighbour, as closing it does, not
/// the workspace's first tab.
#[test]
fn move_active_middle_tab_activates_next_neighbour() {
    let mut state = State::default();
    let src = create_workspace(&mut state, "src");
    let dst = create_workspace(&mut state, "dst");
    let t1 = create_tab(&mut state, &src, "t1");
    let t2 = create_tab(&mut state, &src, "t2");
    let t3 = create_tab(&mut state, &src, "t3");
    set_active(&mut state, &src, &t2);
    let events = move_tab(&mut state, &t2, &src, &dst, false);
    match &events[..] {
        [Event::TabMoved { new_src_active_tab_id, .. }] => {
            assert_eq!(new_src_active_tab_id, &Some(t3.clone()))
        }
        other => panic!("expected TabMoved, got {:?}", other),
    }
    assert_eq!(state.workspaces[&src].tab_ids, vec![t1, t3.clone()]);
    assert_eq!(state.workspaces[&src].active_tab_id, Some(t3));
}

#[test]
fn move_active_last_tab_activates_previous_neighbour() {
    let mut state = State::default();
    let src = create_workspace(&mut state, "src");
    let dst = create_workspace(&mut state, "dst");
    let _t1 = create_tab(&mut state, &src, "t1");
    let t2 = create_tab(&mut state, &src, "t2");
    let t3 = create_tab(&mut state, &src, "t3");
    set_active(&mut state, &src, &t3);
    move_tab(&mut state, &t3, &src, &dst, false);
    assert_eq!(state.workspaces[&src].active_tab_id, Some(t2));
}

/// Moving and closing the same active tab leave the same tab active.
#[test]
fn move_and_delete_of_active_tab_pick_the_same_neighbour() {
    for active_idx in 0..3 {
        let build = || {
            let mut state = State::default();
            let src = create_workspace(&mut state, "src");
            let dst = create_workspace(&mut state, "dst");
            let tabs: Vec<String> = (0..3).map(|i| create_tab(&mut state, &src, &format!("t{i}"))).collect();
            set_active(&mut state, &src, &tabs[active_idx]);
            (state, src, dst, tabs)
        };
        let (mut moved, src, dst, tabs) = build();
        move_tab(&mut moved, &tabs[active_idx], &src, &dst, false);
        let (mut deleted, src2, _, tabs2) = build();
        update(
            &mut deleted,
            Command::DeleteTab { workspace_id: src2.clone(), tab_id: tabs2[active_idx].clone(), force: false },
            &ctx(1),
        );
        let pos_of = |state: &State, ws: &str, tabs: &[String]| {
            let active = state.workspaces[ws].active_tab_id.clone().expect("an active tab");
            tabs.iter().position(|t| *t == active)
        };
        assert_eq!(pos_of(&moved, &src, &tabs), pos_of(&deleted, &src2, &tabs2), "active index {active_idx}");
    }
}

#[test]
fn move_inactive_tab_keeps_source_active_tab() {
    let mut state = State::default();
    let src = create_workspace(&mut state, "src");
    let dst = create_workspace(&mut state, "dst");
    let t1 = create_tab(&mut state, &src, "t1");
    let t2 = create_tab(&mut state, &src, "t2");
    move_tab(&mut state, &t2, &src, &dst, false);
    assert_eq!(state.workspaces[&src].active_tab_id, Some(t1));
}

#[test]
fn create_tab_takes_its_id_from_ctx() {
    let mut state = State::default();
    let ws = create_workspace(&mut state, "w");
    let events = update(
        &mut state,
        Command::CreateTab { workspace_id: ws.clone(), name: "t".into() },
        &ctx_with_id("tab-fixed"),
    );
    assert!(matches!(&events[0], Event::TabCreated { tab_id, .. } if tab_id == "tab-fixed"));
    assert_eq!(state.workspaces[&ws].tab_ids, vec!["tab-fixed".to_string()]);
    assert_eq!(state.tabs["tab-fixed"].workspace_id, ws);
}

#[test]
fn create_tab_refuses_an_id_in_use() {
    let mut state = State::default();
    let ws = create_workspace(&mut state, "w");
    let cmd = || Command::CreateTab { workspace_id: ws.clone(), name: "t".into() };
    update(&mut state, cmd(), &ctx_with_id("dup"));
    let events = update(&mut state, cmd(), &ctx_with_id("dup"));
    assert!(matches!(&events[..], [Event::Error { message, .. }] if message.contains("already in use")));
    assert_eq!(state.workspaces[&ws].tab_ids, vec!["dup".to_string()]);
    assert_eq!(state.tabs.len(), 1);
}

#[test]
fn rename_tab_renames_and_emits_event() {
    let mut state = State::default();
    let ws = create_workspace(&mut state, "w");
    let tab = create_tab(&mut state, &ws, "old");
    let events = update(&mut state, Command::RenameTab { tab_id: tab.clone(), name: "new".into() }, &ctx(1));
    assert!(matches!(
        &events[..],
        [Event::TabRenamed { tab_id, name, .. }] if *tab_id == tab && name == "new"
    ));
    assert_eq!(state.tabs[&tab].name, "new");
}

#[test]
fn rename_tab_to_same_name_is_silent_no_op() {
    let mut state = State::default();
    let ws = create_workspace(&mut state, "w");
    let tab = create_tab(&mut state, &ws, "same");
    let version = state.event_version;
    let events = update(&mut state, Command::RenameTab { tab_id: tab, name: "same".into() }, &ctx(1));
    assert!(events.is_empty());
    assert_eq!(state.event_version, version);
}

#[test]
fn rename_unknown_tab_errors() {
    let mut state = State::default();
    let events = update(&mut state, Command::RenameTab { tab_id: "ghost".into(), name: "n".into() }, &ctx(1));
    assert!(matches!(
        &events[..],
        [Event::Error { code: ErrorCode::InvalidCommand, fatal: false, message, .. }] if message.contains("RenameTab")
    ));
}

#[test]
fn update_tab_meta_passes_patch_through_without_touching_state() {
    let mut state = State::default();
    let ws = create_workspace(&mut state, "w");
    let tab = create_tab(&mut state, &ws, "t");
    let before = state.tabs[&tab].clone();
    let patch = serde_json::json!({ "tab:color": "red" });
    let events = update(
        &mut state,
        Command::UpdateTabMeta { tab_id: tab.clone(), meta_patch: patch.clone() },
        &ctx(1),
    );
    assert!(matches!(
        &events[..],
        [Event::TabMetaUpdated { tab_id, meta_patch, .. }] if *tab_id == tab && *meta_patch == patch
    ));
    assert_eq!(state.tabs[&tab], before);
}

#[test]
fn update_meta_of_unknown_tab_errors() {
    let mut state = State::default();
    let events = update(
        &mut state,
        Command::UpdateTabMeta { tab_id: "ghost".into(), meta_patch: serde_json::json!({}) },
        &ctx(1),
    );
    assert!(matches!(&events[..], [Event::Error { message, .. }] if message.contains("UpdateTabMeta")));
}
