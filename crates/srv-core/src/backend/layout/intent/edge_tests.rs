// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `movetoedge`: a pane becomes a band across the whole layout. The cases below say what the layout is after it; the
//! vectors at the end are shared with the frontend, which predicts the same trees (`layoutEdgeVectors.test.ts`).

use agentmux_common::{FlexDirection, LayoutEdge, LayoutIntent, LayoutNode, LayoutNodeData};

use super::*;

const ACTION: &str = "a0000001-0000-4000-8000-00000000abcd";

fn leaf(id: &str) -> LayoutNode {
    LayoutNode {
        id: id.into(),
        flex_direction: FlexDirection::Row,
        size: 10.0,
        data: Some(LayoutNodeData { block_id: format!("b-{id}"), ..Default::default() }),
        ..Default::default()
    }
}

fn branch(id: &str, dir: FlexDirection, children: Vec<LayoutNode>) -> LayoutNode {
    LayoutNode { id: id.into(), flex_direction: dir, size: 10.0, children, ..Default::default() }
}

fn row(id: &str, children: Vec<LayoutNode>) -> LayoutNode {
    branch(id, FlexDirection::Row, children)
}

fn col(id: &str, children: Vec<LayoutNode>) -> LayoutNode {
    branch(id, FlexDirection::Column, children)
}

fn to_edge(tree: &LayoutNode, node: &str, edge: LayoutEdge) -> Result<LayoutNode, LayoutError> {
    let mut root = Some(tree.clone());
    apply_intent(&mut root, LayoutIntent::MoveToEdge { node_id: node.into(), edge }, &mut NodeIds::new(ACTION))?;
    Ok(root.expect("a tree is left"))
}

/// A tree as nested text: a pane is its id, a row `(a b)`, a column `[a b]`.
fn shape(node: &LayoutNode) -> String {
    if node.children.is_empty() {
        return node.id.clone();
    }
    let inner: Vec<String> = node.children.iter().map(shape).collect();
    match node.flex_direction {
        FlexDirection::Row => format!("({})", inner.join(" ")),
        FlexDirection::Column => format!("[{}]", inner.join(" ")),
    }
}

fn after(tree: &LayoutNode, node: &str, edge: LayoutEdge) -> String {
    shape(&to_edge(tree, node, edge).expect("applied"))
}

#[test]
fn a_band_across_the_top_or_the_bottom_of_a_row_wraps_the_rest_in_a_column() {
    let tree = row("r", vec![leaf("a"), leaf("b"), leaf("c")]);
    assert_eq!(after(&tree, "b", LayoutEdge::Top), "[b (a c)]");
    assert_eq!(after(&tree, "b", LayoutEdge::Bottom), "[(a c) b]");
    // The band is a third of the layout: the rest has twice its flex size.
    let wrapped = to_edge(&tree, "b", LayoutEdge::Top).unwrap();
    assert_eq!((wrapped.children[0].size, wrapped.children[1].size), (10.0, 20.0));
    assert_eq!(wrapped.id, derive_node_id(ACTION, 1), "the new column is the first node the action made");
}

#[test]
fn a_band_into_a_column_of_rows_is_one_more_row() {
    let tree = col("r", vec![leaf("a"), leaf("b"), leaf("c"), leaf("d")]);
    assert_eq!(after(&tree, "c", LayoutEdge::Top), "[c a b d]");
    assert_eq!(after(&tree, "a", LayoutEdge::Bottom), "[b c d a]");
    // The middle: after half of the others.
    assert_eq!(after(&tree, "a", LayoutEdge::Middle), "[b c a d]");
    assert_eq!(after(&tree, "d", LayoutEdge::Middle), "[a b d c]");
}

#[test]
fn what_the_pane_left_behind_closes_before_the_band_is_made() {
    // The pane beside a stack: the stack is what is left, and the band joins it as a row.
    let tree = row("r", vec![leaf("x"), col("c", vec![leaf("a"), leaf("b")])]);
    assert_eq!(after(&tree, "x", LayoutEdge::Top), "[x a b]");
    assert_eq!(after(&tree, "x", LayoutEdge::Bottom), "[a b x]");
    // A pane inside a nested group: that group closes around the other one.
    let nested = row("r", vec![leaf("a"), col("c", vec![leaf("b"), row("d", vec![leaf("e"), leaf("f")])])]);
    assert_eq!(after(&nested, "f", LayoutEdge::Bottom), "[(a [b e]) f]");
}

#[test]
fn a_band_across_the_middle_of_a_row_puts_its_first_half_above_and_the_rest_below() {
    let four = row("r", vec![leaf("a"), leaf("b"), leaf("c"), leaf("d")]);
    assert_eq!(after(&four, "a", LayoutEdge::Middle), "[(b c) a d]");
    assert_eq!(after(&four, "d", LayoutEdge::Middle), "[(a b) d c]");
    let halves = to_edge(&four, "a", LayoutEdge::Middle).unwrap();
    assert_eq!(halves.children.iter().map(|c| c.size).collect::<Vec<_>>(), [15.0, 10.0, 15.0]);
    // Two panes beside the band: one above, one below.
    let three = row("r", vec![leaf("a"), leaf("b"), leaf("c")]);
    assert_eq!(after(&three, "b", LayoutEdge::Middle), "[a b c]");
}

#[test]
fn a_pane_with_no_other_to_part_from_has_no_middle_and_nothing_changes() {
    let two = row("r", vec![leaf("a"), leaf("b")]);
    let mut root = Some(two.clone());
    let refused = apply_intent(&mut root, LayoutIntent::MoveToEdge { node_id: "a".into(), edge: LayoutEdge::Middle }, &mut NodeIds::new(ACTION));
    assert!(matches!(refused, Err(LayoutError::Refused(_))), "{refused:?}");
    assert_eq!(root, Some(two));
    // The top and the bottom of two panes are fine.
    let two = row("r", vec![leaf("a"), leaf("b")]);
    assert_eq!(after(&two, "a", LayoutEdge::Top), "[a b]");
    assert_eq!(after(&two, "a", LayoutEdge::Bottom), "[b a]");
}

#[test]
fn the_root_a_missing_pane_and_a_strip_of_chips_are_refused() {
    let tree = row("r", vec![leaf("a"), leaf("b")]);
    assert!(matches!(to_edge(&tree, "r", LayoutEdge::Top), Err(LayoutError::RootCannotBeTarget)));
    assert!(matches!(to_edge(&tree, "zz", LayoutEdge::Top), Err(LayoutError::NodeNotFound { .. })));
    let mut strip = col("s", vec![leaf("m1"), leaf("m2")]);
    strip.children.iter_mut().for_each(|c| c.minimized = true);
    let tree = row("r", vec![leaf("a"), strip, leaf("b")]);
    assert!(matches!(to_edge(&tree, "s", LayoutEdge::Top), Err(LayoutError::NodeLocked { .. })));
}

#[test]
fn a_pane_that_is_already_the_whole_top_row_stays_the_top_row() {
    let tree = col("r", vec![leaf("a"), leaf("b"), leaf("c")]);
    assert_eq!(after(&tree, "a", LayoutEdge::Top), "[a b c]");
    assert_eq!(after(&tree, "c", LayoutEdge::Bottom), "[a b c]");
    // Its size is its own again: an even share of the rows.
    let same = to_edge(&tree, "a", LayoutEdge::Top).unwrap();
    assert!(same.children.iter().all(|c| c.size == 10.0));
}

// ── Vectors shared with the frontend ───────────────────────────────────────

const EDGE_VECTORS: &str = "../../frontend/layout/tests/fixtures/layout-edge-vectors.json";

fn edge_trees() -> Vec<(&'static str, LayoutNode)> {
    vec![
        ("two side by side", row("r", vec![leaf("a"), leaf("b")])),
        ("three in a row", row("r", vec![leaf("a"), leaf("b"), leaf("c")])),
        ("a column of three", col("r", vec![leaf("a"), leaf("b"), leaf("c")])),
        ("four in a row", row("r", vec![leaf("a"), leaf("b"), leaf("c"), leaf("d")])),
        ("a pane beside a stack", row("r", vec![leaf("a"), col("s", vec![leaf("b"), leaf("c")])])),
        (
            "rows in a column, uneven",
            col("r", vec![row("r1", vec![leaf("a"), leaf("b")]), row("r2", vec![leaf("c"), leaf("d"), leaf("e")])]),
        ),
        (
            "three columns, one nested",
            row("r", vec![leaf("a"), col("s", vec![leaf("b"), row("t", vec![leaf("c"), leaf("d")])]), leaf("e")]),
        ),
    ]
}

fn edge_vectors_json() -> serde_json::Value {
    let mut cases = Vec::new();
    for (name, tree) in edge_trees() {
        for node in leaf_ids(&tree) {
            for edge in [LayoutEdge::Top, LayoutEdge::Bottom, LayoutEdge::Middle] {
                let result = to_edge(&tree, &node, edge);
                cases.push(serde_json::json!({
                    "name": name,
                    "initial": tree,
                    "nodeId": node,
                    "edge": edge,
                    "actionId": ACTION,
                    "applied": result.is_ok(),
                    "final": result.as_ref().ok().unwrap_or(&tree),
                }));
            }
        }
    }
    serde_json::json!({
        "about": "Generated by crates/srv-core/src/backend/layout/intent/edge_tests.rs (srv's result for each case). Regenerate with UPDATE_LAYOUT_EDGE_VECTORS=1 cargo test -p agentmux-srv-core edge_vectors.",
        "cases": cases,
    })
}

#[test]
fn edge_vectors_match_the_committed_file() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(EDGE_VECTORS);
    let generated = edge_vectors_json();
    if std::env::var_os("UPDATE_LAYOUT_EDGE_VECTORS").is_some() {
        // One case per line: a reviewable diff.
        let lines: Vec<String> = generated["cases"].as_array().unwrap().iter().map(|v| v.to_string()).collect();
        let text = format!("{{\"about\":{},\n\"cases\":[\n{}\n]}}\n", generated["about"], lines.join(",\n"));
        std::fs::write(&path, text).unwrap();
    }
    let committed: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}; generate it with UPDATE_LAYOUT_EDGE_VECTORS=1", path.display())),
    )
    .unwrap();
    assert!(
        committed == generated,
        "{} is stale: regenerate it with UPDATE_LAYOUT_EDGE_VECTORS=1 cargo test -p agentmux-srv-core edge_vectors",
        path.display()
    );
}
