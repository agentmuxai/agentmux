// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Shared layout tree types used in both the srv wire protocol (ipc.rs
//! Command/Event variants) and the srv object store (obj.rs LayoutState).
//!
//! Living in agentmux-common so agentmux-srv, agentmux-cef, and agentmux-
//! launcher can all reference the same type definitions without circular
//! dependencies or duplication.
//!
//! Part of srv Phase E.4.B (Phase 3 — wire types).
//! See docs/specs/srv-phase-e4b-formal-spec-2026-05-03.md §4.1, §5, §6.

use serde::{Deserialize, Serialize};

// ── Core tree types ─────────────────────────────────────────────────────────

/// Direction children flow within a layout node (row = horizontal split,
/// column = vertical split). Defaults to `Row` when absent in JSON for
/// tolerance of older blobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FlexDirection {
    #[default]
    Row,
    Column,
}

/// Leaf-only payload — references the block this layout leaf renders.
/// Group nodes (those with `children`) carry no `data`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LayoutNodeData {
    /// The currently-rendered block for this leaf. Always kept in sync with
    /// `active_block_id` when `block_stack` is non-empty — this field is
    /// what every existing (pre-tabs) reader/pruner cares about, so a leaf
    /// with no stack behaves identically to before this field existed.
    #[serde(rename = "blockId")]
    pub block_id: String,
    /// In-pane tabs (SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md §4.3):
    /// every blockId hosted by this leaf, ordered; empty/absent means "no
    /// stack, just `block_id`" — 100% back-compat with every layout written
    /// before this field existed. When non-empty, `block_id` MUST equal
    /// `active_block_id` and MUST be a member of this list.
    #[serde(rename = "blockStack", default, skip_serializing_if = "Vec::is_empty")]
    pub block_stack: Vec<String>,
    /// The active member of `block_stack`. Empty/unused when `block_stack`
    /// is empty. Kept as an explicit field (rather than only ever reading
    /// `block_id`) so intent is unambiguous in the wire format.
    #[serde(rename = "activeBlockId", default, skip_serializing_if = "String::is_empty")]
    pub active_block_id: String,
    /// Catch-all for unknown fields — preserves forward-compat when the
    /// frontend writes additional fields we don't yet model. Uses
    /// `serde_json::Map` (insertion-ordered) for deterministic round-trips.
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// One node in the layout tree. Stable UUID-keyed; `size` is a relative
/// flex unit; `children` form the recursive structure (empty for leaves).
///
/// JSON shape (matches frontend `LayoutNode` in `frontend/layout/lib/types.ts`):
/// ```json
/// { "id": "...", "flexDirection": "row", "size": 1, "children": [...], "data": { "blockId": "..." } }
/// ```
///
/// Note: `Default::size` is 0.0 (Rust f32 default), while the serde
/// deserialization default is 1.0. The `Default` derive is only for the
/// `..Default::default()` spread trick at construction sites — size is
/// always set explicitly in practice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LayoutNode {
    pub id: String,
    #[serde(rename = "flexDirection", default)]
    pub flex_direction: FlexDirection,
    /// Flex size — relative units within the parent's children array.
    /// Custom serializer emits whole-number sizes as integers (`10` not
    /// `10.0`) to preserve byte-equal compat with prior JSON output.
    #[serde(
        default = "default_layout_size",
        serialize_with = "serialize_size_smallest",
    )]
    pub size: f32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<LayoutNode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<LayoutNodeData>,
    /// Catch-all for unknown top-level fields. Uses `serde_json::Map`
    /// (insertion-ordered) for deterministic round-trips.
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

fn default_layout_size() -> f32 {
    1.0
}

fn serialize_size_smallest<S>(value: &f32, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    if value.fract() == 0.0
        && value.is_finite()
        && (i64::MIN as f32..=i64::MAX as f32).contains(value)
    {
        serializer.serialize_i64(*value as i64)
    } else {
        serializer.serialize_f32(*value)
    }
}

// ── Command helper types ────────────────────────────────────────────────────

/// SPEC_864 Phase 2 — the frontend-owned `LayoutState` slices carried by a
/// full-row-replace push (the `UpdateObject`→`LayoutSetTree` route). The
/// reducer applies `focused_node_id`/`magnified_node_id` to its `TabRecord`;
/// `leaforder` and `pending_backend_actions` are opaque pass-through JSON the
/// persist subscriber writes verbatim (no algebra re-run in the subscriber —
/// that would be a new divergence surface).
///
/// Semantics are REPLACE, mirroring the legacy `update_raw` whole-row write
/// this route retires: an absent/`null` `leaforder` or
/// `pending_backend_actions` CLEARS the persisted column (pushing the row
/// without processed actions is how the frontend acks its backend-action
/// queue); an empty `focused_node_id`/`magnified_node_id` clears focus/
/// magnify. A `LayoutSetTree` with `slices: None` (granular/tree-only
/// callers) leaves all four columns untouched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LayoutClientSlices {
    /// Frontend-recomputed geometry cache (`getLeafOrder`). Not read on
    /// reproject; persisted only for legacy readers. `None` = clear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leaforder: Option<serde_json::Value>,
    /// Focused layout-node id; empty = clear.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub focused_node_id: String,
    /// Magnified layout-node id; empty = clear.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub magnified_node_id: String,
    /// The backend→frontend action queue as the client sees it after
    /// processing. `None` = clear (the ack path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_backend_actions: Option<serde_json::Value>,
}

/// A single resize operation: target node + new flex size (0–100 relative units).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResizeOp {
    pub node_id: String,
    /// New flex size. Must be in 0.0..=100.0; reducer validates.
    pub size: f32,
}

/// Position for split commands: where to insert the new node relative to the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitPosition {
    Before,
    After,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_node_data_old_json_without_stack_fields_deserializes() {
        // A layout leaf written before blockStack/activeBlockId existed —
        // must still deserialize cleanly, with the new fields defaulting to
        // "no stack".
        let json = r#"{"blockId":"blk-1"}"#;
        let data: LayoutNodeData = serde_json::from_str(json).unwrap();
        assert_eq!(data.block_id, "blk-1");
        assert!(data.block_stack.is_empty());
        assert!(data.active_block_id.is_empty());
    }

    #[test]
    fn layout_node_data_old_json_without_stack_fields_reserializes_without_them() {
        // Round-tripping a non-stacked leaf must not introduce blockStack/
        // activeBlockId into the JSON — byte-equal-compat with prior output
        // for the overwhelming majority (every non-tabbed pane) case.
        let json = r#"{"blockId":"blk-1"}"#;
        let data: LayoutNodeData = serde_json::from_str(json).unwrap();
        let out = serde_json::to_string(&data).unwrap();
        assert_eq!(out, json);
    }

    #[test]
    fn layout_node_data_with_stack_round_trips() {
        let data = LayoutNodeData {
            block_id: "blk-2".to_string(),
            block_stack: vec!["blk-1".to_string(), "blk-2".to_string()],
            active_block_id: "blk-2".to_string(),
            extra: serde_json::Map::new(),
        };
        let json = serde_json::to_string(&data).unwrap();
        let parsed: LayoutNodeData = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, data);
        assert!(json.contains("\"blockStack\":[\"blk-1\",\"blk-2\"]"));
        assert!(json.contains("\"activeBlockId\":\"blk-2\""));
    }

    #[test]
    fn layout_node_data_preserves_unknown_extra_fields() {
        let json = r#"{"blockId":"blk-1","someFutureField":42}"#;
        let data: LayoutNodeData = serde_json::from_str(json).unwrap();
        assert_eq!(data.extra.get("someFutureField").and_then(|v| v.as_i64()), Some(42));
        let out = serde_json::to_string(&data).unwrap();
        assert!(out.contains("\"someFutureField\":42"));
    }
}
