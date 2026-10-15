// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Layout intents: the frontend's layout-tree actions, sent to srv instead of
//! the tree they produce. Each variant is one `LayoutTreeActionType` of
//! `frontend/layout/lib/types.ts`, with the same `type` tag and field names,
//! so the frontend sends what its own reducer applied.
//! docs/specs/SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30.md §4.
//!
//! The TypeScript derives are behind the `ts` feature, which only srv turns on.

use serde::{Deserialize, Serialize};

use crate::{LayoutNode, SplitPosition};

/// One resize: a node and its new flex size (0 to 100).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export_to = "../../../frontend/types/rpc/"))]
pub struct LayoutResizeOp {
    #[serde(rename = "nodeId")]
    pub node_id: String,
    pub size: f64,
}

/// A layout edit, applied by srv's reducer to its own tree for the tab.
/// Nodes the intent adds (`node`, `newNode`) carry the id the sender gave
/// them; branches srv creates on the way get ids derived from the command's
/// action id (`backend::layout::intent::derive_node_id`), so the sender can
/// predict the whole result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export_to = "../../../frontend/types/rpc/"))]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum LayoutIntent {
    /// Add `node` at the first free slot (`findNextInsertLocation`).
    Insert {
        #[cfg_attr(feature = "ts", ts(type = "any"))]
        node: LayoutNode,
        #[serde(default)]
        magnified: bool,
        #[serde(default)]
        focused: bool,
    },
    /// Add `node` after the slot an index path names.
    #[serde(rename = "insertatindex")]
    InsertAtIndex {
        #[cfg_attr(feature = "ts", ts(type = "any"))]
        node: LayoutNode,
        #[serde(rename = "indexArr")]
        #[cfg_attr(feature = "ts", ts(type = "Array<number>"))]
        index_arr: Vec<i64>,
        #[serde(default)]
        magnified: bool,
        #[serde(default)]
        focused: bool,
    },
    /// Remove a node (and everything under it).
    Delete {
        #[serde(rename = "nodeId")]
        node_id: String,
    },
    /// Re-parent a node: the commit of a pane drag.
    Move {
        #[serde(rename = "nodeId")]
        node_id: String,
        #[serde(rename = "parentId", default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        parent_id: Option<String>,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        index: i64,
        #[serde(rename = "insertAtRoot", default)]
        insert_at_root: bool,
    },
    /// Swap two nodes; each slot keeps its size.
    Swap {
        #[serde(rename = "node1Id")]
        node1_id: String,
        #[serde(rename = "node2Id")]
        node2_id: String,
    },
    /// Set node sizes: the commit of a divider drag. All or nothing.
    Resize {
        #[serde(rename = "resizeOperations")]
        resize_operations: Vec<LayoutResizeOp>,
    },
    /// Put `newNode` where `targetNodeId` is, keeping its size.
    Replace {
        #[serde(rename = "targetNodeId")]
        target_node_id: String,
        #[serde(rename = "newNode")]
        #[cfg_attr(feature = "ts", ts(type = "any"))]
        new_node: LayoutNode,
        #[serde(default)]
        focused: bool,
    },
    /// Put `newNode` beside `targetNodeId`, in a row.
    #[serde(rename = "splithorizontal")]
    SplitHorizontal {
        #[serde(rename = "targetNodeId")]
        target_node_id: String,
        #[serde(rename = "newNode")]
        #[cfg_attr(feature = "ts", ts(type = "any"))]
        new_node: LayoutNode,
        #[cfg_attr(feature = "ts", ts(type = r#""before" | "after""#))]
        position: SplitPosition,
        #[serde(rename = "sizeFraction", default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        size_fraction: Option<f64>,
        #[serde(default)]
        focused: bool,
    },
    /// Put `newNode` beside `targetNodeId`, in a column.
    #[serde(rename = "splitvertical")]
    SplitVertical {
        #[serde(rename = "targetNodeId")]
        target_node_id: String,
        #[serde(rename = "newNode")]
        #[cfg_attr(feature = "ts", ts(type = "any"))]
        new_node: LayoutNode,
        #[cfg_attr(feature = "ts", ts(type = r#""before" | "after""#))]
        position: SplitPosition,
        #[serde(rename = "sizeFraction", default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        size_fraction: Option<f64>,
        #[serde(default)]
        focused: bool,
    },
    /// Show a leaf as a header chip; its size is kept. Refused for the last
    /// pane still showing content.
    Minimize {
        #[serde(rename = "nodeId")]
        node_id: String,
    },
    /// Show a minimized leaf again.
    Restore {
        #[serde(rename = "nodeId")]
        node_id: String,
    },
    /// Empty the tree.
    Clear,
    /// No tree change: only the focused and magnified pane, which travel
    /// with every intent (a click, a magnify toggle).
    View,
}

impl LayoutIntent {
    /// The frontend's name for it, for logs and errors.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Insert { .. } => "insert",
            Self::InsertAtIndex { .. } => "insertatindex",
            Self::Delete { .. } => "delete",
            Self::Move { .. } => "move",
            Self::Swap { .. } => "swap",
            Self::Resize { .. } => "resize",
            Self::Replace { .. } => "replace",
            Self::SplitHorizontal { .. } => "splithorizontal",
            Self::SplitVertical { .. } => "splitvertical",
            Self::Minimize { .. } => "minimize",
            Self::Restore { .. } => "restore",
            Self::Clear => "clear",
            Self::View => "view",
        }
    }
}

/// The focused and magnified pane the sender holds after the intent. Focus
/// is the frontend's to pick (it keeps the focus history and the pane
/// geometry that choose a successor), so srv stores it as sent, cleared if
/// it names no node of the resulting tree.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutViewState {
    pub focused_node_id: String,
    pub magnified_node_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intents_parse_from_the_frontend_action_shape() {
        let split: LayoutIntent = serde_json::from_value(serde_json::json!({
            "type": "splithorizontal",
            "targetNodeId": "t",
            "newNode": {"id": "n", "flexDirection": "row", "size": 10, "data": {"blockId": "b"}},
            "position": "after",
            "sizeFraction": 0.5,
            "focused": true
        }))
        .unwrap();
        assert!(matches!(
            split,
            LayoutIntent::SplitHorizontal { position: SplitPosition::After, size_fraction: Some(f), focused: true, .. } if f == 0.5
        ));
        let resize: LayoutIntent = serde_json::from_value(serde_json::json!({
            "type": "resize",
            "resizeOperations": [{"nodeId": "a", "size": 12.5}]
        }))
        .unwrap();
        assert_eq!(resize.name(), "resize");
        let view: LayoutIntent = serde_json::from_value(serde_json::json!({"type": "view"})).unwrap();
        assert_eq!(view, LayoutIntent::View);
    }

    #[test]
    fn intents_round_trip() {
        let intent = LayoutIntent::Move {
            node_id: "a".into(),
            parent_id: Some("p".into()),
            index: 2,
            insert_at_root: false,
        };
        let json = serde_json::to_value(&intent).unwrap();
        assert_eq!(json["type"], "move");
        assert_eq!(json["parentId"], "p");
        assert_eq!(serde_json::from_value::<LayoutIntent>(json).unwrap(), intent);
    }
}
