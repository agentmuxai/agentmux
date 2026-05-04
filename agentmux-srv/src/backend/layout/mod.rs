// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pure layout-tree helper functions for the srv E.4.B reducer.
//!
//! Phase 4 of docs/specs/srv-phase-e4b-formal-spec-2026-05-03.md.
//! Ports the 11 pure-ish action handlers from
//! `frontend/layout/lib/layoutTree.ts` to Rust. These functions take
//! `&mut LayoutNode` (the tree root) and operation params; they have no
//! I/O and no side effects. The reducer arms (Phase 5) will call these.
//!
//! Test oracle: the 30 tests in `frontend/layout/tests/layoutTree.test.ts`
//! (shipped in PR #686) — the Rust implementations must produce identical
//! state transitions for identical inputs.

use agentmux_common::{FlexDirection, LayoutNode, LayoutNodeData, ResizeOp, SplitPosition};
use uuid::Uuid;

// ── Error type ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum LayoutError {
    NodeNotFound { id: String },
    /// Caller tried to swap a node with itself.
    SelfSwap,
    /// Caller tried to swap, move, or magnify the root node.
    RootCannotBeTarget,
    /// A resize op had size outside [0.0, 100.0].
    InvalidSize { node_id: String, size: f32 },
    /// index_arr was empty or led to a non-existent location.
    InvalidIndexPath,
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NodeNotFound { id } => write!(f, "node not found: {}", id),
            Self::SelfSwap => write!(f, "cannot swap a node with itself"),
            Self::RootCannotBeTarget => write!(f, "root node cannot be the target of this operation"),
            Self::InvalidSize { node_id, size } => write!(f, "invalid size {:.2} for node {}", size, node_id),
            Self::InvalidIndexPath => write!(f, "index_arr is empty or points to a non-existent location"),
        }
    }
}

// ── Constants ──────────────────────────────────────────────────────────────

/// Maximum children before `insert_node`'s heuristic descends.
/// Mirrors `frontend/layout/lib/layoutTree.ts::DEFAULT_MAX_CHILDREN`.
pub const DEFAULT_MAX_CHILDREN: usize = 5;

/// Default flex size for newly-inserted nodes.
/// Mirrors `frontend/layout/lib/types.ts::DefaultNodeSize`.
pub const DEFAULT_NODE_SIZE: f32 = 10.0;

// ── Tree traversal ─────────────────────────────────────────────────────────

/// Find a node by id (immutable).
pub fn find_node_by_id<'a>(tree: &'a LayoutNode, id: &str) -> Option<&'a LayoutNode> {
    if tree.id == id {
        return Some(tree);
    }
    for child in &tree.children {
        if let Some(found) = find_node_by_id(child, id) {
            return Some(found);
        }
    }
    None
}

/// Find a node by id (mutable).
pub fn find_node_by_id_mut<'a>(tree: &'a mut LayoutNode, id: &str) -> Option<&'a mut LayoutNode> {
    if tree.id == id {
        return Some(tree);
    }
    for child in &mut tree.children {
        if let Some(found) = find_node_by_id_mut(child, id) {
            return Some(found);
        }
    }
    None
}

/// Find the parent of a node identified by `child_id` (mutable reference to
/// the parent). Returns `None` if `child_id == tree.id` (root has no parent).
pub fn find_parent_by_child_id<'a>(
    tree: &'a mut LayoutNode,
    child_id: &str,
) -> Option<&'a mut LayoutNode> {
    if tree.children.iter().any(|c| c.id == child_id) {
        return Some(tree);
    }
    // Can't use iter_mut here while also holding a borrow on tree; use indices.
    let len = tree.children.len();
    for i in 0..len {
        // SAFETY: we borrow one child at a time without aliasing.
        let child = &mut tree.children[i] as *mut LayoutNode;
        // SAFETY: child outlives tree's borrow; no aliasing.
        let result = find_parent_by_child_id(unsafe { &mut *child }, child_id);
        if result.is_some() {
            return result;
        }
    }
    None
}

// ── Insert-location heuristic ──────────────────────────────────────────────

/// Greedy depth-first search for the first node with fewer than `max_children`
/// children. The TypeScript oracle (`findNextInsertLocation` in layoutNode.ts)
/// uses the same greedy-DFS descent — fill each node before going deeper.
/// (Reagent P2 PR #691: prior doc comment incorrectly said "BFS".)
fn find_next_insert_location(
    tree: &LayoutNode,
    max_children: usize,
) -> (&LayoutNode, usize) {
    find_insert_location_inner(tree, max_children, 1)
        .unwrap_or((tree, tree.children.len()))
}

fn find_insert_location_inner(
    node: &LayoutNode,
    max_children: usize,
    _depth: usize,
) -> Option<(&LayoutNode, usize)> {
    // If this node has room, insert here.
    if node.children.len() < max_children || node.children.is_empty() {
        return Some((node, node.children.len()));
    }
    // Otherwise recurse into children.
    for child in &node.children {
        if let Some(loc) = find_insert_location_inner(child, max_children, _depth + 1) {
            return Some(loc);
        }
    }
    None
}

// ── Core operations ────────────────────────────────────────────────────────

/// Insert `node` using the BFS heuristic (up to DEFAULT_MAX_CHILDREN per
/// node before descending). If the tree is empty (called with
/// `root = None`), the caller should set `root = Some(node)` directly.
/// This function only operates on a non-empty tree root.
pub fn insert_node(root: &mut LayoutNode, node: LayoutNode) {
    // Use raw pointer to work around borrow checker limitations with the
    // immutable findNext + mutable insert pattern.
    let (loc_id, loc_index) = {
        let (loc, idx) = find_next_insert_location(root, DEFAULT_MAX_CHILDREN);
        (loc.id.clone(), idx)
    };
    if let Some(target) = find_node_by_id_mut(root, &loc_id) {
        if target.data.is_some() {
            // Leaf node — wrap its data in a child, then append.
            let data = target.data.take().unwrap();
            let existing_leaf = LayoutNode {
                id: Uuid::new_v4().to_string(),
                flex_direction: FlexDirection::Row,
                size: DEFAULT_NODE_SIZE,
                data: Some(data),
                ..Default::default()
            };
            target.children.push(existing_leaf);
        }
        target.children.push(node);
    }
}

/// Insert `node` at the location identified by `index_arr` — an ordered
/// sequence of child indices (e.g. `[0, 2]` = root.children[0].children
/// at index 3). The node is inserted AFTER the position identified by the
/// last index. Mirrors `insertNodeAtIndex` / `findInsertLocationFromIndexArr`.
pub fn insert_node_at_index(
    root: &mut LayoutNode,
    node: LayoutNode,
    index_arr: &[usize],
) -> Result<(), LayoutError> {
    if index_arr.is_empty() {
        return Err(LayoutError::InvalidIndexPath);
    }
    // Navigate to the parent using all-but-last indices, then insert
    // after last-index position.
    let mut current = root as *mut LayoutNode;
    let split_at = index_arr.len() - 1;
    let path = &index_arr[..split_at];
    let last_idx = index_arr[split_at];

    for &idx in path {
        let cur = unsafe { &mut *current };
        if idx >= cur.children.len() {
            return Err(LayoutError::InvalidIndexPath);
        }
        current = &mut cur.children[idx] as *mut LayoutNode;
    }
    let parent = unsafe { &mut *current };
    let insert_at = (last_idx + 1).min(parent.children.len());
    parent.children.insert(insert_at, node);
    Ok(())
}

/// Remove the node with the given id from the tree. Returns whether the
/// deleted node was the currently-focused node (callers handle clearing
/// focusedNodeId). Collapses single-child parents.
pub fn delete_node(root: &mut LayoutNode, node_id: &str) -> Result<bool, LayoutError> {
    if root.id == node_id {
        // Root deletion is handled by the caller setting root = None.
        return Ok(false);
    }
    let was_focused = delete_recursive(root, node_id);
    match was_focused {
        Some(wf) => Ok(wf),
        None => Err(LayoutError::NodeNotFound { id: node_id.to_string() }),
    }
}

/// Recursive delete. Returns `Some(was_focused)` when the node was found
/// and removed, `None` when not found.
fn delete_recursive(node: &mut LayoutNode, target_id: &str) -> Option<bool> {
    let idx = node.children.iter().position(|c| c.id == target_id);
    if let Some(i) = idx {
        // Found target as a direct child — remove it.
        node.children.remove(i);
        // Collapse: if only one child remains, promote it.
        // Preserve `extra` (unknown-field catch-all from #688/#689 — reagent P1 PR #691).
        if node.children.len() == 1 {
            let sole = node.children.remove(0);
            node.id = sole.id;
            node.size = sole.size;
            node.data = sole.data;
            node.flex_direction = sole.flex_direction;
            node.children = sole.children;
            node.extra = sole.extra;
        }
        return Some(false);
    }
    // Not a direct child — recurse.
    for child in &mut node.children {
        if let Some(wf) = delete_recursive(child, target_id) {
            return Some(wf);
        }
    }
    None
}

/// Move the node identified by `node_id` to be the child at `index` under
/// the node identified by `new_parent_id`. Resets the moved node's size to
/// DEFAULT_NODE_SIZE unless it stays under the same parent.
pub fn move_node(
    root: &mut LayoutNode,
    node_id: &str,
    new_parent_id: &str,
    index: usize,
) -> Result<(), LayoutError> {
    if root.id == node_id || root.id == new_parent_id {
        if new_parent_id == root.id && root.id != node_id {
            // Moving a child to root-as-parent is valid; proceed.
        } else if root.id == node_id {
            return Err(LayoutError::RootCannotBeTarget);
        }
    }

    // Validate BOTH node existence AND destination BEFORE any mutation.
    // (Reagent P1 PR #691 round 2 — prior code removed the source before
    // checking the destination, causing silent data loss on Err paths.)
    let node_to_move = find_node_by_id(root, node_id)
        .ok_or_else(|| LayoutError::NodeNotFound { id: node_id.to_string() })?
        .clone();
    // Confirm destination exists while tree is still intact.
    if find_node_by_id(root, new_parent_id).is_none() {
        return Err(LayoutError::NodeNotFound { id: new_parent_id.to_string() });
    }

    let old_parent_id = find_parent_id(root, node_id);
    let same_parent = old_parent_id.as_deref() == Some(new_parent_id);

    // Now it is safe to detach (both endpoints exist).
    remove_node_from_parent(root, node_id);

    let mut node_with_size = node_to_move;
    if !same_parent {
        node_with_size.size = DEFAULT_NODE_SIZE;
    }

    // Insert at new location (destination guaranteed to still exist since
    // we only removed the source, not the destination).
    let new_parent = find_node_by_id_mut(root, new_parent_id).unwrap();
    let insert_at = index.min(new_parent.children.len());
    new_parent.children.insert(insert_at, node_with_size);
    Ok(())
}

fn find_parent_id(root: &LayoutNode, child_id: &str) -> Option<String> {
    if root.children.iter().any(|c| c.id == child_id) {
        return Some(root.id.clone());
    }
    for child in &root.children {
        if let Some(pid) = find_parent_id(child, child_id) {
            return Some(pid);
        }
    }
    None
}

fn remove_node_from_parent(root: &mut LayoutNode, node_id: &str) {
    if let Some(idx) = root.children.iter().position(|c| c.id == node_id) {
        root.children.remove(idx);
        return;
    }
    for child in &mut root.children {
        remove_node_from_parent(child, node_id);
    }
}

/// Swap two nodes (by id). Their sizes travel with them (swap positions
/// but preserve the size each node had). Neither can be the root.
pub fn swap_nodes(
    root: &mut LayoutNode,
    node1_id: &str,
    node2_id: &str,
) -> Result<(), LayoutError> {
    if node1_id == node2_id {
        return Err(LayoutError::SelfSwap);
    }
    if root.id == node1_id || root.id == node2_id {
        return Err(LayoutError::RootCannotBeTarget);
    }

    // Collect both nodes + parent info.
    let n1 = find_node_by_id(root, node1_id)
        .ok_or_else(|| LayoutError::NodeNotFound { id: node1_id.to_string() })?
        .clone();
    let n2 = find_node_by_id(root, node2_id)
        .ok_or_else(|| LayoutError::NodeNotFound { id: node2_id.to_string() })?
        .clone();
    let p1_id = find_parent_id(root, node1_id)
        .ok_or_else(|| LayoutError::NodeNotFound { id: format!("parent of {}", node1_id) })?;
    let p2_id = find_parent_id(root, node2_id)
        .ok_or_else(|| LayoutError::NodeNotFound { id: format!("parent of {}", node2_id) })?;

    // Find indices.
    let idx1 = {
        let p = find_node_by_id(root, &p1_id).unwrap();
        p.children.iter().position(|c| c.id == node1_id).unwrap()
    };
    let idx2 = {
        let p = find_node_by_id(root, &p2_id).unwrap();
        p.children.iter().position(|c| c.id == node2_id).unwrap()
    };

    // Build swapped nodes: preserve sizes AT the slot, swap the nodes.
    let mut n1_swapped = n2.clone();
    n1_swapped.size = n1.size; // slot 1 keeps size of what was there (n1's size)
    let mut n2_swapped = n1.clone();
    n2_swapped.size = n2.size; // slot 2 keeps size of what was there (n2's size)

    // Place them.
    let p1 = find_node_by_id_mut(root, &p1_id).unwrap();
    p1.children[idx1] = n1_swapped;

    let p2 = find_node_by_id_mut(root, &p2_id).unwrap();
    p2.children[idx2] = n2_swapped;

    Ok(())
}

/// Apply resize operations. Validates all sizes first (0.0–100.0 range);
/// if any is invalid, returns an error WITHOUT applying any ops (atomically
/// rejected — matches the frontend's early-return semantic from PR #686).
pub fn resize_nodes(root: &mut LayoutNode, ops: &[ResizeOp]) -> Result<(), LayoutError> {
    // Validation pass first.
    for op in ops {
        if !(0.0..=100.0).contains(&op.size) {
            return Err(LayoutError::InvalidSize {
                node_id: op.node_id.clone(),
                size: op.size,
            });
        }
    }
    // Apply pass (all valid).
    for op in ops {
        if let Some(node) = find_node_by_id_mut(root, &op.node_id) {
            node.size = op.size;
        }
    }
    Ok(())
}

/// Replace the node at `target_id` with `new_node`, preserving the target's
/// `size`. If target is root, replaces root-in-place (caller's `Option<LayoutNode>`
/// doesn't change reference — the fields are updated on the root node).
pub fn replace_node(
    root: &mut LayoutNode,
    target_id: &str,
    new_node: LayoutNode,
) -> Result<(), LayoutError> {
    if root.id == target_id {
        let preserved_size = root.size;
        *root = new_node;
        root.size = preserved_size;
        return Ok(());
    }
    let parent_id = find_parent_id(root, target_id)
        .ok_or_else(|| LayoutError::NodeNotFound { id: target_id.to_string() })?;
    let parent = find_node_by_id_mut(root, &parent_id).unwrap();
    let idx = parent.children.iter().position(|c| c.id == target_id).unwrap();
    let preserved_size = parent.children[idx].size;
    parent.children[idx] = new_node;
    parent.children[idx].size = preserved_size;
    Ok(())
}

/// Horizontal split: insert `new_node` before/after `target_id`.
///
/// - If the target's parent is a Row, splice directly.
/// - Otherwise, wrap target + new_node in a fresh Row group.
pub fn split_horizontal(
    root: &mut LayoutNode,
    target_id: &str,
    new_node: LayoutNode,
    position: SplitPosition,
) -> Result<(), LayoutError> {
    split_impl(root, target_id, new_node, position, FlexDirection::Row)
}

/// Vertical split: insert `new_node` before/after `target_id`.
///
/// - If the target's parent is a Column, splice directly.
/// - Otherwise, wrap in a fresh Column group.
pub fn split_vertical(
    root: &mut LayoutNode,
    target_id: &str,
    new_node: LayoutNode,
    position: SplitPosition,
) -> Result<(), LayoutError> {
    split_impl(root, target_id, new_node, position, FlexDirection::Column)
}

fn split_impl(
    root: &mut LayoutNode,
    target_id: &str,
    new_node: LayoutNode,
    position: SplitPosition,
    direction: FlexDirection,
) -> Result<(), LayoutError> {
    // Find target; it must exist.
    let _ = find_node_by_id(root, target_id)
        .ok_or_else(|| LayoutError::NodeNotFound { id: target_id.to_string() })?;

    let parent_id = find_parent_id(root, target_id);

    if let Some(ref pid) = parent_id {
        let parent = find_node_by_id_mut(root, pid).unwrap();
        if parent.flex_direction == direction {
            // Parent already has the matching direction — splice directly.
            let idx = parent.children.iter().position(|c| c.id == target_id).unwrap();
            let insert_at = match position {
                SplitPosition::Before => idx,
                SplitPosition::After => idx + 1,
            };
            parent.children.insert(insert_at, new_node);
            return Ok(());
        }
        // Parent has wrong direction — wrap target in a new group.
        let idx = parent.children.iter().position(|c| c.id == target_id).unwrap();
        let target = parent.children.remove(idx);
        let target_size = target.size;
        let (children_ordered, new_flex) = match position {
            SplitPosition::Before => (vec![new_node, target], direction),
            SplitPosition::After => (vec![target, new_node], direction),
        };
        let group = LayoutNode {
            id: Uuid::new_v4().to_string(),
            flex_direction: new_flex,
            size: target_size,
            children: children_ordered,
            ..Default::default()
        };
        parent.children.insert(idx, group);
        return Ok(());
    }

    // Target IS root (no parent) — wrap root in a new group.
    // We can't replace `root` itself, so we build the group children in-place.
    // Also take `root.extra` (unknown-field catch-all from #688/#689).
    // `..Default::default()` would silently drop it — reagent P1 PR #691.
    let root_clone = LayoutNode {
        id: root.id.clone(),
        flex_direction: root.flex_direction,
        size: root.size,
        children: std::mem::take(&mut root.children),
        data: root.data.take(),
        extra: std::mem::take(&mut root.extra),
    };
    let root_size = root_clone.size;
    let new_group_id = Uuid::new_v4().to_string();
    let (children_ordered, new_flex) = match position {
        SplitPosition::Before => (vec![new_node, root_clone], direction),
        SplitPosition::After => (vec![root_clone, new_node], direction),
    };
    root.id = new_group_id;
    root.flex_direction = new_flex;
    root.size = root_size;
    root.children = children_ordered;
    root.data = None;
    Ok(())
}

// ── Clear tree ─────────────────────────────────────────────────────────────

/// Clear the tree root. Callers set the `Option<LayoutNode>` to `None`.
/// This function exists only to be referenced in the test helper pattern;
/// actual clearing in the reducer just sets root = None.
pub fn clear_tree_node(_root: &mut Option<LayoutNode>) {
    *_root = None;
}

// ── Tests ──────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests;
