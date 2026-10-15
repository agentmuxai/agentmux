// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Ported tests for the Rust layout helpers. These mirror the 30 tests
//! from `frontend/layout/tests/layoutTree.test.ts` (PR #686) — the
//! TypeScript test suite is the behavioral oracle.

use agentmux_common::{FlexDirection, LayoutNode, LayoutNodeData, StackMovePosition};
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

/// Leaf with an explicit flex direction (the `leaf` helper hardcodes Row;
/// the balance oracle cares about each leaf's own direction).
fn leaf_dir(id: &str, block_id: &str, dir: FlexDirection) -> LayoutNode {
    LayoutNode {
        id: id.into(),
        flex_direction: dir,
        size: DEFAULT_NODE_SIZE,
        data: Some(LayoutNodeData {
            block_id: block_id.into(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

// ── balanceNode (ported from frontend/layout/tests/layoutNode.test.ts) ──────

#[test]
fn balance_corrects_flex_directions() {
    // Row[ Row-leaf, Row-leaf ] → children flipped to Column.
    let mut node = group(
        "n",
        FlexDirection::Row,
        DEFAULT_NODE_SIZE,
        vec![
            leaf_dir("a", "node1Inner1", FlexDirection::Row),
            leaf_dir("b", "node1Inner2", FlexDirection::Row),
        ],
    );
    balance_node(&mut node).unwrap();
    assert!(node.data.is_none(), "root should remain a branch");
    assert_ne!(node.children[0].flex_direction, node.flex_direction);
}

#[test]
fn balance_collapses_single_grandchild_1() {
    // Row[ Column[ Row-leaf node1 ] ] → collapses to leaf node1.
    let mut node = group(
        "n",
        FlexDirection::Row,
        DEFAULT_NODE_SIZE,
        vec![group(
            "c",
            FlexDirection::Column,
            DEFAULT_NODE_SIZE,
            vec![leaf_dir("g", "node1", FlexDirection::Row)],
        )],
    );
    balance_node(&mut node).unwrap();
    assert!(node.children.is_empty(), "should collapse to a leaf");
    assert_eq!(node.data.unwrap().block_id, "node1");
}

#[test]
fn balance_collapses_single_grandchild_2() {
    // Row[ Column[ Row[ Column-leaf i1, Column-leaf i2 ] ] ] → hoist to 2 children.
    let mut node = group(
        "n",
        FlexDirection::Row,
        DEFAULT_NODE_SIZE,
        vec![group(
            "c",
            FlexDirection::Column,
            DEFAULT_NODE_SIZE,
            vec![group(
                "gc",
                FlexDirection::Row,
                DEFAULT_NODE_SIZE,
                vec![
                    leaf_dir("i1", "node2Inner1", FlexDirection::Column),
                    leaf_dir("i2", "node2Inner2", FlexDirection::Column),
                ],
            )],
        )],
    );
    balance_node(&mut node).unwrap();
    assert_eq!(node.children.len(), 2);
    assert_eq!(
        node.children[0].data.as_ref().unwrap().block_id,
        "node2Inner1"
    );
}

#[test]
fn balance_collapses_single_grandchild_3() {
    // Row[ Column[ Row[ Column-leaf node3 ] ] ] → collapses to leaf node3.
    let mut node = group(
        "n",
        FlexDirection::Row,
        DEFAULT_NODE_SIZE,
        vec![group(
            "c",
            FlexDirection::Column,
            DEFAULT_NODE_SIZE,
            vec![group(
                "gc",
                FlexDirection::Row,
                DEFAULT_NODE_SIZE,
                vec![leaf_dir("ggc", "node3", FlexDirection::Column)],
            )],
        )],
    );
    balance_node(&mut node).unwrap();
    assert!(node.children.is_empty());
    assert_eq!(node.data.unwrap().block_id, "node3");
}

#[test]
fn balance_collapses_single_grandchild_4() {
    // Row[ Column[ Row[ Column[ Row-leaf i1, Row-leaf i2 ] ] ] ]
    // → node.children.len()==1, grandchildren len==2.
    let mut node = group(
        "n",
        FlexDirection::Row,
        DEFAULT_NODE_SIZE,
        vec![group(
            "c",
            FlexDirection::Column,
            DEFAULT_NODE_SIZE,
            vec![group(
                "gc",
                FlexDirection::Row,
                DEFAULT_NODE_SIZE,
                vec![group(
                    "ggc",
                    FlexDirection::Column,
                    DEFAULT_NODE_SIZE,
                    vec![
                        leaf_dir("i1", "node4Inner1", FlexDirection::Row),
                        leaf_dir("i2", "node4Inner2", FlexDirection::Row),
                    ],
                )],
            )],
        )],
    );
    balance_node(&mut node).unwrap();
    assert_eq!(node.children.len(), 1);
    assert_eq!(node.children[0].children.len(), 2);
    assert_eq!(
        node.children[0].children[0].data.as_ref().unwrap().block_id,
        "node4Inner1"
    );
}

#[test]
fn balance_rejects_invalid_node() {
    // A node with BOTH data and children violates leaf-XOR-branch.
    let mut bad = LayoutNode {
        id: "bad".into(),
        flex_direction: FlexDirection::Row,
        size: DEFAULT_NODE_SIZE,
        children: vec![leaf("x", "b", 1.0)],
        data: Some(LayoutNodeData {
            block_id: "both".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(matches!(
        balance_node(&mut bad),
        Err(LayoutError::InvalidNode { .. })
    ));
}

#[test]
fn balance_slip_anchor_suppresses_hoist() {
    // Row[ Column(_slipAnchor)[ Row[ leaf1, leaf2 ] ] ] — without the slip
    // anchor the single-child-branch chain would hoist/collapse; the anchor
    // preserves it.
    let mut anchored = group(
        "c",
        FlexDirection::Column,
        DEFAULT_NODE_SIZE,
        vec![group(
            "gc",
            FlexDirection::Row,
            DEFAULT_NODE_SIZE,
            vec![
                leaf_dir("l1", "b1", FlexDirection::Column),
                leaf_dir("l2", "b2", FlexDirection::Column),
            ],
        )],
    );
    anchored
        .extra
        .insert("_slipAnchor".into(), serde_json::Value::Bool(true));
    let mut node = group("n", FlexDirection::Row, DEFAULT_NODE_SIZE, vec![anchored]);
    balance_node(&mut node).unwrap();
    // The anchored Column wrapper survives (not hoisted), still wrapping gc.
    assert_eq!(node.children.len(), 1);
    assert_eq!(node.children[0].children.len(), 1, "chain preserved by anchor");
}

#[test]
fn balance_column_dissolve_suppresses_direction_flip() {
    // Sibling(Column)[ dissolved-colA(Column, columnDissolve)[ leaf1, leaf2 ], leaf3 ]
    // — mirrors what layoutMinimize's `_dissolveColumn` produces: a dissolved
    // column nested as the first child of a same-direction Column sibling.
    // Without the columnDissolve guard, the unconditional direction-alternation
    // rule would flip colA to Row, laying leaf1/leaf2 out side-by-side instead
    // of stacked.
    let mut dissolved_col = group(
        "colA",
        FlexDirection::Column,
        DEFAULT_NODE_SIZE,
        vec![
            leaf_dir("l1", "b1", FlexDirection::Row),
            leaf_dir("l2", "b2", FlexDirection::Row),
        ],
    );
    dissolved_col
        .extra
        .insert("columnDissolve".into(), serde_json::json!({"targetColumnId": "sibling"}));
    let mut sibling = group(
        "sibling",
        FlexDirection::Column,
        DEFAULT_NODE_SIZE,
        vec![dissolved_col, leaf_dir("l3", "b3", FlexDirection::Row)],
    );
    balance_node(&mut sibling).unwrap();
    assert_eq!(
        sibling.children[0].flex_direction,
        FlexDirection::Column,
        "dissolved column must stay Column so its stacked minimized panes render vertically"
    );
}

// ── Minimize lock (locked-state spec, 2026-07-16) ───────────────────────────

/// Leaf carrying the display-mode minimize flag (current model — geometry
/// derived at render, stored size untouched).
fn minimized_leaf(id: &str, block_id: &str, size: f32) -> LayoutNode {
    let mut node = leaf(id, block_id, size);
    node.minimized = true;
    node
}

#[test]
fn validate_invariants_flags_display_mode_flag_on_branch() {
    let mut branch = group(
        "br",
        FlexDirection::Column,
        DEFAULT_NODE_SIZE,
        vec![leaf("l1", "b1", 100.0), leaf("l2", "b2", 100.0)],
    );
    branch.minimized = true;
    let root = Some(group(
        "root",
        FlexDirection::Row,
        DEFAULT_NODE_SIZE,
        vec![branch, leaf("l3", "b3", 100.0)],
    ));
    let violations = validate_layout_invariants(&root);
    assert!(
        violations.iter().any(|v| v.starts_with("MIN_MARKER_ON_BRANCH")),
        "expected MIN_MARKER_ON_BRANCH, got: {:?}",
        violations
    );
}

#[test]
fn validate_invariants_flags_all_leaves_locked_via_display_flag() {
    let root = Some(group(
        "root",
        FlexDirection::Column,
        DEFAULT_NODE_SIZE,
        vec![minimized_leaf("l1", "b1", 200.0), minimized_leaf("l2", "b2", 200.0)],
    ));
    let violations = validate_layout_invariants(&root);
    assert!(
        violations.iter().any(|v| v.starts_with("ALL_LEAVES_LOCKED")),
        "expected ALL_LEAVES_LOCKED, got: {:?}",
        violations
    );
}

/// LEGACY MODEL — leaf carrying the pre-display-mode minimize-lock fields
/// (round-tripped through `extra`): `minimizedSize` (original size) +
/// `minimizedLockedSize` (the size the node was locked to while minimized).
/// These markers remain recognized until persisted trees are migrated by the
/// frontend's `rebuildMinimizedSet`.
fn locked_leaf(id: &str, block_id: &str, locked_size: f32) -> LayoutNode {
    let mut node = leaf(id, block_id, locked_size);
    node.extra
        .insert("minimizedSize".into(), serde_json::json!(200.0));
    node.extra
        .insert("minimizedLockedSize".into(), serde_json::json!(locked_size));
    node
}

#[test]
fn validate_invariants_clean_on_healthy_locked_tree() {
    let root = Some(group(
        "root",
        FlexDirection::Column,
        DEFAULT_NODE_SIZE,
        vec![locked_leaf("l1", "b1", 33.0), leaf("l2", "b2", 367.0)],
    ));
    assert!(validate_layout_invariants(&root).is_empty());
    assert!(validate_layout_invariants(&None).is_empty());
}

#[test]
fn validate_invariants_flags_the_recovered_0_53_6_corruption() {
    // Fixture: the exact corrupted shape recovered from a live v0.53.6
    // instance's db_layout (issue #2179) — a BRANCH carrying the leaf-only
    // `minimizedSize` marker, with a slipped leaf trapped inside it.
    let mut term = leaf("term", "b-term", 2.2);
    term.extra
        .insert("slipMinimize".into(), serde_json::json!({"targetColumnId": "x"}));
    let agent = leaf("agent", "b-agent", 4.4);
    let mut mid = group("mid", FlexDirection::Row, 6.8, vec![term, agent]);
    mid.extra.insert("minimizedSize".into(), serde_json::json!(9.0));
    let mut outer = group("outer", FlexDirection::Column, 30.0, vec![mid]);
    outer.extra.insert("_slipAnchor".into(), serde_json::Value::Bool(true));
    let root = Some(group(
        "root",
        FlexDirection::Row,
        10.0,
        vec![outer, leaf("memory", "b-memory", 10.0)],
    ));

    let violations = validate_layout_invariants(&root);
    assert!(
        violations.iter().any(|v| v.starts_with("MIN_MARKER_ON_BRANCH")),
        "expected MIN_MARKER_ON_BRANCH, got: {:?}",
        violations
    );
}

#[test]
fn validate_invariants_flags_all_leaves_locked() {
    // Every leaf minimize-locked = an all-headers window with nothing
    // restorable in view (the frontend's last-expanded-pane guard exists to
    // prevent exactly this).
    let root = Some(group(
        "root",
        FlexDirection::Column,
        DEFAULT_NODE_SIZE,
        vec![locked_leaf("l1", "b1", 33.0), locked_leaf("l2", "b2", 33.0)],
    ));
    let violations = validate_layout_invariants(&root);
    assert!(
        violations.iter().any(|v| v.starts_with("ALL_LEAVES_LOCKED")),
        "expected ALL_LEAVES_LOCKED, got: {:?}",
        violations
    );
    // A healthy mixed tree does not trip it (covered by
    // validate_invariants_clean_on_healthy_locked_tree).
}

#[test]
fn validate_invariants_flags_flipped_dissolve_lock_mismatch_and_intruder() {
    // A dissolved column with Row direction (#2176 signature), a tampered
    // locked size, and an unlocked zero-size intruder child.
    let mut tampered = locked_leaf("l1", "b1", 33.0);
    tampered.size = 120.0;
    let intruder = leaf("l9", "b9", 0.0);
    let mut dissolved = group("colA", FlexDirection::Row, 66.0, vec![tampered, intruder]);
    dissolved
        .extra
        .insert("columnDissolve".into(), serde_json::json!({"targetColumnId": "host"}));
    let root = Some(group(
        "host",
        FlexDirection::Column,
        400.0,
        vec![dissolved, leaf("content", "b3", 300.0)],
    ));

    let violations = validate_layout_invariants(&root);
    for expected in [
        "DISSOLVED_NOT_COLUMN",
        "LOCK_SIZE_MISMATCH",
        "DISSOLVED_CHILD_UNLOCKED",
        "NONPOSITIVE_SIZE",
    ] {
        assert!(
            violations.iter().any(|v| v.starts_with(expected)),
            "expected {}, got: {:?}",
            expected,
            violations
        );
    }
}

#[test]
fn balance_drops_empty_branch_child() {
    // Row[ leaf1, leaf2, empty-branch ] → empty branch dropped, 2 remain.
    let mut node = group(
        "n",
        FlexDirection::Row,
        DEFAULT_NODE_SIZE,
        vec![
            leaf_dir("l1", "b1", FlexDirection::Column),
            leaf_dir("l2", "b2", FlexDirection::Column),
            group("empty", FlexDirection::Row, DEFAULT_NODE_SIZE, vec![]),
        ],
    );
    balance_node(&mut node).unwrap();
    assert_eq!(node.children.len(), 2);
    assert!(node.children.iter().all(|c| c.data.is_some()));
}

// ── isInstanceLabel filter (JS analogue: insert location) ──────────────────

// ── insertNodeAtIndex ────────────────────────────────────────────────────────

// ── swapNode ──────────────────────────────────────────────────────────────────

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
fn delete_node_keeps_the_promoted_leafs_minimized_flag() {
    let mut kept = leaf("n1", "keep", 5.0);
    kept.minimized = true;
    let mut root = group("root", FlexDirection::Row, 10.0, vec![kept, leaf("n2", "drop", 5.0)]);
    delete_node(&mut root, "n2").unwrap();
    assert_eq!(root.id, "n1");
    assert!(root.minimized, "the promoted leaf stays minimized");
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

// ── replaceNode ──────────────────────────────────────────────────────────────

// ── Purity ───────────────────────────────────────────────────────────────────

// ── insertNode ID stability ───────────────────────────────────────────────────

// ── insertNodeAtIndex leaf promotion ─────────────────────────────────────────

// ── swapNode ancestor rejection ──────────────────────────────────────────────

// ── moveNode ─────────────────────────────────────────────────────────────────

// ── resizeNode missing target ────────────────────────────────────────────────

// ── Purity ───────────────────────────────────────────────────────────────────

#[test]
fn find_node_by_id_does_not_mutate() {
    let n1 = leaf("n1", "b1", 5.0);
    let root = group("root", FlexDirection::Row, 10.0, vec![n1]);
    let child_count_before = root.children.len();
    let _ = find_node_by_id(&root, "n1");
    assert_eq!(root.children.len(), child_count_before);
}

// ── pruneDanglingBlockRefs ────────────────────────────────────────────────────
// Reducer-write-path self-healing added after
// docs/investigations/INVESTIGATION_LAYOUT_DEAD_SPACE_STALE_TREE_RESURRECTION_2026_07_08.md —
// "no layout leaf ever references a nonexistent block" as an unconditional
// invariant of the write path, not something individual callers must
// remember to uphold correctly.

fn live(ids: &[&str]) -> std::collections::HashSet<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

#[test]
fn prune_is_a_noop_on_a_clean_tree() {
    let mut root = Some(group(
        "root",
        FlexDirection::Row,
        10.0,
        vec![leaf("a", "ba", 5.0), leaf("b", "bb", 5.0)],
    ));
    let before = root.clone();
    let pruned = prune_dangling_block_refs(&mut root, &live(&["ba", "bb"]));
    assert_eq!(pruned, 0);
    assert_eq!(root, before);
}

#[test]
fn prune_is_a_noop_on_an_empty_tree() {
    let mut root: Option<LayoutNode> = None;
    let pruned = prune_dangling_block_refs(&mut root, &live(&["ba"]));
    assert_eq!(pruned, 0);
    assert!(root.is_none());
}

#[test]
fn prune_clears_the_tree_when_root_is_the_sole_dangling_leaf() {
    // The exact shape of the bug this closes: a single-block tab whose
    // block was deleted, leaving the root itself as the dangling leaf.
    let mut root = Some(leaf("only", "deleted-block", 1.0));
    let pruned = prune_dangling_block_refs(&mut root, &live(&[]));
    assert_eq!(pruned, 1);
    assert!(root.is_none());
}

#[test]
fn prune_removes_a_non_root_dangling_leaf_and_collapses_the_sole_survivor() {
    let mut root = Some(group(
        "root",
        FlexDirection::Row,
        10.0,
        vec![leaf("a", "ba", 5.0), leaf("b", "deleted-block", 5.0)],
    ));
    let pruned = prune_dangling_block_refs(&mut root, &live(&["ba"]));
    assert_eq!(pruned, 1);
    // delete_node's single-child collapse promotes "a" into root's place —
    // same behavior an explicit user delete would produce.
    let r = root.unwrap();
    assert_eq!(r.id, "a");
    assert_eq!(r.data.as_ref().unwrap().block_id, "ba");
    assert!(r.children.is_empty());
}

#[test]
fn prune_removes_multiple_dangling_leaves_in_one_pass() {
    let mut root = Some(group(
        "root",
        FlexDirection::Row,
        10.0,
        vec![
            leaf("a", "ba", 3.0),
            leaf("b", "gone-1", 3.0),
            leaf("c", "gone-2", 3.0),
        ],
    ));
    let pruned = prune_dangling_block_refs(&mut root, &live(&["ba"]));
    assert_eq!(pruned, 2);
    let r = root.unwrap();
    // Down to the sole survivor — same single-child collapse as above.
    assert_eq!(r.data.as_ref().unwrap().block_id, "ba");
}

#[test]
fn prune_leaves_container_nodes_alone() {
    // Container/group nodes have data: None — must never be mistaken for
    // a dangling leaf just because their (nonexistent) block_id can't
    // match anything live.
    let mut root = Some(group(
        "root",
        FlexDirection::Row,
        10.0,
        vec![leaf("a", "ba", 5.0)],
    ));
    let pruned = prune_dangling_block_refs(&mut root, &live(&["ba"]));
    assert_eq!(pruned, 0);
    assert_eq!(root.unwrap().id, "root");
}

// ── prune, in-pane-tabs stack-member awareness ──────────────────────────────
// SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md §4.3.

fn leaf_with_stack(id: &str, active: &str, stack: &[&str], size: f32) -> LayoutNode {
    LayoutNode {
        id: id.into(),
        flex_direction: FlexDirection::Row,
        size,
        data: Some(LayoutNodeData {
            block_id: active.into(),
            active_block_id: active.into(),
            block_stack: stack.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[test]
fn prune_is_a_noop_when_every_stack_member_is_live() {
    let mut root = Some(leaf_with_stack("a", "b1", &["b1", "b2", "b3"], 1.0));
    let pruned = prune_dangling_block_refs(&mut root, &live(&["b1", "b2", "b3"]));
    assert_eq!(pruned, 0);
    assert_eq!(
        root.unwrap().data.unwrap().block_stack,
        vec!["b1".to_string(), "b2".to_string(), "b3".to_string()]
    );
}

#[test]
fn prune_drops_dangling_non_active_stack_members_without_touching_the_leaf() {
    // b2's underlying block was deleted independently — the leaf itself
    // stays fully intact (still renders fine via its active member b1),
    // only the stale stack entry is removed.
    let mut root = Some(leaf_with_stack("a", "b1", &["b1", "b2", "b3"], 1.0));
    let pruned = prune_dangling_block_refs(&mut root, &live(&["b1", "b3"]));
    assert_eq!(pruned, 1);
    let r = root.unwrap();
    assert_eq!(r.id, "a"); // leaf survives untouched
    let data = r.data.unwrap();
    assert_eq!(data.block_id, "b1");
    assert_eq!(data.block_stack, vec!["b1".to_string(), "b3".to_string()]);
}

#[test]
fn prune_never_removes_the_active_member_from_its_own_stack() {
    // The active member is guaranteed live by the whole-leaf pass before
    // stack-member pruning ever runs, but this asserts the invariant
    // directly rather than only indirectly via that ordering.
    let mut root = Some(leaf_with_stack("a", "b1", &["b1", "b2"], 1.0));
    let pruned = prune_dangling_block_refs(&mut root, &live(&["b1"]));
    assert_eq!(pruned, 1);
    let data = root.unwrap().data.unwrap();
    assert_eq!(data.block_stack, vec!["b1".to_string()]);
}

#[test]
fn prune_stack_members_recurses_into_group_children() {
    let mut root = Some(group(
        "root",
        FlexDirection::Row,
        10.0,
        vec![
            leaf("a", "ba", 5.0),
            leaf_with_stack("b", "b1", &["b1", "gone"], 5.0),
        ],
    ));
    let pruned = prune_dangling_block_refs(&mut root, &live(&["ba", "b1"]));
    assert_eq!(pruned, 1);
    let r = root.unwrap();
    let stacked_leaf = &r.children[1];
    assert_eq!(
        stacked_leaf.data.as_ref().unwrap().block_stack,
        vec!["b1".to_string()]
    );
}

#[test]
fn prune_whole_leaf_pass_and_stack_member_pass_compose_in_one_call() {
    // A leaf whose ACTIVE block is gone (whole-leaf pruning fires) alongside
    // a sibling whose non-active stack member is gone (stack-member pruning
    // fires) — one prune_dangling_block_refs call must catch both, with the
    // combined count.
    let mut root = Some(group(
        "root",
        FlexDirection::Row,
        10.0,
        vec![
            leaf("a", "deleted-active", 5.0),
            leaf_with_stack("b", "b1", &["b1", "deleted-member"], 5.0),
        ],
    ));
    let pruned = prune_dangling_block_refs(&mut root, &live(&["b1"]));
    assert_eq!(pruned, 2);
    // "a" was the dangling leaf and got collapsed away; "b" survives with
    // its dangling stack member removed.
    let r = root.unwrap();
    assert_eq!(r.id, "b");
    assert_eq!(
        r.data.unwrap().block_stack,
        vec!["b1".to_string()]
    );
}

// ── prune, dangling ACTIVE member with a live sibling (review finding) ──────
// A leaf's active block can go dangling through a path other than
// closeBlockInStack (which always reactivates a live neighbor first) — the
// whole-leaf pass must not delete the leaf out from under still-live
// sibling blocks; it must reactivate one of them instead.

#[test]
fn prune_reactivates_a_live_sibling_when_the_active_member_goes_dangling() {
    // active = "b1" (dangling); stack still lists both b1 and the live b2.
    let mut root = Some(leaf_with_stack("a", "b1", &["b1", "b2"], 1.0));
    let pruned = prune_dangling_block_refs(&mut root, &live(&["b2"]));
    assert_eq!(pruned, 1);
    let r = root.unwrap();
    assert_eq!(r.id, "a"); // leaf survives, not deleted
    let data = r.data.unwrap();
    assert_eq!(data.block_id, "b2");
    assert_eq!(data.active_block_id, "b2");
    assert_eq!(data.block_stack, vec!["b2".to_string()]); // dangling b1 dropped
}

#[test]
fn prune_reactivates_the_first_live_member_when_several_siblings_are_also_dangling() {
    let mut root = Some(leaf_with_stack("a", "b1", &["b1", "gone", "b3"], 1.0));
    let pruned = prune_dangling_block_refs(&mut root, &live(&["b3"]));
    // 1 for the active-member reactivation + 1 for "gone" pruned from the
    // stack by the (still-running) non-active stack-member pass.
    assert_eq!(pruned, 2);
    let data = root.unwrap().data.unwrap();
    assert_eq!(data.block_id, "b3");
    assert_eq!(data.block_stack, vec!["b3".to_string()]);
}

#[test]
fn prune_deletes_the_leaf_when_the_active_member_dangles_and_no_sibling_is_live() {
    // Every member of the stack (including the active one) is dead — the
    // whole leaf is genuinely gone, same as if it had no stack at all.
    let mut root = Some(leaf_with_stack("only", "b1", &["b1", "b2"], 1.0));
    let pruned = prune_dangling_block_refs(&mut root, &live(&[]));
    assert_eq!(pruned, 1);
    assert!(root.is_none());
}

#[test]
fn prune_active_member_repair_composes_with_a_sibling_leaf_that_is_fully_dangling() {
    let mut root = Some(group(
        "root",
        FlexDirection::Row,
        10.0,
        vec![
            leaf("a", "deleted-active", 5.0),
            leaf_with_stack("b", "b1", &["b1", "b2"], 5.0),
        ],
    ));
    let pruned = prune_dangling_block_refs(&mut root, &live(&["b2"]));
    assert_eq!(pruned, 2); // "a" deleted + "b" repaired
    let r = root.unwrap();
    assert_eq!(r.id, "b");
    let data = r.data.unwrap();
    assert_eq!(data.block_id, "b2");
    assert_eq!(data.block_stack, vec!["b2".to_string()]);
}

// ── pane tabs (SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.2, §3.6) ─────

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn next_visible_member_prefers_the_right_hand_neighbour_then_the_left() {
    // [a, b, c] with b removed → c; with c removed → b (the new last).
    assert_eq!(next_visible_member(&strings(&["a", "c"]), 1, |_| true), Some("c".into()));
    assert_eq!(next_visible_member(&strings(&["a", "b"]), 2, |_| true), Some("b".into()));
    // Ineligible neighbours are skipped, right side first.
    assert_eq!(next_visible_member(&strings(&["a", "x", "c"]), 1, |m| m != "x"), Some("c".into()));
    assert_eq!(next_visible_member(&strings(&["a", "x"]), 1, |m| m != "x"), Some("a".into()));
    assert_eq!(next_visible_member(&strings(&["x"]), 0, |m| m != "x"), None);
}

#[test]
fn a_dangling_visible_tab_is_replaced_by_its_right_hand_live_neighbour() {
    // Used to pick the FIRST live member ("a"), disagreeing with the
    // frontend's right-neighbour rule. Visible "b" dies → "c".
    let mut root = Some(leaf_with_stack("pane", "b", &["a", "b", "c"], 1.0));
    prune_dangling_block_refs(&mut root, &live(&["a", "c"]));
    let data = root.unwrap().data.unwrap();
    assert_eq!(data.block_id, "c");
    assert_eq!(data.active_block_id, "c");
    assert_eq!(data.block_stack, strings(&["a", "c"]));
}

#[test]
fn push_stack_member_turns_a_single_block_pane_into_a_stack() {
    let mut root = leaf("pane", "a", 1.0);
    assert!(push_stack_member(&mut root, "a", "b", true));
    let data = root.data.unwrap();
    assert_eq!(data.block_stack, strings(&["a", "b"]));
    assert_eq!(data.block_id, "b");
    assert_eq!(data.active_block_id, "b");
}

#[test]
fn push_stack_member_finds_the_pane_by_a_background_member_and_can_stay_in_the_background() {
    let mut root = group(
        "root",
        FlexDirection::Row,
        10.0,
        vec![leaf("other", "z", 1.0), leaf_with_stack("pane", "a", &["a", "b"], 1.0)],
    );
    assert!(push_stack_member(&mut root, "b", "c", false));
    let data = root.children[1].data.clone().unwrap();
    assert_eq!(data.block_stack, strings(&["a", "b", "c"]));
    assert_eq!(data.block_id, "a", "not activated");
    assert!(!push_stack_member(&mut root, "missing", "d", true), "no pane holds the target");
}

#[test]
fn push_stack_member_does_not_duplicate_an_existing_member() {
    let mut root = leaf_with_stack("pane", "a", &["a", "b"], 1.0);
    assert!(push_stack_member(&mut root, "a", "b", true));
    let data = root.data.unwrap();
    assert_eq!(data.block_stack, strings(&["a", "b"]));
    assert_eq!(data.block_id, "b");
}

#[test]
fn activate_stack_member_switches_the_visible_tab() {
    let mut root = leaf_with_stack("pane", "a", &["a", "b"], 1.0);
    assert!(activate_stack_member(&mut root, "b"));
    let data = root.data.clone().unwrap();
    assert_eq!((data.block_id.as_str(), data.active_block_id.as_str()), ("b", "b"));
    assert!(!activate_stack_member(&mut root, "nope"));
}

// ── move_stack_member (SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §4.1) ──────

#[test]
fn move_stack_member_is_a_noop_when_block_and_target_are_the_same() {
    let mut root = leaf_with_stack("pane", "a", &["a", "b", "c"], 1.0);
    let before = root.clone();
    assert!(move_stack_member(&mut root, "b", "b", StackMovePosition::After, false));
    assert_eq!(root, before);
}

// ReAgent P1 on PR #3441: the self-target short-circuit above used to fire
// BEFORE any existence check, reporting success (and leaving `activate`
// silently ignored) even for a block that doesn't exist anywhere.

#[test]
fn move_stack_member_self_target_refuses_when_the_block_does_not_exist() {
    let mut root = leaf_with_stack("pane", "a", &["a", "b"], 1.0);
    let before = root.clone();
    assert!(!move_stack_member(&mut root, "missing", "missing", StackMovePosition::After, false));
    assert_eq!(root, before);
}

#[test]
fn move_stack_member_self_target_still_activates_when_requested() {
    // A background member dropped onto itself with `activate: true` must
    // still become visible — every other branch of this function honors
    // `activate`; the self-target short-circuit must not be an exception.
    let mut root = leaf_with_stack("pane", "a", &["a", "b", "c"], 1.0);
    assert!(move_stack_member(&mut root, "b", "b", StackMovePosition::After, true));
    let data = root.data.unwrap();
    assert_eq!(data.block_stack, strings(&["a", "b", "c"]), "position unchanged — it's a self-target");
    assert_eq!((data.block_id.as_str(), data.active_block_id.as_str()), ("b", "b"));
}

#[test]
fn move_stack_member_reorders_within_the_same_leaf_before_target() {
    let mut root = leaf_with_stack("pane", "a", &["a", "b", "c"], 1.0);
    assert!(move_stack_member(&mut root, "c", "b", StackMovePosition::Before, false));
    let data = root.data.unwrap();
    assert_eq!(data.block_stack, strings(&["a", "c", "b"]));
}

#[test]
fn move_stack_member_reorders_within_the_same_leaf_after_target() {
    let mut root = leaf_with_stack("pane", "a", &["a", "b", "c"], 1.0);
    assert!(move_stack_member(&mut root, "a", "b", StackMovePosition::After, false));
    let data = root.data.unwrap();
    assert_eq!(data.block_stack, strings(&["b", "a", "c"]));
}

#[test]
fn move_stack_member_reorders_to_end_within_the_same_leaf() {
    let mut root = leaf_with_stack("pane", "a", &["a", "b", "c"], 1.0);
    assert!(move_stack_member(&mut root, "a", "b", StackMovePosition::End, false));
    let data = root.data.unwrap();
    assert_eq!(data.block_stack, strings(&["b", "c", "a"]));
}

#[test]
fn move_stack_member_reorder_does_not_change_which_tab_is_visible() {
    // The currently-visible member is reordered without `activate` — it
    // must stay visible. A naive remove-then-reinsert composition (reusing
    // `remove_stack_member`'s neighbour-activation as-is) would wrongly
    // switch the visible tab here, since that function's reassignment rule
    // assumes the member is actually leaving its leaf.
    let mut root = leaf_with_stack("pane", "b", &["a", "b", "c"], 1.0);
    assert!(move_stack_member(&mut root, "b", "c", StackMovePosition::After, false));
    let data = root.data.unwrap();
    assert_eq!(data.block_stack, strings(&["a", "c", "b"]));
    assert_eq!(data.block_id, "b", "reordering the visible tab must not switch away from it");
    assert_eq!(data.active_block_id, "b");
}

#[test]
fn move_stack_member_moves_across_leaves_removing_from_source() {
    let mut root = group(
        "root",
        FlexDirection::Row,
        10.0,
        vec![leaf_with_stack("src", "a", &["a", "b"], 1.0), leaf_with_stack("dst", "c", &["c", "d"], 1.0)],
    );
    assert!(move_stack_member(&mut root, "b", "d", StackMovePosition::After, false));
    let src = root.children[0].data.clone().unwrap();
    assert_eq!(src.block_stack, strings(&["a"]), "removed from the source leaf");
    let dst = root.children[1].data.clone().unwrap();
    assert_eq!(dst.block_stack, strings(&["c", "d", "b"]), "placed after the target in the destination leaf");
    assert_eq!(dst.block_id, "c", "not activated");
}

#[test]
fn move_stack_member_cross_leaf_promotes_a_single_block_destination_into_a_stack() {
    let mut root = group(
        "root",
        FlexDirection::Row,
        10.0,
        vec![leaf_with_stack("src", "a", &["a", "b"], 1.0), leaf("dst", "c", 1.0)],
    );
    assert!(move_stack_member(&mut root, "b", "c", StackMovePosition::Before, true));
    let dst = root.children[1].data.clone().unwrap();
    assert_eq!(dst.block_stack, strings(&["b", "c"]));
    assert_eq!((dst.block_id.as_str(), dst.active_block_id.as_str()), ("b", "b"), "activate: true");
}

// ── moving a pane's ONLY tab removes the emptied pane
// (SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md §4.3) ──

#[test]
fn move_stack_member_last_member_moves_and_removes_the_source_leaf() {
    let mut root = group(
        "root",
        FlexDirection::Row,
        10.0,
        vec![
            leaf("src", "a", 1.0),
            leaf_with_stack("dst", "c", &["c", "d"], 1.0),
            leaf("other", "z", 1.0),
        ],
    );
    assert!(move_stack_member(&mut root, "a", "d", StackMovePosition::After, true));
    let ids: Vec<&str> = root.children.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, vec!["dst", "other"], "the emptied source pane is gone, the others untouched");
    let dst = root.children[0].data.clone().unwrap();
    assert_eq!(dst.block_stack, strings(&["c", "d", "a"]), "placed after the target");
    assert_eq!((dst.block_id.as_str(), dst.active_block_id.as_str()), ("a", "a"), "activate: true");
    assert!(validate_layout_invariants(&Some(root)).is_empty());
}

#[test]
fn move_stack_member_last_member_collapses_a_single_child_parent() {
    // src and dst are the only two children: deleting src leaves the root
    // with one child, which delete_node collapses by copying that child's
    // id/data up into the root. Placement is by block id, so the moved tab
    // still ends up in (what is now) the root.
    let mut root = group(
        "root",
        FlexDirection::Row,
        10.0,
        vec![leaf("src", "a", 1.0), leaf_with_stack("dst", "c", &["c", "d"], 1.0)],
    );
    assert!(move_stack_member(&mut root, "a", "c", StackMovePosition::End, false));
    assert!(root.children.is_empty(), "collapsed to a single leaf: {root:?}");
    assert_eq!(root.id, "dst", "the root absorbed the surviving leaf");
    let data = root.data.clone().unwrap();
    assert_eq!(data.block_stack, strings(&["c", "d", "a"]));
    assert_eq!(data.block_id, "c", "not activated");
    assert!(validate_layout_invariants(&Some(root)).is_empty());
}

#[test]
fn move_stack_member_last_member_into_a_single_block_destination_promotes_it_to_a_stack() {
    let mut root = group(
        "root",
        FlexDirection::Row,
        10.0,
        vec![leaf("src", "a", 1.0), leaf("dst", "c", 1.0), leaf("other", "z", 1.0)],
    );
    assert!(move_stack_member(&mut root, "a", "c", StackMovePosition::Before, true));
    let dst = root.children[0].data.clone().unwrap();
    assert_eq!(root.children[0].id, "dst");
    assert_eq!(dst.block_stack, strings(&["a", "c"]));
    assert_eq!((dst.block_id.as_str(), dst.active_block_id.as_str()), ("a", "a"));
    assert!(validate_layout_invariants(&Some(root)).is_empty());
}

#[test]
fn move_stack_member_last_member_with_missing_target_leaves_tree_untouched() {
    let mut root = group(
        "root",
        FlexDirection::Row,
        10.0,
        vec![leaf("src", "a", 1.0), leaf("dst", "c", 1.0)],
    );
    let before = root.clone();
    assert!(!move_stack_member(&mut root, "a", "missing", StackMovePosition::End, true));
    assert_eq!(root, before, "no partial mutation when the destination doesn't exist");
}

#[test]
fn move_stack_member_root_leaf_only_tab_has_nowhere_to_go() {
    // A lone root leaf has nowhere else to go: the "target" can only be the
    // block itself (a self-target no-op) or a block that isn't in the tree.
    let mut root = leaf("solo", "a", 1.0);
    let before = root.clone();
    assert!(!move_stack_member(&mut root, "a", "missing", StackMovePosition::End, false));
    assert_eq!(root, before);
}

#[test]
fn source_leaf_emptied_by_move_reports_only_the_emptying_case() {
    let root = group(
        "root",
        FlexDirection::Row,
        10.0,
        vec![leaf("src", "a", 1.0), leaf_with_stack("dst", "c", &["c", "d"], 1.0)],
    );
    assert_eq!(source_leaf_emptied_by_move(&root, "a", "d"), Some("src".to_string()), "lone tab to another pane");
    assert_eq!(source_leaf_emptied_by_move(&root, "c", "a"), None, "multi-member source survives");
    assert_eq!(source_leaf_emptied_by_move(&root, "c", "d"), None, "same-pane reorder");
    assert_eq!(source_leaf_emptied_by_move(&root, "a", "a"), None, "self-target");
    assert_eq!(source_leaf_emptied_by_move(&root, "a", "missing"), None, "no destination");
    assert_eq!(source_leaf_emptied_by_move(&root, "missing", "c"), None, "no source");
    assert_eq!(source_leaf_emptied_by_move(&leaf("solo", "a", 1.0), "a", "x"), None, "root leaf");
}

#[test]
fn move_stack_member_refuses_when_target_block_is_not_in_any_leaf() {
    let mut root = leaf_with_stack("pane", "a", &["a", "b"], 1.0);
    let before = root.clone();
    assert!(!move_stack_member(&mut root, "a", "missing", StackMovePosition::After, false));
    assert_eq!(root, before, "must not partially mutate when the destination doesn't exist");
}

#[test]
fn move_stack_member_refuses_when_source_block_is_not_in_any_leaf() {
    let mut root = leaf_with_stack("pane", "a", &["a", "b"], 1.0);
    let before = root.clone();
    assert!(!move_stack_member(&mut root, "missing", "a", StackMovePosition::After, false));
    assert_eq!(root, before);
}

#[test]
fn invariants_flag_an_inconsistent_stack() {
    let mut bad = leaf_with_stack("pane", "a", &["b", "b"], 1.0);
    bad.data.as_mut().unwrap().active_block_id = "b".into();
    let violations = validate_layout_invariants(&Some(bad));
    assert!(violations.iter().any(|v| v.starts_with("STACK_VISIBLE_NOT_MEMBER")), "{violations:?}");
    assert!(violations.iter().any(|v| v.starts_with("STACK_ACTIVE_MISMATCH")), "{violations:?}");
    assert!(violations.iter().any(|v| v.starts_with("STACK_DUPLICATE_MEMBER")), "{violations:?}");
    assert!(validate_layout_invariants(&Some(leaf_with_stack("ok", "a", &["a", "b"], 1.0))).is_empty());
}

// ── I1: each of a tab's blocks is in exactly one pane ───────────────────────

#[test]
fn invariants_flag_a_block_in_two_panes() {
    let tree = group(
        "root",
        FlexDirection::Row,
        1.0,
        vec![leaf_with_stack("p1", "a", &["a", "b"], 1.0), leaf("p2", "b", 1.0)],
    );
    let violations = validate_layout_invariants(&Some(tree));
    assert!(violations.iter().any(|v| v.starts_with("BLOCK_IN_TWO_PANES @ b")), "{violations:?}");
    let healthy = group("root", FlexDirection::Row, 1.0, vec![leaf_with_stack("p1", "a", &["a", "b"], 1.0), leaf("p2", "c", 1.0)]);
    assert!(validate_layout_invariants(&Some(healthy)).is_empty());
}

#[test]
fn unplaced_blocks_counts_background_tabs_as_placed() {
    let tree = Some(group(
        "root",
        FlexDirection::Row,
        1.0,
        vec![leaf_with_stack("p1", "a", &["a", "b"], 1.0), leaf("p2", "c", 1.0)],
    ));
    let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert!(unplaced_blocks(&tree, &ids(&["a", "b", "c"])).is_empty());
    assert_eq!(unplaced_blocks(&tree, &ids(&["a", "d", "c", "e"])), ids(&["d", "e"]));
    assert_eq!(unplaced_blocks(&None, &ids(&["a"])), ids(&["a"]));
}
