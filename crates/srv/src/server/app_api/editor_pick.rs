// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which Editor pane an agent's open goes into, when a tab has several.
//!
//! The order is fixed, so the same layout always gives the same answer:
//! 1. one the tab shows now (the front tab of its pane, not minimized, not behind a magnified pane);
//! 2. the nearest to the pane that asked, by distance in the layout tree, with one after it (below or
//!    to its right) before one before it;
//! 3. the focused pane;
//! 4. one that needs no switch of its pane's tab;
//! 5. the first in layout order.

use crate::backend::obj::LayoutState;
use agentmux_common::LayoutNode;

/// The Editor chosen, and a sentence for the log saying why.
pub(super) struct Pick {
    pub block_id: String,
    pub why: &'static str,
}

/// The child indexes from `node` down to the pane holding `block_id` (in its stack or in front).
fn path_to(node: &LayoutNode, block_id: &str) -> Option<Vec<usize>> {
    if node.children.is_empty() {
        let holds = node
            .data
            .as_ref()
            .is_some_and(|d| d.block_id == block_id || d.block_stack.iter().any(|b| b == block_id));
        return holds.then(Vec::new);
    }
    node.children.iter().enumerate().find_map(|(i, child)| {
        let mut path = path_to(child, block_id)?;
        path.insert(0, i);
        Some(path)
    })
}

/// Steps between two panes through their nearest common branch.
fn tree_distance(a: &[usize], b: &[usize]) -> usize {
    let shared = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    (a.len() - shared) + (b.len() - shared)
}

/// Whether showing `block_id` takes switching its pane to it: it is a background tab of a pane stack.
pub(super) fn needs_activation(layout: &LayoutState, block_id: &str) -> bool {
    let shown = super::shown_block_ids(layout);
    !shown.iter().any(|s| s == block_id)
        && layout
            .rootnode
            .as_ref()
            .and_then(|root| crate::backend::layout::find_leaf_containing_block(root, block_id))
            .and_then(|leaf| leaf.data.as_ref())
            .is_some_and(|d| d.block_stack.len() > 1 && d.block_id != block_id)
}

/// The Editor of `editors` (in layout order) an open from the pane `caller` should go into.
pub(super) fn choose(layout: &LayoutState, caller: Option<&str>, editors: &[String]) -> Option<Pick> {
    let root = layout.rootnode.as_ref()?;
    let shown = super::shown_block_ids(layout);
    let caller_path = caller.and_then(|c| path_to(root, c));
    let mut ranked: Vec<_> = editors
        .iter()
        .enumerate()
        .map(|(order, id)| {
            let path = path_to(root, id);
            let on_screen = shown.contains(id);
            let (distance, after) = match (&caller_path, &path) {
                (Some(from), Some(to)) => (tree_distance(from, to), to > from),
                _ => (usize::MAX, false),
            };
            let focused = !layout.focusednodeid.is_empty()
                && crate::backend::layout::find_leaf_containing_block(root, id).is_some_and(|l| l.id == layout.focusednodeid);
            let key = (!on_screen, distance, !after, !focused, needs_activation(layout, id), order);
            (key, id, on_screen, editors.len())
        })
        .collect();
    ranked.sort_by(|a, b| a.0.cmp(&b.0));
    let (_, id, on_screen, count) = ranked.into_iter().next()?;
    let on_screen_count = editors.iter().filter(|e| shown.contains(e)).count();
    let why = match (on_screen, on_screen_count, count) {
        (_, _, 1) => "the tab's only Editor pane",
        (true, 1, _) => "the only Editor pane the tab shows",
        (true, _, _) => "the Editor pane the tab shows nearest to the pane that asked",
        (false, _, _) => "no Editor pane is on screen: the nearest one that is hidden",
    };
    Some(Pick { block_id: id.clone(), why })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(id: &str, block: &str) -> LayoutNode {
        LayoutNode {
            id: id.into(),
            data: Some(agentmux_common::LayoutNodeData { block_id: block.into(), ..Default::default() }),
            ..Default::default()
        }
    }
    fn stack(id: &str, front: &str, members: &[&str]) -> LayoutNode {
        let mut node = leaf(id, front);
        let data = node.data.as_mut().unwrap();
        data.block_stack = members.iter().map(|m| m.to_string()).collect();
        data.active_block_id = front.into();
        node
    }
    fn group(id: &str, children: Vec<LayoutNode>) -> LayoutNode {
        LayoutNode { id: id.into(), children, ..Default::default() }
    }
    fn layout(root: LayoutNode) -> LayoutState {
        LayoutState { rootnode: Some(root), ..Default::default() }
    }
    fn names(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }
    fn pick(layout: &LayoutState, caller: &str, editors: &[&str]) -> String {
        choose(layout, Some(caller), &names(editors)).unwrap().block_id
    }

    /// The reported layout: the agent's pane bottom left with an empty Editor beside it, and an Editor
    /// that comes first in layout order inside a pane group at the top right.
    #[test]
    fn the_editor_next_to_the_caller_wins_over_one_first_in_layout_order() {
        let tree = group(
            "root",
            vec![
                group("top", vec![leaf("t1", "kcont"), stack("grp", "ed-group", &["help", "ed-group"])]),
                group("bottom", vec![leaf("agent", "agent"), leaf("free", "ed-free")]),
            ],
        );
        let l = layout(tree);
        assert_eq!(pick(&l, "agent", &["ed-group", "ed-free"]), "ed-free");
    }

    #[test]
    fn an_editor_on_screen_wins_over_a_nearer_one_that_is_hidden() {
        let mut hidden = leaf("hid", "ed-hidden");
        hidden.minimized = true;
        let l = layout(group("root", vec![leaf("agent", "agent"), hidden, leaf("far", "ed-far")]));
        assert_eq!(pick(&l, "agent", &["ed-hidden", "ed-far"]), "ed-far");
        // With none on screen, the nearest hidden one is chosen.
        let mut a = leaf("a", "ed-a");
        a.minimized = true;
        let mut b = leaf("b", "ed-b");
        b.minimized = true;
        let l = layout(group("root", vec![a, leaf("agent", "agent"), b]));
        assert_eq!(pick(&l, "agent", &["ed-a", "ed-b"]), "ed-b", "one after the caller is nearer first");
    }

    #[test]
    fn at_equal_distance_the_one_below_or_to_the_right_comes_first() {
        let l = layout(group("root", vec![leaf("before", "ed-before"), leaf("agent", "agent"), leaf("after", "ed-after")]));
        assert_eq!(pick(&l, "agent", &["ed-before", "ed-after"]), "ed-after");
    }

    #[test]
    fn then_the_focused_pane_then_layout_order() {
        // Both before the caller and equally far from it.
        let tree = group("root", vec![leaf("e1", "ed-1"), leaf("e2", "ed-2"), group("g", vec![leaf("agent", "agent"), leaf("x", "x")])]);
        let mut l = layout(tree);
        assert_eq!(pick(&l, "agent", &["ed-1", "ed-2"]), "ed-1", "layout order when nothing else decides");
        l.focusednodeid = "e2".into();
        assert_eq!(pick(&l, "agent", &["ed-1", "ed-2"]), "ed-2", "the focused pane");
    }

    #[test]
    fn a_background_tab_is_the_last_resort_and_says_it_needs_switching() {
        let l = layout(group("root", vec![leaf("agent", "agent"), stack("s", "term", &["term", "ed-back"])]));
        assert!(needs_activation(&l, "ed-back"));
        assert!(!needs_activation(&l, "term"));
        assert_eq!(pick(&l, "agent", &["ed-back"]), "ed-back");
    }

    #[test]
    fn the_reason_is_named() {
        let l = layout(group("root", vec![leaf("agent", "agent"), leaf("e", "ed")]));
        assert_eq!(choose(&l, Some("agent"), &names(&["ed"])).unwrap().why, "the tab's only Editor pane");
        let l = layout(group("root", vec![leaf("agent", "agent"), leaf("e1", "ed-1"), leaf("e2", "ed-2")]));
        assert_eq!(
            choose(&l, Some("agent"), &names(&["ed-1", "ed-2"])).unwrap().why,
            "the Editor pane the tab shows nearest to the pane that asked"
        );
        assert!(choose(&l, Some("agent"), &[]).is_none());
    }
}
