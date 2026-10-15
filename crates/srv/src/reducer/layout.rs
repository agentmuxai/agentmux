// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use agentmux_common::ipc::{ErrorCode, Event};

use crate::state::State;

mod intent;
pub(super) use intent::handle_layout_apply_intent;
#[allow(unused_imports)]
pub(crate) use intent::RECENT_INTENTS;

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
        clear_dangling_focus_magnify(tab);
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
    report_layout_violations(&tab_id, "LayoutSetTree", &tab.rootnode);
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
    // A direct-target match (`focused == node_id`) doesn't catch
    // either case. Reconcile by walking the post-delete tree and
    // clearing any focus/magnify id that no longer resolves.
    let (was_focused, was_magnified) = clear_dangling_focus_magnify(tab);

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
/// in its (already-edited) tree. Returns `(was_focused, was_magnified)`:
/// whether each one was cleared. Shared by every edit that can remove a leaf
/// (a node delete, a pane tab move that empties its source pane, a
/// `LayoutSetTree` prune); see `handle_layout_delete_node` for the two ways
/// an id can dangle after a delete.
fn clear_dangling_focus_magnify(tab: &mut crate::state::TabRecord) -> (bool, bool) {
    let dangles = |id: &str| -> bool {
        !id.is_empty()
            && tab
                .rootnode
                .as_ref()
                .is_none_or(|root| crate::backend::layout::find_node_by_id(root, id).is_none())
    };
    let was_focused = dangles(&tab.focused_node_id);
    let was_magnified = dangles(&tab.magnified_node_id);
    if was_focused {
        tab.focused_node_id = String::new();
    }
    if was_magnified {
        tab.magnified_node_id = String::new();
    }
    (was_focused, was_magnified)
}

/// Layout doctor (#2179): attribute any structural corruption to the write
/// that made it, loudly, instead of letting it persist silently.
fn report_layout_violations(tab_id: &str, op: &str, tree: &Option<agentmux_common::LayoutNode>) {
    let violations = crate::backend::layout::validate_layout_invariants(tree);
    if !violations.is_empty() {
        tracing::error!(tab_id, op, violations = ?violations, "layout-doctor: invariant violation(s) after a layout write");
    }
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
///
/// Each batch bumps the tab's `layout_revision` and stamps it on every action
/// as `rev`, so a frontend push that never saw this batch is refused instead
/// of erasing it (`stale_push_revision`).
pub(super) fn handle_layout_queue_backend_actions(
    state: &mut State,
    tab_id: String,
    mut actions: serde_json::Value,
    correlation_id: String,
) -> Vec<Event> {
    let Some(tab) = state.tabs.get_mut(&tab_id) else {
        return unknown_tab(state, "LayoutQueueBackendActions", &tab_id);
    };
    match actions.as_array_mut() {
        Some(arr) if !arr.is_empty() => {
            tab.layout_revision += 1;
            for action in arr.iter_mut().filter_map(|a| a.as_object_mut()) {
                action.insert("rev".into(), tab.layout_revision.into());
            }
        }
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

/// The tab's current `layout_revision` when a frontend tree push built on
/// `base_revision` is stale, else `None`. A stale push never saw some queued
/// backend action (a tab pushed into a pane, a pane removed), and applying it
/// would drop that change from the tree and erase it from the queue the push
/// replaces. The pusher resyncs: it applies the actions it missed, then pushes
/// again. SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.5.
pub(crate) fn stale_push_revision(state: &State, tab_id: &str, base_revision: u64) -> Option<u64> {
    let current = state.tabs.get(tab_id)?.layout_revision;
    (base_revision < current).then_some(current)
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
    report_layout_violations(&tab_id, op, &new_tree);
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
/// — a block never exists without a pane. `block_id` is the dispatcher's `Ctx::new_id`.
pub(super) fn handle_create_block_in_stack(
    state: &mut State,
    tab_id: String,
    target_block_id: String,
    block_id: String,
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
    let mut events = super::block::create_block_with_id(state, tab_id.clone(), block_id, meta);
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
    let applied = tab.rootnode.as_mut().is_some_and(|root| {
        crate::backend::layout::move_stack_member(root, &block_id, &target_block_id, position, activate)
    });
    if !applied {
        return op_error(state, format!("LayoutStackMove: {not_found} (tab {tab_id})"));
    }
    let (was_focused, was_magnified) = clear_dangling_focus_magnify(tab);
    let new_tree = tab.rootnode.clone();
    report_layout_violations(&tab_id, "LayoutStackMove", &new_tree);
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

#[cfg(test)]
mod tests;
