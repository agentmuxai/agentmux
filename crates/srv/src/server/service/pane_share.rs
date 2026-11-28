// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What fraction of its tab's area each docked pane has, from the layout tree srv holds
//! (docs/specs/SPEC_AGENT_PANE_PLACEMENT_FLOATING_2026_11_22.md §2.3): `Pane layout`'s `share`.
//!
//! A node's `size` is a flex unit among its siblings, so a leaf's share is the product, down the path from the root,
//! of its size over the sum of its siblings' sizes. Panes stacked as tabs of one leaf all have that leaf's share.
//! It is the layout as stored: what the window draws can differ by gaps and minimum sizes, which the measured pixel
//! `size` (not part of this module) shows.

use std::collections::HashMap;

use agentmux_common::LayoutNode;

/// The share of the tab (0 to 1) of every block the layout `root` holds.
pub(crate) fn shares(root: &LayoutNode) -> HashMap<String, f64> {
    fn walk(node: &LayoutNode, share: f64, out: &mut HashMap<String, f64>) {
        if node.children.is_empty() {
            if let Some(data) = &node.data {
                let stack = if data.block_stack.is_empty() { std::slice::from_ref(&data.block_id) } else { data.block_stack.as_slice() };
                for block in stack.iter().filter(|b| !b.is_empty()) {
                    out.insert(block.clone(), share);
                }
            }
            return;
        }
        let total: f64 = node.children.iter().map(|c| f64::from(c.size).max(0.0)).sum();
        let each = 1.0 / node.children.len() as f64;
        for child in &node.children {
            let of_parent = if total > 0.0 { f64::from(child.size).max(0.0) / total } else { each };
            walk(child, share * of_parent, out);
        }
    }
    let mut out = HashMap::new();
    walk(root, 1.0, &mut out);
    out
}

/// A share as `Pane layout` shows it: to three places.
pub(crate) fn rounded(share: f64) -> f64 {
    (share * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentmux_common::{FlexDirection, LayoutNodeData};
    use serde_json::json;

    fn leaf(block: &str, size: f32) -> LayoutNode {
        LayoutNode { id: format!("n-{block}"), size, data: Some(LayoutNodeData { block_id: block.into(), ..Default::default() }), ..Default::default() }
    }

    fn group(direction: FlexDirection, size: f32, children: Vec<LayoutNode>) -> LayoutNode {
        LayoutNode { id: "g".into(), flex_direction: direction, size, children, ..Default::default() }
    }

    #[test]
    fn a_lone_pane_has_the_whole_tab() {
        assert_eq!(shares(&leaf("a", 1.0))["a"], 1.0);
    }

    #[test]
    fn siblings_share_by_their_sizes_and_nested_groups_multiply() {
        // a takes 2 of 3 of the width; b and c split the rest of it by height, 1 to 3.
        let root = group(FlexDirection::Row, 1.0, vec![leaf("a", 2.0), group(FlexDirection::Column, 1.0, vec![leaf("b", 1.0), leaf("c", 3.0)])]);
        let s = shares(&root);
        assert!((s["a"] - 2.0 / 3.0).abs() < 1e-9, "{s:?}");
        assert!((s["b"] - 1.0 / 12.0).abs() < 1e-9, "{s:?}");
        assert!((s["c"] - 3.0 / 12.0).abs() < 1e-9, "{s:?}");
        assert!((s.values().sum::<f64>() - 1.0).abs() < 1e-9, "the panes fill the tab: {s:?}");
    }

    #[test]
    fn panes_stacked_as_tabs_of_one_leaf_have_that_leafs_share() {
        let mut stacked = leaf("b2", 1.0);
        stacked.data = Some(LayoutNodeData { block_id: "b2".into(), block_stack: vec!["b1".into(), "b2".into()], active_block_id: "b2".into(), ..Default::default() });
        let s = shares(&group(FlexDirection::Row, 1.0, vec![leaf("a", 1.0), stacked]));
        assert_eq!((s["a"], s["b1"], s["b2"]), (0.5, 0.5, 0.5));
    }

    #[test]
    fn sizes_that_are_zero_split_the_space_equally_rather_than_dividing_by_zero() {
        let s = shares(&group(FlexDirection::Row, 1.0, vec![leaf("a", 0.0), leaf("b", 0.0)]));
        assert_eq!((s["a"], s["b"]), (0.5, 0.5));
    }

    #[tokio::test]
    async fn the_layout_report_gives_each_docked_pane_its_share_and_a_pane_in_no_leaf_none() {
        use crate::backend::obj::{Block, LayoutState, Tab, Window, Workspace};
        let store = crate::server::tests::test_state().mstore;
        let id = |name: &str| format!("{name}-{}", uuid::Uuid::new_v4());
        let (win, ws, tab, layout) = (id("win"), id("ws"), id("tab"), id("layout"));
        let blocks = [id("a"), id("b"), id("loose")];
        for b in &blocks {
            store.insert(&mut Block { oid: b.clone(), ..Default::default() }).unwrap();
        }
        let root = group(FlexDirection::Row, 1.0, vec![leaf(&blocks[0], 1.0), leaf(&blocks[1], 3.0)]);
        store.insert(&mut LayoutState { oid: layout.clone(), rootnode: Some(root), ..Default::default() }).unwrap();
        store.insert(&mut Tab { oid: tab.clone(), layoutstate: layout, blockids: blocks.to_vec(), ..Default::default() }).unwrap();
        store.insert(&mut Workspace { oid: ws.clone(), tabids: vec![tab.clone()], activetabid: tab, ..Default::default() }).unwrap();
        store.insert(&mut Window { oid: win, workspaceid: ws, ..Default::default() }).unwrap();

        let report = crate::server::service::agent_layout(&store);

        let ours = report["windows"].as_array().unwrap().iter().flat_map(|w| w["tabs"].as_array().unwrap().iter()).flat_map(|t| t["panes"].as_array().unwrap().iter()).filter(|p| blocks.iter().any(|b| p["block_id"] == *b)).collect::<Vec<_>>();
        assert_eq!(ours.len(), 3);
        let share = |b: &String| ours.iter().find(|p| p["block_id"] == *b).unwrap().get("share").cloned();
        assert_eq!((share(&blocks[0]), share(&blocks[1])), (Some(json!(0.25)), Some(json!(0.75))));
        assert_eq!(share(&blocks[2]), None, "a block made with no placement is in the tab but in no pane");
    }

    #[test]
    fn a_share_is_shown_to_three_places() {
        assert_eq!(rounded(1.0 / 3.0), 0.333);
        assert_eq!(rounded(0.5), 0.5);
    }
}
