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
    // When the tree is wiped, focused/magnified ids would point at
    // non-existent nodes; clear them (the persist side does the same).
    if new_tree.is_none() {
        tab.focused_node_id = String::new();
        tab.magnified_node_id = String::new();
    }
    let (pushed_focused, pushed_magnified) = (tab.focused_node_id.clone(), tab.magnified_node_id.clone());
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
    // A prune can clear focus/magnify; the persisted row must get the same
    // values. With slices they ride the event; without, they get their own.
    let (focused, magnified) = (tab.focused_node_id.clone(), tab.magnified_node_id.clone());
    let mut slices = slices;
    if let Some(s) = slices.as_mut() {
        s.focused_node_id = focused.clone();
        s.magnified_node_id = magnified.clone();
    }
    let slices_carry_focus = slices.is_some();
    let v = state.bump_version();
    let mut events = vec![Event::LayoutTreeReplaced {
        tab_id: tab_id.clone(),
        new_tree,
        correlation_id,
        slices,
        version: v,
    }];
    if !slices_carry_focus && focused != pushed_focused {
        let v = state.bump_version();
        events.push(Event::FocusedNodeChanged { tab_id: tab_id.clone(), node_id: focused, version: v });
    }
    if !slices_carry_focus && magnified != pushed_magnified {
        let v = state.bump_version();
        events.push(Event::MagnifiedNodeChanged { tab_id, node_id: magnified, version: v });
    }
    events
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
    let (was_focused, was_magnified) = clear_dangling_focus_magnify(tab, &pre_focused, &pre_magnified);

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

/// Clear the tab's focus/magnify ids when they no longer resolve to a node
/// in its (already-edited) tree, given their values from BEFORE the edit.
/// Returns `(was_focused, was_magnified)`: whether each one was cleared.
/// Shared by every edit that can remove a leaf — a node delete, and a pane
/// tab move that empties its source pane — see `handle_layout_delete_node`
/// for the two ways an id can dangle after a delete.
fn clear_dangling_focus_magnify(
    tab: &mut crate::state::TabRecord,
    pre_focused: &str,
    pre_magnified: &str,
) -> (bool, bool) {
    let id_resolves = |id: &str| -> bool {
        if id.is_empty() {
            return true;
        }
        match tab.rootnode.as_ref() {
            None => false,
            Some(root) => crate::backend::layout::find_node_by_id(root, id).is_some(),
        }
    };
    let was_focused = !pre_focused.is_empty() && !id_resolves(pre_focused);
    let was_magnified = !pre_magnified.is_empty() && !id_resolves(pre_magnified);
    if was_focused {
        tab.focused_node_id = String::new();
    }
    if was_magnified {
        tab.magnified_node_id = String::new();
    }
    (was_focused, was_magnified)
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
    // One tab of a stacked pane: remove just that member and keep the pane
    // and its other tabs. Deleting the whole leaf (the old behaviour, via
    // `find_node_id_by_block` on the visible block) took every sibling tab
    // with it. SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.4.
    if let Some(root) = state.tabs.get_mut(&tab_id).and_then(|tab| tab.rootnode.as_mut()) {
        if crate::backend::layout::remove_stack_member(root, &block_id) {
            let new_tree = state.tabs.get(&tab_id).and_then(|tab| tab.rootnode.clone());
            let v = state.bump_version();
            return vec![Event::LayoutTreeReplaced {
                tab_id,
                new_tree,
                correlation_id,
                // Tree only: focus, magnify, leaf order and the pending
                // action queue are left exactly as they are.
                slices: None,
                version: v,
            }];
        }
    }
    let node_id = match state
        .tabs
        .get(&tab_id)
        .and_then(|tab| tab.rootnode.as_ref())
        .and_then(|root| crate::backend::layout::find_leaf_containing_block(root, &block_id))
        .map(|leaf| leaf.id.clone())
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

// ── Pane tabs (SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.2) ─────────

/// Apply a pane-tab edit to the tab's tree and emit the tree-only
/// `LayoutTreeReplaced` (no slices: focus, magnify, leaf order and the
/// pending-action queue are untouched). `edit` returns false when it found
/// nothing to act on.
fn stack_tree_edit(
    state: &mut State,
    op: &str,
    tab_id: String,
    correlation_id: String,
    not_found: String,
    edit: impl FnOnce(&mut agentmux_common::LayoutNode) -> bool,
) -> Vec<Event> {
    let applied = match state.tabs.get_mut(&tab_id) {
        None => return unknown_tab(state, op, &tab_id),
        Some(tab) => tab.rootnode.as_mut().is_some_and(edit),
    };
    if !applied {
        return op_error(state, format!("{op}: {not_found} (tab {tab_id})"));
    }
    let new_tree = state.tabs.get(&tab_id).and_then(|tab| tab.rootnode.clone());
    let violations = crate::backend::layout::validate_layout_invariants(&new_tree);
    if !violations.is_empty() {
        tracing::error!(tab_id = %tab_id, op, violations = ?violations, "layout-doctor: invariant violation(s) after a pane-tab edit");
    }
    let v = state.bump_version();
    vec![Event::LayoutTreeReplaced {
        tab_id,
        new_tree,
        correlation_id,
        slices: None,
        version: v,
    }]
}

pub(super) fn handle_layout_stack_push(
    state: &mut State,
    tab_id: String,
    target_block_id: String,
    block_id: String,
    activate: bool,
    correlation_id: String,
) -> Vec<Event> {
    // Only a block that lives in this tab may join one of its panes.
    if state.blocks.get(&block_id).map(|b| b.tab_id.as_str()) != Some(tab_id.as_str()) {
        return op_error(state, format!("LayoutStackPush: block {block_id} is not in tab {tab_id}"));
    }
    let not_found = format!("no pane holds block {target_block_id}");
    stack_tree_edit(state, "LayoutStackPush", tab_id, correlation_id, not_found, |root| {
        crate::backend::layout::push_stack_member(root, &target_block_id, &block_id, activate)
    })
}

pub(super) fn handle_layout_stack_activate(
    state: &mut State,
    tab_id: String,
    block_id: String,
    correlation_id: String,
) -> Vec<Event> {
    let not_found = format!("no pane holds block {block_id}");
    stack_tree_edit(state, "LayoutStackActivate", tab_id, correlation_id, not_found, |root| {
        crate::backend::layout::activate_stack_member(root, &block_id)
    })
}

/// Create a block and place it in a pane's tab stack in one step. Placement
/// is validated FIRST: if no pane holds `target_block_id`, nothing is created
/// — a block never exists without a pane.
pub(super) fn handle_create_block_in_stack(
    state: &mut State,
    tab_id: String,
    target_block_id: String,
    meta: serde_json::Value,
    activate: bool,
) -> Vec<Event> {
    let Some(tab) = state.tabs.get(&tab_id) else {
        return unknown_tab(state, "CreateBlockInStack", &tab_id);
    };
    let placeable = tab
        .rootnode
        .as_ref()
        .is_some_and(|root| crate::backend::layout::find_leaf_containing_block(root, &target_block_id).is_some());
    if !placeable {
        return op_error(
            state,
            format!("CreateBlockInStack: no pane holds block {target_block_id} (tab {tab_id})"),
        );
    }
    let mut events = super::block::handle_create_block(state, tab_id.clone(), meta);
    let Some(block_id) = events.iter().find_map(|e| match e {
        Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
        _ => None,
    }) else {
        return events; // the create itself failed; its Error event says why
    };
    events.extend(handle_layout_stack_push(state, tab_id, target_block_id, block_id, activate, String::new()));
    events
}

/// Reorder `block_id` within its own pane, or move it into a different
/// pane — a same-leaf reorder never changes which member is visible unless
/// `activate` is set. Refuses (an `Error` event, tree untouched) when either
/// block doesn't exist in the tab's tree.
///
/// Moving a pane's ONLY tab into another pane empties it, and the emptied
/// pane is removed (`move_stack_member`). That one case can leave focus or
/// magnify pointing at a node that's gone, so instead of the tree-only
/// `LayoutTreeReplaced` it's reported exactly like a node delete: focus and
/// magnify reconciled, and a `LayoutNodeDeleted` for the source leaf that
/// carries the full resulting tree (the tab's new member included). No block
/// is deleted either way — the moved block lives on in its new pane.
/// SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §4.1,
/// SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md §4.3.
pub(super) fn handle_layout_stack_move(
    state: &mut State,
    tab_id: String,
    block_id: String,
    target_block_id: String,
    position: agentmux_common::StackMovePosition,
    activate: bool,
    correlation_id: String,
) -> Vec<Event> {
    let not_found = format!("block {block_id} or {target_block_id} not found");
    let emptied_source = state
        .tabs
        .get(&tab_id)
        .and_then(|tab| tab.rootnode.as_ref())
        .and_then(|root| crate::backend::layout::source_leaf_emptied_by_move(root, &block_id, &target_block_id));
    let Some(source_node_id) = emptied_source else {
        return stack_tree_edit(state, "LayoutStackMove", tab_id, correlation_id, not_found, |root| {
            crate::backend::layout::move_stack_member(root, &block_id, &target_block_id, position, activate)
        });
    };

    // The emptied-pane case. `source_leaf_emptied_by_move` only returns Some
    // for a tab that exists with a tree, so both lookups below succeed.
    let tab = state.tabs.get_mut(&tab_id).expect("resolved above");
    let pre_focused = tab.focused_node_id.clone();
    let pre_magnified = tab.magnified_node_id.clone();
    let applied = tab.rootnode.as_mut().is_some_and(|root| {
        crate::backend::layout::move_stack_member(root, &block_id, &target_block_id, position, activate)
    });
    if !applied {
        return op_error(state, format!("LayoutStackMove: {not_found} (tab {tab_id})"));
    }
    let (was_focused, was_magnified) = clear_dangling_focus_magnify(tab, &pre_focused, &pre_magnified);
    let new_tree = tab.rootnode.clone();
    let violations = crate::backend::layout::validate_layout_invariants(&new_tree);
    if !violations.is_empty() {
        tracing::error!(tab_id = %tab_id, op = "LayoutStackMove", violations = ?violations, "layout-doctor: invariant violation(s) after a pane-tab edit");
    }
    let v = state.bump_version();
    vec![Event::LayoutNodeDeleted {
        tab_id,
        node_id: source_node_id,
        new_tree,
        // The destination leaf is still there, so the tree is never empty.
        tree_cleared: false,
        was_focused,
        was_magnified,
        correlation_id,
        version: v,
    }]
}

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

/// Clear focused/magnified ids that no longer resolve in the tab's tree
/// (e.g. after `LayoutSetTree` pruned a dangling leaf) — same intent as
/// `handle_layout_delete_node`'s post-walk.
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
}
