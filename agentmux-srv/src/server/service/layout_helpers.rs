// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Layout-tree + pending-action writers used by the tear-off / promote /
//! redock handlers. SPEC_864 Phase 3/4 — all of them route through the
//! reducer (`seed_layout_via_reducer` / `queue_layout_actions_via_reducer`);
//! nothing here writes wstore directly anymore.
//! `setup_torn_off_block_layout` is re-exported crate-wide (the pane
//! app-API uses it).

use crate::backend::obj::*;

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

/// Floating-pane re-dock — enqueue an "insert" action on the TARGET
/// tab's `LayoutState.pendingbackendactions` so the target window's
/// frontend adds a new leaf for the redocked block through its
/// `LayoutTreeActionType.InsertNode` reducer pathway.
///
/// Why this and not direct rootnode/leaforder writes? The frontend's
/// LayoutModel maintains its own in-memory tree state and doesn't
/// auto-sync from external `LayoutState` WaveObj updates — so a
/// backend `store.update` to the rootnode lands in the WOS cache
/// but the LayoutModel never picks it up, and the next frontend-
/// initiated `object.UpdateObject` overwrites the backend version
/// with the LayoutModel's stale tree. The pending-actions queue
/// (`onBackendUpdate` in `layoutPersistence.ts:50`) is the canonical
/// channel for "backend wants the frontend to mutate its layout
/// tree". Source-delete on tear-off uses the same channel via
/// `queue_source_layout_delete`.
///
/// SPEC_864 Phase 4 — appends through the reducer
/// (`queue_layout_actions_via_reducer`), not `store.update`.
pub(super) async fn queue_target_layout_insert(
    state: &super::super::AppState,
    target_tab_id: &str,
    block_id: &str,
) -> Result<(), String> {
    let action = LayoutActionData {
        // Matches `LayoutTreeActionType.InsertNode = "insert"` in
        // `frontend/layout/lib/types.ts:73`.
        actiontype: "insert".to_string(),
        actionid: uuid::Uuid::new_v4().to_string(),
        blockid: block_id.to_string(),
        nodesize: None,
        nodesizefraction: None,
        indexarr: None,
        focused: true,
        magnified: false,
        ephemeral: false,
        targetblockid: String::new(),
        position: String::new(),
    };
    super::reducer_helpers::queue_layout_actions_via_reducer(state, target_tab_id, vec![action])
        .await
}

/// Phase 4b/4c — enqueue a directional split action on the TARGET tab's
/// `LayoutState.pendingbackendactions` so the redocked block lands in
/// the exact slot the ghost overlay previewed.
///
/// `dir` maps the `DropDirection` enum values to action types:
/// * 0/4 (Top/OuterTop)    → `SplitVertical`,   position `before`
/// * 2/6 (Bottom/OuterBot) → `SplitVertical`,   position `after`
/// * 3/7 (Left/OuterLeft)  → `SplitHorizontal`, position `before`
/// * 1/5 (Right/OuterRight)→ `SplitHorizontal`, position `after`
/// * 8   (Center)          → falls through to `InsertNode` (handled by caller)
///
/// Phase 4c: rather than an absolute `nodesize` guess (which was only correct
/// when the target happened to be at `DefaultNodeSize`), this sends
/// `nodesizefraction` — the new node's share of the target's CURRENT size —
/// which the frontend applies against the target's live size at split time
/// (`layoutTree.ts`). Inner directions use 0.5 (50/50, matching the ghost's
/// half-leaf); outer directions use 0.2 (matching the ghost's exact `leaf/5`
/// band — see `ANALYSIS_FLOATING_PANE_GHOST_LANDING_DISCONNECT_2026_07_04.md`).
pub(super) async fn queue_target_layout_split(
    state: &super::super::AppState,
    target_tab_id: &str,
    block_id: &str,
    target_block_id: &str,
    dir: u8,
) -> Result<(), String> {
    const INNER_FRACTION: f64 = 0.5;
    const OUTER_FRACTION: f64 = 0.2;
    let (actiontype, position, nodesizefraction): (&str, &str, f64) = match dir {
        0 => ("splitvertical",   "before", INNER_FRACTION),
        4 => ("splitvertical",   "before", OUTER_FRACTION),
        2 => ("splitvertical",   "after",  INNER_FRACTION),
        6 => ("splitvertical",   "after",  OUTER_FRACTION),
        3 => ("splithorizontal", "before", INNER_FRACTION),
        7 => ("splithorizontal", "before", OUTER_FRACTION),
        1 => ("splithorizontal", "after",  INNER_FRACTION),
        5 => ("splithorizontal", "after",  OUTER_FRACTION),
        // Center (8) or unknown — caller should use queue_target_layout_insert.
        _ => return queue_target_layout_insert(state, target_tab_id, block_id).await,
    };
    let action = LayoutActionData {
        actiontype: actiontype.to_string(),
        actionid: uuid::Uuid::new_v4().to_string(),
        blockid: block_id.to_string(),
        nodesize: None,
        nodesizefraction: Some(nodesizefraction),
        indexarr: None,
        focused: true,
        magnified: false,
        ephemeral: false,
        targetblockid: target_block_id.to_string(),
        position: position.to_string(),
    };
    super::reducer_helpers::queue_layout_actions_via_reducer(state, target_tab_id, vec![action])
        .await
}

/// Phase E.5.5 — append a layout-delete action to the source tab's
/// `LayoutState.pendingbackendactions` so the source window's
/// frontend tears the moved block out of its layout tree on next
/// poll. Used by the TearOffBlock/PromoteBlockToTab/RedockFloatingPane
/// RPC handlers (the wcore-direct `tear_off_block` this used to mirror
/// was dead code, deleted in SPEC_864 Phase 5).
///
/// Also used by `sagas::delete_block` (`pub(crate)`, not `pub(super)`,
/// for that reason) — a direct `LayoutDeleteNodeByBlock` reducer
/// dispatch prunes `db_layout` correctly but, per this function's own
/// "why this and not direct writes" doc above, does nothing to tell a
/// frontend that already has the tab's tree loaded. Without this call
/// too, that frontend's next unrelated edit in the tab re-pushes its
/// stale copy — still containing the just-deleted block's leaf — and
/// silently resurrects it as a dangling reference (empty/dead pane;
/// see `INVESTIGATION_LAYOUT_DEAD_SPACE_STALE_TREE_RESURRECTION_2026_07_08.md`).
pub(crate) async fn queue_source_layout_delete(
    state: &super::super::AppState,
    source_tab_id: &str,
    block_id: &str,
) -> Result<(), String> {
    let action = LayoutActionData {
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
    };
    super::reducer_helpers::queue_layout_actions_via_reducer(state, source_tab_id, vec![action])
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::tests::test_state;
    use agentmux_common::ipc::{Command, Event};

    /// SPEC_864 Phase 4 — the queue helpers dispatch through the reducer,
    /// which rejects unknown tabs, so tests must create the tab via
    /// reducer commands (same pattern as
    /// `layout_seeders_route_through_reducer_coherently` in server tests)
    /// rather than reading the wcore-seeded tab.
    async fn reducer_known_tab(state: &crate::server::AppState) -> String {
        async fn dispatch_apply(state: &crate::server::AppState, cmd: Command) -> Vec<Event> {
            let events = super::super::reducer_helpers::dispatch_to_reducer(state, cmd).await;
            for ev in &events {
                crate::persist_subscriber::apply_event_to_wstore(ev, &state.wstore).unwrap();
            }
            events
        }
        let ws_evs = dispatch_apply(state, Command::CreateWorkspace { name: "ws".into() }).await;
        let ws_id = ws_evs
            .iter()
            .find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            })
            .unwrap();
        let tab_evs = dispatch_apply(
            state,
            Command::CreateTab {
                workspace_id: ws_id,
                name: "t1".into(),
            },
        )
        .await;
        tab_evs
            .iter()
            .find_map(|e| match e {
                Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            })
            .unwrap()
    }

    fn pending_actions(state: &crate::server::AppState, tab_id: &str) -> Vec<LayoutActionData> {
        let tab = state.wstore.must_get::<Tab>(tab_id).unwrap();
        let layout = state.wstore.must_get::<LayoutState>(&tab.layoutstate).unwrap();
        layout.pendingbackendactions.unwrap_or_default()
    }

    fn last_action(state: &crate::server::AppState, tab_id: &str) -> LayoutActionData {
        pending_actions(state, tab_id)
            .last()
            .expect("an action was queued")
            .clone()
    }

    #[tokio::test]
    async fn queue_target_layout_split_maps_every_direction_to_an_exact_fraction() {
        // Phase 4c: every non-Center direction must carry `nodesizefraction`
        // (not the old hardcoded-integer `nodesize` guess) so the frontend
        // can derive an exact size from the target's live size. See
        // ANALYSIS_FLOATING_PANE_GHOST_LANDING_DISCONNECT_2026_07_04.md.
        let state = test_state();
        let tab_id = reducer_known_tab(&state).await;

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
            queue_target_layout_split(&state, &tab_id, "new-block", "target-block", dir)
                .await
                .unwrap();
            let action = last_action(&state, &tab_id);
            assert_eq!(action.actiontype, expected_type, "dir={dir}");
            assert_eq!(action.position, expected_pos, "dir={dir}");
            assert_eq!(action.nodesizefraction, Some(expected_fraction), "dir={dir}");
            assert_eq!(action.nodesize, None, "dir={dir}: absolute nodesize is no longer used for splits");
            assert_eq!(action.targetblockid, "target-block");
        }
        // 8 splits queued → 8 actions APPENDED (not replaced).
        assert_eq!(pending_actions(&state, &tab_id).len(), cases.len());
    }

    #[tokio::test]
    async fn queue_target_layout_split_falls_back_to_insert_for_center_and_unknown_dirs() {
        let state = test_state();
        let tab_id = reducer_known_tab(&state).await;

        for dir in [8u8, 200u8] {
            queue_target_layout_split(&state, &tab_id, "new-block", "target-block", dir)
                .await
                .unwrap();
            let action = last_action(&state, &tab_id);
            assert_eq!(action.actiontype, "insert", "dir={dir}");
            assert_eq!(action.nodesizefraction, None, "dir={dir}");
        }
    }

    /// SPEC_864 Phase 4 — queue writers hit the reducer's unknown-tab
    /// validation instead of silently writing wstore.
    #[tokio::test]
    async fn queue_helpers_error_on_reducer_unknown_tab() {
        let state = test_state();
        let err = queue_source_layout_delete(&state, "ghost-tab", "b1")
            .await
            .expect_err("unknown tab must error");
        assert!(err.contains("unknown tab"), "got: {err}");
    }
}
