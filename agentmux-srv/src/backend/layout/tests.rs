// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Ported tests for the Rust layout helpers. These mirror the 30 tests
//! from `frontend/layout/tests/layoutTree.test.ts` (PR #686) — the
//! TypeScript test suite is the behavioral oracle.

use agentmux_common::{FlexDirection, LayoutNode, LayoutNodeData, ResizeOp, SplitPosition};
use uuid::Uuid;
use super::*;

// ── Helpers ─────────────────────────────────────────────────────────────────

fn leaf(id: &str, block_id: &str, size: f32) -> LayoutNode {
    LayoutNode {
        id: id.into(),
        flex_direction: FlexDirection::Row,
        size,
        data: Some(LayoutNodeData {
            block_id: block_id.into(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn group(id: &str, dir: FlexDirection, size: f32, children: Vec<LayoutNode>) -> LayoutNode {
    LayoutNode {
        id: id.into(),
        flex_direction: dir,
        size,
        children,
        ..Default::default()
    }
}

// ── isInstanceLabel filter (JS analogue: insert location) ──────────────────

#[test]
fn insert_node_into_empty_root_promotes_to_leaf() {
    let mut root = leaf("root", "b1", 1.0);
    let new = leaf("new", "b2", 1.0);
    insert_node(&mut root, new);
    // Root should now have children.
    assert!(!root.children.is_empty());
}

#[test]
fn insert_node_appends_to_existing_non_full_node() {
    let mut root = group("root", FlexDirection::Row, 10.0, vec![
        leaf("c1", "b1", 5.0),
    ]);
    let new = leaf("new", "b2", 5.0);
    insert_node(&mut root, new.clone());
    assert_eq!(root.children.len(), 2);
    assert!(root.children.iter().any(|c| c.id == "new"));
}

// ── insertNodeAtIndex ────────────────────────────────────────────────────────

#[test]
fn insert_node_at_index_inserts_after_path() {
    let mut root = group("root", FlexDirection::Row, 10.0, vec![
        leaf("c1", "b1", 5.0),
        leaf("c2", "b2", 5.0),
    ]);
    let new = leaf("new", "b3", 5.0);
    insert_node_at_index(&mut root, new, &[0]).unwrap();
    // Should be inserted after index 0 (at position 1).
    assert_eq!(root.children[1].id, "new");
}

#[test]
fn insert_node_at_index_empty_arr_returns_error() {
    let mut root = leaf("root", "b1", 1.0);
    let new = leaf("new", "b2", 1.0);
    let result = insert_node_at_index(&mut root, new, &[]);
    assert!(matches!(result, Err(LayoutError::InvalidIndexPath)));
}

// ── swapNode ──────────────────────────────────────────────────────────────────

#[test]
fn swap_nodes_swaps_positions_and_preserves_slot_sizes() {
    let n1 = leaf("n1", "b1", 30.0);
    let n2 = leaf("n2", "b2", 70.0);
    let mut root = group("root", FlexDirection::Row, 10.0, vec![n1, n2]);
    swap_nodes(&mut root, "n1", "n2").unwrap();
    // n2 is now at slot 0, n1 at slot 1.
    assert_eq!(root.children[0].id, "n2");
    assert_eq!(root.children[1].id, "n1");
    // Sizes stay at the slot (30 at slot 0, 70 at slot 1).
    assert_eq!(root.children[0].size, 30.0);
    assert_eq!(root.children[1].size, 70.0);
}

#[test]
fn swap_nodes_rejects_root_node() {
    let child = leaf("child", "b1", 5.0);
    let mut root = group("root", FlexDirection::Row, 10.0, vec![child]);
    let result = swap_nodes(&mut root, "root", "child");
    assert!(matches!(result, Err(LayoutError::RootCannotBeTarget)));
}

#[test]
fn swap_nodes_rejects_self_swap() {
    let n1 = leaf("n1", "b1", 5.0);
    let n2 = leaf("n2", "b2", 5.0);
    let mut root = group("root", FlexDirection::Row, 10.0, vec![n1, n2]);
    let result = swap_nodes(&mut root, "n1", "n1");
    assert!(matches!(result, Err(LayoutError::SelfSwap)));
}

// ── deleteNode ────────────────────────────────────────────────────────────────

#[test]
fn delete_node_removes_child() {
    let n1 = leaf("n1", "keep", 5.0);
    let n2 = leaf("n2", "drop", 5.0);
    let mut root = group("root", FlexDirection::Row, 10.0, vec![n1, n2]);
    delete_node(&mut root, "n2").unwrap();
    // Collapses to root being the kept leaf.
    assert!(!root.children.iter().any(|c| c.id == "n2"));
}

#[test]
fn delete_node_collapses_single_child_parent() {
    let n1 = leaf("n1", "keep", 5.0);
    let n2 = leaf("n2", "drop", 5.0);
    let mut root = group("root", FlexDirection::Row, 10.0, vec![n1, n2]);
    delete_node(&mut root, "n2").unwrap();
    // After dropping n2, only n1 remains. The group should collapse.
    // root is now the "keep" leaf.
    assert_eq!(root.children.len(), 0);
    assert_eq!(root.data.as_ref().map(|d| d.block_id.as_str()), Some("keep"));
}

#[test]
fn delete_node_unknown_id_returns_error() {
    let child = leaf("child", "b1", 5.0);
    let mut root = group("root", FlexDirection::Row, 10.0, vec![child]);
    let result = delete_node(&mut root, "ghost");
    assert!(matches!(result, Err(LayoutError::NodeNotFound { .. })));
}

#[test]
fn delete_node_root_returns_ok_without_deleting() {
    let mut root = leaf("root", "b1", 1.0);
    // Caller should set Option<LayoutNode> = None; we just return Ok(()).
    let result = delete_node(&mut root, "root");
    assert!(result.is_ok());
}

// ── resizeNode ────────────────────────────────────────────────────────────────

#[test]
fn resize_nodes_applies_size_to_matching_node() {
    let n1 = leaf("n1", "b1", 50.0);
    let mut root = group("root", FlexDirection::Row, 10.0, vec![n1]);
    resize_nodes(&mut root, &[ResizeOp { node_id: "n1".into(), size: 80.0 }]).unwrap();
    let node = find_node_by_id(&root, "n1").unwrap();
    assert_eq!(node.size, 80.0);
}

#[test]
fn resize_nodes_rejects_out_of_range_and_applies_nothing() {
    let n1 = leaf("n1", "b1", 50.0);
    let n2 = leaf("n2", "b2", 50.0);
    let mut root = group("root", FlexDirection::Row, 10.0, vec![n1, n2]);
    // First op out of range → whole batch rejected.
    let result = resize_nodes(&mut root, &[
        ResizeOp { node_id: "n1".into(), size: 150.0 },
        ResizeOp { node_id: "n2".into(), size: 25.0 },
    ]);
    assert!(matches!(result, Err(LayoutError::InvalidSize { .. })));
    // Neither node mutated.
    assert_eq!(find_node_by_id(&root, "n1").unwrap().size, 50.0);
    assert_eq!(find_node_by_id(&root, "n2").unwrap().size, 50.0);
}

// ── focusNode (find_node_by_id, id lookup) ────────────────────────────────────

#[test]
fn find_node_by_id_finds_nested_node() {
    let inner = leaf("inner", "b1", 5.0);
    let middle = group("middle", FlexDirection::Column, 5.0, vec![inner]);
    let root = group("root", FlexDirection::Row, 10.0, vec![middle]);
    assert!(find_node_by_id(&root, "inner").is_some());
    assert!(find_node_by_id(&root, "middle").is_some());
    assert!(find_node_by_id(&root, "ghost").is_none());
}

// ── magnifyNodeToggle (via split to verify two-level structure) ─────────────

#[test]
fn split_horizontal_inserts_after_in_row_parent() {
    let child = leaf("c1", "b1", 5.0);
    let root_inner = group("root", FlexDirection::Row, 10.0, vec![child]);
    let mut root = root_inner;
    let new = leaf("new", "b2", 5.0);
    split_horizontal(&mut root, "c1", new, SplitPosition::After).unwrap();
    assert_eq!(root.children.len(), 2);
    assert_eq!(root.children[1].id, "new");
}

#[test]
fn split_horizontal_inserts_before_in_row_parent() {
    let child = leaf("c1", "b1", 5.0);
    let mut root = group("root", FlexDirection::Row, 10.0, vec![child]);
    let new = leaf("new", "b2", 5.0);
    split_horizontal(&mut root, "c1", new, SplitPosition::Before).unwrap();
    assert_eq!(root.children.len(), 2);
    assert_eq!(root.children[0].id, "new");
}

#[test]
fn split_vertical_wraps_root_when_no_parent() {
    let mut root = leaf("only", "b1", 10.0);
    let new = leaf("new", "b2", 10.0);
    split_vertical(&mut root, "only", new, SplitPosition::After).unwrap();
    // Root should now be a Column with 2 children.
    assert_eq!(root.flex_direction, FlexDirection::Column);
    assert_eq!(root.children.len(), 2);
}

#[test]
fn split_vertical_wraps_root_when_no_parent_before() {
    let mut root = leaf("only", "b1", 10.0);
    let new = leaf("new", "b2", 10.0);
    split_vertical(&mut root, "only", new, SplitPosition::Before).unwrap();
    // Root wraps into a Column; new node lands at index 0.
    assert_eq!(root.flex_direction, FlexDirection::Column);
    assert_eq!(root.children.len(), 2);
    assert_eq!(root.children[0].id, "new");
}

#[test]
fn split_horizontal_missing_target_returns_error() {
    let mut root = leaf("root", "b1", 10.0);
    let new = leaf("new", "b2", 10.0);
    let result = split_horizontal(&mut root, "ghost", new, SplitPosition::After);
    assert!(matches!(result, Err(LayoutError::NodeNotFound { .. })));
}

// ── clearTree ────────────────────────────────────────────────────────────────

#[test]
fn clear_tree_sets_root_to_none() {
    let mut root: Option<LayoutNode> = Some(leaf("root", "b1", 10.0));
    clear_tree_node(&mut root);
    assert!(root.is_none());
}

// ── replaceNode ──────────────────────────────────────────────────────────────

#[test]
fn replace_node_replaces_root_preserving_size() {
    let mut root = leaf("old-root", "b1", 100.0);
    let replacement = leaf("new-root", "b2", 50.0);
    replace_node(&mut root, "old-root", replacement).unwrap();
    assert_eq!(root.id, "new-root");
    assert_eq!(root.size, 100.0); // preserved
}

#[test]
fn replace_node_replaces_child_preserving_size() {
    let c1 = leaf("c1", "b1", 30.0);
    let c2 = leaf("c2", "b2", 70.0);
    let mut root = group("root", FlexDirection::Row, 10.0, vec![c1, c2]);
    let replacement = leaf("rep", "b3", 99.0);
    replace_node(&mut root, "c2", replacement).unwrap();
    assert_eq!(root.children[1].id, "rep");
    assert_eq!(root.children[1].size, 70.0); // c2's size preserved
}

#[test]
fn replace_node_focused_flag_separate_concern() {
    // The Rust helpers don't handle focus — that's the reducer's job.
    // Just verify replace_node doesn't error.
    let child = leaf("child", "b1", 5.0);
    let mut root = group("root", FlexDirection::Row, 10.0, vec![child]);
    let rep = leaf("rep", "b2", 5.0);
    assert!(replace_node(&mut root, "child", rep).is_ok());
}

// ── Purity ───────────────────────────────────────────────────────────────────

// ── insertNode ID stability ───────────────────────────────────────────────────

#[test]
fn insert_node_preserves_leaf_id_in_intermediate() {
    let mut root = leaf("root", "b1", 1.0);
    let new = leaf("new", "b2", 1.0);
    insert_node(&mut root, new);
    // After leaf promotion, the intermediate child should keep the original root ID.
    assert!(root.children.iter().any(|c| c.id == "root" && c.data.is_some()));
    // The group wrapper should have a new ID.
    assert_ne!(root.id, "root");
    assert!(root.data.is_none());
}

#[test]
fn insert_node_preserves_extra_fields_through_promotion() {
    // When promoting a leaf to a group, the leaf's `extra` (forward-compat
    // catch-all for unknown frontend fields) must travel with the data, not
    // get defaulted on the new group wrapper.
    let mut root = leaf("root", "b1", 1.0);
    root.extra.insert(
        "futureField".into(),
        serde_json::Value::String("preserve-me".into()),
    );
    let new = leaf("new", "b2", 1.0);
    insert_node(&mut root, new);
    let intermediate = root
        .children
        .iter()
        .find(|c| c.id == "root")
        .expect("intermediate with original ID exists");
    assert_eq!(
        intermediate.extra.get("futureField"),
        Some(&serde_json::Value::String("preserve-me".into())),
    );
    // And the group wrapper should NOT have inherited the leaf's extra.
    assert!(root.extra.is_empty());
}

// ── insertNodeAtIndex leaf promotion ─────────────────────────────────────────

#[test]
fn insert_node_at_index_promotes_leaf_parent() {
    let mut root = leaf("root", "b1", 1.0);
    let new = leaf("new", "b2", 1.0);
    insert_node_at_index(&mut root, new, &[0]).unwrap();
    // Root should have been promoted to a group with 2 children:
    // the wrapped original leaf + the new node.
    assert!(root.data.is_none());
    assert_eq!(root.children.len(), 2);
    // The original leaf data should be in a child with the original root ID.
    assert!(root.children.iter().any(|c| c.id == "root" && c.data.is_some()));
}

#[test]
fn insert_node_at_index_clamps_out_of_range_segment() {
    // [a, b, c], indexArr=[10] — TS oracle: clamp to lastChildIndex=2,
    // insert at 2+1=3 → [a, b, c, new]. Rust must NOT error.
    let a = leaf("a", "b1", 1.0);
    let b = leaf("b", "b2", 1.0);
    let c = leaf("c", "b3", 1.0);
    let mut root = group("root", FlexDirection::Row, 10.0, vec![a, b, c]);
    let new = leaf("new", "b4", 1.0);
    insert_node_at_index(&mut root, new, &[10]).unwrap();
    assert_eq!(root.children.len(), 4);
    assert_eq!(root.children[3].id, "new");
}

#[test]
fn insert_node_at_index_clamps_intermediate_segment() {
    // root=[a, middle], middle=[m1, m2]. indexArr=[5, 0] — TS clamps
    // first segment to lastChildIndex=1 (descends into middle), then
    // inserts after middle.children[0] → middle = [m1, new, m2].
    let m1 = leaf("m1", "b1", 1.0);
    let m2 = leaf("m2", "b2", 1.0);
    let middle = group("middle", FlexDirection::Column, 5.0, vec![m1, m2]);
    let a = leaf("a", "b3", 5.0);
    let mut root = group("root", FlexDirection::Row, 10.0, vec![a, middle]);
    let new = leaf("new", "b4", 1.0);
    insert_node_at_index(&mut root, new, &[5, 0]).unwrap();
    let middle_after = find_node_by_id(&root, "middle").unwrap();
    assert_eq!(middle_after.children.len(), 3);
    assert_eq!(middle_after.children[0].id, "m1");
    assert_eq!(middle_after.children[1].id, "new");
    assert_eq!(middle_after.children[2].id, "m2");
}

#[test]
fn insert_node_at_index_stops_descent_at_leaf_with_extra_segments() {
    // root=[a], a is a leaf. indexArr=[0, 5] — TS hits the leaf at the
    // first descent and stops; `normalizeIndex(5)` against a leaf
    // (length-fallback=1) clamps to 0, so the result is "promote leaf,
    // insert after position 0". Final: the descended-into node is
    // promoted to a group; ensure_group_node moves the original ID
    // ("a", with the data) onto the intermediate child and gives the
    // wrapper a fresh UUID. So root.children[0] is the wrapper (no data,
    // 2 children: the renamed intermediate carrying "a" + the new node).
    let a = leaf("a", "b1", 5.0);
    let mut root = group("root", FlexDirection::Row, 10.0, vec![a]);
    let new = leaf("new", "b2", 1.0);
    insert_node_at_index(&mut root, new, &[0, 5]).unwrap();
    assert_eq!(root.children.len(), 1);
    let wrapper = &root.children[0];
    assert!(wrapper.data.is_none(), "wrapper should be a group");
    assert_eq!(wrapper.children.len(), 2);
    // Intermediate keeps the original leaf ID + data; "new" is the second.
    let intermediate = wrapper.children.iter().find(|c| c.id == "a").expect("a-id child");
    assert!(intermediate.data.is_some());
    assert!(wrapper.children.iter().any(|c| c.id == "new"));
}

// ── swapNode ancestor rejection ──────────────────────────────────────────────

#[test]
fn swap_nodes_rejects_ancestor() {
    let inner = leaf("inner", "b1", 5.0);
    let middle = group("middle", FlexDirection::Column, 5.0, vec![inner]);
    let mut root = group("root", FlexDirection::Row, 10.0, vec![middle]);
    let result = swap_nodes(&mut root, "middle", "inner");
    assert!(matches!(result, Err(LayoutError::RootCannotBeTarget)));
}

// ── moveNode ─────────────────────────────────────────────────────────────────

#[test]
fn move_node_same_parent_target_eq_cur_plus_one_is_no_op() {
    // [a, b], move a to index 1 — TS oracle: no-op (cur=0, target=cur+1).
    // The TS insert-then-remove flow inserts a copy at index 1 making
    // [a, a, b], then removes the first a, producing [a, b] unchanged.
    let a = leaf("a", "b1", 5.0);
    let b = leaf("b", "b2", 5.0);
    let mut root = group("root", FlexDirection::Row, 10.0, vec![a, b]);
    move_node(&mut root, "a", "root", 1).unwrap();
    assert_eq!(root.children[0].id, "a");
    assert_eq!(root.children[1].id, "b");
}

#[test]
fn move_node_same_parent_earlier_index() {
    let a = leaf("a", "b1", 5.0);
    let b = leaf("b", "b2", 5.0);
    let c = leaf("c", "b3", 5.0);
    let mut root = group("root", FlexDirection::Row, 10.0, vec![a, b, c]);
    move_node(&mut root, "c", "root", 0).unwrap();
    assert_eq!(root.children[0].id, "c");
    assert_eq!(root.children[1].id, "a");
    assert_eq!(root.children[2].id, "b");
}

#[test]
fn move_node_same_parent_later_index() {
    // [a, b, c], move a to index 2 — TS oracle: [b, a, c].
    // (cur=0, target=2; the TS insert-then-remove flow lands a between b
    // and c. Detach-then-insert in Rust must compensate by inserting at
    // index 1 to match.) See PR #691 Codex P1 + frontend layoutTree.ts:248.
    let a = leaf("a", "b1", 5.0);
    let b = leaf("b", "b2", 5.0);
    let c = leaf("c", "b3", 5.0);
    let mut root = group("root", FlexDirection::Row, 10.0, vec![a, b, c]);
    move_node(&mut root, "a", "root", 2).unwrap();
    assert_eq!(root.children[0].id, "b");
    assert_eq!(root.children[1].id, "a");
    assert_eq!(root.children[2].id, "c");
}

#[test]
fn move_node_same_parent_target_past_end_appends() {
    // [a, b, c], move a to index 3 — out-of-range target in same parent.
    // TS appends after the detach: addChildAt(parent, 3, a) on [a,b,c]
    // produces [a,b,c,a], then removeChild(parent, a, 0) removes index 0,
    // leaving [b, c, a]. Compensation rule (target > cur → target-1) plus
    // clamping to len gives the same result.
    let a = leaf("a", "b1", 5.0);
    let b = leaf("b", "b2", 5.0);
    let c = leaf("c", "b3", 5.0);
    let mut root = group("root", FlexDirection::Row, 10.0, vec![a, b, c]);
    move_node(&mut root, "a", "root", 3).unwrap();
    assert_eq!(root.children[0].id, "b");
    assert_eq!(root.children[1].id, "c");
    assert_eq!(root.children[2].id, "a");
}

#[test]
fn move_node_rejects_root_as_source() {
    let child = leaf("child", "b1", 5.0);
    let mut root = group("root", FlexDirection::Row, 10.0, vec![child]);
    let result = move_node(&mut root, "root", "root", 0);
    assert!(matches!(result, Err(LayoutError::RootCannotBeTarget)));
}

#[test]
fn move_node_rejects_descendant_destination() {
    let inner = leaf("inner", "b1", 5.0);
    let middle = group("middle", FlexDirection::Column, 5.0, vec![inner]);
    let mut root = group("root", FlexDirection::Row, 10.0, vec![middle]);
    let result = move_node(&mut root, "middle", "inner", 0);
    assert!(matches!(result, Err(LayoutError::NodeNotFound { .. })));
}

#[test]
fn move_node_preserves_size_same_parent() {
    let a = leaf("a", "b1", 50.0);
    let b = leaf("b", "b2", 50.0);
    let mut root = group("root", FlexDirection::Row, 10.0, vec![a, b]);
    move_node(&mut root, "a", "root", 1).unwrap();
    // Moving within same parent preserves size.
    assert_eq!(find_node_by_id(&root, "a").unwrap().size, 50.0);
}

#[test]
fn move_node_resets_size_when_changing_parent() {
    let a = leaf("a", "b1", 50.0);
    let inner = leaf("inner", "b2", 50.0);
    let middle = group("middle", FlexDirection::Column, 5.0, vec![inner]);
    let mut root = group("root", FlexDirection::Row, 10.0, vec![a, middle]);
    move_node(&mut root, "a", "middle", 0).unwrap();
    // Moving to a different parent resets size to DEFAULT_NODE_SIZE.
    assert_eq!(find_node_by_id(&root, "a").unwrap().size, DEFAULT_NODE_SIZE);
}

// ── resizeNode missing target ────────────────────────────────────────────────

#[test]
fn resize_nodes_rejects_missing_node() {
    let n1 = leaf("n1", "b1", 50.0);
    let mut root = group("root", FlexDirection::Row, 10.0, vec![n1]);
    let result = resize_nodes(&mut root, &[
        ResizeOp { node_id: "ghost".into(), size: 80.0 },
    ]);
    assert!(matches!(result, Err(LayoutError::NodeNotFound { .. })));
    // n1 should NOT have been mutated.
    assert_eq!(find_node_by_id(&root, "n1").unwrap().size, 50.0);
}

// ── Purity ───────────────────────────────────────────────────────────────────

#[test]
fn find_node_by_id_does_not_mutate() {
    let n1 = leaf("n1", "b1", 5.0);
    let root = group("root", FlexDirection::Row, 10.0, vec![n1]);
    let child_count_before = root.children.len();
    let _ = find_node_by_id(&root, "n1");
    assert_eq!(root.children.len(), child_count_before);
}
