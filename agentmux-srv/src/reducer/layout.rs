// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use agentmux_common::ipc::{ErrorCode, Event};

use crate::state::State;


/// Phase E.4 (Option A) — set the focused layout-node id on a tab.
/// Errors (non-fatal) if the tab is unknown to the reducer; no-op
/// short-circuit when the value is already current. Empty `node_id`
/// clears the field. Bumps the version only on real changes so a
/// burst of identical sets doesn't churn the event stream.
pub(super) fn handle_set_focused_node(state: &mut State, tab_id: String, node_id: String) -> Vec<Event> {
    let Some(tab) = state.tabs.get_mut(&tab_id) else {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("SetFocusedNode: unknown tab {}", tab_id),
            fatal: false,
            version: v,
        }];
    };
    if tab.focused_node_id == node_id {
        return Vec::new();
    }
    tab.focused_node_id = node_id.clone();
    let v = state.bump_version();
    vec![Event::FocusedNodeChanged {
        tab_id,
        node_id,
        version: v,
    }]
}

/// Phase E.4 (Option A) — set the magnified layout-node id on a tab.
/// Same shape as `handle_set_focused_node`. Empty `node_id` is the
/// toggle-off / clear case.
pub(super) fn handle_set_magnified_node(state: &mut State, tab_id: String, node_id: String) -> Vec<Event> {
    let Some(tab) = state.tabs.get_mut(&tab_id) else {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("SetMagnifiedNode: unknown tab {}", tab_id),
            fatal: false,
            version: v,
        }];
    };
    if tab.magnified_node_id == node_id {
        return Vec::new();
    }
    tab.magnified_node_id = node_id.clone();
    let v = state.bump_version();
    vec![Event::MagnifiedNodeChanged {
        tab_id,
        node_id,
        version: v,
    }]
}

pub(super) fn handle_layout_clear(
    state: &mut State,
    tab_id: String,
    correlation_id: String,
) -> Vec<Event> {
    let Some(tab) = state.tabs.get_mut(&tab_id) else {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("LayoutClear: unknown tab {}", tab_id),
            fatal: false,
            version: v,
        }];
    };
    tab.rootnode = None;
    tab.focused_node_id = String::new();
    tab.magnified_node_id = String::new();
    let v = state.bump_version();
    vec![Event::LayoutCleared {
        tab_id,
        correlation_id,
        version: v,
    }]
}

pub(super) fn handle_layout_set_tree(
    state: &mut State,
    tab_id: String,
    new_tree: Option<agentmux_common::LayoutNode>,
    correlation_id: String,
    slices: Option<agentmux_common::LayoutClientSlices>,
) -> Vec<Event> {
    // Computed before the `state.tabs` borrow below — `blocks` and `tabs`
    // are disjoint fields, but an owned snapshot avoids any doubt.
    let live_blocks: std::collections::HashSet<String> = state.blocks.keys().cloned().collect();
    let Some(tab) = state.tabs.get_mut(&tab_id) else {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("LayoutSetTree: unknown tab {}", tab_id),
            fatal: false,
            version: v,
        }];
    };
    tab.rootnode = new_tree.clone();
    // SPEC_864 Phase 2 — a full-row push carries focus/magnify in `slices`
    // (REPLACE semantics; empty string = clear). leaforder /
    // pending_backend_actions are not modeled in TabRecord — they ride the
    // event through to the persist subscriber untouched.
    if let Some(s) = &slices {
        tab.focused_node_id = s.focused_node_id.clone();
        tab.magnified_node_id = s.magnified_node_id.clone();
    }
    // when the tree is wiped, focused/
    // magnified ids would point at non-existent nodes. Match
    // `handle_layout_clear`'s contract for the empty-tree case.
    if new_tree.is_none() {
        tab.focused_node_id = String::new();
        tab.magnified_node_id = String::new();
    }
    // Referential-integrity enforcement (see
    // `backend::layout::prune_dangling_block_refs`'s doc comment): this is
    // the exact vector — a wholesale tree push — that let a stale frontend
    // copy resurrect a deleted block's leaf
    // (INVESTIGATION_LAYOUT_DEAD_SPACE_STALE_TREE_RESURRECTION_2026_07_08.md).
    // Re-derive `new_tree`/the event from the now-authoritative (possibly
    // pruned) `tab.rootnode` rather than the caller's original value, so
    // what gets persisted matches what the reducer actually holds — a
    // pruned `tab.rootnode` with a stale `new_tree` in the emitted event
    // would just move the divergence downstream instead of closing it.
    let pruned = crate::backend::layout::prune_dangling_block_refs(&mut tab.rootnode, &live_blocks);
    if pruned > 0 {
        reconcile_focus_magnify(tab);
        tracing::warn!(
            tab_id = %tab_id,
            pruned,
            "reducer: LayoutSetTree pruned dangling layout leaf/leaves referencing since-deleted blocks"
        );
    }
    // Minimize-lock enforcement (same write-point-invariant shape as the
    // prune above; SPEC_LAYOUT_MINIMIZE_LOCKED_STATE_REDESIGN_2026_07_16.md):
    // a pushed tree that resized a minimized node gets snapped back here.
    let snapped = crate::backend::layout::enforce_minimized_locks(&mut tab.rootnode);
    if snapped > 0 {
        tracing::warn!(
            tab_id = %tab_id,
            snapped,
            "reducer: LayoutSetTree snapped minimize-locked node size(s) back to their locked values"
        );
    }
    // Layout doctor (issue #2179): loudly attribute any surviving structural
    // corruption to this write instead of letting it persist silently.
    let violations = crate::backend::layout::validate_layout_invariants(&tab.rootnode);
    if !violations.is_empty() {
        tracing::error!(
            tab_id = %tab_id,
            violations = ?violations,
            "layout-doctor: invariant violation(s) in tree persisted by LayoutSetTree"
        );
    }
    let new_tree = tab.rootnode.clone();
    let v = state.bump_version();
    vec![Event::LayoutTreeReplaced {
        tab_id,
        new_tree,
        correlation_id,
        slices,
        version: v,
    }]
}

pub(super) fn handle_layout_insert_node(
    state: &mut State,
    tab_id: String,
    node: agentmux_common::LayoutNode,
    parent_id: Option<String>,
    index: Option<usize>,
    focus_after: bool,
    magnify_after: bool,
    correlation_id: String,
) -> Vec<Event> {
    let Some(tab) = state.tabs.get_mut(&tab_id) else {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("LayoutInsertNode: unknown tab {}", tab_id),
            fatal: false,
            version: v,
        }];
    };
    // Three insert paths, in priority order:
    //   1. Empty tree → promote new node to root.
    //   2. parent_id given → insert under that specific parent at
    //      `index` (or append if None). If parent_id is given but
    //      doesn't resolve to a group node, REJECT with
    //      Event::Error rather than fall back to the heuristic.
    //      A silent-fallback path is a consistency hole: the
    //      emitted event would echo the requested parent_id while
    //      the actual mutation
    //      went elsewhere, diverging the persist subscriber and
    //      replay consumers from the reducer.
    //   3. parent_id None → `findNextInsertLocation` heuristic.
    let empty_tree = tab.rootnode.is_none();
    if empty_tree {
        // Empty-tree promotion must reject any explicit `parent_id`/
        // `index` for the same divergence reason as the explicit-
        // parent path below — the event would echo a target the
        // tree cannot resolve, and a replay consumer or the persist
        // subscriber would diverge from the reducer. Per
        // `srv-phase-e4b-formal-spec-2026-05-03.md` §7.1, both
        // fields must be `None` for empty-tree promote.
        if parent_id.is_some() || index.is_some() {
            let v = state.bump_version();
            return vec![Event::Error {
                code: ErrorCode::InvalidCommand,
                message: format!(
                    "LayoutInsertNode: empty tree cannot honour explicit parent_id={:?} / index={:?} (tab {})",
                    parent_id, index, tab_id
                ),
                fatal: false,
                version: v,
            }];
        }
        tab.rootnode = Some(node.clone());
    } else if let Some(pid) = parent_id.as_deref() {
        let root = tab.rootnode.as_mut().expect("non-empty checked above");
        match crate::backend::layout::find_node_by_id_mut(root, pid) {
            // Minimize-locked parents (a dissolved column has `data: None`,
            // so it would otherwise satisfy the group-node arm below) cannot
            // host inserts — minimized is a locked state
            // (SPEC_LAYOUT_MINIMIZE_LOCKED_STATE_REDESIGN_2026_07_16.md).
            Some(parent_node) if crate::backend::layout::is_effectively_minimized(parent_node) => {
                let v = state.bump_version();
                return vec![Event::Error {
                    code: ErrorCode::InvalidCommand,
                    message: format!(
                        "LayoutInsertNode: parent_id {:?} is minimize-locked, cannot host inserts (tab {})",
                        pid, tab_id
                    ),
                    fatal: false,
                    version: v,
                }];
            }
            Some(parent_node) if parent_node.data.is_none() => {
                let len = parent_node.children.len();
                let target = index.map(|i| i.min(len)).unwrap_or(len);
                parent_node.children.insert(target, node.clone());
            }
            Some(_) => {
                // parent_id resolves to a leaf — can't host children.
                let v = state.bump_version();
                return vec![Event::Error {
                    code: ErrorCode::InvalidCommand,
                    message: format!(
                        "LayoutInsertNode: parent_id {:?} is a leaf node, cannot host children (tab {})",
                        pid, tab_id
                    ),
                    fatal: false,
                    version: v,
                }];
            }
            None => {
                let v = state.bump_version();
                return vec![Event::Error {
                    code: ErrorCode::InvalidCommand,
                    message: format!(
                        "LayoutInsertNode: parent_id {:?} not found in tree (tab {})",
                        pid, tab_id
                    ),
                    fatal: false,
                    version: v,
                }];
            }
        }
    } else {
        let root = tab.rootnode.as_mut().expect("non-empty checked above");
        crate::backend::layout::insert_node(root, node.clone());
    }
    // honour focus_after / magnify_after.
    // The schema documents these as the side effects callers rely on
    // for "insert + activate" flows; ignoring them desyncs the snapshot
    // from the event the caller's handler observed.
    //
    // a magnified node must also be the
    // focused one. Without this, a `magnify_after=true,
    // focus_after=false` insert leaves `focused_node_id` pointing at
    // the prior pane while `magnified_node_id` points at the new one
    // — a UI invariant violation (frontend treats magnify-implies-
    // focus). Treat magnify as implying focus.
    if focus_after || magnify_after {
        tab.focused_node_id = node.id.clone();
    }
    if magnify_after {
        tab.magnified_node_id = node.id.clone();
    }
    let v = state.bump_version();
    vec![Event::LayoutNodeInserted {
        tab_id,
        node,
        // pass the command's
        // parent_id / index through to the event so subscribers see
        // what the caller asked for, not a hardcoded `None, None`.
        // The pure helper currently uses the `findNextInsertLocation`
        // heuristic and ignores these hints — but the event is the
        // record of what was *requested*; subscribers can correlate
        // with the resulting tree by inspecting `node` itself.
        parent_id,
        index,
        correlation_id,
        version: v,
    }]
}

pub(super) fn handle_layout_delete_node(
    state: &mut State,
    tab_id: String,
    node_id: String,
    correlation_id: String,
) -> Vec<Event> {
    let Some(tab) = state.tabs.get_mut(&tab_id) else {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("LayoutDeleteNode: unknown tab {}", tab_id),
            fatal: false,
            version: v,
        }];
    };
    // Snapshot pre-delete focus/magnify so we can both detect
    // direct-target hits AND post-walk for indirect orphaning.
    let pre_focused = tab.focused_node_id.clone();
    let pre_magnified = tab.magnified_node_id.clone();
    let Some(root) = tab.rootnode.as_mut() else {
        // Empty tree — nothing to delete; idempotent no-op (no event).
        return Vec::new();
    };
    // `backend::layout::delete_node` leaves
    // root deletion to the caller (returns Ok(()) with the root
    // unmodified). Detect the root case here and clear the tree
    // wholesale so the reducer state matches the
    // `LayoutNodeDeleted` event we emit.
    if root.id == node_id {
        tab.rootnode = None;
    } else if let Err(e) = crate::backend::layout::delete_node(root, &node_id) {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("LayoutDeleteNode: {} (tab {})", e, tab_id),
            fatal: false,
            version: v,
        }];
    }

    // Two ways focus/magnify ids can reference nodes that no longer
    // exist in the tree post-delete; both must be cleared:
    //   - `delete_recursive` collapse-sole-child rewrites a
    //        parent's id to the promoted child's id. If
    //        focused/magnified was the parent's original id, that
    //        id is gone from the tree even though the same physical
    //        node remains.
    //   - Deleting a container removes all descendants. If
    //        focused/magnified was a descendant, it's gone too.
    // Direct-target match (`pre_focused == node_id`) doesn't catch
    // either case. Reconcile by walking the post-delete tree and
    // clearing any focus/magnify id that no longer resolves.
    let id_resolves = |id: &str| -> bool {
        if id.is_empty() {
            return true;
        }
        match tab.rootnode.as_ref() {
            None => false,
            Some(root) => crate::backend::layout::find_node_by_id(root, id).is_some(),
        }
    };
    let was_focused = !pre_focused.is_empty() && !id_resolves(&pre_focused);
    let was_magnified = !pre_magnified.is_empty() && !id_resolves(&pre_magnified);
    if was_focused {
        tab.focused_node_id = String::new();
    }
    if was_magnified {
        tab.magnified_node_id = String::new();
    }

    // SPEC_864 site #6 — carry the resulting tree so the persist
    // subscriber can write db_layout without re-running the algebra
    // (same single-writer shape as the 7 structural arms). A delete is
    // the one structural op that can legitimately EMPTY the tree
    // (root-orphan case), so `tree_cleared` disambiguates a real clear
    // from a version-skewed sender's absent field.
    let new_tree = tab.rootnode.clone();
    let tree_cleared = new_tree.is_none();
    let v = state.bump_version();
    vec![Event::LayoutNodeDeleted {
        tab_id,
        node_id,
        new_tree,
        tree_cleared,
        was_focused,
        was_magnified,
        correlation_id,
        version: v,
    }]
}

/// SPEC_864 site #6 — delete the layout node holding `block_id`.
/// Resolution happens here because the reducer owns the tree; the
/// caller (the `delete_block` saga) only knows the block id.
///
/// Unresolved block → silent idempotent no-op (empty event vec), NOT an
/// error: every `delete_block` saga run dispatches this, and a block
/// may legitimately have no layout node (the frontend's own delete
/// flow already pushed a pruned tree via `LayoutSetTree`, the block
/// was floating/never laid out, or the tab tree is empty).
pub(super) fn handle_layout_delete_node_by_block(
    state: &mut State,
    tab_id: String,
    block_id: String,
    correlation_id: String,
) -> Vec<Event> {
    let node_id = match state
        .tabs
        .get(&tab_id)
        .and_then(|tab| tab.rootnode.as_ref())
        .and_then(|root| crate::backend::layout::find_node_id_by_block(root, &block_id))
    {
        Some(id) => id,
        // Unknown tab, empty tree, or block not in the tree — no-op.
        None => return Vec::new(),
    };
    handle_layout_delete_node(state, tab_id, node_id, correlation_id)
}

/// SPEC_864 Phase 4 — append backend actions to a tab's
/// `pendingbackendactions` queue. The reducer does NOT model the queue
/// in `TabRecord` (it is a transient backend→frontend mailbox the
/// frontend drains via REPLACE-clear slices, not layout-tree state), so
/// this arm only validates and passes the payload through; the persist
/// subscriber APPENDs it to `LayoutState.pendingbackendactions`.
///
/// `actions` must be a non-empty JSON array of `LayoutActionData`
/// objects (the type lives in `agentmux-srv::backend::obj`, not the
/// common crate — hence the raw-value carriage).
pub(super) fn handle_layout_queue_backend_actions(
    state: &mut State,
    tab_id: String,
    actions: serde_json::Value,
    correlation_id: String,
) -> Vec<Event> {
    if !state.tabs.contains_key(&tab_id) {
        return unknown_tab(state, "LayoutQueueBackendActions", &tab_id);
    }
    match actions.as_array() {
        Some(arr) if !arr.is_empty() => {}
        _ => {
            return op_error(
                state,
                format!(
                    "LayoutQueueBackendActions: actions must be a non-empty JSON array (tab {})",
                    tab_id
                ),
            )
        }
    }
    let v = state.bump_version();
    vec![Event::LayoutBackendActionsQueued {
        tab_id,
        actions,
        correlation_id,
        version: v,
    }]
}

// ── Phase 3 — remaining structural arms ─────────────────────────────────────
//
// Each arm resolves the tab, calls the existing pure fn in `backend::layout`,
// runs `balance_node` (matching the frontend's post-action normalize so the
// reducer's tree equals what the frontend would produce), reconciles any
// dangling focus/magnify, and emits the granular event. The pure fns and the
// commands/events already exist; this wires them.

/// `Event::Error` for an unknown tab.
fn unknown_tab(state: &mut State, op: &str, tab_id: &str) -> Vec<Event> {
    let v = state.bump_version();
    vec![Event::Error {
        code: ErrorCode::InvalidCommand,
        message: format!("{}: unknown tab {}", op, tab_id),
        fatal: false,
        version: v,
    }]
}

/// `Event::Error` for an operation failure (pure-fn or balance error).
fn op_error(state: &mut State, message: String) -> Vec<Event> {
    let v = state.bump_version();
    vec![Event::Error {
        code: ErrorCode::InvalidCommand,
        message,
        fatal: false,
        version: v,
    }]
}

/// Clear focused/magnified ids that no longer resolve in the (post-op,
/// post-balance) tree. `balance_node`'s single-leaf collapse can rewrite a
/// parent's id and structural ops can remove nodes, so every structural arm
/// reconciles — same intent as `handle_layout_delete_node`'s post-walk.
fn reconcile_focus_magnify(tab: &mut crate::state::TabRecord) {
    let resolves = |id: &str| -> bool {
        if id.is_empty() {
            return true;
        }
        match tab.rootnode.as_ref() {
            Some(root) => crate::backend::layout::find_node_by_id(root, id).is_some(),
            None => false,
        }
    };
    let drop_focus = !resolves(&tab.focused_node_id);
    let drop_magnify = !resolves(&tab.magnified_node_id);
    if drop_focus {
        tab.focused_node_id.clear();
    }
    if drop_magnify {
        tab.magnified_node_id.clear();
    }
}

pub(super) fn handle_layout_move_node(
    state: &mut State,
    tab_id: String,
    node_id: String,
    new_parent_id: String,
    index: usize,
    correlation_id: String,
) -> Vec<Event> {
    if let Err(events) = apply_atomic(state, &tab_id, "LayoutMoveNode", |root| {
        crate::backend::layout::move_node(root, &node_id, &new_parent_id, index)
    }) {
        return events;
    }
    let tab = state.tabs.get_mut(&tab_id).expect("tab present after apply_atomic");
    reconcile_focus_magnify(tab);
    let new_tree = tab.rootnode.clone();
    let v = state.bump_version();
    vec![Event::LayoutNodeMoved {
        tab_id,
        new_tree,
        node_id,
        new_parent_id,
        index,
        correlation_id,
        version: v,
    }]
}

pub(super) fn handle_layout_swap_nodes(
    state: &mut State,
    tab_id: String,
    node1_id: String,
    node2_id: String,
    correlation_id: String,
) -> Vec<Event> {
    if let Err(events) = apply_atomic(state, &tab_id, "LayoutSwapNodes", |root| {
        crate::backend::layout::swap_nodes(root, &node1_id, &node2_id)
    }) {
        return events;
    }
    let tab = state.tabs.get_mut(&tab_id).expect("tab present after apply_atomic");
    reconcile_focus_magnify(tab);
    let new_tree = tab.rootnode.clone();
    let v = state.bump_version();
    vec![Event::LayoutNodesSwapped {
        tab_id,
        new_tree,
        node1_id,
        node2_id,
        correlation_id,
        version: v,
    }]
}

pub(super) fn handle_layout_resize_nodes(
    state: &mut State,
    tab_id: String,
    ops: Vec<agentmux_common::ResizeOp>,
    correlation_id: String,
) -> Vec<Event> {
    if let Err(events) = apply_atomic(state, &tab_id, "LayoutResizeNodes", |root| {
        crate::backend::layout::resize_nodes(root, &ops)
    }) {
        return events;
    }
    let tab = state.tabs.get_mut(&tab_id).expect("tab present after apply_atomic");
    reconcile_focus_magnify(tab);
    let new_tree = tab.rootnode.clone();
    let v = state.bump_version();
    vec![Event::LayoutNodesResized {
        tab_id,
        new_tree,
        ops,
        correlation_id,
        version: v,
    }]
}

pub(super) fn handle_layout_replace_node(
    state: &mut State,
    tab_id: String,
    target_id: String,
    new_node: agentmux_common::LayoutNode,
    focus_after: bool,
    correlation_id: String,
) -> Vec<Event> {
    let new_id = new_node.id.clone();
    let new_node_event = new_node.clone();
    if let Err(events) = apply_atomic(state, &tab_id, "LayoutReplaceNode", |root| {
        crate::backend::layout::replace_node(root, &target_id, new_node)
    }) {
        return events;
    }
    let tab = state.tabs.get_mut(&tab_id).expect("tab present after apply_atomic");
    if focus_after {
        tab.focused_node_id = new_id;
    }
    reconcile_focus_magnify(tab);
    let new_tree = tab.rootnode.clone();
    let v = state.bump_version();
    vec![Event::LayoutNodeReplaced {
        tab_id,
        new_tree,
        target_id,
        new_node: new_node_event,
        correlation_id,
        version: v,
    }]
}

pub(super) fn handle_layout_split_horizontal(
    state: &mut State,
    tab_id: String,
    target_id: String,
    new_node: agentmux_common::LayoutNode,
    position: agentmux_common::SplitPosition,
    focus_after: bool,
    correlation_id: String,
) -> Vec<Event> {
    let new_id = new_node.id.clone();
    let new_node_event = new_node.clone();
    if let Err(events) = apply_atomic(state, &tab_id, "LayoutSplitHorizontal", |root| {
        crate::backend::layout::split_horizontal(root, &target_id, new_node, position)
    }) {
        return events;
    }
    let tab = state.tabs.get_mut(&tab_id).expect("tab present after apply_atomic");
    if focus_after {
        tab.focused_node_id = new_id;
    }
    reconcile_focus_magnify(tab);
    let new_tree = tab.rootnode.clone();
    let v = state.bump_version();
    vec![Event::LayoutSplitHorizontalApplied {
        tab_id,
        new_tree,
        target_id,
        new_node: new_node_event,
        position,
        correlation_id,
        version: v,
    }]
}

pub(super) fn handle_layout_split_vertical(
    state: &mut State,
    tab_id: String,
    target_id: String,
    new_node: agentmux_common::LayoutNode,
    position: agentmux_common::SplitPosition,
    focus_after: bool,
    correlation_id: String,
) -> Vec<Event> {
    let new_id = new_node.id.clone();
    let new_node_event = new_node.clone();
    if let Err(events) = apply_atomic(state, &tab_id, "LayoutSplitVertical", |root| {
        crate::backend::layout::split_vertical(root, &target_id, new_node, position)
    }) {
        return events;
    }
    let tab = state.tabs.get_mut(&tab_id).expect("tab present after apply_atomic");
    if focus_after {
        tab.focused_node_id = new_id;
    }
    reconcile_focus_magnify(tab);
    let new_tree = tab.rootnode.clone();
    let v = state.bump_version();
    vec![Event::LayoutSplitVerticalApplied {
        tab_id,
        new_tree,
        target_id,
        new_node: new_node_event,
        position,
        correlation_id,
        version: v,
    }]
}

pub(super) fn handle_layout_insert_node_at_index(
    state: &mut State,
    tab_id: String,
    node: agentmux_common::LayoutNode,
    index_arr: Vec<usize>,
    focus_after: bool,
    magnify_after: bool,
    correlation_id: String,
) -> Vec<Event> {
    let new_id = node.id.clone();
    let node_event = node.clone();
    let live_blocks: std::collections::HashSet<String> = state.blocks.keys().cloned().collect();
    let Some(tab) = state.tabs.get_mut(&tab_id) else {
        return unknown_tab(state, "LayoutInsertNodeAtIndex", &tab_id);
    };
    // Build the new tree on a clone, balance, and commit only on success
    // (atomic — no partial mutation on error). Matches the frontend oracle:
    // `insertNodeAtIndex` PROMOTES the node to root when the tree is empty
    // (frontend/layout/lib/layoutTree.ts:303-304), so strong-reducer authority
    // promotes too rather than rejecting — `index_arr` is moot on an empty
    // tree. (Replay stays consistent: the same insert-at-index semantics
    // promote on empty; the event documents the request.)
    let working_result: Result<agentmux_common::LayoutNode, String> = match tab.rootnode.as_ref() {
        Some(root) => {
            let mut w = root.clone();
            crate::backend::layout::insert_node_at_index(&mut w, node, &index_arr)
                .map(|_| w)
                .map_err(|e| format!("LayoutInsertNodeAtIndex: {} (tab {})", e, tab_id))
        }
        None => Ok(node),
    };
    let mut working = match working_result {
        Ok(w) => w,
        Err(msg) => return op_error(state, msg),
    };
    if let Err(e) = crate::backend::layout::balance_node(&mut working) {
        return op_error(
            state,
            format!("LayoutInsertNodeAtIndex balance: {} (tab {})", e, tab_id),
        );
    }
    tab.rootnode = Some(working);
    // magnify implies focus (frontend invariant; see handle_layout_insert_node).
    if focus_after || magnify_after {
        tab.focused_node_id = new_id.clone();
    }
    if magnify_after {
        tab.magnified_node_id = new_id;
    }
    // Referential-integrity enforcement, same choke point as
    // `handle_layout_set_tree` — see `prune_dangling_block_refs`'s doc
    // comment. The caller-supplied `node` could carry a stale `block_id`.
    let pruned = crate::backend::layout::prune_dangling_block_refs(&mut tab.rootnode, &live_blocks);
    if pruned > 0 {
        tracing::warn!(
            tab_id = %tab_id,
            pruned,
            "reducer: LayoutInsertNodeAtIndex pruned dangling layout leaf/leaves referencing since-deleted blocks"
        );
    }
    // Minimize-lock enforcement, same choke point (see handle_layout_set_tree).
    let snapped = crate::backend::layout::enforce_minimized_locks(&mut tab.rootnode);
    if snapped > 0 {
        tracing::warn!(
            tab_id = %tab_id,
            snapped,
            "reducer: LayoutInsertNodeAtIndex snapped minimize-locked node size(s) back to their locked values"
        );
    }
    // Layout doctor (issue #2179), same rationale as handle_layout_set_tree.
    let violations = crate::backend::layout::validate_layout_invariants(&tab.rootnode);
    if !violations.is_empty() {
        tracing::error!(
            tab_id = %tab_id,
            violations = ?violations,
            "layout-doctor: invariant violation(s) in tree persisted by LayoutInsertNodeAtIndex"
        );
    }
    reconcile_focus_magnify(tab);
    let new_tree = tab.rootnode.clone();
    let v = state.bump_version();
    vec![Event::LayoutNodeInsertedAtIndex {
        tab_id,
        new_tree,
        node: node_event,
        index_arr,
        correlation_id,
        version: v,
    }]
}

/// Apply a fallible structural mutation to a tab's layout tree atomically:
/// operate on a *clone*, run `balance_node`, and commit back to
/// `tab.rootnode` only if both succeed. On any error the tab's tree is left
/// untouched — no partial mutation is visible to a later snapshot or to the
/// next operation. [codex P2 #1868]
///
/// On error, returns the `Event::Error` vec the caller should return verbatim
/// (unknown tab, empty tree, op failure, or balance failure). On success,
/// returns `Ok(())`; the caller re-fetches the tab for focus/magnify
/// reconciliation and to emit the granular event.
fn apply_atomic<F>(
    state: &mut State,
    tab_id: &str,
    op: &str,
    f: F,
) -> Result<(), Vec<Event>>
where
    F: FnOnce(&mut agentmux_common::LayoutNode) -> Result<(), crate::backend::layout::LayoutError>,
{
    let Some(tab) = state.tabs.get_mut(tab_id) else {
        return Err(unknown_tab(state, op, tab_id));
    };
    let Some(current) = tab.rootnode.as_ref() else {
        return Err(op_error(state, format!("{}: empty tree (tab {})", op, tab_id)));
    };
    let mut working = current.clone();
    if let Err(e) = f(&mut working) {
        return Err(op_error(state, format!("{}: {} (tab {})", op, e, tab_id)));
    }
    if let Err(e) = crate::backend::layout::balance_node(&mut working) {
        return Err(op_error(state, format!("{} balance: {} (tab {})", op, e, tab_id)));
    }
    tab.rootnode = Some(working);
    // Referential-integrity enforcement, same choke point as
    // `handle_layout_set_tree` — see `prune_dangling_block_refs`'s doc
    // comment. `replace_node`/`split_horizontal`/`split_vertical` accept a
    // caller-supplied new node that could carry a stale `block_id`; this
    // also opportunistically cleans up any pre-existing dangling leaf a
    // move/swap/resize just happened to touch, for free. Every caller of
    // `apply_atomic` already calls `reconcile_focus_magnify` on its own
    // next line, so a pruned focused/magnified id is handled there — no
    // need to duplicate that call here.
    let live_blocks: std::collections::HashSet<String> = state.blocks.keys().cloned().collect();
    let tab = state.tabs.get_mut(tab_id).expect("tab present — just written above");
    let pruned = crate::backend::layout::prune_dangling_block_refs(&mut tab.rootnode, &live_blocks);
    if pruned > 0 {
        tracing::warn!(
            tab_id = %tab_id,
            op,
            pruned,
            "reducer: pruned dangling layout leaf/leaves referencing since-deleted blocks"
        );
    }
    // Minimize-lock enforcement, same choke point (see handle_layout_set_tree).
    // The per-op guards (`LayoutError::NodeLocked`) reject ops that *target* a
    // locked node; this pass additionally corrects indirect damage — e.g. a
    // legitimate resize pair whose unit redistribution squeezed a locked
    // sibling through `balance_node`'s rebuild.
    let snapped = crate::backend::layout::enforce_minimized_locks(&mut tab.rootnode);
    if snapped > 0 {
        tracing::warn!(
            tab_id = %tab_id,
            op,
            snapped,
            "reducer: snapped minimize-locked node size(s) back to their locked values"
        );
    }
    // Layout doctor (issue #2179), same rationale as handle_layout_set_tree.
    let violations = crate::backend::layout::validate_layout_invariants(&tab.rootnode);
    if !violations.is_empty() {
        tracing::error!(
            tab_id = %tab_id,
            op,
            violations = ?violations,
            "layout-doctor: invariant violation(s) in tree persisted by structural op"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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

    // ── Phase E.4.B Phase 5 — layout reducer arms ─────────────────
    //
    // Tests for the 4 arms shipped in this PR. All arms share the
    // same shape (lookup tab → mutate `tab.rootnode` via pure helper
    // → emit Event::Layout*); the unit tests below verify state
    // mutation and event shape per arm. The pure helpers themselves
    // have their own ~40 tests in `agentmux-srv/src/backend/layout/`.

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
    fn layout_clear_wipes_rootnode_focus_magnify_and_emits_event() {
        let (mut state, tab_id) = fresh_tab();
        // Pre-load some state.
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(leaf_node("n1", "b1"));
        state.tabs.get_mut(&tab_id).unwrap().focused_node_id = "n1".into();
        state.tabs.get_mut(&tab_id).unwrap().magnified_node_id = "n1".into();

        let events = update(
            &mut state,
            Command::LayoutClear {
                tab_id: tab_id.clone(),
                correlation_id: "corr-1".into(),
            },
            &ctx(1),
        );

        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            Event::LayoutCleared { correlation_id, .. } if correlation_id == "corr-1"
        ));
        let tab = &state.tabs[&tab_id];
        assert!(tab.rootnode.is_none(), "rootnode wiped");
        assert_eq!(tab.focused_node_id, "");
        assert_eq!(tab.magnified_node_id, "");
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
    fn layout_clear_unknown_tab_emits_error() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::LayoutClear {
                tab_id: "nope".into(),
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::Error { code: ErrorCode::InvalidCommand, .. }));
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

    /// Minimize-lock enforcement at the set-tree choke point
    /// (SPEC_LAYOUT_MINIMIZE_LOCKED_STATE_REDESIGN_2026_07_16.md): a pushed
    /// tree in which a minimized node's size was tampered with (e.g. a
    /// resize that slipped past a stale frontend's guards) must be snapped
    /// back to its recorded locked size, with the delta repaid to the
    /// unlocked sibling, before the tree is persisted.
    #[test]
    fn layout_set_tree_snaps_tampered_minimized_size_back_to_its_lock() {
        let (mut state, tab_id) = fresh_tab();
        seed_block(&mut state, &tab_id, "b1");
        seed_block(&mut state, &tab_id, "b2");

        let mut minimized = leaf_node("min", "b1");
        minimized.size = 120.0; // tampered: locked value is 33.0
        minimized
            .extra
            .insert("minimizedSize".into(), serde_json::json!(200.0));
        minimized
            .extra
            .insert("minimizedLockedSize".into(), serde_json::json!(33.0));
        let mut sibling = leaf_node("sib", "b2");
        sibling.size = 280.0;
        let pushed_tree = agentmux_common::LayoutNode {
            id: "root".into(),
            children: vec![minimized, sibling],
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
        let root = state.tabs[&tab_id].rootnode.as_ref().unwrap();
        assert_eq!(root.children[0].size, 33.0, "tampered size snapped back to lock");
        assert_eq!(root.children[1].size, 367.0, "delta repaid to the unlocked sibling");
        // The emitted event's tree must reflect the healed sizes too — it is
        // what the persist subscriber writes to db_layout.
        if let Event::LayoutTreeReplaced { new_tree, .. } = &events[0] {
            assert_eq!(new_tree.as_ref().unwrap().children[0].size, 33.0);
        }
    }

    /// Minimize-lock guard on the explicit-parent insert path (reagent P1,
    /// PR #2180): a dissolved column has `data: None`, so without its own
    /// locked check it would satisfy the "is a group node" arm and accept
    /// the insert — letting an agent/API caller seed a full pane inside a
    /// header strip.
    #[test]
    fn layout_insert_node_rejects_minimize_locked_parent() {
        let (mut state, tab_id) = fresh_tab();
        seed_block(&mut state, &tab_id, "b1");
        seed_block(&mut state, &tab_id, "b2");
        seed_block(&mut state, &tab_id, "b3");

        let mut l1 = leaf_node("l1", "b1");
        l1.extra.insert("minimizedSize".into(), serde_json::json!(200.0));
        let mut l2 = leaf_node("l2", "b2");
        l2.extra.insert("minimizedSize".into(), serde_json::json!(200.0));
        let mut dissolved = agentmux_common::LayoutNode {
            id: "colA".into(),
            children: vec![l1, l2],
            ..Default::default()
        };
        dissolved
            .extra
            .insert("columnDissolve".into(), serde_json::json!({"targetColumnId": "root"}));
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(agentmux_common::LayoutNode {
            id: "root".into(),
            children: vec![dissolved, leaf_node("content", "b3")],
            ..Default::default()
        });

        let events = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node: leaf_node("new", "b3"),
                parent_id: Some("colA".into()),
                index: None,
                focus_after: false,
                magnify_after: false,
                correlation_id: "corr-locked".into(),
            },
            &ctx(1),
        );

        assert!(matches!(&events[0], Event::Error { code: ErrorCode::InvalidCommand, .. }));
        // Dissolved column untouched.
        let root = state.tabs[&tab_id].rootnode.as_ref().unwrap();
        assert_eq!(root.children[0].children.len(), 2);
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
    fn layout_insert_node_into_empty_tree_promotes_to_root() {
        let (mut state, tab_id) = fresh_tab();
        let node = leaf_node("first", "b1");
        let events = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node: node.clone(),
                parent_id: None,
                index: None,
                focus_after: false,
                magnify_after: false,
                correlation_id: "corr-ins".into(),
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::LayoutNodeInserted { .. }));
        assert_eq!(state.tabs[&tab_id].rootnode.as_ref().map(|n| n.id.as_str()), Some("first"));
    }

    #[test]
    fn layout_insert_node_into_existing_tree_uses_helper() {
        let (mut state, tab_id) = fresh_tab();
        // Pre-load a single-leaf tree.
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(leaf_node("root", "b1"));
        let new_node = leaf_node("added", "b2");
        let events = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node: new_node,
                parent_id: None,
                index: None,
                focus_after: false,
                magnify_after: false,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::LayoutNodeInserted { .. }));
        // Helper turned the leaf root into a group with both leaves;
        // exact shape is the helper's contract — we just assert the
        // tree changed and contains both block ids.
        let root = state.tabs[&tab_id].rootnode.as_ref().expect("rootnode set");
        let collected = collect_block_ids(root);
        assert!(collected.contains(&"b1".to_string()));
        assert!(collected.contains(&"b2".to_string()));
    }

    fn collect_block_ids(node: &agentmux_common::LayoutNode) -> Vec<String> {
        let mut ids = Vec::new();
        if let Some(d) = &node.data {
            if !d.block_id.is_empty() {
                ids.push(d.block_id.clone());
            }
        }
        for c in &node.children {
            ids.extend(collect_block_ids(c));
        }
        ids
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
        // empty-tree set must match
        // `LayoutClear`'s contract — focused/magnified ids would
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

    #[test]
    fn layout_insert_node_honours_focus_after() {
        // focus_after=true must update
        // focused_node_id so the snapshot matches the event.
        let (mut state, tab_id) = fresh_tab();
        let node = leaf_node("new", "b1");
        let _ = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node,
                parent_id: None,
                index: None,
                focus_after: true,
                magnify_after: false,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert_eq!(state.tabs[&tab_id].focused_node_id, "new");
        assert_eq!(state.tabs[&tab_id].magnified_node_id, "");
    }

    #[test]
    fn layout_insert_node_magnify_after_implies_focus() {
        // magnify-implies-focus. Even when
        // focus_after=false, setting magnify_after=true must also
        // update focused_node_id so it doesn't dangle on the prior
        // pane (UI invariant: a magnified pane is the focused pane).
        let (mut state, tab_id) = fresh_tab();
        state.tabs.get_mut(&tab_id).unwrap().focused_node_id = "prev".into();
        let node = leaf_node("new", "b1");
        let _ = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node,
                parent_id: None,
                index: None,
                focus_after: false,
                magnify_after: true,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert_eq!(
            state.tabs[&tab_id].focused_node_id, "new",
            "magnify_after must imply focus_after"
        );
        assert_eq!(state.tabs[&tab_id].magnified_node_id, "new");
    }

    #[test]
    fn layout_insert_node_honours_explicit_parent_id_and_index() {
        // with parent_id given, insert at
        // that node at the requested index instead of running the
        // heuristic helper.
        let (mut state, tab_id) = fresh_tab();
        let mut group = leaf_node("group", "");
        group.data = None;
        group.children = vec![leaf_node("a", "ba"), leaf_node("c", "bc")];
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(group);

        let _ = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node: leaf_node("b", "bb"),
                parent_id: Some("group".into()),
                index: Some(1), // between a and c
                focus_after: false,
                magnify_after: false,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );

        let root = state.tabs[&tab_id].rootnode.as_ref().expect("root");
        let ids: Vec<_> = root.children.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"], "explicit index honoured");
    }

    #[test]
    fn layout_insert_node_index_clamps_when_out_of_range() {
        // Out-of-range index clamps to the end (matches frontend
        // `findNextInsertLocation` defensive semantics).
        let (mut state, tab_id) = fresh_tab();
        let mut group = leaf_node("group", "");
        group.data = None;
        group.children = vec![leaf_node("a", "ba")];
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(group);

        let _ = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node: leaf_node("b", "bb"),
                parent_id: Some("group".into()),
                index: Some(99),
                focus_after: false,
                magnify_after: false,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        let root = state.tabs[&tab_id].rootnode.as_ref().expect("root");
        let ids: Vec<_> = root.children.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"], "out-of-range index clamps to end");
    }

    // ── Phase 3 — the 7 remaining structural arms ──────────────────────────
    fn group_node(id: &str, children: Vec<agentmux_common::LayoutNode>) -> agentmux_common::LayoutNode {
        agentmux_common::LayoutNode {
            id: id.to_string(),
            size: 10.0,
            children,
            ..Default::default()
        }
    }

    fn tree_has(state: &State, tab_id: &str, node_id: &str) -> bool {
        state.tabs[tab_id].rootnode.as_ref().is_some_and(|r| {
            crate::backend::layout::find_node_by_id(r, node_id).is_some()
        })
    }

    #[test]
    fn layout_move_node_reparents_and_emits_event() {
        let (mut state, tab_id) = fresh_tab();
        seed_block(&mut state, &tab_id, "ba");
        seed_block(&mut state, &tab_id, "bb");
        seed_block(&mut state, &tab_id, "bc");
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(group_node(
            "root",
            vec![
                group_node("g1", vec![leaf_node("a", "ba"), leaf_node("b", "bb")]),
                leaf_node("c", "bc"),
            ],
        ));
        let events = update(
            &mut state,
            Command::LayoutMoveNode {
                tab_id: tab_id.clone(),
                node_id: "c".into(),
                new_parent_id: "g1".into(),
                index: 0,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::LayoutNodeMoved { .. }));
        assert!(tree_has(&state, &tab_id, "c"), "c still in tree");
        let root = state.tabs[&tab_id].rootnode.as_ref().unwrap();
        assert!(
            !root.children.iter().any(|ch| ch.id == "c"),
            "c reparented out of root's direct children"
        );
    }

    #[test]
    fn layout_swap_nodes_swaps_positions() {
        let (mut state, tab_id) = fresh_tab();
        seed_block(&mut state, &tab_id, "ba");
        seed_block(&mut state, &tab_id, "bb");
        state.tabs.get_mut(&tab_id).unwrap().rootnode =
            Some(group_node("root", vec![leaf_node("a", "ba"), leaf_node("b", "bb")]));
        let events = update(
            &mut state,
            Command::LayoutSwapNodes {
                tab_id: tab_id.clone(),
                node1_id: "a".into(),
                node2_id: "b".into(),
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::LayoutNodesSwapped { .. }));
        let root = state.tabs[&tab_id].rootnode.as_ref().unwrap();
        let ids: Vec<_> = root.children.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["b", "a"], "positions swapped");
    }

    #[test]
    fn layout_resize_nodes_applies_sizes() {
        let (mut state, tab_id) = fresh_tab();
        seed_block(&mut state, &tab_id, "ba");
        seed_block(&mut state, &tab_id, "bb");
        state.tabs.get_mut(&tab_id).unwrap().rootnode =
            Some(group_node("root", vec![leaf_node("a", "ba"), leaf_node("b", "bb")]));
        let events = update(
            &mut state,
            Command::LayoutResizeNodes {
                tab_id: tab_id.clone(),
                ops: vec![
                    agentmux_common::ResizeOp { node_id: "a".into(), size: 30.0 },
                    agentmux_common::ResizeOp { node_id: "b".into(), size: 70.0 },
                ],
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::LayoutNodesResized { .. }));
        let root = state.tabs[&tab_id].rootnode.as_ref().unwrap();
        let a = crate::backend::layout::find_node_by_id(root, "a").unwrap();
        assert_eq!(a.size, 30.0);
    }

    #[test]
    fn layout_replace_node_swaps_in_new_and_honours_focus() {
        let (mut state, tab_id) = fresh_tab();
        seed_block(&mut state, &tab_id, "ba");
        seed_block(&mut state, &tab_id, "bb");
        seed_block(&mut state, &tab_id, "bx");
        state.tabs.get_mut(&tab_id).unwrap().rootnode =
            Some(group_node("root", vec![leaf_node("a", "ba"), leaf_node("b", "bb")]));
        let events = update(
            &mut state,
            Command::LayoutReplaceNode {
                tab_id: tab_id.clone(),
                target_id: "a".into(),
                new_node: leaf_node("x", "bx"),
                focus_after: true,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::LayoutNodeReplaced { .. }));
        assert!(tree_has(&state, &tab_id, "x"));
        assert!(!tree_has(&state, &tab_id, "a"), "a replaced");
        assert_eq!(state.tabs[&tab_id].focused_node_id, "x");
    }

    #[test]
    fn layout_replace_with_malformed_node_is_atomic() {
        // A new_node with neither data nor children fails balance AFTER
        // replace_node has swapped it in. The arm must leave the tab's tree
        // untouched (operate on a clone, commit only on success) and emit
        // only Error — no partial mutation. [codex P2 #1868]
        let (mut state, tab_id) = fresh_tab();
        let original = group_node("root", vec![leaf_node("a", "ba"), leaf_node("b", "bb")]);
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(original.clone());
        let malformed = agentmux_common::LayoutNode {
            id: "bad".into(),
            // BOTH data and children violates leaf-XOR-branch → balance_node
            // returns Err(InvalidNode) after replace_node has swapped it in.
            data: Some(agentmux_common::LayoutNodeData {
                block_id: "x".into(),
                ..Default::default()
            }),
            children: vec![leaf_node("inner", "bi")],
            ..Default::default()
        };
        let events = update(
            &mut state,
            Command::LayoutReplaceNode {
                tab_id: tab_id.clone(),
                target_id: "a".into(),
                new_node: malformed,
                focus_after: false,
                correlation_id: "c".into(),
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
        assert_eq!(
            state.tabs[&tab_id].rootnode,
            Some(original),
            "tree left untouched on error (atomic)"
        );
    }

    #[test]
    fn layout_split_horizontal_wraps_root_and_focuses_new() {
        let (mut state, tab_id) = fresh_tab();
        seed_block(&mut state, &tab_id, "ba");
        seed_block(&mut state, &tab_id, "bx");
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(leaf_node("a", "ba"));
        let events = update(
            &mut state,
            Command::LayoutSplitHorizontal {
                tab_id: tab_id.clone(),
                target_id: "a".into(),
                new_node: leaf_node("x", "bx"),
                position: agentmux_common::SplitPosition::After,
                focus_after: true,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::LayoutSplitHorizontalApplied { .. }));
        assert!(tree_has(&state, &tab_id, "a"));
        assert!(tree_has(&state, &tab_id, "x"));
        let root = state.tabs[&tab_id].rootnode.as_ref().unwrap();
        assert!(root.data.is_none(), "root became a group");
        assert_eq!(root.children.len(), 2);
        assert_eq!(state.tabs[&tab_id].focused_node_id, "x");
    }

    #[test]
    fn layout_split_vertical_wraps_root() {
        let (mut state, tab_id) = fresh_tab();
        seed_block(&mut state, &tab_id, "ba");
        seed_block(&mut state, &tab_id, "bx");
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(leaf_node("a", "ba"));
        let events = update(
            &mut state,
            Command::LayoutSplitVertical {
                tab_id: tab_id.clone(),
                target_id: "a".into(),
                new_node: leaf_node("x", "bx"),
                position: agentmux_common::SplitPosition::Before,
                focus_after: false,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::LayoutSplitVerticalApplied { .. }));
        assert!(tree_has(&state, &tab_id, "a"));
        assert!(tree_has(&state, &tab_id, "x"));
        assert_eq!(state.tabs[&tab_id].rootnode.as_ref().unwrap().children.len(), 2);
    }

    #[test]
    fn layout_insert_node_at_index_inserts_after_path() {
        let (mut state, tab_id) = fresh_tab();
        seed_block(&mut state, &tab_id, "ba");
        seed_block(&mut state, &tab_id, "bb");
        seed_block(&mut state, &tab_id, "bx");
        state.tabs.get_mut(&tab_id).unwrap().rootnode =
            Some(group_node("root", vec![leaf_node("a", "ba"), leaf_node("b", "bb")]));
        let events = update(
            &mut state,
            Command::LayoutInsertNodeAtIndex {
                tab_id: tab_id.clone(),
                node: leaf_node("x", "bx"),
                index_arr: vec![0],
                focus_after: false,
                magnify_after: false,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::LayoutNodeInsertedAtIndex { .. }));
        let root = state.tabs[&tab_id].rootnode.as_ref().unwrap();
        let ids: Vec<_> = root.children.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "x", "b"], "inserted after index 0");
    }

    #[test]
    fn layout_insert_node_at_index_into_empty_promotes_to_root() {
        // Matches the frontend oracle (insertNodeAtIndex promotes to root on
        // an empty tree; layoutTree.ts:303-304) — not a reject.
        let (mut state, tab_id) = fresh_tab();
        seed_block(&mut state, &tab_id, "b");
        let events = update(
            &mut state,
            Command::LayoutInsertNodeAtIndex {
                tab_id: tab_id.clone(),
                node: leaf_node("only", "b"),
                index_arr: vec![0],
                focus_after: false,
                magnify_after: false,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::LayoutNodeInsertedAtIndex { .. }));
        assert_eq!(
            state.tabs[&tab_id].rootnode.as_ref().map(|n| n.id.as_str()),
            Some("only"),
            "empty-tree insert-at-index promotes the node to root"
        );
    }

    #[test]
    fn layout_move_node_unknown_tab_errors() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::LayoutMoveNode {
                tab_id: "nope".into(),
                node_id: "a".into(),
                new_parent_id: "b".into(),
                index: 0,
                correlation_id: "c".into(),
            },
            &ctx(1),
        );
        assert!(matches!(
            &events[0],
            Event::Error { code: ErrorCode::InvalidCommand, .. }
        ));
    }

    #[test]
    fn layout_swap_nodes_empty_tree_errors() {
        let (mut state, tab_id) = fresh_tab();
        let events = update(
            &mut state,
            Command::LayoutSwapNodes {
                tab_id: tab_id.clone(),
                node1_id: "a".into(),
                node2_id: "b".into(),
                correlation_id: "c".into(),
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
    }

    #[test]
    fn layout_insert_node_into_empty_tree_with_explicit_parent_id_emits_error() {
        // empty-tree
        // promotion must reject explicit parent_id — otherwise the
        // event echoes a target that subscribers can't resolve.
        let (mut state, tab_id) = fresh_tab();
        let events = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node: leaf_node("first", "b1"),
                parent_id: Some("does-not-exist".into()),
                index: None,
                focus_after: false,
                magnify_after: false,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert!(matches!(
            &events[0],
            Event::Error { code: ErrorCode::InvalidCommand, .. }
        ));
        assert!(
            state.tabs[&tab_id].rootnode.is_none(),
            "tree must stay empty on rejection"
        );
    }

    #[test]
    fn layout_insert_node_into_empty_tree_with_explicit_index_emits_error() {
        // Same rationale but with `index` only — the spec §7.1
        // requires both fields be `None` for empty-tree promote.
        let (mut state, tab_id) = fresh_tab();
        let events = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node: leaf_node("first", "b1"),
                parent_id: None,
                index: Some(0),
                focus_after: false,
                magnify_after: false,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert!(matches!(
            &events[0],
            Event::Error { code: ErrorCode::InvalidCommand, .. }
        ));
        assert!(state.tabs[&tab_id].rootnode.is_none());
    }

    #[test]
    fn layout_insert_node_with_unknown_parent_id_emits_error() {
        // silent fallback to heuristic
        // diverges the event from the actual mutation. Reject
        // explicit-but-invalid parent_id with Event::Error so
        // subscribers (especially the persist subscriber, future)
        // see a consistent record.
        let (mut state, tab_id) = fresh_tab();
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(leaf_node("only", "b1"));

        let events = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node: leaf_node("added", "b2"),
                parent_id: Some("does-not-exist".into()),
                index: None,
                focus_after: false,
                magnify_after: false,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert!(matches!(
            &events[0],
            Event::Error { code: ErrorCode::InvalidCommand, .. }
        ));
        // Tree must be unchanged.
        let root = state.tabs[&tab_id].rootnode.as_ref().expect("root");
        assert_eq!(root.id, "only");
        assert!(root.children.is_empty());
    }

    #[test]
    fn layout_insert_node_with_leaf_parent_id_emits_error() {
        // parent_id resolves to a leaf (has data) — leaf can't host
        // children, so treat as invalid the same as a missing parent.
        let (mut state, tab_id) = fresh_tab();
        let mut root = leaf_node("group", "");
        root.data = None;
        root.children = vec![leaf_node("a", "ba")];
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(root);

        let events = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node: leaf_node("b", "bb"),
                parent_id: Some("a".into()), // leaf, not a group
                index: None,
                focus_after: false,
                magnify_after: false,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert!(matches!(
            &events[0],
            Event::Error { code: ErrorCode::InvalidCommand, .. }
        ));
    }

    #[test]
    fn layout_insert_node_with_neither_flag_leaves_state_alone() {
        // Anti-vacuity guard: confirm the false-flag path is the
        // baseline (otherwise the focus_after/magnify_after tests
        // wouldn't be measuring anything).
        let (mut state, tab_id) = fresh_tab();
        state.tabs.get_mut(&tab_id).unwrap().focused_node_id = "prev".into();
        let _ = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node: leaf_node("new", "b1"),
                parent_id: None,
                index: None,
                focus_after: false,
                magnify_after: false,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert_eq!(state.tabs[&tab_id].focused_node_id, "prev");
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
    fn layout_insert_node_event_echoes_parent_id_and_index() {
        // The emitted event must echo the command's parent_id /
        // index so subscribers see what was requested. Tree pre-
        // populated with a group so the explicit-parent path
        // doesn't take the empty-tree rejection branch.
        let (mut state, tab_id) = fresh_tab();
        let mut group = leaf_node("group", "");
        group.data = None;
        group.children = vec![leaf_node("a", "ba")];
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(group);

        let events = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node: leaf_node("new", "b1"),
                parent_id: Some("group".into()),
                index: Some(0),
                focus_after: false,
                magnify_after: false,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        match &events[0] {
            Event::LayoutNodeInserted { parent_id, index, .. } => {
                assert_eq!(parent_id.as_deref(), Some("group"));
                assert_eq!(*index, Some(0));
            }
            other => panic!("expected LayoutNodeInserted, got {:?}", other),
        }
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
}
