// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Layout-tree + pending-action writers used by the tear-off / promote /
//! redock handlers. Layout state migration is E.4 territory; until then these
//! write straight to wstore. `setup_torn_off_block_layout` is re-exported
//! crate-wide (the pane app-API uses it).

use crate::backend::obj::*;
use crate::backend::storage::store::Store;

/// Phase E.5.5 — set up the layout tree for a tab that just received
/// its first block via the TearOffBlock saga. Called from the
/// TearOffBlock RPC handler after the saga's reducer-state portion
/// (CreateTab + MoveBlock) completes. Mirrors the layout-rootnode
/// + leaforder construction that `wcore::tear_off_block` previously
/// embedded in its single function.
///
/// Layout state migration is E.4 — until then layout writes are
/// wcore-direct and not reducer-routed. Best-effort: a failure here
/// leaves the new tab with the moved block but a malformed layout;
/// the user-visible symptom is an empty render in the new window.
pub(crate) fn setup_torn_off_block_layout(
    store: &Store,
    new_tab_id: &str,
    block_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let new_tab = store.must_get::<Tab>(new_tab_id)?;
    let mut layout = store.must_get::<LayoutState>(&new_tab.layoutstate)?;
    let node_id = uuid::Uuid::new_v4().to_string();
    // Phase E.4.B Phase 2 — construct typed LayoutNode (was inline JSON).
    layout.rootnode = Some(LayoutNode {
        id: node_id.clone(),
        flex_direction: FlexDirection::Row,
        size: 1.0,
        children: Vec::new(),
        data: Some(LayoutNodeData {
            block_id: block_id.to_string(),
            ..Default::default()
        }),
        ..Default::default()
    });
    layout.leaforder = Some(vec![LeafOrderEntry {
        nodeid: node_id,
        blockid: block_id.to_string(),
    }]);
    store.update(&mut layout)?;
    Ok(())
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
pub(super) fn queue_target_layout_insert(
    store: &Store,
    target_tab_id: &str,
    block_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let target_tab = store.must_get::<Tab>(target_tab_id)?;
    let mut target_layout = store.must_get::<LayoutState>(&target_tab.layoutstate)?;
    let mut actions = target_layout.pendingbackendactions.take().unwrap_or_default();
    actions.push(LayoutActionData {
        // Matches `LayoutTreeActionType.InsertNode = "insert"` in
        // `frontend/layout/lib/types.ts:73`.
        actiontype: "insert".to_string(),
        actionid: uuid::Uuid::new_v4().to_string(),
        blockid: block_id.to_string(),
        nodesize: None,
        indexarr: None,
        focused: true,
        magnified: false,
        ephemeral: false,
        targetblockid: String::new(),
        position: String::new(),
    });
    target_layout.pendingbackendactions = Some(actions);
    store.update(&mut target_layout)?;
    Ok(())
}

/// Phase 4b — enqueue a directional split action on the TARGET tab's
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
/// Outer directions use `nodesize = Some(3)` so the new node occupies ≈23%
/// (3 / 13) — the nearest representable integer to the ghost's `height/5`
/// (20%) when the target node is at DefaultNodeSize (10).
/// Inner directions use `nodesize = None` (DefaultNodeSize = 10 → 50/50).
pub(super) fn queue_target_layout_split(
    store: &Store,
    target_tab_id: &str,
    block_id: &str,
    target_block_id: &str,
    dir: u8,
) -> Result<(), Box<dyn std::error::Error>> {
    // Inner directions (Top=0,Right=1,Bottom=2,Left=3): new node at
    // DefaultNodeSize (10) → 50/50 split, matching the ghost's half-leaf.
    // Outer directions (4-7): nodesize=3 → ≈23% (3/13), close to the
    // ghost's 20% (1/5). The exact ratio depends on the target node's
    // current flex size which isn't available server-side; 3 is a
    // reasonable integer approximation when the target is at DefaultNodeSize.
    let (actiontype, position, nodesize): (&str, &str, Option<u32>) = match dir {
        0 | 4 => ("splitvertical",   "before", if dir >= 4 { Some(3) } else { None }),
        2 | 6 => ("splitvertical",   "after",  if dir >= 4 { Some(3) } else { None }),
        3 | 7 => ("splithorizontal", "before", if dir >= 4 { Some(3) } else { None }),
        1 | 5 => ("splithorizontal", "after",  if dir >= 4 { Some(3) } else { None }),
        // Center (8) or unknown — caller should use queue_target_layout_insert.
        _ => return queue_target_layout_insert(store, target_tab_id, block_id),
    };
    let target_tab = store.must_get::<Tab>(target_tab_id)?;
    let mut target_layout = store.must_get::<LayoutState>(&target_tab.layoutstate)?;
    let mut actions = target_layout.pendingbackendactions.take().unwrap_or_default();
    actions.push(LayoutActionData {
        actiontype: actiontype.to_string(),
        actionid: uuid::Uuid::new_v4().to_string(),
        blockid: block_id.to_string(),
        nodesize,
        indexarr: None,
        focused: true,
        magnified: false,
        ephemeral: false,
        targetblockid: target_block_id.to_string(),
        position: position.to_string(),
    });
    target_layout.pendingbackendactions = Some(actions);
    store.update(&mut target_layout)?;
    Ok(())
}

/// Phase E.5.5 — append a layout-delete action to the source tab's
/// `LayoutState.pendingbackendactions` so the source window's
/// frontend tears the moved block out of its layout tree on next
/// poll. Mirrors the action-queueing portion of
/// `wcore::tear_off_block`. Layout migration is E.4.
pub(super) fn queue_source_layout_delete(
    store: &Store,
    source_tab_id: &str,
    block_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let source_tab = store.must_get::<Tab>(source_tab_id)?;
    let mut source_layout = store.must_get::<LayoutState>(&source_tab.layoutstate)?;
    let mut actions = source_layout.pendingbackendactions.take().unwrap_or_default();
    actions.push(LayoutActionData {
        actiontype: "delete".to_string(),
        actionid: uuid::Uuid::new_v4().to_string(),
        blockid: block_id.to_string(),
        nodesize: None,
        indexarr: None,
        focused: false,
        magnified: false,
        ephemeral: false,
        targetblockid: String::new(),
        position: String::new(),
    });
    source_layout.pendingbackendactions = Some(actions);
    store.update(&mut source_layout)?;
    Ok(())
}
