// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where a torn-off pane came from, and where a redock puts it back
//! (SPEC_FLOATING_PANE_REDOCK_TO_ORIGIN_2026_11_03.md).
//!
//! At tear-off the source tab's tree is read before the pane leaves it, and a small record goes into the
//! block's meta (`floater:origin`, persisted with the block): the tab, the pane's tab-mate if one stays, and up
//! to four neighbouring panes (nearest first) with the split axis, side and size share the pane had next to
//! each. A redock by the button walks that record against the origin tab's tree as it is now and returns the
//! first place that still exists; with none left it does what it always did. The functions that read and
//! decide are pure, over a `LayoutNode`.

use agentmux_common::ipc::Command;
use agentmux_common::{FlexDirection, LayoutNode, SplitPosition};
use serde::{Deserialize, Serialize};

use super::super::AppState;
use super::layout_intents::{place_block, Placement};
use super::reducer_helpers::{dispatch_to_reducer, publish_events};
use crate::backend::layout::{find_leaf_containing_block, leaf_members};

/// The block meta key that holds an `OriginRecord`.
pub(crate) const META_KEY: &str = "floater:origin";

/// How many neighbours are remembered.
const MAX_ANCHORS: usize = 4;

/// The share of the two panes' size the torn-off pane may take back, so a stale or odd size never gives a
/// sliver or hides the neighbour.
const FRACTION_RANGE: (f64, f64) = (0.15, 0.85);

/// A neighbouring pane the torn-off pane can be put back beside.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Anchor {
    /// A block of the neighbouring pane.
    pub block_id: String,
    /// Side by side (a row) rather than stacked (a column).
    pub horizontal: bool,
    /// The torn-off pane sat after the neighbour (to its right, or below it).
    pub after: bool,
    /// The share of the two panes' combined size the torn-off pane had.
    pub fraction: f64,
}

/// Where a floating pane was torn from.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub(crate) struct OriginRecord {
    pub tab_id: String,
    pub workspace_id: String,
    /// A block that stayed in the pane when only a pane tab left: the tab goes back into it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_mate: Option<String>,
    /// Neighbouring panes, nearest first.
    #[serde(default)]
    pub anchors: Vec<Anchor>,
}

/// Where a redock puts the pane.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Resolution {
    /// As a tab of the pane that block is in.
    StackMate(String),
    /// Beside this neighbour, on its recorded side, with its recorded share.
    Split(Anchor),
    /// Nothing recognisable is left: where the caller was going to put it.
    Fallback,
}

impl Resolution {
    /// What the redock's answer calls it.
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Resolution::StackMate(_) => "stack",
            Resolution::Split(_) => "neighbour",
            Resolution::Fallback => "fallback",
        }
    }
}

/// The nodes from the root down to the pane holding `block_id`, as (parent, index of the child on the path),
/// and that pane.
fn path_to<'a>(
    node: &'a LayoutNode,
    block_id: &str,
    path: &mut Vec<(&'a LayoutNode, usize)>,
) -> Option<&'a LayoutNode> {
    if node
        .data
        .as_ref()
        .is_some_and(|d| d.block_id == block_id || d.block_stack.iter().any(|b| b == block_id))
    {
        return Some(node);
    }
    for (i, child) in node.children.iter().enumerate() {
        path.push((node, i));
        if let Some(found) = path_to(child, block_id, path) {
            return Some(found);
        }
        path.pop();
    }
    None
}

/// The visible block of the first pane under `node` that is not a minimized chip.
fn first_shown_block(node: &LayoutNode) -> Option<String> {
    if let Some(data) = node.data.as_ref() {
        return (!node.minimized).then(|| data.block_id.clone());
    }
    node.children.iter().find_map(first_shown_block)
}

fn share(own: f32, other: f32) -> f64 {
    let total = f64::from(own) + f64::from(other);
    if total <= 0.0 {
        return 0.5;
    }
    (f64::from(own) / total).clamp(FRACTION_RANGE.0, FRACTION_RANGE.1)
}

/// The record for a pane about to leave `tab_id`: `torn` are the blocks that leave, the first being the one the
/// record is stored on. `None` when there is nothing to come back to (a pane alone in its tab, with no tab-mate).
pub(crate) fn record_from_tree(
    root: &LayoutNode,
    torn: &[String],
    tab_id: &str,
    workspace_id: &str,
) -> Option<OriginRecord> {
    let first = torn.first()?;
    let mut path = Vec::new();
    let pane = path_to(root, first, &mut path)?;
    let stack_mate = pane
        .data
        .as_ref()
        .and_then(|d| leaf_members(d).into_iter().find(|m| !torn.contains(m)));
    let mut anchors = Vec::new();
    // Level by level from the pane outward: its previous and next sibling, then its parent's.
    'levels: for (parent, idx) in path.iter().rev() {
        let horizontal = parent.flex_direction == FlexDirection::Row;
        let own = parent.children[*idx].size;
        let before = idx.checked_sub(1).map(|i| (i, true));
        let after = (idx + 1 < parent.children.len()).then_some((idx + 1, false));
        for (i, sits_after) in before.into_iter().chain(after) {
            let neighbour = &parent.children[i];
            let Some(block_id) = first_shown_block(neighbour) else {
                continue;
            };
            anchors.push(Anchor {
                block_id,
                horizontal,
                after: sits_after,
                fraction: share(own, neighbour.size),
            });
            if anchors.len() == MAX_ANCHORS {
                break 'levels;
            }
        }
    }
    if anchors.is_empty() && stack_mate.is_none() {
        return None;
    }
    Some(OriginRecord {
        tab_id: tab_id.to_string(),
        workspace_id: workspace_id.to_string(),
        stack_mate,
        anchors,
    })
}

/// Where `rec` puts the pane back, given the origin tab's tree as it is now. A tab-mate that is still in a pane
/// wins (the pane never left); else the first neighbour that is still a shown pane, which is the nearest one
/// since the list is ordered by nearness; else `Fallback`. "Beyond recognition" is exactly that last case: the
/// tab is gone (the caller's check), or no recorded neighbour is a pane of it that is not a chip.
pub(crate) fn resolve(root: Option<&LayoutNode>, rec: &OriginRecord) -> Resolution {
    let Some(root) = root else {
        return Resolution::Fallback;
    };
    if let Some(mate) = rec
        .stack_mate
        .as_ref()
        .filter(|m| find_leaf_containing_block(root, m).is_some())
    {
        return Resolution::StackMate(mate.clone());
    }
    rec.anchors
        .iter()
        .find(|a| find_leaf_containing_block(root, &a.block_id).is_some_and(|pane| !pane.minimized))
        .map_or(Resolution::Fallback, |a| Resolution::Split(a.clone()))
}

async fn patch_block_meta(state: &AppState, block_id: &str, patch: serde_json::Value) {
    let events = dispatch_to_reducer(
        state,
        Command::UpdateBlockMeta {
            block_id: block_id.to_string(),
            meta_patch: patch,
        },
    )
    .await;
    for ev in &events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore) {
            tracing::warn!(block = %block_id, "floater origin: meta write failed: {e}");
        }
    }
    publish_events(state, &events);
}

/// The record for the blocks about to be torn out of `tab_id`, read from srv's tree. Call before they leave it.
pub(crate) async fn record_for(
    state: &AppState,
    tab_id: &str,
    workspace_id: &str,
    torn: &[String],
) -> Option<OriginRecord> {
    let s = state.srv_state.lock().await;
    let root = s.tabs.get(tab_id)?.rootnode.as_ref()?;
    record_from_tree(root, torn, tab_id, workspace_id)
}

/// Keep `rec` on each of the blocks that left (whichever one a redock asks about finds it), so it survives a restart.
pub(crate) async fn store(state: &AppState, blocks: &[String], rec: &OriginRecord) {
    let Ok(value) = serde_json::to_value(rec) else {
        return;
    };
    for block_id in blocks {
        patch_block_meta(state, block_id, serde_json::json!({ META_KEY: value })).await;
    }
}

/// Keep `rec` on the blocks that just left, or, when there is none to keep, drop any older one they carry.
pub(crate) async fn remember(state: &AppState, blocks: &[String], rec: Option<&OriginRecord>) {
    match rec {
        Some(rec) => store(state, blocks, rec).await,
        None => clear(state, blocks).await,
    }
}

/// Drop the record from each of these blocks: the pane is docked again, by whatever path, or a new tear-off has
/// nothing to remember and must not leave an older record to be found later.
pub(crate) async fn clear(state: &AppState, blocks: &[String]) {
    for block_id in blocks {
        let has_record = state
            .mstore
            .must_get::<crate::backend::obj::Block>(block_id)
            .is_ok_and(|b| b.meta.contains_key(META_KEY));
        if has_record {
            patch_block_meta(state, block_id, serde_json::json!({ META_KEY: null })).await;
        }
    }
}

/// The record on `block_id` and where it resolves to now. `None`: no record, or its tab is gone or its workspace
/// is open in no window (the pane would go where nobody sees it), which a caller treats as `Fallback`.
pub(crate) async fn resolve_for_block(
    state: &AppState,
    block_id: &str,
) -> Option<(OriginRecord, Resolution)> {
    let block = state
        .mstore
        .must_get::<crate::backend::obj::Block>(block_id)
        .ok()?;
    let rec: OriginRecord = serde_json::from_value(block.meta.get(META_KEY)?.clone()).ok()?;
    let s = state.srv_state.lock().await;
    let tab = s.tabs.get(&rec.tab_id)?;
    if tab.workspace_id != rec.workspace_id
        || !s
            .windows
            .values()
            .any(|w| w.workspace_id == rec.workspace_id)
    {
        return None;
    }
    let resolution = resolve(tab.rootnode.as_ref(), &rec);
    Some((rec, resolution))
}

/// Where a redock by the button goes: the origin's tab and workspace and how the pane is placed there, or, when
/// nothing is recognisable, the tab the caller named and `Fallback`.
pub(crate) async fn target_for(
    state: &AppState,
    block_id: &str,
    fallback_tab_id: String,
    fallback_ws_id: String,
) -> (String, String, Resolution) {
    match resolve_for_block(state, block_id).await {
        Some((rec, resolution)) if resolution != Resolution::Fallback => {
            (rec.tab_id, rec.workspace_id, resolution)
        }
        _ => (fallback_tab_id, fallback_ws_id, Resolution::Fallback),
    }
}

/// Put `block_id` beside the neighbour `anchor` names, on its recorded side and with its recorded share. Falls
/// back to the layout's next free place when that pane is gone by now (`place_block`).
pub(crate) async fn place_at_anchor(
    state: &AppState,
    tab_id: &str,
    block_id: &str,
    anchor: &Anchor,
) -> Result<(), String> {
    let placement = Placement::Split {
        target_block_id: anchor.block_id.clone(),
        horizontal: anchor.horizontal,
        position: if anchor.after {
            SplitPosition::After
        } else {
            SplitPosition::Before
        },
        size_fraction: Some(anchor.fraction),
        focused: true,
    };
    place_block(state, tab_id, block_id, placement).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentmux_common::LayoutNodeData;

    fn leaf(id: &str, block: &str, size: f32) -> LayoutNode {
        LayoutNode {
            id: id.into(),
            size,
            data: Some(LayoutNodeData {
                block_id: block.into(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn group(id: &str, dir: FlexDirection, children: Vec<LayoutNode>) -> LayoutNode {
        LayoutNode {
            id: id.into(),
            flex_direction: dir,
            size: 10.0,
            children,
            ..Default::default()
        }
    }

    /// `[a | b | c]` side by side, sizes 10, 20, 10.
    fn row() -> LayoutNode {
        group(
            "root",
            FlexDirection::Row,
            vec![
                leaf("na", "a", 10.0),
                leaf("nb", "b", 20.0),
                leaf("nc", "c", 10.0),
            ],
        )
    }

    fn rec(root: &LayoutNode, torn: &[&str]) -> OriginRecord {
        let torn: Vec<String> = torn.iter().map(|s| s.to_string()).collect();
        record_from_tree(root, &torn, "tab", "ws").expect("a record")
    }

    fn without(root: &LayoutNode, block: &str) -> LayoutNode {
        let mut out = root.clone();
        out.children
            .retain(|c| c.data.as_ref().is_none_or(|d| d.block_id != block));
        out
    }

    #[test]
    fn the_record_lists_the_neighbours_nearest_first_with_their_side_and_share() {
        let r = rec(&row(), &["b"]);
        assert_eq!(
            (
                r.tab_id.as_str(),
                r.workspace_id.as_str(),
                r.stack_mate.clone()
            ),
            ("tab", "ws", None)
        );
        assert_eq!(
            r.anchors,
            vec![
                Anchor {
                    block_id: "a".into(),
                    horizontal: true,
                    after: true,
                    fraction: 20.0 / 30.0
                },
                Anchor {
                    block_id: "c".into(),
                    horizontal: true,
                    after: false,
                    fraction: 20.0 / 30.0
                },
            ]
        );
    }

    #[test]
    fn a_pane_alone_in_its_tab_has_no_record_and_a_stack_mate_alone_is_enough() {
        assert_eq!(
            record_from_tree(&leaf("only", "x", 10.0), &["x".to_string()], "tab", "ws"),
            None
        );
        let mut pane = leaf("p", "a", 10.0);
        pane.data.as_mut().unwrap().block_stack = vec!["a".into(), "b".into()];
        let r =
            record_from_tree(&pane, &["b".to_string()], "tab", "ws").expect("stack-mate record");
        assert_eq!(r.stack_mate.as_deref(), Some("a"));
        assert!(r.anchors.is_empty());
    }

    #[test]
    fn the_record_goes_up_the_tree_for_more_neighbours_and_stops_at_four() {
        // [a | [b / c / d] | e]: tearing off c has b and d beside it, then a and e around its column.
        let col = group(
            "col",
            FlexDirection::Column,
            vec![
                leaf("nb", "b", 10.0),
                leaf("nc", "c", 10.0),
                leaf("nd", "d", 10.0),
            ],
        );
        let root = group(
            "root",
            FlexDirection::Row,
            vec![leaf("na", "a", 10.0), col, leaf("ne", "e", 10.0)],
        );
        let r = rec(&root, &["c"]);
        let ids: Vec<&str> = r.anchors.iter().map(|a| a.block_id.as_str()).collect();
        assert_eq!(ids, ["b", "d", "a", "e"]);
        assert!(
            !r.anchors[0].horizontal && r.anchors[2].horizontal,
            "the axis is each level's own"
        );
        assert_eq!(r.anchors.len(), MAX_ANCHORS);
    }

    #[test]
    fn a_neighbour_that_is_a_chip_is_not_remembered() {
        let mut root = row();
        root.children[0].minimized = true;
        let r = rec(&root, &["b"]);
        assert_eq!(
            r.anchors
                .iter()
                .map(|a| a.block_id.as_str())
                .collect::<Vec<_>>(),
            ["c"]
        );
    }

    #[test]
    fn a_neighbour_that_is_still_there_is_used_whatever_happened_to_sizes() {
        let r = rec(&row(), &["b"]);
        let mut resized = without(&row(), "b");
        resized.children[0].size = 3.0;
        resized.children[1].size = 40.0;
        assert_eq!(
            resolve(Some(&resized), &r),
            Resolution::Split(r.anchors[0].clone())
        );
    }

    #[test]
    fn a_stack_mate_that_is_still_in_a_pane_wins_over_a_neighbour() {
        let mut pane = leaf("nb", "b", 20.0);
        pane.data.as_mut().unwrap().block_stack = vec!["b".into(), "t".into()];
        let root = group(
            "root",
            FlexDirection::Row,
            vec![leaf("na", "a", 10.0), pane, leaf("nc", "c", 10.0)],
        );
        let r = rec(&root, &["t"]);
        assert_eq!(r.stack_mate.as_deref(), Some("b"));
        assert_eq!(resolve(Some(&root), &r), Resolution::StackMate("b".into()));
    }

    #[test]
    fn b_the_nearest_surviving_neighbour_is_used_when_the_first_is_gone() {
        let r = rec(&row(), &["b"]);
        let now = without(&without(&row(), "b"), "a");
        assert_eq!(
            resolve(Some(&now), &r),
            Resolution::Split(r.anchors[1].clone())
        );
    }

    #[test]
    fn c_nothing_recognisable_falls_back() {
        let r = rec(&row(), &["b"]);
        assert_eq!(
            resolve(Some(&leaf("x", "other", 10.0)), &r),
            Resolution::Fallback,
            "every neighbour closed"
        );
        assert_eq!(resolve(None, &r), Resolution::Fallback, "no tree");
        let mut chips = without(&row(), "b");
        chips.children.iter_mut().for_each(|c| c.minimized = true);
        assert_eq!(
            resolve(Some(&chips), &r),
            Resolution::Fallback,
            "the neighbours are chips now"
        );
    }

    #[test]
    fn the_record_round_trips_through_block_meta() {
        let r = rec(&row(), &["b"]);
        let back: OriginRecord = serde_json::from_value(serde_json::to_value(&r).unwrap()).unwrap();
        assert_eq!(back, r);
    }
}
