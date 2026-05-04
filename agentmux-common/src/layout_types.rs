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
    #[serde(rename = "blockId")]
    pub block_id: String,
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
