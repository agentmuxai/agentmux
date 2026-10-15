// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Layout intents applied to a tree: a step-for-step port of the frontend's
//! tree actions (`frontend/layout/lib/layoutTree.ts`, with `addChildAt` and
//! `addIntermediateNode` from `layoutNode.ts`), followed by the same
//! `balance_node` the frontend runs after every action. The frontend applies
//! an action locally as a prediction and srv's result is the one that is
//! stored, so the two must agree: `intent_vectors` replays one shared set of
//! random sequences through both. Where the frontend would corrupt the tree
//! (a move into the moved node's own subtree, a swap of a node with its
//! ancestor) or apply half an action (an invalid resize), srv refuses instead.
//! SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30.md §4.

use agentmux_common::{FlexDirection, LayoutIntent, LayoutNode, LayoutNodeData, LayoutResizeOp, SplitPosition};

use super::{
    balance_node, collect_insert_candidates, find_node_by_id, find_node_by_id_mut, find_parent_id,
    is_effectively_minimized, LayoutError, DEFAULT_MAX_CHILDREN, DEFAULT_NODE_SIZE,
};

/// The id of the `n`th node (from 1) an intent creates: the action id with its
/// first eight hex digits replaced by `n`, or `<action id>-<n>` when the action
/// id isn't a UUID. The frontend derives the same ids (`deriveNodeId`,
/// `layoutNode.ts`), so the branches both sides create are named alike.
pub fn derive_node_id(action_id: &str, n: u32) -> String {
    if action_id.len() >= 9 && action_id.as_bytes()[8] == b'-' && action_id.is_char_boundary(8) {
        format!("{:08x}{}", n, &action_id[8..])
    } else {
        format!("{action_id}-{n}")
    }
}

/// Mints the ids for the nodes one intent creates, in creation order.
pub struct NodeIds {
    action_id: String,
    used: u32,
}

impl NodeIds {
    pub fn new(action_id: &str) -> Self {
        Self { action_id: action_id.to_string(), used: 0 }
    }

    fn next(&mut self) -> String {
        self.used += 1;
        derive_node_id(&self.action_id, self.used)
    }
}

/// What an intent does to focus and magnify, as the frontend's action
/// functions write them. Applied only when the sender didn't say which pane it
/// has focused (srv's own placements).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ViewEffect {
    pub focus: Option<String>,
    pub magnify: Option<String>,
    /// Clear focus if it is this node (a delete).
    pub unfocus: Option<String>,
    /// Clear focus and magnify (a clear).
    pub clear: bool,
}

/// Apply `intent` to `root`, then normalize it (`balance_node`). On an error
/// `root` may be half-edited: callers apply to a copy.
pub fn apply_intent(
    root: &mut Option<LayoutNode>,
    intent: LayoutIntent,
    ids: &mut NodeIds,
) -> Result<ViewEffect, LayoutError> {
    let effect = apply_unbalanced(root, intent, ids)?;
    if let Some(tree) = root.as_mut() {
        balance_node(tree)?;
    }
    Ok(effect)
}

fn apply_unbalanced(
    root: &mut Option<LayoutNode>,
    intent: LayoutIntent,
    ids: &mut NodeIds,
) -> Result<ViewEffect, LayoutError> {
    let mut effect = ViewEffect::default();
    match intent {
        LayoutIntent::Insert { node, magnified, focused } => {
            let node_id = node.id.clone();
            let was_empty = root.is_none();
            insert_node(root, node, ids)?;
            set_insert_effect(&mut effect, node_id, was_empty, magnified, focused);
        }
        LayoutIntent::InsertAtIndex { node, index_arr, magnified, focused } => {
            let node_id = node.id.clone();
            let was_empty = root.is_none();
            insert_node_at_index(root, node, &index_arr, ids)?;
            set_insert_effect(&mut effect, node_id, was_empty, magnified, focused);
        }
        LayoutIntent::Delete { node_id } => {
            delete_node(root, &node_id)?;
            effect.unfocus = Some(node_id);
        }
        LayoutIntent::Move { node_id, parent_id, index, insert_at_root } => {
            if insert_at_root {
                // Only a drop onto a single-pane root produces it, and then
                // nothing else is in the tree to move.
                return Err(LayoutError::Refused("move: insertAtRoot is not supported".into()));
            }
            let parent_id = parent_id.ok_or_else(|| LayoutError::Refused("move: no parentId".into()))?;
            move_node(tree_mut(root)?, &node_id, &parent_id, index, ids)?;
        }
        LayoutIntent::Swap { node1_id, node2_id } => swap_nodes(tree_mut(root)?, &node1_id, &node2_id)?,
        LayoutIntent::Resize { resize_operations } => resize_nodes(tree_mut(root)?, &resize_operations)?,
        LayoutIntent::Replace { target_node_id, new_node, focused } => {
            let new_id = new_node.id.clone();
            replace_node(tree_mut(root)?, &target_node_id, new_node)?;
            if focused {
                effect.focus = Some(new_id);
            }
        }
        LayoutIntent::SplitHorizontal { target_node_id, new_node, position, size_fraction, focused } => {
            let new_id = new_node.id.clone();
            split(root, &target_node_id, new_node, position, size_fraction, FlexDirection::Row, ids)?;
            if focused {
                effect.focus = Some(new_id);
            }
        }
        LayoutIntent::SplitVertical { target_node_id, new_node, position, size_fraction, focused } => {
            let new_id = new_node.id.clone();
            split(root, &target_node_id, new_node, position, size_fraction, FlexDirection::Column, ids)?;
            if focused {
                effect.focus = Some(new_id);
            }
        }
        LayoutIntent::Clear => {
            *root = None;
            effect.clear = true;
        }
        LayoutIntent::View => {}
    }
    Ok(effect)
}

/// `insertNode`'s focus/magnify writes: magnify only into a non-empty tree.
fn set_insert_effect(effect: &mut ViewEffect, node_id: String, was_empty: bool, magnified: bool, focused: bool) {
    if magnified && !was_empty {
        effect.magnify = Some(node_id.clone());
        effect.focus = Some(node_id.clone());
    }
    if focused {
        effect.focus = Some(node_id);
    }
}

fn tree_mut(root: &mut Option<LayoutNode>) -> Result<&mut LayoutNode, LayoutError> {
    root.as_mut().ok_or_else(|| LayoutError::Refused("the layout is empty".into()))
}

fn not_found(id: &str) -> LayoutError {
    LayoutError::NodeNotFound { id: id.to_string() }
}

fn reverse(dir: FlexDirection) -> FlexDirection {
    match dir {
        FlexDirection::Row => FlexDirection::Column,
        FlexDirection::Column => FlexDirection::Row,
    }
}

fn new_node(
    flex_direction: FlexDirection,
    size: f32,
    children: Vec<LayoutNode>,
    data: Option<LayoutNodeData>,
    ids: &mut NodeIds,
) -> LayoutNode {
    LayoutNode { id: ids.next(), flex_direction, size, children, data, extra: Default::default() }
}

// ── layoutNode.ts primitives ────────────────────────────────────────────────

/// `addChildAt`: a child with the parent's direction gives up its own
/// children to the parent (a branch) or flips (a leaf).
fn add_child_at(node: &mut LayoutNode, idx: i64, mut child: LayoutNode, ids: &mut NodeIds) {
    if node.children.is_empty() {
        add_intermediate_node(node, ids);
    }
    let to_add = if child.flex_direction != node.flex_direction {
        vec![child]
    } else if !child.children.is_empty() {
        child.children
    } else {
        child.flex_direction = reverse(node.flex_direction);
        vec![child]
    };
    if node.children.len() as i64 <= idx {
        node.children.extend(to_add);
    } else if idx >= 0 {
        let at = idx as usize;
        node.children.splice(at..at, to_add);
    }
}

/// `addIntermediateNode`: push a node's data (or children) one level down.
/// The new child takes the node's id and the node a fresh one, so whatever
/// pointed at the node (focus) now points at its content.
fn add_intermediate_node(node: &mut LayoutNode, ids: &mut NodeIds) {
    let mut intermediate = if node.data.is_some() {
        let mut leaf = new_node(reverse(node.flex_direction), DEFAULT_NODE_SIZE, Vec::new(), node.data.take(), ids);
        // `minimized` belongs to the content, not the branch it becomes.
        if let Some(flag) = node.extra.remove("minimized") {
            leaf.extra.insert("minimized".into(), flag);
        }
        leaf
    } else {
        let inner = new_node(node.flex_direction, DEFAULT_NODE_SIZE, std::mem::take(&mut node.children), None, ids);
        new_node(reverse(node.flex_direction), DEFAULT_NODE_SIZE, vec![inner], None, ids)
    };
    std::mem::swap(&mut intermediate.id, &mut node.id);
    node.children = vec![intermediate];
}

/// `removeChild`: the first child with this id at or after `starting`.
fn remove_child(parent: &mut LayoutNode, child_id: &str, starting: usize) {
    if let Some(i) = parent.children.iter().skip(starting).position(|c| c.id == child_id) {
        parent.children.remove(starting + i);
    }
}

// ── layoutTree.ts actions ───────────────────────────────────────────────────

fn insert_node(root: &mut Option<LayoutNode>, node: LayoutNode, ids: &mut NodeIds) -> Result<(), LayoutError> {
    let Some(tree) = root.as_mut() else {
        *root = Some(node);
        return Ok(());
    };
    let (loc_id, index) = next_insert_location(tree)
        .ok_or_else(|| LayoutError::Refused("insert: no unlocked insert location".into()))?;
    let loc = find_node_by_id_mut(tree, &loc_id).ok_or_else(|| not_found(&loc_id))?;
    add_child_at(loc, index as i64, node, ids);
    Ok(())
}

/// `findNextInsertLocation`: the lowest-scoring slot, ties to the first found.
fn next_insert_location(tree: &LayoutNode) -> Option<(String, usize)> {
    let mut candidates = Vec::new();
    collect_insert_candidates(tree, DEFAULT_MAX_CHILDREN, 1, &mut candidates);
    let score = |depth: usize, index: usize| (depth as f64).powi((index + DEFAULT_MAX_CHILDREN) as i32);
    candidates
        .iter()
        .enumerate()
        .min_by(|(ia, a), (ib, b)| {
            score(a.depth, a.index)
                .partial_cmp(&score(b.depth, b.index))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(ia.cmp(ib))
        })
        .map(|(_, c)| (c.node.id.clone(), c.index))
}

fn insert_node_at_index(
    root: &mut Option<LayoutNode>,
    node: LayoutNode,
    index_arr: &[i64],
    ids: &mut NodeIds,
) -> Result<(), LayoutError> {
    let Some(tree) = root.as_mut() else {
        *root = Some(node);
        return Ok(());
    };
    let (loc_id, index) = insert_location_from_index_arr(tree, index_arr).ok_or(LayoutError::InvalidIndexPath)?;
    let loc = find_node_by_id_mut(tree, &loc_id).ok_or_else(|| not_found(&loc_id))?;
    if is_effectively_minimized(loc) {
        return Err(LayoutError::NodeLocked { id: loc_id });
    }
    add_child_at(loc, index + 1, node, ids);
    Ok(())
}

/// `findInsertLocationFromIndexArr`, including its arithmetic for negative
/// indices. A path that leaves the tree is refused.
fn insert_location_from_index_arr(node: &LayoutNode, index_arr: &[i64]) -> Option<(String, i64)> {
    let (&first, rest) = index_arr.split_first()?;
    let len = node.children.len().max(1) as i64;
    let next = if first < 0 { len - first.max(-len) } else { first.min(len - 1) };
    if rest.is_empty() || node.children.is_empty() {
        return Some((node.id.clone(), next));
    }
    let child = node.children.get(usize::try_from(next).ok()?)?;
    insert_location_from_index_arr(child, rest)
}

/// `deleteNode`: unlink the node. The parent it leaves is tidied by the
/// balance pass that follows, not here.
fn delete_node(root: &mut Option<LayoutNode>, node_id: &str) -> Result<(), LayoutError> {
    let tree = root.as_mut().ok_or_else(|| not_found(node_id))?;
    if tree.id == node_id {
        *root = None;
        return Ok(());
    }
    let parent_id = find_parent_id(tree, node_id).ok_or_else(|| not_found(node_id))?;
    let parent = find_node_by_id_mut(tree, &parent_id).ok_or_else(|| not_found(&parent_id))?;
    remove_child(parent, node_id, 0);
    Ok(())
}

/// The `delete` intent on a tree whose root stays: unlink the node, then
/// balance (what the frontend does when a move empties a pane).
pub fn delete_and_balance(root: &mut LayoutNode, node_id: &str) -> Result<(), LayoutError> {
    if root.id == node_id {
        return Err(LayoutError::RootCannotBeTarget);
    }
    let parent_id = find_parent_id(root, node_id).ok_or_else(|| not_found(node_id))?;
    let parent = find_node_by_id_mut(root, &parent_id).ok_or_else(|| not_found(&parent_id))?;
    remove_child(parent, node_id, 0);
    balance_node(root)
}

/// `moveNode`: insert the node at its new place, then remove it from the old
/// one, which leaves a same-parent move one slot earlier when it moves right.
fn move_node(tree: &mut LayoutNode, node_id: &str, parent_id: &str, index: i64, ids: &mut NodeIds) -> Result<(), LayoutError> {
    if tree.id == node_id {
        return Err(LayoutError::RootCannotBeTarget);
    }
    let mut node = find_node_by_id(tree, node_id).ok_or_else(|| not_found(node_id))?.clone();
    let parent = find_node_by_id(tree, parent_id).ok_or_else(|| not_found(parent_id))?;
    if is_effectively_minimized(&node) {
        return Err(LayoutError::NodeLocked { id: node_id.to_string() });
    }
    if is_effectively_minimized(parent) {
        return Err(LayoutError::NodeLocked { id: parent_id.to_string() });
    }
    if find_node_by_id(&node, parent_id).is_some() {
        return Err(LayoutError::Refused(format!("move: {parent_id} is inside the moved node {node_id}")));
    }
    if index < 0 {
        return Err(LayoutError::Refused(format!("move: negative index {index}")));
    }
    let old_parent_id = find_parent_id(tree, node_id).ok_or_else(|| not_found(node_id))?;
    let mut starting = 0;
    if old_parent_id == parent_id {
        let current = parent.children.iter().position(|c| c.id == node_id).unwrap_or(0) as i64;
        if current >= index {
            starting = index as usize + 1;
        }
    } else {
        node.size = DEFAULT_NODE_SIZE;
    }
    let parent = find_node_by_id_mut(tree, parent_id).ok_or_else(|| not_found(parent_id))?;
    add_child_at(parent, index, node, ids);
    let old_parent = find_node_by_id_mut(tree, &old_parent_id).ok_or_else(|| not_found(&old_parent_id))?;
    remove_child(old_parent, node_id, starting);
    Ok(())
}

/// `swapNode`: exchange two nodes; each slot keeps its size.
fn swap_nodes(tree: &mut LayoutNode, node1_id: &str, node2_id: &str) -> Result<(), LayoutError> {
    if node1_id == tree.id || node2_id == tree.id {
        return Err(LayoutError::RootCannotBeTarget);
    }
    if node1_id == node2_id {
        return Err(LayoutError::SelfSwap);
    }
    let node1 = find_node_by_id(tree, node1_id).ok_or_else(|| not_found(node1_id))?.clone();
    let node2 = find_node_by_id(tree, node2_id).ok_or_else(|| not_found(node2_id))?.clone();
    for (id, node) in [(node1_id, &node1), (node2_id, &node2)] {
        if is_effectively_minimized(node) {
            return Err(LayoutError::NodeLocked { id: id.to_string() });
        }
    }
    if find_node_by_id(&node1, node2_id).is_some() || find_node_by_id(&node2, node1_id).is_some() {
        return Err(LayoutError::Refused("swap: one node contains the other".into()));
    }
    let slot1 = slot_of(tree, node1_id)?;
    let slot2 = slot_of(tree, node2_id)?;
    let (size1, size2) = (node1.size, node2.size);
    let (mut into_slot1, mut into_slot2) = (node2, node1);
    into_slot1.size = size1;
    into_slot2.size = size2;
    // By index, found before either write: with one parent, the first write
    // leaves two children with node2's id.
    place(tree, &slot1, into_slot1)?;
    place(tree, &slot2, into_slot2)
}

/// A node's parent id and its index there.
fn slot_of(tree: &LayoutNode, node_id: &str) -> Result<(String, usize), LayoutError> {
    let parent_id = find_parent_id(tree, node_id).ok_or_else(|| not_found(node_id))?;
    let parent = find_node_by_id(tree, &parent_id).ok_or_else(|| not_found(&parent_id))?;
    let index = parent.children.iter().position(|c| c.id == node_id).ok_or_else(|| not_found(node_id))?;
    Ok((parent_id, index))
}

fn place(tree: &mut LayoutNode, (parent_id, index): &(String, usize), node: LayoutNode) -> Result<(), LayoutError> {
    let parent = find_node_by_id_mut(tree, parent_id).ok_or_else(|| not_found(parent_id))?;
    let slot = parent.children.get_mut(*index).ok_or_else(|| not_found(parent_id))?;
    *slot = node;
    Ok(())
}

/// `resizeNode`, all or nothing: every op is checked before any applies.
fn resize_nodes(tree: &mut LayoutNode, ops: &[LayoutResizeOp]) -> Result<(), LayoutError> {
    for op in ops {
        let node = find_node_by_id(tree, &op.node_id).ok_or_else(|| not_found(&op.node_id))?;
        if is_effectively_minimized(node) {
            return Err(LayoutError::NodeLocked { id: op.node_id.clone() });
        }
        if op.node_id.is_empty() || !(0.0..=100.0).contains(&op.size) {
            return Err(LayoutError::InvalidSize { node_id: op.node_id.clone(), size: op.size as f32 });
        }
    }
    for op in ops {
        if let Some(node) = find_node_by_id_mut(tree, &op.node_id) {
            node.size = op.size as f32;
        }
    }
    Ok(())
}

/// `replaceNode`: the new node takes the target's slot and size.
fn replace_node(tree: &mut LayoutNode, target_id: &str, mut new_node: LayoutNode) -> Result<(), LayoutError> {
    if tree.id == target_id {
        new_node.size = tree.size;
        *tree = new_node;
        return Ok(());
    }
    let parent_id = find_parent_id(tree, target_id).ok_or_else(|| not_found(target_id))?;
    let parent = find_node_by_id_mut(tree, &parent_id).ok_or_else(|| not_found(&parent_id))?;
    let slot = parent.children.iter_mut().find(|c| c.id == target_id).ok_or_else(|| not_found(target_id))?;
    new_node.size = slot.size;
    *slot = new_node;
    Ok(())
}

/// `splitHorizontal` / `splitVertical`: splice next to the target in a parent
/// already running in `direction`, else wrap the target and the new node in a
/// new `direction` branch that takes the target's old footprint.
fn split(
    root: &mut Option<LayoutNode>,
    target_id: &str,
    mut new_node: LayoutNode,
    position: SplitPosition,
    size_fraction: Option<f64>,
    direction: FlexDirection,
    ids: &mut NodeIds,
) -> Result<(), LayoutError> {
    let tree = tree_mut(root)?;
    let target = find_node_by_id_mut(tree, target_id).ok_or_else(|| not_found(target_id))?;
    if is_effectively_minimized(target) {
        return Err(LayoutError::NodeLocked { id: target_id.to_string() });
    }
    let original_size = target.size;
    if let Some(fraction) = size_fraction {
        // `applySizeFraction`: carve the new node out of the target.
        let fraction = fraction.clamp(0.05, 0.95);
        new_node.size = (fraction * f64::from(original_size)) as f32;
        target.size = ((1.0 - fraction) * f64::from(original_size)) as f32;
    }
    let ordered = |target: LayoutNode, new_node: LayoutNode| match position {
        SplitPosition::Before => vec![new_node, target],
        SplitPosition::After => vec![target, new_node],
    };
    let Some(parent_id) = find_parent_id(tree, target_id) else {
        let target = root.take().expect("checked above");
        *root = Some(new_node_group(direction, original_size, ordered(target, new_node), ids));
        return Ok(());
    };
    let parent = find_node_by_id_mut(tree, &parent_id).ok_or_else(|| not_found(&parent_id))?;
    let index = parent.children.iter().position(|c| c.id == target_id).ok_or_else(|| not_found(target_id))?;
    if parent.flex_direction == direction {
        let at = match position {
            SplitPosition::Before => index,
            SplitPosition::After => index + 1,
        };
        parent.children.insert(at, new_node);
    } else {
        let target = parent.children.remove(index);
        let group = new_node_group(direction, original_size, ordered(target, new_node), ids);
        parent.children.insert(index, group);
    }
    Ok(())
}

fn new_node_group(direction: FlexDirection, size: f32, children: Vec<LayoutNode>, ids: &mut NodeIds) -> LayoutNode {
    new_node(direction, size, children, None, ids)
}

// ── Legacy minimize fields (`rebuildMinimizedSet`, layoutMinimize.ts) ───────

/// Migrate the pre-display-mode minimize fields to the `minimized` flag, as the
/// frontend does when it loads a tree, so srv computes on the tree the
/// frontend sees. Returns how many fields it migrated (0 for a current tree).
pub fn migrate_legacy_minimize(root: &mut LayoutNode) -> usize {
    migrate_node(root, DEFAULT_NODE_SIZE)
}

fn migrate_node(node: &mut LayoutNode, sane_share: f32) -> usize {
    let mut migrated = 0;
    let is_leaf = node.children.is_empty();
    if let Some(original) = node.extra.remove("minimizedSize") {
        if is_leaf {
            if let Some(size) = original.as_f64() {
                node.size = size as f32;
            }
            node.extra.insert("minimized".into(), true.into());
        }
        migrated += 1;
    }
    if node.extra.remove("slipMinimize").is_some() {
        node.size = sane_share;
        if is_leaf {
            node.extra.insert("minimized".into(), true.into());
        }
        migrated += 1;
    }
    if node.extra.remove("columnDissolve").is_some() {
        node.size = sane_share;
        migrated += 1;
    }
    for key in ["minimizedLockedSize", "_slipAnchor"] {
        if node.extra.remove(key).is_some() {
            migrated += 1;
        }
    }
    for i in 0..node.children.len() {
        let share = mean_sibling_size(&node.children, i);
        migrated += migrate_node(&mut node.children[i], share);
    }
    migrated
}

/// The mean size of child `i`'s positive-sized siblings, else the default.
fn mean_sibling_size(children: &[LayoutNode], i: usize) -> f32 {
    let sizes: Vec<f32> = children
        .iter()
        .enumerate()
        .filter(|(j, c)| *j != i && c.size > 0.0)
        .map(|(_, c)| c.size)
        .collect();
    if sizes.is_empty() {
        DEFAULT_NODE_SIZE
    } else {
        sizes.iter().sum::<f32>() / sizes.len() as f32
    }
}

#[cfg(test)]
mod tests;
