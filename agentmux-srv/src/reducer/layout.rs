// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use agentmux_common::ipc::{Command, ErrorCode, Event};

use crate::state::State;

use super::Ctx;

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
) -> Vec<Event> {
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
    // when the tree is wiped, focused/
    // magnified ids would point at non-existent nodes. Match
    // `handle_layout_clear`'s contract for the empty-tree case.
    if new_tree.is_none() {
        tab.focused_node_id = String::new();
        tab.magnified_node_id = String::new();
    }
    let v = state.bump_version();
    vec![Event::LayoutTreeReplaced {
        tab_id,
        new_tree,
        correlation_id,
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

    let v = state.bump_version();
    vec![Event::LayoutNodeDeleted {
        tab_id,
        node_id,
        was_focused,
        was_magnified,
        correlation_id,
        version: v,
    }]
}
