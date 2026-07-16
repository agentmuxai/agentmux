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
    /// A node failed the leaf-XOR-branch invariant during balance — it has
    /// both `data` and `children`, or neither. The TS oracle (`validateNode`
    /// in `layoutNode.ts`) throws "Invalid node" here.
    InvalidNode { id: String },
    /// The op targets a minimize-locked node (minimized leaf, slipped header,
    /// or dissolved column). Minimized is a locked state — see
    /// `docs/specs/SPEC_LAYOUT_MINIMIZE_LOCKED_STATE_REDESIGN_2026_07_16.md`.
    NodeLocked { id: String },
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NodeNotFound { id } => write!(f, "node not found: {}", id),
            Self::SelfSwap => write!(f, "cannot swap a node with itself"),
            Self::RootCannotBeTarget => write!(f, "root node cannot be the target of this operation"),
            Self::InvalidSize { node_id, size } => write!(f, "invalid size {:.2} for node {}", size, node_id),
            Self::InvalidIndexPath => write!(f, "index_arr is empty or points to a non-existent location"),
            Self::InvalidNode { id } => write!(f, "invalid node (leaf-XOR-branch violated): {}", id),
            Self::NodeLocked { id } => write!(f, "node is minimize-locked: {}", id),
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
            // Move `extra` (forward-compat catch-all from #688/#689) to the
            // intermediate so unknown fields the frontend wrote to the leaf
            // travel with the data, not stay on the new group wrapper.
            // Mirrors the same transfer in `delete_recursive` line below.
            extra: std::mem::take(&mut node.extra),
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

/// Find the id of the (first) leaf node whose `data.block_id` matches
/// `block_id`. SPEC_864 site #6 — lets `LayoutDeleteNodeByBlock` resolve a
/// block to its layout node inside the reducer arm (the reducer owns the
/// tree; callers like the `delete_block` saga only know the block id).
/// Blocks appear at most once in a well-formed tree; first match wins.
pub fn find_node_id_by_block(tree: &LayoutNode, block_id: &str) -> Option<String> {
    if tree
        .data
        .as_ref()
        .is_some_and(|d| d.block_id == block_id)
    {
        return Some(tree.id.clone());
    }
    for child in &tree.children {
        if let Some(found) = find_node_id_by_block(child, block_id) {
            return Some(found);
        }
    }
    None
}

/// Removes every leaf whose `data.block_id` is not in `live_block_ids`,
/// using the same collapse semantics as an explicit user delete
/// (`delete_node`'s single-child-parent promotion). Returns the number of
/// leaves pruned (0 = tree was already clean — the common case, so this is
/// cheap to call unconditionally on every write).
///
/// This is the reducer's referential-integrity enforcement for `db_layout`,
/// added after
/// `docs/investigations/INVESTIGATION_LAYOUT_DEAD_SPACE_STALE_TREE_RESURRECTION_2026_07_08.md`
/// found that a single point-fix (notifying the frontend on one specific
/// delete path) only closes the one mechanism it targets — any other
/// current or future caller that writes a stale/bad tree without going
/// through that same notification channel can still leave a dangling
/// `blockId` behind. Calling this at every layout-tree write site in the
/// reducer (the single writer of `db_layout` since SPEC_864) makes "no
/// layout leaf ever references a nonexistent block" an unconditional
/// invariant of the write path itself, not a property individual callers
/// have to remember to uphold — the same shape as WRR (Window Reality
/// Reconciliation, `agentmux-cef/src/wrr/`) enforcing "no window/pool
/// drift" at the point where reality is observed, rather than trusting
/// every caller to keep the model in sync.
///
/// Re-scans the tree fresh after each removal rather than working off a
/// pre-computed list of dangling node ids, since `delete_node`'s
/// single-child collapse can rewrite a surviving parent's id out from
/// under a stale id — see the comment on `delete_recursive`. Terminates
/// because each iteration strictly shrinks the tree (bounded by leaf
/// count).
pub fn prune_dangling_block_refs(
    root: &mut Option<LayoutNode>,
    live_block_ids: &std::collections::HashSet<String>,
) -> usize {
    let mut pruned = 0;
    loop {
        let Some(tree) = root.as_ref() else { break };
        let Some(dangling_id) = first_dangling_leaf_id(tree, live_block_ids) else { break };
        pruned += 1;
        if tree.id == dangling_id {
            // Root is itself the dangling leaf (single-block tab) —
            // delete_node leaves root deletion to the caller.
            *root = None;
            continue;
        }
        if let Some(t) = root.as_mut() {
            // A `NodeNotFound` here would mean the id vanished via this
            // same loop's prior collapse without our fresh re-scan
            // catching it first — shouldn't happen given the re-scan, but
            // isn't a correctness problem either way: the node is gone,
            // which is the outcome we wanted.
            let _ = delete_node(t, &dangling_id);
        }
    }
    pruned
}

/// A minimize-locked node: a minimized leaf (`minimizedSize`), a slipped
/// header (`slipMinimize`), or a dissolved column (`columnDissolve`). The
/// fields are written by the frontend minimize subsystem and round-trip
/// through the untyped `extra` catch-all on this side. Mirrors
/// `isNodeLocked` in `frontend/layout/lib/layoutMinimize.ts`.
pub fn is_node_locked(node: &LayoutNode) -> bool {
    node.extra.contains_key("minimizedSize")
        || node.extra.contains_key("slipMinimize")
        || node.extra.contains_key("columnDissolve")
}

/// Write-point enforcement of the minimize lock (the invariant-at-the-write
/// companion to `prune_dangling_block_refs`, per
/// `SPEC_LAYOUT_MINIMIZE_LOCKED_STATE_REDESIGN_2026_07_16.md`): snap every
/// locked node's `size` back to its recorded `minimizedLockedSize`,
/// returning the delta to the nearest unlocked sibling (next preferred,
/// then previous) so the parent's flex-unit budget is conserved. Mirrors
/// `enforceMinimizedLocks` in `frontend/layout/lib/layoutMinimize.ts`.
/// Returns the number of snapped nodes (0 = tree already honored the locks
/// — the common case, cheap to call unconditionally on every write).
///
/// If every sibling is locked (inside a dissolved column) the delta is
/// dropped: flex sizes are relative within the parent, so the locked
/// nodes' shares stay proportionally correct.
pub fn enforce_minimized_locks(root: &mut Option<LayoutNode>) -> usize {
    fn walk(node: &mut LayoutNode) -> usize {
        let mut snapped = 0;
        let locked_flags: Vec<bool> = node.children.iter().map(is_node_locked).collect();
        for i in 0..node.children.len() {
            let locked_size = node.children[i]
                .extra
                .get("minimizedLockedSize")
                .and_then(|v| v.as_f64())
                .map(|v| v as f32);
            if let (true, Some(locked_size)) = (locked_flags[i], locked_size) {
                let delta = node.children[i].size - locked_size;
                if delta.abs() > 1e-4 {
                    node.children[i].size = locked_size;
                    let beneficiary = (i + 1..node.children.len())
                        .find(|&j| !locked_flags[j])
                        .or_else(|| (0..i).rev().find(|&j| !locked_flags[j]));
                    if let Some(j) = beneficiary {
                        // Floor at 1 flex unit (same floor as the slip/
                        // undissolve restore paths): a tampered size BELOW
                        // the lock makes the delta negative, and the
                        // repayment must not drive the beneficiary's size
                        // to zero or negative.
                        node.children[j].size = (node.children[j].size + delta).max(1.0);
                    }
                    snapped += 1;
                }
            }
        }
        for child in &mut node.children {
            snapped += walk(child);
        }
        snapped
    }
    root.as_mut().map_or(0, walk)
}

/// Layout doctor — structural invariant validation, mirroring
/// `validateLayoutInvariants` in `frontend/layout/lib/layoutInvariants.ts`
/// (keep the check lists in sync). Returns human-readable violation strings;
/// empty = healthy. Never mutates. Reducer choke points call this after
/// prune/enforce and `tracing::error!` any findings, so a corrupted tree is
/// attributed to the write that produced it instead of being reconstructed
/// later from db_layout archaeology (issue #2179).
pub fn validate_layout_invariants(root: &Option<LayoutNode>) -> Vec<String> {
    let mut violations = Vec::new();
    fn short(id: &str) -> &str {
        &id[..id.len().min(8)]
    }
    fn walk(node: &LayoutNode, is_root: bool, violations: &mut Vec<String>) {
        let is_branch = !node.children.is_empty();
        let is_leaf = node.data.is_some();
        let has = |k: &str| node.extra.contains_key(k);

        // I1 — leaf XOR branch.
        if is_branch == is_leaf {
            violations.push(format!(
                "LEAF_XOR_BRANCH @ {}: data {}, children {}",
                short(&node.id),
                if is_leaf { "present" } else { "absent" },
                if is_branch { "present" } else { "absent" }
            ));
        }
        // I2 — minimizedSize / slipMinimize are leaf-only markers.
        if is_branch && (has("minimizedSize") || has("slipMinimize")) {
            violations.push(format!(
                "MIN_MARKER_ON_BRANCH @ {}: branch has {}",
                short(&node.id),
                if has("minimizedSize") { "minimizedSize" } else { "slipMinimize" }
            ));
        }
        // I3 — columnDissolve is branch-only.
        if !is_branch && has("columnDissolve") {
            violations.push(format!("DISSOLVE_ON_LEAF @ {}", short(&node.id)));
        }
        // I4 — a dissolved column must stack vertically (#2176 signature).
        if has("columnDissolve") && node.flex_direction != FlexDirection::Column {
            violations.push(format!(
                "DISSOLVED_NOT_COLUMN @ {}: flex_direction={:?}",
                short(&node.id),
                node.flex_direction
            ));
        }
        // I5 — every child of a dissolved column is itself locked.
        if has("columnDissolve") {
            for c in &node.children {
                if !is_node_locked(c) {
                    violations.push(format!(
                        "DISSOLVED_CHILD_UNLOCKED @ {}: inside dissolved column {}",
                        short(&c.id),
                        short(&node.id)
                    ));
                }
            }
        }
        let locked_size = node.extra.get("minimizedLockedSize").and_then(|v| v.as_f64());
        // I6 — a locked node's size honors its recorded lock (#2180).
        if is_node_locked(node) {
            if let Some(ls) = locked_size {
                if (node.size - ls as f32).abs() > 1e-4 {
                    violations.push(format!(
                        "LOCK_SIZE_MISMATCH @ {}: size={} locked={}",
                        short(&node.id),
                        node.size,
                        ls
                    ));
                }
            }
        } else if locked_size.is_some() {
            // I7 — minimizedLockedSize must not outlive its lock marker.
            violations.push(format!("ORPHAN_LOCKED_SIZE @ {}", short(&node.id)));
        }
        // I8 — sizes are positive.
        if !is_root && !(node.size > 0.0) {
            violations.push(format!("NONPOSITIVE_SIZE @ {}: size={}", short(&node.id), node.size));
        }
        for c in &node.children {
            walk(c, false, violations);
        }
    }
    if let Some(tree) = root {
        walk(tree, true, &mut violations);
        // I9 — at least one pane stays expanded. A tree whose every leaf is
        // minimize-locked is an all-headers window with nothing restorable
        // in view; the frontend's minimize toggle guards against producing
        // this (`countExpandedLeaves` in layoutMinimize.ts).
        fn count_leaves(node: &LayoutNode, leaves: &mut usize, expanded: &mut usize) {
            if node.children.is_empty() {
                if node.data.is_some() {
                    *leaves += 1;
                    if !is_node_locked(node) {
                        *expanded += 1;
                    }
                }
                return;
            }
            for c in &node.children {
                count_leaves(c, leaves, expanded);
            }
        }
        let (mut leaves, mut expanded) = (0usize, 0usize);
        count_leaves(tree, &mut leaves, &mut expanded);
        if leaves > 0 && expanded == 0 {
            violations.push(format!(
                "ALL_LEAVES_LOCKED @ {}: all {} leaves are minimize-locked; no expanded pane remains",
                &tree.id[..tree.id.len().min(8)],
                leaves
            ));
        }
    }
    violations
}

/// First leaf (depth-first) whose `data.block_id` is set but not present in
/// `live_block_ids`. `data.block_id` is only ever non-empty on leaves —
/// container/group nodes have `data: None` (see `ensure_group_node`).
fn first_dangling_leaf_id(
    node: &LayoutNode,
    live_block_ids: &std::collections::HashSet<String>,
) -> Option<String> {
    if let Some(data) = &node.data {
        if !data.block_id.is_empty() && !live_block_ids.contains(&data.block_id) {
            return Some(node.id.clone());
        }
    }
    for child in &node.children {
        if let Some(found) = first_dangling_leaf_id(child, live_block_ids) {
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
/// last index.
///
/// Mirrors `findInsertLocationFromIndexArr` in
/// `frontend/layout/lib/layoutNode.ts`:
/// - Each segment is **clamped** to `[0, children.len() - 1]`. Out-of-range
///   indices do NOT error — they resolve to the last child slot.
/// - Descent **stops at a leaf**. Any remaining segments after we hit a node
///   with no children are ignored, and the leaf becomes the insert target
///   (where `ensure_group_node` then promotes it). This matches the TS
///   oracle's `if (indexArr.length == 0 || !node.children) return ...`.
///
/// This tolerance is required for "identical state transitions" with the
/// frontend reducer under concurrent edits — the frontend may issue a
/// stale or over-deep `indexArr` against a tree that has since shrunk.
pub fn insert_node_at_index(
    root: &mut LayoutNode,
    node: LayoutNode,
    index_arr: &[usize],
) -> Result<(), LayoutError> {
    if index_arr.is_empty() {
        return Err(LayoutError::InvalidIndexPath);
    }
    let mut current = root as *mut LayoutNode;
    let mut clamped_idx: usize = 0;
    let last_seg = index_arr.len() - 1;

    for (seg_pos, &idx) in index_arr.iter().enumerate() {
        let cur = unsafe { &mut *current };
        if cur.children.is_empty() {
            // Leaf reached early — TS treats `node.children?.length ?? 1`
            // as 1 here, so any non-negative idx clamps to 0. Stop descent.
            clamped_idx = 0;
            break;
        }
        clamped_idx = idx.min(cur.children.len() - 1);
        if seg_pos < last_seg {
            current = &mut cur.children[clamped_idx] as *mut LayoutNode;
        }
    }

    let parent = unsafe { &mut *current };
    // Minimized is a locked state: an `index_arr` that resolves into a
    // dissolved column (locked container) or onto a minimized leaf (which
    // `ensure_group_node` would otherwise promote into a group) is rejected.
    if is_node_locked(parent) {
        return Err(LayoutError::NodeLocked { id: parent.id.clone() });
    }
    ensure_group_node(parent);
    let insert_at = (clamped_idx + 1).min(parent.children.len());
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
    // Minimized is a locked state: a locked node can't be moved (restore it
    // first), and nothing may be inserted into a dissolved column.
    if is_node_locked(&node_to_move) {
        return Err(LayoutError::NodeLocked { id: node_id.to_string() });
    }
    // Confirm destination exists while tree is still intact.
    match find_node_by_id(root, new_parent_id) {
        None => return Err(LayoutError::NodeNotFound { id: new_parent_id.to_string() }),
        Some(p) if is_node_locked(p) => {
            return Err(LayoutError::NodeLocked { id: new_parent_id.to_string() });
        }
        Some(_) => {}
    }
    // Confirm destination is NOT a descendant of the node being moved.
    if find_node_by_id(&node_to_move, new_parent_id).is_some() {
        return Err(LayoutError::NodeNotFound {
            id: format!("{} (is a descendant of the moved node {})", new_parent_id, node_id),
        });
    }

    let old_parent_id = find_parent_id(root, node_id);
    let same_parent = old_parent_id.as_deref() == Some(new_parent_id);

    // Capture the node's current index inside the new parent BEFORE the
    // detach so we can compensate for the source-removal shift below.
    // Only meaningful when same_parent (otherwise cur_idx is None).
    let cur_idx_in_new_parent = if same_parent {
        find_node_by_id(root, new_parent_id)
            .and_then(|p| p.children.iter().position(|c| c.id == node_id))
    } else {
        None
    };

    // Now it is safe to detach (both endpoints exist).
    remove_node_from_parent(root, node_id);

    let mut node_with_size = node_to_move;
    if !same_parent {
        node_with_size.size = DEFAULT_NODE_SIZE;
    }

    // Mirror the TypeScript oracle (`moveNode` in
    // frontend/layout/lib/layoutTree.ts): TS does insert-then-remove with
    // a `startingIndex` shift, which makes same-parent moves where the
    // requested index is past the current position resolve to one slot
    // earlier in the final array (because the to-be-removed source eats
    // a slot from `index`). We do detach-then-insert here, so we apply
    // that compensation by decrementing `index` when target > cur_idx.
    let effective_index = match cur_idx_in_new_parent {
        Some(ci) if index > ci => index - 1,
        _ => index,
    };

    // Insert at new location (destination guaranteed to still exist).
    let new_parent = find_node_by_id_mut(root, new_parent_id).unwrap();
    ensure_group_node(new_parent);
    let insert_at = effective_index.min(new_parent.children.len());
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
    // Minimized is a locked state: locked nodes don't swap.
    if is_node_locked(&n1) {
        return Err(LayoutError::NodeLocked { id: node1_id.to_string() });
    }
    if is_node_locked(&n2) {
        return Err(LayoutError::NodeLocked { id: node2_id.to_string() });
    }
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
    // Validation pass 2: all target nodes exist and none is minimize-locked.
    // Ops come in before/after pairs whose unit sum is conserved — applying
    // only the unlocked half would leak flex units, so it's all-or-nothing
    // (same atomic-reject semantics as the passes above, and the same guard
    // as the frontend's `resizeNode`).
    for op in ops {
        match find_node_by_id(root, &op.node_id) {
            None => return Err(LayoutError::NodeNotFound { id: op.node_id.clone() }),
            Some(node) if is_node_locked(node) => {
                return Err(LayoutError::NodeLocked { id: op.node_id.clone() });
            }
            Some(_) => {}
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
    // Find target; it must exist and must not be minimize-locked (splitting
    // a locked node would spawn a full pane inside a header strip).
    let target = find_node_by_id(root, target_id)
        .ok_or_else(|| LayoutError::NodeNotFound { id: target_id.to_string() })?;
    if is_node_locked(target) {
        return Err(LayoutError::NodeLocked { id: target_id.to_string() });
    }

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

// ── Tree normalization (balanceNode) ────────────────────────────────────────

/// Validate the leaf-XOR-branch invariant. Mirrors `validateNode` in
/// `frontend/layout/lib/layoutNode.ts`: a node must have **either** `data`
/// **or** `children`, never both and never neither. (In TS the "empty
/// children array" case is a separate check; here an empty `children` vec
/// with no `data` is the same "neither" case and is rejected by the single
/// XOR below.)
fn validate_node(node: &LayoutNode) -> bool {
    let has_children = !node.children.is_empty();
    let has_data = node.data.is_some();
    has_children != has_data
}

/// Port of `balanceNode` in `frontend/layout/lib/layoutNode.ts`. Recursively
/// normalizes the tree: validates each node, alternates child flex direction,
/// hoists redundant single-child-branch chains, drops empty branches, and
/// collapses a branch with a single leaf child into that leaf.
///
/// Returns `Err(InvalidNode)` where the TS oracle throws "Invalid node", so
/// the reducer can surface an `Event::Error` rather than panic.
///
/// Faithfulness notes (matched to the TS oracle, deliberately NOT "improved"):
/// - The direction flip mutates only the *iterated* child; hoisted
///   grandchildren are returned unflipped (a later pass alternates them) —
///   matches the TS `flatMap`.
/// - `_slipAnchor` (read from the untyped `extra` catch-all, where the
///   frontend stores it) suppresses the single-child-branch hoist.
/// - `columnDissolve` (same `extra` catch-all) suppresses the direction-flip
///   alternation, so a dissolved column's own children stay stacked.
/// - The single-leaf collapse copies the child's `data` + `id` only; the node
///   keeps its own `size` / `flex_direction` / `extra` — exactly as TS does.
pub fn balance_node(node: &mut LayoutNode) -> Result<(), LayoutError> {
    // BEFORE-walk: validate, then rebuild children (flip / hoist / drop).
    if !validate_node(node) {
        return Err(LayoutError::InvalidNode { id: node.id.clone() });
    }
    let parent_flex = node.flex_direction;
    let mut rebuilt: Vec<LayoutNode> = Vec::with_capacity(node.children.len());
    for mut child in std::mem::take(&mut node.children) {
        // A dissolved column (`columnDissolve` set, in `extra`) must keep the
        // flexDirection it had when its leaf children were minimized — that's
        // what stacks them vertically inside the header strip. It's nested
        // under a sibling column of the same direction by design (see
        // `_dissolveColumn` in layoutMinimize.ts), so the alternation rule
        // below would otherwise flip it and lay the headers out sideways.
        let column_dissolve = child.extra.contains_key("columnDissolve");
        if child.flex_direction == parent_flex && !column_dissolve {
            child.flex_direction = reverse_flex_direction(parent_flex);
        }
        let slip_anchor = child
            .extra
            .get("_slipAnchor")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        // Hoist: child has exactly one child and that grandchild is itself a
        // branch (TS `child.children[0].children` truthy) — replace `child`
        // with the grandchild's children. Suppressed for slip anchors.
        if child.children.len() == 1 && !child.children[0].children.is_empty() && !slip_anchor {
            let grandchild = child.children.remove(0);
            rebuilt.extend(grandchild.children);
            continue;
        }
        // Drop an empty branch (no data, no children). A leaf (data present,
        // children empty) is kept — TS `children?.length === 0` is false for a
        // leaf whose `children` is `undefined`.
        if child.children.is_empty() && child.data.is_none() {
            continue;
        }
        rebuilt.push(child);
    }
    node.children = rebuilt;

    // Recurse into the rebuilt children (walkNodes' `forEach`, which iterates
    // the post-rebuild children because the before-callback ran first).
    for child in &mut node.children {
        balance_node(child)?;
    }

    // AFTER-walk: collapse a single leaf child into this node.
    if node.children.len() == 1 && node.children[0].children.is_empty() {
        let only = node.children.remove(0);
        node.data = only.data;
        node.id = only.id;
        node.children = Vec::new();
    }
    Ok(())
}

// ── Tests ──────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests;
