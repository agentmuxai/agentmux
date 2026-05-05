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

use agentmux_common::{FlexDirection, LayoutNode, ResizeOp, SplitPosition};
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

// ── Flex-direction utilities ───────────────────────────────────────────────

/// Reverse a flex direction (Row ↔ Column).
/// Mirrors `frontend/layout/lib/layoutNode.ts::reverseFlexDirection`.
fn reverse_flex_direction(dir: FlexDirection) -> FlexDirection {
    match dir {
        FlexDirection::Row => FlexDirection::Column,
        FlexDirection::Column => FlexDirection::Row,
    }
}

/// If `node` is a leaf (has `data`), promote it to a group by wrapping its
/// data in an intermediate child. Mirrors TypeScript `addIntermediateNode`
/// in `frontend/layout/lib/layoutTree.ts`.
///
/// After this call:
/// - `node.data` is `None` and `node.children` is non-empty.
/// - The intermediate child receives the node's ORIGINAL ID so any
///   frontend reference (e.g. `focusedNodeId`) still points at the
///   leaf data after promotion.
/// - The group wrapper gets a fresh UUID, and its flex direction is
///   the reverse of the original leaf's so the new sibling axis is
///   perpendicular to the parent's (matches TS layout semantics).
///
/// No-op when `node` is already a group (no `data`).
fn ensure_group_node(node: &mut LayoutNode) {
    if node.data.is_some() {
        let old_id = node.id.clone();
        let old_flex = node.flex_direction;
        let intermediate = LayoutNode {
            id: old_id,
            flex_direction: reverse_flex_direction(old_flex),
            size: DEFAULT_NODE_SIZE,
            data: node.data.take(),
            ..Default::default()
        };
        node.id = Uuid::new_v4().to_string();
        node.children.push(intermediate);
    }
}

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

// ── Insert-location heuristic ──────────────────────────────────────────────

/// Candidate for insertion location, collected during tree traversal.
struct InsertCandidate<'a> {
    node: &'a LayoutNode,
    index: usize,
    depth: usize,
}

/// Find the best insertion location using the TypeScript scoring heuristic
/// (`Math.pow(depth, index + maxChildren)`). Collects all candidates, then
/// sorts by ascending score. Mirrors `findNextInsertLocation` in
/// `frontend/layout/lib/layoutNode.ts`.
fn find_next_insert_location(
    tree: &LayoutNode,
    max_children: usize,
) -> (&LayoutNode, usize) {
    let mut candidates = Vec::new();
    collect_insert_candidates(tree, max_children, 1, &mut candidates);
    candidates.sort_by(|a, b| {
        let a_score = (a.depth as f64).powi((a.index + max_children) as i32);
        let b_score = (b.depth as f64).powi((b.index + max_children) as i32);
        a_score.partial_cmp(&b_score).unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates
        .into_iter()
        .next()
        .map(|c| (c.node, c.index))
        .unwrap_or((tree, tree.children.len()))
}

fn collect_insert_candidates<'a>(
    node: &'a LayoutNode,
    max_children: usize,
    depth: usize,
    out: &mut Vec<InsertCandidate<'a>>,
) {
    // Leaf node (has data but no children) — TS returns index 1.
    if node.data.is_some() && node.children.is_empty() {
        out.push(InsertCandidate { node, index: 1, depth });
        return;
    }
    if node.children.len() < max_children {
        out.push(InsertCandidate {
            node,
            index: node.children.len(),
            depth,
        });
    }
    // TS iterates children in REVERSE order.
    for child in node.children.iter().rev() {
        collect_insert_candidates(child, max_children, depth + 1, out);
    }
}

// ── Core operations ────────────────────────────────────────────────────────

/// Insert `node` using the TypeScript scoring heuristic (up to
/// DEFAULT_MAX_CHILDREN per node before descending). If the target location
/// is a leaf, it is promoted to a group first via `ensure_group_node`.
/// Mirrors `insertNode` / `addChildAt` in `frontend/layout/lib/layoutTree.ts`.
pub fn insert_node(root: &mut LayoutNode, node: LayoutNode) {
    let loc_id = find_next_insert_location(root, DEFAULT_MAX_CHILDREN).0.id.clone();
    if let Some(target) = find_node_by_id_mut(root, &loc_id) {
        ensure_group_node(target);
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
    ensure_group_node(parent);
    let insert_at = (last_idx + 1).min(parent.children.len());
    parent.children.insert(insert_at, node);
    Ok(())
}

/// Remove the node with the given id from the tree.
/// Returns `Ok(())` on success, or `Err(NodeNotFound)` if `node_id` is not
/// in the tree. Callers are responsible for clearing `focused_node_id` if
/// needed (spec §7.1). Collapses single-child parents.
pub fn delete_node(root: &mut LayoutNode, node_id: &str) -> Result<(), LayoutError> {
    if root.id == node_id {
        // Root deletion is handled by the caller setting root = None.
        return Ok(());
    }
    match delete_recursive(root, node_id) {
        true => Ok(()),
        false => Err(LayoutError::NodeNotFound { id: node_id.to_string() }),
    }
}

/// Recursive delete. Returns `true` when the node was found and removed,
/// `false` when not found.
fn delete_recursive(node: &mut LayoutNode, target_id: &str) -> bool {
    let idx = node.children.iter().position(|c| c.id == target_id);
    if let Some(i) = idx {
        // Found target as a direct child — remove it.
        node.children.remove(i);
        // Collapse: if only one child remains, promote it.
        // Preserve `extra` (unknown-field catch-all from #688/#689).
        if node.children.len() == 1 {
            let sole = node.children.remove(0);
            node.id = sole.id;
            node.size = sole.size;
            node.data = sole.data;
            node.flex_direction = sole.flex_direction;
            node.children = sole.children;
            node.extra = sole.extra;
        }
        return true;
    }
    // Not a direct child — recurse.
    for child in &mut node.children {
        if delete_recursive(child, target_id) {
            return true;
        }
    }
    false
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
    let node_to_move = find_node_by_id(root, node_id)
        .ok_or_else(|| LayoutError::NodeNotFound { id: node_id.to_string() })?
        .clone();
    // Confirm destination exists while tree is still intact.
    if find_node_by_id(root, new_parent_id).is_none() {
        return Err(LayoutError::NodeNotFound { id: new_parent_id.to_string() });
    }
    // Confirm destination is NOT a descendant of the node being moved.
    if find_node_by_id(&node_to_move, new_parent_id).is_some() {
        return Err(LayoutError::NodeNotFound {
            id: format!("{} (is a descendant of the moved node {})", new_parent_id, node_id),
        });
    }

    let old_parent_id = find_parent_id(root, node_id);
    let same_parent = old_parent_id.as_deref() == Some(new_parent_id);

    // Now it is safe to detach (both endpoints exist).
    remove_node_from_parent(root, node_id);

    let mut node_with_size = node_to_move;
    if !same_parent {
        node_with_size.size = DEFAULT_NODE_SIZE;
    }

    // Insert at new location (destination guaranteed to still exist).
    let new_parent = find_node_by_id_mut(root, new_parent_id).unwrap();
    ensure_group_node(new_parent);
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

    // Detect ancestor/descendant relationship. If n1 is an ancestor of n2
    // (or vice versa), the first placement replaces a subtree that contains
    // the second node's parent, making the second lookup fail.
    if find_node_by_id(&n1, node2_id).is_some() || find_node_by_id(&n2, node1_id).is_some() {
        return Err(LayoutError::RootCannotBeTarget);
    }

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

    // Place them. p2_id is guaranteed to still exist (ancestor check above).
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
    // Validation pass 1: sizes.
    for op in ops {
        if !(0.0..=100.0).contains(&op.size) {
            return Err(LayoutError::InvalidSize {
                node_id: op.node_id.clone(),
                size: op.size,
            });
        }
    }
    // Validation pass 2: all target nodes exist.
    for op in ops {
        if find_node_by_id(root, &op.node_id).is_none() {
            return Err(LayoutError::NodeNotFound { id: op.node_id.clone() });
        }
    }
    // Apply pass (all valid).
    for op in ops {
        let node = find_node_by_id_mut(root, &op.node_id).unwrap();
        node.size = op.size;
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
