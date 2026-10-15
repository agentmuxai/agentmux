// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pure layout-tree functions for the srv reducer: traversal, pane-tab
//! stacks, the write-point integrity passes (dangling blocks, invariants),
//! `balance_node`, and the frontend's tree actions as layout intents
//! ([`intent`]). No I/O, no side effects.

pub mod intent;

use agentmux_common::{FlexDirection, LayoutNode, StackMovePosition};

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
    /// An intent the reducer won't apply, with the reason.
    Refused(String),
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
            Self::Refused(reason) => f.write_str(reason),
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
#[cfg(test)]
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

/// A leaf's members: its `block_stack`, or `[block_id]` for a leaf with no
/// stack (mirrors the frontend's `effectiveStack`, `layoutStack.ts`).
pub fn leaf_members(data: &agentmux_common::LayoutNodeData) -> Vec<String> {
    if data.block_stack.is_empty() {
        vec![data.block_id.clone()]
    } else {
        data.block_stack.clone()
    }
}

/// Find the (first) leaf that has `block_id` as its visible block OR as any
/// member of its `block_stack`. Unlike [`find_node_id_by_block`], which only
/// matches the visible block, this finds the pane a background tab lives in.
/// Returns the leaf node itself.
pub fn find_leaf_containing_block<'a>(tree: &'a LayoutNode, block_id: &str) -> Option<&'a LayoutNode> {
    if let Some(d) = tree.data.as_ref() {
        if d.block_id == block_id || d.block_stack.iter().any(|b| b == block_id) {
            return Some(tree);
        }
    }
    tree.children
        .iter()
        .find_map(|child| find_leaf_containing_block(child, block_id))
}

/// Which tab becomes visible once the one at `removed_idx` of the ORIGINAL
/// stack is gone: the nearest eligible member to its right, else the nearest
/// to its left. `remaining` is the stack with it removed; `eligible` filters
/// (e.g. "still exists"). The one rule every removal path uses — frontend
/// `removeMemberFromStack` mirrors it (`frontend/layout/lib/stackMembers.ts`).
/// SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.6.
pub fn next_visible_member(
    remaining: &[String],
    removed_idx: usize,
    eligible: impl Fn(&String) -> bool,
) -> Option<String> {
    let split = removed_idx.min(remaining.len());
    remaining[split..]
        .iter()
        .find(|m| eligible(m))
        .or_else(|| remaining[..split].iter().rev().find(|m| eligible(m)))
        .cloned()
}

/// Add `block_id` to the tab stack of the pane holding `target_block_id`
/// (as its visible tab or a background member). A pane with no stack yet
/// becomes a two-member stack. Already a member → only (re)activated if
/// asked. Returns false when no pane holds `target_block_id`.
/// SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.2 (`LayoutStackPush`).
pub fn push_stack_member(tree: &mut LayoutNode, target_block_id: &str, block_id: &str, activate: bool) -> bool {
    if let Some(data) = tree.data.as_mut() {
        if data.block_id == target_block_id || data.block_stack.iter().any(|b| b == target_block_id) {
            if data.block_stack.is_empty() {
                data.block_stack = vec![data.block_id.clone()];
                data.active_block_id = data.block_id.clone();
            }
            if !data.block_stack.iter().any(|b| b == block_id) {
                data.block_stack.push(block_id.to_string());
            }
            if activate {
                data.block_id = block_id.to_string();
                data.active_block_id = block_id.to_string();
            }
            return true;
        }
    }
    tree.children
        .iter_mut()
        .any(|child| push_stack_member(child, target_block_id, block_id, activate))
}

/// Make `block_id` the visible tab of the pane whose stack holds it.
/// Returns false when no pane's stack holds it.
/// SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.2 (`LayoutStackActivate`).
pub fn activate_stack_member(tree: &mut LayoutNode, block_id: &str) -> bool {
    if let Some(data) = tree.data.as_mut() {
        if leaf_members(data).iter().any(|m| m == block_id) {
            data.block_id = block_id.to_string();
            if !data.block_stack.is_empty() {
                data.active_block_id = block_id.to_string();
            }
            return true;
        }
    }
    tree.children.iter_mut().any(|child| activate_stack_member(child, block_id))
}

/// Remove `block_id` from the stack of the leaf holding it, WITHOUT removing
/// the leaf, when that leaf has other members. If it was the visible tab, the
/// right-hand neighbour becomes visible (the new last member when it was
/// rightmost) — the same rule as the frontend's `removeMemberFromStack`
/// (`frontend/layout/lib/stackMembers.ts`). Returns false and changes nothing
/// when `block_id` is in no leaf or is its leaf's only member (removing it
/// means removing the leaf — the caller's job).
/// SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.4.
pub fn remove_stack_member(tree: &mut LayoutNode, block_id: &str) -> bool {
    if let Some(data) = tree.data.as_mut() {
        let members = leaf_members(data);
        if let Some(idx) = members.iter().position(|m| m == block_id) {
            if members.len() <= 1 {
                return false;
            }
            let next: Vec<String> = members.into_iter().filter(|m| m != block_id).collect();
            if data.block_id == block_id || data.active_block_id == block_id {
                let visible = next_visible_member(&next, idx, |_| true).expect("next is non-empty");
                data.block_id = visible.clone();
                data.active_block_id = visible;
            }
            data.block_stack = next;
            return true;
        }
    }
    tree.children
        .iter_mut()
        .any(|child| remove_stack_member(child, block_id))
}

/// Move `block_id` to `position` relative to `target_block_id`: a same-leaf
/// reorder when both are already members of one leaf's stack, otherwise a
/// cross-leaf move (removed from its source leaf via [`remove_stack_member`],
/// placed into `target_block_id`'s leaf via a position-aware push). Returns
/// `false`, leaving the tree untouched, when either block doesn't exist in
/// the tree.
///
/// When `block_id` is its source leaf's ONLY member, the cross-leaf move
/// empties that pane, so the source leaf is deleted from the tree and the
/// tree balanced, as the frontend does ([`intent::delete_and_balance`]). This is a move:
/// the block itself lives on in the destination.
/// [`source_leaf_emptied_by_move`] tells a caller in advance whether this
/// will happen, so it can reconcile focus/magnify that pointed at that leaf.
/// SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md §4.3.
///
/// Deliberately NOT a same-leaf-and-cross-leaf-both composition of
/// `remove_stack_member` + a push: `remove_stack_member`'s neighbour
/// reassignment assumes the member is actually leaving its leaf, and wrongly
/// switches the visible tab if applied to a same-leaf reorder of the
/// currently-visible member (see
/// `move_stack_member_reorder_does_not_change_which_tab_is_visible` in
/// `tests.rs`). The same-leaf branch below never touches
/// `block_id`/`active_block_id` unless `activate` is explicitly set.
///
/// SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §4.1.
pub fn move_stack_member(
    tree: &mut LayoutNode,
    block_id: &str,
    target_block_id: &str,
    position: StackMovePosition,
    activate: bool,
) -> bool {
    if block_id == target_block_id {
        // Not a real move — nothing to reorder against — but still confirm
        // the block actually exists (a bogus self-target must not report
        // success), and still honor `activate` exactly like every other
        // branch below does (ReAgent P1 on PR #3441: this used to
        // short-circuit before either check).
        if find_leaf_containing_block(tree, block_id).is_none() {
            return false;
        }
        return if activate { activate_stack_member(tree, block_id) } else { true };
    }
    let Some(source_leaf) = find_leaf_containing_block(tree, block_id) else {
        return false;
    };
    let source_node_id = source_leaf.id.clone();
    let source_members = leaf_members(source_leaf.data.as_ref().expect("leaf has data"));
    if source_members.iter().any(|m| m == target_block_id) {
        return reorder_within_leaf(tree, block_id, target_block_id, position, activate);
    }
    // Validate the destination exists BEFORE mutating anything — otherwise
    // a bad `target_block_id` would still remove `block_id` from its source
    // leaf and then fail to place it anywhere, corrupting the tree.
    if find_leaf_containing_block(tree, target_block_id).is_none() {
        return false;
    }
    if source_members.len() <= 1 {
        // The pane's only tab: the move empties it, so the pane goes. Place
        // the block first (located by block id, which survives the collapse
        // below), then delete the source leaf by node id — both inside this
        // one `&mut` edit, so no caller ever sees the block in two leaves or
        // in none. The source can't be the root: the destination is a
        // different leaf in the same tree, so the root has ≥2 leaves under it.
        if tree.id == source_node_id {
            return false;
        }
        if !push_stack_member_at(tree, target_block_id, block_id, position, activate) {
            return false;
        }
        return intent::delete_and_balance(tree, &source_node_id).is_ok();
    }
    if !remove_stack_member(tree, block_id) {
        return false;
    }
    push_stack_member_at(tree, target_block_id, block_id, position, activate)
}

/// The node id of `block_id`'s leaf, when `move_stack_member(tree, block_id,
/// target_block_id, ..)` would delete that leaf: `block_id` is its only
/// member and `target_block_id` lives in a different leaf. `None` for every
/// other move (reorder, multi-member source, missing blocks, self-target).
pub fn source_leaf_emptied_by_move(tree: &LayoutNode, block_id: &str, target_block_id: &str) -> Option<String> {
    if block_id == target_block_id {
        return None;
    }
    let source = find_leaf_containing_block(tree, block_id)?;
    let members = leaf_members(source.data.as_ref()?);
    if members.len() != 1 || source.id == tree.id {
        return None;
    }
    find_leaf_containing_block(tree, target_block_id)
        .filter(|target| target.id != source.id)
        .map(|_| source.id.clone())
}

/// Splice `block_id` to `position` relative to `target_block_id` within the
/// SAME leaf's stack. Both must already be members of that leaf (checked by
/// the caller, `move_stack_member`). Only touches `block_id`/`active_block_id`
/// when `activate` is set — a reorder alone never changes which member is
/// visible.
fn reorder_within_leaf(
    tree: &mut LayoutNode,
    block_id: &str,
    target_block_id: &str,
    position: StackMovePosition,
    activate: bool,
) -> bool {
    if let Some(data) = tree.data.as_mut() {
        let members = leaf_members(data);
        if members.iter().any(|m| m == block_id) && members.iter().any(|m| m == target_block_id) {
            let mut next: Vec<String> = members.into_iter().filter(|m| m != block_id).collect();
            let target_idx = next.iter().position(|m| m == target_block_id).expect("target survives removal");
            let insert_at = match position {
                StackMovePosition::Before => target_idx,
                StackMovePosition::After => target_idx + 1,
                StackMovePosition::End => next.len(),
            };
            next.insert(insert_at, block_id.to_string());
            data.block_stack = next;
            if activate {
                data.block_id = block_id.to_string();
                data.active_block_id = block_id.to_string();
            }
            return true;
        }
    }
    tree.children
        .iter_mut()
        .any(|child| reorder_within_leaf(child, block_id, target_block_id, position, activate))
}

/// Position-aware variant of [`push_stack_member`]: adds `block_id` to the
/// stack of the leaf holding `target_block_id`, at `position` relative to
/// `target_block_id` instead of always appending. Promotes a single-block
/// leaf (empty `block_stack`) into a real stack first, exactly like
/// `push_stack_member` does.
fn push_stack_member_at(
    tree: &mut LayoutNode,
    target_block_id: &str,
    block_id: &str,
    position: StackMovePosition,
    activate: bool,
) -> bool {
    if let Some(data) = tree.data.as_mut() {
        if data.block_id == target_block_id || data.block_stack.iter().any(|b| b == target_block_id) {
            if data.block_stack.is_empty() {
                data.block_stack = vec![data.block_id.clone()];
                data.active_block_id = data.block_id.clone();
            }
            if !data.block_stack.iter().any(|b| b == block_id) {
                let target_idx = data.block_stack.iter().position(|b| b == target_block_id).expect("target present");
                let insert_at = match position {
                    StackMovePosition::Before => target_idx,
                    StackMovePosition::After => target_idx + 1,
                    StackMovePosition::End => data.block_stack.len(),
                };
                data.block_stack.insert(insert_at, block_id.to_string());
            }
            if activate {
                data.block_id = block_id.to_string();
                data.active_block_id = block_id.to_string();
            }
            return true;
        }
    }
    tree.children
        .iter_mut()
        .any(|child| push_stack_member_at(child, target_block_id, block_id, position, activate))
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
/// Reconciliation, `crates/cef/src/wrr/`) enforcing "no window/pool
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
    // In-pane tabs: if a leaf's ACTIVE block went dangling through some
    // path other than `closeBlockInStack` (which always reactivates a live
    // neighbor before removing a block), but a live sibling still sits in
    // `block_stack`, reactivate that sibling in place rather than letting
    // the whole-leaf scan below delete the entire leaf — that would
    // silently discard the still-live sibling blocks along with it. MUST
    // run before the whole-leaf loop so that loop sees the repaired
    // `block_id`. Leaves with no live sibling are left untouched here —
    // `block_id` still points at the dangling id, so the loop below
    // correctly identifies and deletes the (genuinely fully-dead) leaf.
    if let Some(tree) = root.as_mut() {
        pruned += reactivate_dangling_active_stack_members(tree, live_block_ids);
    }
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
    // In-pane tabs (SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md §4.3):
    // a surviving leaf's ACTIVE block (`data.block_id`) is already
    // guaranteed live by the loop above, but a non-active `block_stack`
    // member's block can independently be deleted without ever making the
    // leaf itself dangling — the leaf still renders fine via its active
    // member. Without this pass those ids would linger in `block_stack`
    // forever, so a later tab-switch could try to activate a block that no
    // longer exists.
    if let Some(tree) = root.as_mut() {
        pruned += prune_dangling_stack_members(tree, live_block_ids);
    }
    pruned
}

/// Recursively repair leaves whose ACTIVE block (`data.block_id`) is
/// dangling but a live sibling still exists in `block_stack`: drop the
/// dead entry from the stack and reactivate the first remaining live
/// member (`block_id`/`active_block_id` both updated). Leaves with no live
/// sibling are left with `block_id` still pointing at the dangling id —
/// the caller's whole-leaf pass then correctly deletes them, same as
/// before this function existed. Returns the number of leaves repaired.
fn reactivate_dangling_active_stack_members(
    node: &mut LayoutNode,
    live_block_ids: &std::collections::HashSet<String>,
) -> usize {
    let mut repaired = 0;
    if let Some(data) = node.data.as_mut() {
        if !data.block_stack.is_empty() && !live_block_ids.contains(&data.block_id) {
            // Drop the dangling active member from the stack — it's gone
            // either way, whether or not a live sibling survives it.
            // Its replacement follows the same right-hand-neighbour rule as
            // every other removal (`next_visible_member`) — this used to pick
            // the FIRST live member, disagreeing with the frontend.
            let dead_idx = data.block_stack.iter().position(|id| id == &data.block_id).unwrap_or(0);
            data.block_stack.retain(|id| id != &data.block_id);
            if let Some(replacement) =
                next_visible_member(&data.block_stack, dead_idx, |id| live_block_ids.contains(id))
            {
                data.block_id = replacement.clone();
                data.active_block_id = replacement;
                repaired += 1;
            }
            // else: block_id is untouched (still the dangling id) — no live
            // sibling to fall back to, so this leaf is genuinely fully dead.
        }
    }
    for child in &mut node.children {
        repaired += reactivate_dangling_active_stack_members(child, live_block_ids);
    }
    repaired
}

/// Recursively drop dead entries from every leaf's `block_stack`. The
/// active member (`data.block_id`) is never removed here — by the time
/// this runs, the caller's whole-leaf pruning loop has already guaranteed
/// it's live. Returns the number of stack entries removed.
fn prune_dangling_stack_members(
    node: &mut LayoutNode,
    live_block_ids: &std::collections::HashSet<String>,
) -> usize {
    let mut removed = 0;
    if let Some(data) = node.data.as_mut() {
        if !data.block_stack.is_empty() {
            let before = data.block_stack.len();
            data.block_stack
                .retain(|id| id == &data.block_id || live_block_ids.contains(id));
            removed += before - data.block_stack.len();
        }
    }
    for child in &mut node.children {
        removed += prune_dangling_stack_members(child, live_block_ids);
    }
    removed
}

/// A minimize-locked node: a minimized leaf (`minimizedSize`), a slipped
/// header (`slipMinimize`), or a dissolved column (`columnDissolve`). The
/// fields are written by the frontend minimize subsystem and round-trip
/// through the untyped `extra` catch-all on this side. Mirrors
/// `isNodeLocked` in `frontend/layout/lib/layoutMinimize.ts`.
pub fn is_node_locked(node: &LayoutNode) -> bool {
    // Current model: the `minimized` display-mode flag (geometry derived at
    // render on the frontend; stored sizes never touched). The remaining keys
    // are legacy markers from the pre-display-mode model, recognized until
    // persisted trees are migrated by the frontend's `rebuildMinimizedSet`.
    node.minimized
        || node.extra.contains_key("minimizedSize")
        || node.extra.contains_key("slipMinimize")
        || node.extra.contains_key("columnDissolve")
}

/// True when a node renders as minimized: a locked leaf, or a branch whose
/// every child is effectively minimized (a stacked chip strip). Mirrors
/// `isEffectivelyMinimized` in `frontend/layout/lib/layoutInvariants.ts`.
/// Reducer guards use THIS (not `is_node_locked`): a fully-collapsed branch
/// has no branch-level marker in the display-mode model, but its stored size
/// becomes load-bearing again the moment a child restores — so it must be
/// just as untargetable as a minimized leaf.
pub fn is_effectively_minimized(node: &LayoutNode) -> bool {
    if node.children.is_empty() {
        return is_node_locked(node);
    }
    node.children.iter().all(is_effectively_minimized)
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
        // I2 — the `minimized` flag and the legacy minimizedSize/slipMinimize
        // markers are leaf-only.
        if is_branch && (node.minimized || has("minimizedSize") || has("slipMinimize")) {
            violations.push(format!(
                "MIN_MARKER_ON_BRANCH @ {}: branch has {}",
                short(&node.id),
                if node.minimized {
                    "minimized"
                } else if has("minimizedSize") {
                    "minimizedSize"
                } else {
                    "slipMinimize"
                }
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
        // I10 — a pane's tab stack is consistent: the visible block is a
        // member, `active_block_id` agrees with it, and no block appears
        // twice (SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.1, I2).
        if let Some(data) = node.data.as_ref().filter(|d| !d.block_stack.is_empty()) {
            if !data.block_stack.contains(&data.block_id) {
                violations.push(format!("STACK_VISIBLE_NOT_MEMBER @ {}", short(&node.id)));
            }
            if !data.active_block_id.is_empty() && data.active_block_id != data.block_id {
                violations.push(format!("STACK_ACTIVE_MISMATCH @ {}", short(&node.id)));
            }
            let unique: std::collections::HashSet<&String> = data.block_stack.iter().collect();
            if unique.len() != data.block_stack.len() {
                violations.push(format!("STACK_DUPLICATE_MEMBER @ {}", short(&node.id)));
            }
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
        // I11 — a block is in at most one pane (the tree half of
        // SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.1 I1; the other
        // half, "in at least one", needs the tab's blocks: `unplaced_blocks`).
        let mut pane_of: std::collections::HashMap<String, &str> = std::collections::HashMap::new();
        for_each_leaf(tree, &mut |leaf, data| {
            for member in leaf_members(data) {
                if let Some(first) = pane_of.insert(member.clone(), &leaf.id).filter(|first| *first != leaf.id) {
                    violations.push(format!("BLOCK_IN_TWO_PANES @ {}: also in {}", short(&member), short(first)));
                }
            }
        });
    }
    violations
}

/// The tab's blocks that are in no pane of its tree: the other half of I1
/// (SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.1). Briefly true of a
/// block on its way into a pane (a queued placement the frontend hasn't
/// applied yet); a block that stays unplaced is an orphan.
pub fn unplaced_blocks(root: &Option<LayoutNode>, block_ids: &[String]) -> Vec<String> {
    let mut placed = std::collections::HashSet::new();
    if let Some(tree) = root {
        for_each_leaf(tree, &mut |_, data| placed.extend(leaf_members(data)));
    }
    block_ids.iter().filter(|b| !placed.contains(*b)).cloned().collect()
}

/// Visit every leaf (a node with `data`), depth first.
fn for_each_leaf<'a>(node: &'a LayoutNode, f: &mut impl FnMut(&'a LayoutNode, &'a agentmux_common::LayoutNodeData)) {
    if let Some(data) = node.data.as_ref() {
        f(node, data);
    }
    for child in &node.children {
        for_each_leaf(child, f);
    }
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

fn collect_insert_candidates<'a>(
    node: &'a LayoutNode,
    max_children: usize,
    depth: usize,
    out: &mut Vec<InsertCandidate<'a>>,
) {
    // Minimized is a locked state: never a blind-insert target. Without
    // this, `ensure_group_node` promoting a minimized leaf into a branch
    // could leave the tree in a state where an agent/API-driven insert
    // silently un-collapses a fully-minimized subtree (every child
    // minimized → one child isn't anymore, with no user action on that
    // branch at all). Mirrors `findNextInsertLocationHelper`'s guard in
    // `frontend/layout/lib/layoutNode.ts` (2026-07-18). For a branch this
    // also skips recursing into its children — they're all effectively
    // minimized too by definition, so recursion would find nothing anyway.
    if is_effectively_minimized(node) {
        return;
    }
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
        // Collapse: if only one child remains, promote it, with its
        // `minimized` flag and `extra` (unknown-field catch-all from #688/#689).
        if node.children.len() == 1 {
            let sole = node.children.remove(0);
            node.id = sole.id;
            node.size = sole.size;
            node.data = sole.data;
            node.flex_direction = sole.flex_direction;
            node.children = sole.children;
            node.minimized = sole.minimized;
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
