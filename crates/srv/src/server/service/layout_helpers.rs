// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Layout-tree + pending-action writers used by the tear-off / promote /
//! redock handlers. SPEC_864 Phase 3/4 — all of them route through the
//! reducer (`seed_layout_via_reducer` / `queue_layout_actions_via_reducer`);
//! nothing here writes mstore directly anymore.
//! `setup_torn_off_block_layout` is re-exported crate-wide (the pane
//! app-API uses it).

use crate::backend::obj::*;
use agentmux_common::SplitPosition;

use super::layout_intents::Placement;

/// Phase E.5.5 — set up the layout tree for a tab that just received
/// its first block via the TearOffBlock saga. Called from the
/// TearOffBlock / PromoteBlockToTab handlers and the floating
/// `pane.open` path after the saga's reducer-state portion
/// (CreateTab + MoveBlock) completes. (NOT RedockFloatingPane — that
/// redocks into an EXISTING tab and goes through
/// `queue_target_layout_insert`/`_split` instead; reagent P2 on #1971.)
///
/// SPEC_864 Phase 3 — reducer-routed: dispatches `LayoutSetTree` via
/// `seed_layout_via_reducer` (single writer of `db_layout`; the reducer's
/// `TabRecord.rootnode` stays authoritative) instead of the retired
/// wcore-direct `rootnode`/`leaforder` write. Every caller runs post-saga,
/// so the tab is always reducer-known. Best-effort at the call sites: a
/// failure leaves the new tab with the moved block but a malformed layout;
/// the user-visible symptom is an empty render in the new window.
pub(crate) async fn setup_torn_off_block_layout(
    state: &super::super::AppState,
    new_tab_id: &str,
    block_id: &str,
) -> Result<(), String> {
    let node_id = uuid::Uuid::new_v4().to_string();
    // Phase E.4.B Phase 2 — construct typed LayoutNode (was inline JSON).
    let rootnode = LayoutNode {
        id: node_id.clone(),
        flex_direction: FlexDirection::Row,
        size: 1.0,
        children: Vec::new(),
        data: Some(LayoutNodeData {
            block_id: block_id.to_string(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let leaforder = vec![LeafOrderEntry {
        nodeid: node_id,
        blockid: block_id.to_string(),
    }];
    // Focused id stays empty — the legacy writer never set it for a
    // torn-off tab; the frontend focuses on load.
    super::reducer_helpers::seed_layout_via_reducer(
        state,
        new_tab_id,
        rootnode,
        String::new(),
        leaforder,
        String::new(),
    )
    .await
}

/// Place a block (re-docked, re-placed by the orphan reaper) at the first
/// free slot of its tab: srv's tree first, then the queued `insert` for the
/// window showing the tab (`layout_intents::place_block`).
pub(crate) async fn queue_target_layout_insert(
    state: &super::super::AppState,
    target_tab_id: &str,
    block_id: &str,
) -> Result<(), String> {
    super::layout_intents::place_block(state, target_tab_id, block_id, Placement::Append { focused: true }).await
}

/// Tell the frontend that `block_id` was added — already, in the reducer — as
/// the visible tab of the pane holding `target_block_id`
/// (`Command::CreateBlockInStack`). Matches
/// `LayoutTreeActionType.StackPush = "stackpush"` in
/// `frontend/layout/lib/types.ts`. SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.3.
pub(crate) async fn queue_target_stack_push(
    state: &super::super::AppState,
    tab_id: &str,
    block_id: &str,
    target_block_id: &str,
) -> Result<(), String> {
    let action = LayoutActionData {
        actiontype: "stackpush".to_string(),
        actionid: uuid::Uuid::new_v4().to_string(),
        blockid: block_id.to_string(),
        nodesize: None,
        nodesizefraction: None,
        indexarr: None,
        focused: true,
        magnified: false,
        ephemeral: false,
        targetblockid: target_block_id.to_string(),
        position: String::new(),
        nodeid: None,
    };
    super::reducer_helpers::queue_layout_actions_via_reducer(state, tab_id, vec![action]).await
}

/// Tell the frontend that `block_id` was moved — already, in the reducer —
/// to `position` relative to `target_block_id` (`Command::LayoutStackMove`):
/// a same-pane reorder when both are already in one leaf's stack, otherwise
/// a cross-pane move. Matches `LayoutTreeActionType.StackMove = "stackmove"`
/// in `frontend/layout/lib/types.ts`. `position` is the same
/// `"before"`/`"after"`/`"end"` string `pane.moveTab`'s caller supplied —
/// reused verbatim, not re-derived, so the frontend applies the identical
/// operation rather than inferring it from resulting tree state. `activate`
/// rides on `LayoutActionData.focused` — the same field `queue_target_stack_push`
/// above already overloads for this purpose (that call hardcodes `true`,
/// since `CreateBlockInStack` always activates; this one carries the
/// caller's actual `activate` value since a reorder may or may not).
/// SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §4.1, Phase 3.
pub(crate) async fn queue_target_stack_move(
    state: &super::super::AppState,
    tab_id: &str,
    block_id: &str,
    target_block_id: &str,
    position: &str,
    activate: bool,
    action_id: &str,
) -> Result<(), String> {
    let action = LayoutActionData {
        actiontype: "stackmove".to_string(),
        actionid: action_id_or_new(action_id),
        blockid: block_id.to_string(),
        nodesize: None,
        nodesizefraction: None,
        indexarr: None,
        focused: activate,
        magnified: false,
        ephemeral: false,
        targetblockid: target_block_id.to_string(),
        position: position.to_string(),
        nodeid: None,
    };
    super::reducer_helpers::queue_layout_actions_via_reducer(state, tab_id, vec![action]).await
}

/// Tell the frontend that `block_id` became the visible tab of its pane
/// (`Command::LayoutStackActivate`). Matches
/// `LayoutTreeActionType.StackActivate = "stackactivate"`.
pub(crate) async fn queue_stack_activate(
    state: &super::super::AppState,
    tab_id: &str,
    block_id: &str,
    action_id: &str,
) -> Result<(), String> {
    let action = LayoutActionData {
        actiontype: "stackactivate".to_string(),
        actionid: action_id_or_new(action_id),
        ..source_delete_action(block_id)
    };
    super::reducer_helpers::queue_layout_actions_via_reducer(state, tab_id, vec![action]).await
}

/// Tell the frontend that `block_id` left its pane (one tab of a stack, or the
/// pane with its last tab): the `delete` action, under the caller's id.
pub(crate) async fn queue_stack_remove(
    state: &super::super::AppState,
    tab_id: &str,
    block_id: &str,
    action_id: &str,
) -> Result<(), String> {
    let action = LayoutActionData { actionid: action_id_or_new(action_id), ..source_delete_action(block_id) };
    super::reducer_helpers::queue_layout_actions_via_reducer(state, tab_id, vec![action]).await
}

/// The id a frontend minted for its own command, so that frontend skips the
/// queued copy (`processedActionIds`, layoutPersistence.ts); a fresh one when
/// it sent none or an unusable one.
fn action_id_or_new(action_id: &str) -> String {
    if action_id.is_empty() || action_id.len() > 64 {
        uuid::Uuid::new_v4().to_string()
    } else {
        action_id.to_string()
    }
}

/// Phase 4b/4c — place a redocked block in the exact slot the ghost
/// overlay previewed: a split of the pane holding `target_block_id`.
///
/// `dir` maps the `DropDirection` enum values to splits:
/// * 0/4 (Top/OuterTop)    → vertical,   before
/// * 2/6 (Bottom/OuterBot) → vertical,   after
/// * 3/7 (Left/OuterLeft)  → horizontal, before
/// * 1/5 (Right/OuterRight)→ horizontal, after
/// * 8   (Center)          → the first free slot instead
///
/// The new pane takes a fraction of the target's current size: 0.5 for an
/// inner direction (the ghost's half-leaf), 0.2 for an outer one (its exact
/// `leaf/5` band) — see `ANALYSIS_FLOATING_PANE_GHOST_LANDING_DISCONNECT_2026_07_04.md`.
pub(super) async fn queue_target_layout_split(
    state: &super::super::AppState,
    target_tab_id: &str,
    block_id: &str,
    target_block_id: &str,
    dir: u8,
) -> Result<(), String> {
    const INNER_FRACTION: f64 = 0.5;
    const OUTER_FRACTION: f64 = 0.2;
    let (horizontal, position, fraction) = match dir {
        0 => (false, SplitPosition::Before, INNER_FRACTION),
        4 => (false, SplitPosition::Before, OUTER_FRACTION),
        2 => (false, SplitPosition::After, INNER_FRACTION),
        6 => (false, SplitPosition::After, OUTER_FRACTION),
        3 => (true, SplitPosition::Before, INNER_FRACTION),
        7 => (true, SplitPosition::Before, OUTER_FRACTION),
        1 => (true, SplitPosition::After, INNER_FRACTION),
        5 => (true, SplitPosition::After, OUTER_FRACTION),
        _ => return queue_target_layout_insert(state, target_tab_id, block_id).await,
    };
    let placement = Placement::Split {
        target_block_id: target_block_id.to_string(),
        horizontal,
        position,
        size_fraction: Some(fraction),
        focused: true,
    };
    super::layout_intents::place_block(state, target_tab_id, block_id, placement).await
}

/// Take a block that moved (tear-off, promote, redock) or was deleted
/// (`sagas::delete_block`, `sagas::close_pane`) out of its source tab's
/// layout: srv's tree, then a queued `delete` so a window that already shows
/// the tab drops it too (a window that missed it would show a pane whose block
/// is gone; `INVESTIGATION_LAYOUT_DEAD_SPACE_STALE_TREE_RESURRECTION_2026_07_08.md`).
pub(crate) async fn queue_source_layout_delete(
    state: &super::super::AppState,
    source_tab_id: &str,
    block_id: &str,
) -> Result<(), String> {
    // srv's own tree first (a no-op when a caller already took it out), so
    // the window's mirror of the `delete` is never the only copy of it.
    super::layout_intents::remove_block_from_layout(state, source_tab_id, block_id).await?;
    super::reducer_helpers::queue_layout_actions_via_reducer(state, source_tab_id, vec![source_delete_action(block_id)])
        .await
}

/// The frontend `delete` action for one block: its handler removes just that
/// stack member (the leaf goes with the last one) and never runs `closeNode`.
pub(crate) fn source_delete_action(block_id: &str) -> LayoutActionData {
    LayoutActionData {
        actiontype: "delete".to_string(),
        actionid: uuid::Uuid::new_v4().to_string(),
        blockid: block_id.to_string(),
        nodesize: None,
        nodesizefraction: None,
        indexarr: None,
        focused: false,
        magnified: false,
        ephemeral: false,
        targetblockid: String::new(),
        position: String::new(),
        nodeid: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::tests::test_state;
    use agentmux_common::ipc::{Command, Event};

    async fn dispatch_apply(state: &crate::server::AppState, cmd: Command) -> Vec<Event> {
        let events = super::super::reducer_helpers::dispatch_to_reducer(state, cmd).await;
        for ev in &events {
            crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
        }
        events
    }

    /// A reducer-known tab (the queue helpers dispatch through the reducer,
    /// which rejects unknown tabs) holding one pane of block `target`.
    /// Returns `(tab_id, target)`.
    async fn tab_with_one_pane(state: &crate::server::AppState) -> (String, String) {
        let ws = dispatch_apply(state, Command::CreateWorkspace { name: "ws".into() }).await;
        let Some(Event::WorkspaceCreated { workspace_id, .. }) = ws.first() else { panic!("{ws:?}") };
        let tab = dispatch_apply(state, Command::CreateTab { workspace_id: workspace_id.clone(), name: "t1".into() }).await;
        let tab_id = tab
            .iter()
            .find_map(|e| match e {
                Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            })
            .unwrap();
        let target = new_block(state, &tab_id).await;
        super::super::layout_intents::place_block(state, &tab_id, &target, Placement::Append { focused: false })
            .await
            .unwrap();
        (tab_id, target)
    }

    async fn new_block(state: &crate::server::AppState, tab_id: &str) -> String {
        let ev = dispatch_apply(state, Command::CreateBlock { tab_id: tab_id.into(), meta: serde_json::Value::Null }).await;
        ev.iter()
            .find_map(|e| match e {
                Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
                _ => None,
            })
            .unwrap()
    }

    fn pending_actions(state: &crate::server::AppState, tab_id: &str) -> Vec<LayoutActionData> {
        let tab = state.mstore.must_get::<Tab>(tab_id).unwrap();
        let layout = state.mstore.must_get::<LayoutState>(&tab.layoutstate).unwrap();
        layout.pendingbackendactions.unwrap_or_default()
    }

    fn last_action(state: &crate::server::AppState, tab_id: &str) -> LayoutActionData {
        pending_actions(state, tab_id).last().expect("an action was queued").clone()
    }

    async fn stored_tree(state: &crate::server::AppState, tab_id: &str) -> agentmux_common::LayoutNode {
        let tab = state.mstore.must_get::<Tab>(tab_id).unwrap();
        let row = state.mstore.must_get::<LayoutState>(&tab.layoutstate).unwrap();
        assert_eq!(row.rootnode, state.srv_state.lock().await.tabs[tab_id].rootnode, "the row is the reducer tree");
        row.rootnode.expect("a tree")
    }

    #[tokio::test]
    async fn a_split_is_placed_in_srv_and_queued_with_its_node_id_and_fraction() {
        // Phase 4c: every non-Center direction carries `nodesizefraction`, the
        // exact share of the target's live size; see
        // ANALYSIS_FLOATING_PANE_GHOST_LANDING_DISCONNECT_2026_07_04.md.
        let cases: &[(u8, &str, &str, f64)] = &[
            (0, "splitvertical", "before", 0.5),
            (4, "splitvertical", "before", 0.2),
            (2, "splitvertical", "after", 0.5),
            (6, "splitvertical", "after", 0.2),
            (3, "splithorizontal", "before", 0.5),
            (7, "splithorizontal", "before", 0.2),
            (1, "splithorizontal", "after", 0.5),
            (5, "splithorizontal", "after", 0.2),
        ];
        for &(dir, expected_type, expected_pos, expected_fraction) in cases {
            let state = test_state();
            let (tab_id, target) = tab_with_one_pane(&state).await;
            let block = new_block(&state, &tab_id).await;
            queue_target_layout_split(&state, &tab_id, &block, &target, dir).await.unwrap();
            let action = last_action(&state, &tab_id);
            assert_eq!(action.actiontype, expected_type, "dir={dir}");
            assert_eq!(action.position, expected_pos, "dir={dir}");
            assert_eq!(action.nodesizefraction, Some(expected_fraction), "dir={dir}");
            assert_eq!(action.targetblockid, target);
            let tree = stored_tree(&state, &tab_id).await;
            let leaf = crate::backend::layout::find_leaf_containing_block(&tree, &block).expect("placed in srv");
            assert_eq!(action.nodeid.as_deref(), Some(leaf.id.as_str()), "dir={dir}");
            assert!((leaf.size - 10.0 * expected_fraction as f32).abs() < 1e-4, "dir={dir}: size {}", leaf.size);
        }
    }

    #[tokio::test]
    async fn center_unknown_dirs_and_a_missing_target_fall_back_to_an_insert() {
        let state = test_state();
        let (tab_id, target) = tab_with_one_pane(&state).await;
        for (dir, against) in [(8u8, target.as_str()), (200u8, target.as_str()), (1u8, "no-such-block")] {
            let block = new_block(&state, &tab_id).await;
            queue_target_layout_split(&state, &tab_id, &block, against, dir).await.unwrap();
            let action = last_action(&state, &tab_id);
            assert_eq!(action.actiontype, "insert", "dir={dir}");
            assert_eq!(action.nodesizefraction, None, "dir={dir}");
            let tree = stored_tree(&state, &tab_id).await;
            assert!(crate::backend::layout::find_leaf_containing_block(&tree, &block).is_some(), "dir={dir}");
        }
    }

    #[tokio::test]
    async fn a_source_delete_takes_the_block_out_of_srv_first() {
        let state = test_state();
        let (tab_id, target) = tab_with_one_pane(&state).await;
        let other = new_block(&state, &tab_id).await;
        queue_target_layout_insert(&state, &tab_id, &other).await.unwrap();
        queue_source_layout_delete(&state, &tab_id, &other).await.unwrap();
        let tree = stored_tree(&state, &tab_id).await;
        assert!(crate::backend::layout::find_leaf_containing_block(&tree, &other).is_none());
        assert!(crate::backend::layout::find_leaf_containing_block(&tree, &target).is_some());
        assert_eq!(last_action(&state, &tab_id).actiontype, "delete");
        // Again, once it is gone: srv has nothing left to do, the window still hears.
        queue_source_layout_delete(&state, &tab_id, &other).await.unwrap();
        assert_eq!(pending_actions(&state, &tab_id).iter().filter(|a| a.actiontype == "delete").count(), 2);
    }

    #[tokio::test]
    async fn queue_helpers_error_on_reducer_unknown_tab() {
        let state = test_state();
        let err = queue_source_layout_delete(&state, "ghost-tab", "b1")
            .await
            .expect_err("unknown tab must error");
        assert!(err.contains("unknown tab"), "got: {err}");
    }
}
