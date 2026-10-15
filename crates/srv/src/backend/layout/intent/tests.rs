// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};

use agentmux_common::{FlexDirection, LayoutIntent, LayoutNode, LayoutNodeData, LayoutResizeOp, SplitPosition};
use proptest::prelude::*;

use super::*;

fn leaf(id: &str, block: &str) -> LayoutNode {
    LayoutNode {
        id: id.into(),
        flex_direction: FlexDirection::Row,
        size: 10.0,
        data: Some(LayoutNodeData { block_id: block.into(), ..Default::default() }),
        ..Default::default()
    }
}

fn branch(id: &str, dir: FlexDirection, children: Vec<LayoutNode>) -> LayoutNode {
    LayoutNode { id: id.into(), flex_direction: dir, size: 10.0, children, ..Default::default() }
}

fn minimized(mut node: LayoutNode) -> LayoutNode {
    node.extra.insert("minimized".into(), true.into());
    node
}

const ACTION: &str = "a0000001-0000-4000-8000-00000000abcd";

fn apply(tree: &mut Option<LayoutNode>, intent: LayoutIntent) -> Result<ViewEffect, LayoutError> {
    apply_intent(tree, intent, &mut NodeIds::new(ACTION))
}

/// Child ids of `id`'s node, in order.
fn kids(tree: &Option<LayoutNode>, id: &str) -> Vec<String> {
    let root = tree.as_ref().expect("tree");
    find_node_by_id(root, id).expect("node").children.iter().map(|c| c.id.clone()).collect()
}

fn root_kids(tree: &Option<LayoutNode>) -> Vec<String> {
    tree.as_ref().expect("tree").children.iter().map(|c| c.id.clone()).collect()
}

fn three_in_a_row() -> Option<LayoutNode> {
    Some(branch("r", FlexDirection::Row, vec![leaf("a", "ba"), leaf("b", "bb"), leaf("c", "bc")]))
}

#[test]
fn derived_ids_replace_the_first_eight_hex_digits() {
    assert_eq!(derive_node_id(ACTION, 1), "00000001-0000-4000-8000-00000000abcd");
    assert_eq!(derive_node_id(ACTION, 26), "0000001a-0000-4000-8000-00000000abcd");
    assert_eq!(derive_node_id("short", 2), "short-2");
}

#[test]
fn insert_into_an_empty_tree_makes_the_node_the_root() {
    let mut tree = None;
    let effect = apply(&mut tree, LayoutIntent::Insert { node: leaf("a", "ba"), magnified: true, focused: true }).unwrap();
    assert_eq!(tree.as_ref().unwrap().id, "a");
    // `magnified` only applies to a non-empty tree.
    assert_eq!(effect, ViewEffect { focus: Some("a".into()), ..Default::default() });
}

#[test]
fn insert_next_to_a_single_leaf_promotes_it_with_a_derived_id() {
    let mut tree = Some(leaf("a", "ba"));
    apply(&mut tree, LayoutIntent::Insert { node: leaf("n", "bn"), magnified: false, focused: false }).unwrap();
    let root = tree.as_ref().unwrap();
    assert_eq!(root.id, derive_node_id(ACTION, 1), "the branch gets the first derived id");
    assert_eq!(root_kids(&tree), ["a", "n"], "the old leaf keeps its id");
    assert_eq!(root.children[0].flex_direction, FlexDirection::Column);
}

#[test]
fn insert_fills_the_root_before_descending() {
    let mut tree = three_in_a_row();
    apply(&mut tree, LayoutIntent::Insert { node: leaf("n", "bn"), magnified: false, focused: false }).unwrap();
    assert_eq!(root_kids(&tree), ["a", "b", "c", "n"]);
}

#[test]
fn insert_skips_minimized_leaves_and_refuses_when_everything_is() {
    let mut tree = Some(branch(
        "r",
        FlexDirection::Row,
        (0..5).map(|i| minimized(leaf(&format!("m{i}"), &format!("b{i}")))).collect(),
    ));
    let err = apply(&mut tree, LayoutIntent::Insert { node: leaf("n", "bn"), magnified: false, focused: false });
    assert!(matches!(err, Err(LayoutError::Refused(_))), "{err:?}");
}

#[test]
fn split_horizontal_in_a_row_splices_and_carves_the_fraction() {
    let mut tree = three_in_a_row();
    let effect = apply(
        &mut tree,
        LayoutIntent::SplitHorizontal {
            target_node_id: "b".into(),
            new_node: leaf("n", "bn"),
            position: SplitPosition::Before,
            size_fraction: Some(0.2),
            focused: true,
        },
    )
    .unwrap();
    assert_eq!(root_kids(&tree), ["a", "n", "b", "c"]);
    let root = tree.as_ref().unwrap();
    assert_eq!(root.children[1].size, 2.0);
    assert_eq!(root.children[2].size, 8.0);
    assert_eq!(effect.focus.as_deref(), Some("n"));
}

#[test]
fn split_vertical_in_a_row_wraps_the_target_in_a_column() {
    let mut tree = three_in_a_row();
    apply(
        &mut tree,
        LayoutIntent::SplitVertical {
            target_node_id: "b".into(),
            new_node: leaf("n", "bn"),
            position: SplitPosition::After,
            size_fraction: None,
            focused: false,
        },
    )
    .unwrap();
    let group = derive_node_id(ACTION, 1);
    assert_eq!(root_kids(&tree), ["a", group.as_str(), "c"]);
    assert_eq!(kids(&tree, &group), ["b", "n"]);
    let root = tree.as_ref().unwrap();
    assert_eq!(root.children[1].flex_direction, FlexDirection::Column);
}

#[test]
fn split_of_the_root_leaf_wraps_the_root() {
    let mut tree = Some(leaf("a", "ba"));
    apply(
        &mut tree,
        LayoutIntent::SplitHorizontal {
            target_node_id: "a".into(),
            new_node: leaf("n", "bn"),
            position: SplitPosition::After,
            size_fraction: Some(0.5),
            focused: false,
        },
    )
    .unwrap();
    assert_eq!(tree.as_ref().unwrap().id, derive_node_id(ACTION, 1));
    assert_eq!(root_kids(&tree), ["a", "n"]);
}

#[test]
fn split_of_a_minimized_pane_is_refused() {
    let mut tree = Some(branch("r", FlexDirection::Row, vec![leaf("a", "ba"), minimized(leaf("m", "bm"))]));
    let err = apply(
        &mut tree,
        LayoutIntent::SplitVertical {
            target_node_id: "m".into(),
            new_node: leaf("n", "bn"),
            position: SplitPosition::After,
            size_fraction: None,
            focused: false,
        },
    );
    assert_eq!(err, Err(LayoutError::NodeLocked { id: "m".into() }));
}

#[test]
fn delete_leaves_tidying_to_the_balance_pass() {
    let mut tree = Some(branch(
        "r",
        FlexDirection::Row,
        vec![leaf("a", "ba"), branch("col", FlexDirection::Column, vec![leaf("b", "bb"), leaf("c", "bc")])],
    ));
    let effect = apply(&mut tree, LayoutIntent::Delete { node_id: "c".into() }).unwrap();
    assert_eq!(effect.unfocus.as_deref(), Some("c"));
    // The one-leaf column collapses into that leaf, keeping the column's own
    // direction and size (as `balanceNode` does).
    let root = tree.as_ref().unwrap();
    assert_eq!(root_kids(&tree), ["a", "b"]);
    assert_eq!(root.children[1].flex_direction, FlexDirection::Column);
}

#[test]
fn delete_of_the_root_empties_the_tree() {
    let mut tree = Some(leaf("a", "ba"));
    apply(&mut tree, LayoutIntent::Delete { node_id: "a".into() }).unwrap();
    assert!(tree.is_none());
    assert!(matches!(
        apply(&mut tree, LayoutIntent::Delete { node_id: "a".into() }),
        Err(LayoutError::NodeNotFound { .. })
    ));
}

#[test]
fn move_within_a_parent_lands_one_slot_earlier_when_moving_right() {
    let mut tree = three_in_a_row();
    let mv = |index| LayoutIntent::Move { node_id: "a".into(), parent_id: Some("r".into()), index, insert_at_root: false };
    apply(&mut tree, mv(2)).unwrap();
    assert_eq!(root_kids(&tree), ["b", "a", "c"], "index 2 before the removal is index 1 after it");
    let size = tree.as_ref().unwrap().children[1].size;
    assert_eq!(size, 10.0, "a move within its parent keeps the size");
}

#[test]
fn move_onto_a_leaf_promotes_it_and_resets_the_size() {
    let mut tree = Some(branch(
        "r",
        FlexDirection::Row,
        vec![leaf("a", "ba"), LayoutNode { size: 30.0, ..leaf("b", "bb") }, leaf("c", "bc")],
    ));
    apply(
        &mut tree,
        LayoutIntent::Move { node_id: "b".into(), parent_id: Some("c".into()), index: 0, insert_at_root: false },
    )
    .unwrap();
    let group = derive_node_id(ACTION, 1);
    assert_eq!(root_kids(&tree), ["a", group.as_str()]);
    assert_eq!(kids(&tree, &group), ["b", "c"]);
    let root = tree.as_ref().unwrap();
    assert_eq!(root.children[1].children[0].size, DEFAULT_NODE_SIZE);
}

#[test]
fn move_refuses_what_would_corrupt_the_tree() {
    let nested = || {
        Some(branch(
            "r",
            FlexDirection::Row,
            vec![leaf("a", "ba"), branch("col", FlexDirection::Column, vec![leaf("b", "bb"), leaf("c", "bc")])],
        ))
    };
    let cases = [
        (LayoutIntent::Move { node_id: "col".into(), parent_id: Some("b".into()), index: 0, insert_at_root: false }, "into its own subtree"),
        (LayoutIntent::Move { node_id: "r".into(), parent_id: Some("a".into()), index: 0, insert_at_root: false }, "the root"),
        (LayoutIntent::Move { node_id: "a".into(), parent_id: Some("col".into()), index: -1, insert_at_root: false }, "a negative index"),
        (LayoutIntent::Move { node_id: "a".into(), parent_id: None, index: 0, insert_at_root: true }, "insertAtRoot"),
        (LayoutIntent::Move { node_id: "zz".into(), parent_id: Some("r".into()), index: 0, insert_at_root: false }, "a missing node"),
    ];
    for (intent, what) in cases {
        let mut tree = nested();
        assert!(apply(&mut tree, intent).is_err(), "move with {what} must be refused");
    }
}

#[test]
fn swap_exchanges_nodes_and_each_slot_keeps_its_size() {
    let mut tree = Some(branch(
        "r",
        FlexDirection::Row,
        vec![LayoutNode { size: 30.0, ..leaf("a", "ba") }, leaf("b", "bb"), leaf("c", "bc")],
    ));
    apply(&mut tree, LayoutIntent::Swap { node1_id: "a".into(), node2_id: "c".into() }).unwrap();
    let root = tree.as_ref().unwrap();
    assert_eq!(root_kids(&tree), ["c", "b", "a"]);
    assert_eq!((root.children[0].size, root.children[2].size), (30.0, 10.0));
}

#[test]
fn swap_refuses_the_root_itself_and_ancestors() {
    let mut tree = Some(branch(
        "r",
        FlexDirection::Row,
        vec![leaf("a", "ba"), branch("col", FlexDirection::Column, vec![leaf("b", "bb"), leaf("c", "bc")])],
    ));
    for (n1, n2) in [("a", "a"), ("r", "a"), ("col", "b")] {
        let before = tree.clone();
        assert!(apply(&mut tree.clone(), LayoutIntent::Swap { node1_id: n1.into(), node2_id: n2.into() }).is_err());
        assert_eq!(tree, before);
    }
    tree = None;
    assert!(apply(&mut tree, LayoutIntent::Swap { node1_id: "a".into(), node2_id: "b".into() }).is_err());
}

#[test]
fn resize_is_all_or_nothing() {
    let mut tree = three_in_a_row();
    let ops = |size| {
        vec![
            LayoutResizeOp { node_id: "a".into(), size: 5.0 },
            LayoutResizeOp { node_id: "b".into(), size },
        ]
    };
    assert!(apply(&mut tree.clone(), LayoutIntent::Resize { resize_operations: ops(150.0) }).is_err());
    apply(&mut tree, LayoutIntent::Resize { resize_operations: ops(15.0) }).unwrap();
    let root = tree.as_ref().unwrap();
    assert_eq!((root.children[0].size, root.children[1].size), (5.0, 15.0));
}

#[test]
fn resize_of_a_minimized_pane_is_refused() {
    let mut tree = Some(branch("r", FlexDirection::Row, vec![leaf("a", "ba"), minimized(leaf("m", "bm"))]));
    let ops = vec![LayoutResizeOp { node_id: "m".into(), size: 5.0 }];
    assert_eq!(apply(&mut tree, LayoutIntent::Resize { resize_operations: ops }), Err(LayoutError::NodeLocked { id: "m".into() }));
}

#[test]
fn replace_keeps_the_slot_size() {
    let mut tree = Some(branch("r", FlexDirection::Row, vec![LayoutNode { size: 30.0, ..leaf("a", "ba") }, leaf("b", "bb")]));
    let effect = apply(
        &mut tree,
        LayoutIntent::Replace { target_node_id: "a".into(), new_node: leaf("n", "bn"), focused: true },
    )
    .unwrap();
    let root = tree.as_ref().unwrap();
    assert_eq!(root_kids(&tree), ["n", "b"]);
    assert_eq!(root.children[0].size, 30.0);
    assert_eq!(effect.focus.as_deref(), Some("n"));
}

#[test]
fn insert_at_index_follows_the_path_and_inserts_after_it() {
    let mut tree = Some(branch(
        "r",
        FlexDirection::Row,
        vec![leaf("a", "ba"), branch("col", FlexDirection::Column, vec![leaf("b", "bb"), leaf("c", "bc")])],
    ));
    apply(
        &mut tree,
        LayoutIntent::InsertAtIndex { node: leaf("n", "bn"), index_arr: vec![1, 0], magnified: false, focused: false },
    )
    .unwrap();
    assert_eq!(kids(&tree, "col"), ["b", "n", "c"]);
    assert!(apply(
        &mut tree,
        LayoutIntent::InsertAtIndex { node: leaf("x", "bx"), index_arr: vec![], magnified: false, focused: false },
    )
    .is_err());
}

#[test]
fn clear_and_view() {
    let mut tree = three_in_a_row();
    apply(&mut tree, LayoutIntent::View).unwrap();
    let balanced = tree.clone();
    assert_eq!(apply(&mut tree, LayoutIntent::View).unwrap(), ViewEffect::default());
    assert_eq!(tree, balanced, "a view intent leaves a normalized tree as it is");
    let effect = apply(&mut tree, LayoutIntent::Clear).unwrap();
    assert!(tree.is_none() && effect.clear);
}

#[test]
fn legacy_minimize_fields_migrate_to_the_flag() {
    let mut slipped = leaf("s", "bs");
    slipped.size = 1.0;
    slipped.extra.insert("slipMinimize".into(), serde_json::json!({"targetColumnId": "x"}));
    let mut squeezed = leaf("q", "bq");
    squeezed.size = 2.0;
    squeezed.extra.insert("minimizedSize".into(), 25.into());
    squeezed.extra.insert("minimizedLockedSize".into(), 2.into());
    let mut root = branch("r", FlexDirection::Row, vec![leaf("a", "ba"), slipped, squeezed]);
    root.extra.insert("_slipAnchor".into(), true.into());
    assert_eq!(migrate_legacy_minimize(&mut root), 4);
    let flag = |n: &LayoutNode| n.extra.get("minimized").and_then(|v| v.as_bool());
    assert_eq!((root.children[1].size, flag(&root.children[1])), (6.0, Some(true)), "slip: mean of a (10) and q (2)");
    assert_eq!((root.children[2].size, flag(&root.children[2])), (25.0, Some(true)));
    assert!(root.extra.is_empty() && root.children[2].extra.len() == 1);
    assert_eq!(migrate_legacy_minimize(&mut root), 0, "a current tree is untouched");
}

// ── Random sequences ───────────────────────────────────────────────────────
//
// One generator drives both the property test below and the vectors the
// frontend replays (`frontend/layout/tests/layoutIntentVectors.test.ts`), so
// srv and the frontend's own reducer are checked on the same sequences.

/// xorshift64*: deterministic, so the vectors file is reproducible.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn one_in(&mut self, n: u64) -> bool {
        self.next().is_multiple_of(n)
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

struct Generator {
    rng: Rng,
    next_block: u32,
    next_action: u32,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Step {
    action_id: String,
    intent: LayoutIntent,
    applied: bool,
}

fn all_nodes(tree: &Option<LayoutNode>) -> Vec<&LayoutNode> {
    fn walk<'a>(n: &'a LayoutNode, out: &mut Vec<&'a LayoutNode>) {
        out.push(n);
        for c in &n.children {
            walk(c, out);
        }
    }
    let mut out = Vec::new();
    if let Some(root) = tree {
        walk(root, &mut out);
    }
    out
}

impl Generator {
    fn new(seed: u64) -> Self {
        Self { rng: Rng::new(seed), next_block: 0, next_action: 0 }
    }

    fn new_leaf(&mut self) -> LayoutNode {
        self.next_block += 1;
        leaf(&format!("n{}", self.next_block), &format!("b{}", self.next_block))
    }

    fn action_id(&mut self) -> String {
        self.next_action += 1;
        format!("a{:07x}-0000-4000-8000-{:012x}", self.next_action, self.next_action)
    }

    /// A tree built by inserts and splits, with some panes minimized (never all).
    fn initial_tree(&mut self) -> Option<LayoutNode> {
        let mut tree = None;
        for _ in 0..1 + self.rng.below(6) {
            let intent = if tree.is_none() { self.insert() } else { self.split(&tree) };
            let id = self.action_id();
            let _ = apply_intent(&mut tree, intent, &mut NodeIds::new(&id));
        }
        if let Some(root) = tree.as_mut() {
            let leaves = count_leaves(root);
            let mut left_expanded = leaves;
            mark_minimized(root, &mut self.rng, &mut left_expanded);
        }
        tree
    }

    fn insert(&mut self) -> LayoutIntent {
        LayoutIntent::Insert { node: self.new_leaf(), magnified: self.rng.one_in(8), focused: self.rng.one_in(2) }
    }

    fn split(&mut self, tree: &Option<LayoutNode>) -> LayoutIntent {
        let leaves: Vec<String> = all_nodes(tree).iter().filter(|n| n.children.is_empty()).map(|n| n.id.clone()).collect();
        let target_node_id = self.rng.pick(&leaves).clone();
        let new_node = self.new_leaf();
        let position = if self.rng.one_in(2) { SplitPosition::Before } else { SplitPosition::After };
        let size_fraction = *self.rng.pick(&[None, Some(0.5), Some(0.2)]);
        let focused = self.rng.one_in(2);
        if self.rng.one_in(2) {
            LayoutIntent::SplitHorizontal { target_node_id, new_node, position, size_fraction, focused }
        } else {
            LayoutIntent::SplitVertical { target_node_id, new_node, position, size_fraction, focused }
        }
    }

    /// An intent of the shapes the frontend sends: leaves as targets, a move
    /// never into the moved node itself.
    fn intent(&mut self, tree: &Option<LayoutNode>) -> LayoutIntent {
        let nodes = all_nodes(tree);
        let Some(root) = tree else { return self.insert() };
        let leaves: Vec<String> = nodes.iter().filter(|n| n.children.is_empty()).map(|n| n.id.clone()).collect();
        let non_root_leaves: Vec<String> = leaves.iter().filter(|id| **id != root.id).cloned().collect();
        let branches: Vec<&LayoutNode> = nodes.iter().copied().filter(|n| !n.children.is_empty()).collect();
        match self.rng.below(40) {
            0..=5 => self.insert(),
            6..=15 => self.split(tree),
            16..=19 => LayoutIntent::Delete { node_id: self.rng.pick(&leaves).clone() },
            20..=25 if !non_root_leaves.is_empty() => {
                let node_id = self.rng.pick(&non_root_leaves).clone();
                let parents: Vec<&LayoutNode> = nodes.iter().copied().filter(|n| n.id != node_id).collect();
                let parent = *self.rng.pick(&parents);
                let index = self.rng.below(parent.children.len() + 2) as i64;
                LayoutIntent::Move { node_id, parent_id: Some(parent.id.clone()), index, insert_at_root: false }
            }
            26..=28 if non_root_leaves.len() >= 2 => {
                let node1_id = self.rng.pick(&non_root_leaves).clone();
                let others: Vec<String> = non_root_leaves.iter().filter(|id| **id != node1_id).cloned().collect();
                LayoutIntent::Swap { node1_id, node2_id: self.rng.pick(&others).clone() }
            }
            29..=32 if !branches.is_empty() => {
                let parent = *self.rng.pick(&branches);
                let sizes = [2.5, 5.0, 7.5, 10.0, 12.5, 20.0];
                let mut ops = Vec::new();
                for child in parent.children.iter().take(2) {
                    ops.push(LayoutResizeOp { node_id: child.id.clone(), size: *self.rng.pick(&sizes) });
                }
                LayoutIntent::Resize { resize_operations: ops }
            }
            33..=34 => LayoutIntent::Replace {
                target_node_id: self.rng.pick(&leaves).clone(),
                new_node: self.new_leaf(),
                focused: self.rng.one_in(2),
            },
            35..=36 => {
                let mut index_arr: Vec<i64> = (0..1 + self.rng.below(3)).map(|_| self.rng.below(4) as i64).collect();
                if self.rng.one_in(3) {
                    *index_arr.last_mut().unwrap() = -1 - self.rng.below(2) as i64;
                }
                LayoutIntent::InsertAtIndex { node: self.new_leaf(), index_arr, magnified: false, focused: self.rng.one_in(2) }
            }
            37 if self.rng.one_in(6) => LayoutIntent::Clear,
            _ => LayoutIntent::View,
        }
    }

    /// Apply a random sequence; returns the starting tree, the steps and the
    /// tree after each one.
    fn sequence(&mut self, len: usize) -> (Option<LayoutNode>, Vec<Step>, Vec<Option<LayoutNode>>) {
        let initial = self.initial_tree();
        let mut tree = initial.clone();
        let mut steps = Vec::new();
        let mut trees = Vec::new();
        for _ in 0..len {
            let intent = self.intent(&tree);
            let action_id = self.action_id();
            let mut next = tree.clone();
            let applied = apply_intent(&mut next, intent.clone(), &mut NodeIds::new(&action_id)).is_ok();
            if applied {
                tree = next;
            }
            steps.push(Step { action_id, intent, applied });
            trees.push(tree.clone());
        }
        (initial, steps, trees)
    }
}

fn count_leaves(node: &LayoutNode) -> usize {
    if node.children.is_empty() {
        1
    } else {
        node.children.iter().map(count_leaves).sum()
    }
}

fn mark_minimized(node: &mut LayoutNode, rng: &mut Rng, expanded: &mut usize) {
    if node.children.is_empty() {
        if *expanded > 1 && rng.one_in(5) {
            node.extra.insert("minimized".into(), true.into());
            *expanded -= 1;
        }
        return;
    }
    for child in &mut node.children {
        mark_minimized(child, rng, expanded);
    }
}

/// The blocks a sequence leaves in the tree, from the intents alone.
fn expected_blocks(initial: &Option<LayoutNode>, steps: &[Step], trees_before: &[Option<LayoutNode>]) -> Vec<String> {
    let mut blocks: HashSet<String> = blocks_of(initial).into_iter().collect();
    for (step, before) in steps.iter().zip(trees_before) {
        if !step.applied {
            continue;
        }
        match &step.intent {
            LayoutIntent::Insert { node, .. } | LayoutIntent::InsertAtIndex { node, .. } => {
                blocks.extend(blocks_of(&Some(node.clone())));
            }
            LayoutIntent::SplitHorizontal { new_node, .. } | LayoutIntent::SplitVertical { new_node, .. } => {
                blocks.extend(blocks_of(&Some(new_node.clone())));
            }
            LayoutIntent::Replace { target_node_id, new_node, .. } => {
                let target = before.as_ref().and_then(|r| find_node_by_id(r, target_node_id)).cloned();
                for b in blocks_of(&target) {
                    blocks.remove(&b);
                }
                blocks.extend(blocks_of(&Some(new_node.clone())));
            }
            LayoutIntent::Delete { node_id } => {
                let target = before.as_ref().and_then(|r| find_node_by_id(r, node_id)).cloned();
                for b in blocks_of(&target) {
                    blocks.remove(&b);
                }
            }
            LayoutIntent::Clear => blocks.clear(),
            _ => {}
        }
    }
    let mut out: Vec<String> = blocks.into_iter().collect();
    out.sort();
    out
}

fn blocks_of(tree: &Option<LayoutNode>) -> Vec<String> {
    all_nodes(tree).iter().filter_map(|n| n.data.as_ref().map(|d| d.block_id.clone())).collect()
}

/// The invariants every tree an intent produces holds.
fn check_tree(tree: &Option<LayoutNode>) -> Result<(), String> {
    let violations: Vec<String> = super::super::validate_layout_invariants(tree)
        .into_iter()
        .filter(|v| !v.starts_with("ALL_LEAVES_LOCKED"))
        .collect();
    if !violations.is_empty() {
        return Err(format!("violations {violations:?}"));
    }
    let mut ids = HashMap::new();
    for node in all_nodes(tree) {
        if ids.insert(node.id.clone(), ()).is_some() {
            return Err(format!("duplicate node id {}", node.id));
        }
        if node.data.is_none() && node.children.is_empty() {
            return Err(format!("empty branch {}", node.id));
        }
        if node.children.iter().any(|c| c.children.len() == 1 && c.children[0].children.is_empty()) {
            return Err(format!("a branch under {} holds a single leaf", node.id));
        }
    }
    let blocks = blocks_of(tree);
    let unique: HashSet<&String> = blocks.iter().collect();
    if unique.len() != blocks.len() {
        return Err(format!("a block in two panes: {blocks:?}"));
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// A random sequence of intents keeps the tree normalized and valid, and
    /// keeps exactly the panes the intents say it should.
    #[test]
    fn random_intent_sequences_keep_every_invariant(seed in any::<u64>(), len in 1usize..40) {
        let mut generator = Generator::new(seed);
        let (initial, steps, trees) = generator.sequence(len);
        prop_assert!(check_tree(&initial).is_ok(), "{:?}", check_tree(&initial));
        let mut befores = vec![initial.clone()];
        befores.extend(trees.iter().cloned());
        for (i, tree) in trees.iter().enumerate() {
            prop_assert!(check_tree(tree).is_ok(), "after step {} {:?}: {:?}", i, steps[i], check_tree(tree));
            let mut got = blocks_of(tree);
            got.sort();
            prop_assert_eq!(&got, &expected_blocks(&initial, &steps[..=i], &befores[..=i]), "blocks after step {}", i);
        }
    }
}

// ── Vectors shared with the frontend ───────────────────────────────────────

const VECTORS: &str = "../../frontend/layout/tests/fixtures/layout-intent-vectors.json";
const VECTOR_COUNT: u64 = 80;
const VECTOR_STEPS: usize = 14;

fn vectors_json() -> serde_json::Value {
    let vectors: Vec<serde_json::Value> = (1..=VECTOR_COUNT)
        .map(|seed| {
            let mut generator = Generator::new(seed);
            let (initial, steps, trees) = generator.sequence(VECTOR_STEPS);
            serde_json::json!({
                "seed": seed,
                "initial": initial,
                "steps": steps,
                "final": trees.last().cloned().flatten(),
            })
        })
        .collect();
    serde_json::json!({
        "about": "Generated by crates/srv/src/backend/layout/intent/tests.rs (srv's result for each sequence). Regenerate with UPDATE_LAYOUT_INTENT_VECTORS=1 cargo test -p agentmux-srv intent_vectors.",
        "vectors": vectors,
    })
}

/// srv's results for the vectors are the committed ones; the frontend replays
/// the same file through its own reducer.
#[test]
fn intent_vectors_match_the_committed_file() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(VECTORS);
    let generated = vectors_json();
    if std::env::var_os("UPDATE_LAYOUT_INTENT_VECTORS").is_some() {
        // One vector per line: a reviewable diff without a megabyte of indentation.
        let lines: Vec<String> = generated["vectors"].as_array().unwrap().iter().map(|v| v.to_string()).collect();
        let text = format!("{{\"about\":{},\n\"vectors\":[\n{}\n]}}\n", generated["about"], lines.join(",\n"));
        std::fs::write(&path, text).unwrap();
        return;
    }
    let committed: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display())),
    )
    .unwrap();
    assert!(
        committed == generated,
        "{} is stale: regenerate it with UPDATE_LAYOUT_INTENT_VECTORS=1 cargo test -p agentmux-srv intent_vectors",
        path.display()
    );
}

#[test]
fn vectors_exercise_every_intent_and_some_refusals() {
    let json = vectors_json();
    let mut seen: HashSet<String> = HashSet::new();
    let mut refused = 0;
    for v in json["vectors"].as_array().unwrap() {
        for step in v["steps"].as_array().unwrap() {
            seen.insert(step["intent"]["type"].as_str().unwrap().to_string());
            if step["applied"] == false {
                refused += 1;
            }
        }
    }
    for kind in ["insert", "insertatindex", "delete", "move", "swap", "resize", "replace", "splithorizontal", "splitvertical", "view"] {
        assert!(seen.contains(kind), "no {kind} in the vectors");
    }
    assert!(refused > 0, "the vectors should include refused intents");
}

/// When the frontend disagrees with a vector: write srv's tree after each
/// step of one seed, to find the first step that differs.
/// `SEED=7 OUT=steps.json cargo test -p agentmux-srv dump_vector_steps -- --ignored`
#[test]
#[ignore]
fn dump_vector_steps() {
    let seed: u64 = std::env::var("SEED").unwrap().parse().unwrap();
    let mut generator = Generator::new(seed);
    let (initial, steps, trees) = generator.sequence(VECTOR_STEPS);
    let mut out = vec![serde_json::json!({"initial": initial})];
    for (s, t) in steps.iter().zip(trees.iter()) {
        out.push(serde_json::json!({"step": s, "tree": t}));
    }
    std::fs::write(std::env::var("OUT").unwrap(), serde_json::to_string_pretty(&out).unwrap()).unwrap();
}
