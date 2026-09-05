// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! StoreObj types: Rust equivalents of Go structs from pkg/obj/wtype.go.
//! All `#[serde(rename = "...")]` tags match Go JSON tags for wire compatibility.


use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::oref::ORef;

// ---- Custom serde for MetaMapType ----
// Go serializes nil maps as `null` and initialized maps as `{}`.
// Rust's HashMap is always initialized (empty = `{}`), so we need
// to serialize empty HashMap as `null` to match Go's wire format.
// We also need to accept `null` on deserialization (from DB or network).
fn serialize_meta_as_null_if_empty<S>(meta: &MetaMapType, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    if meta.is_empty() {
        serializer.serialize_none()
    } else {
        meta.serialize(serializer)
    }
}

fn deserialize_meta_or_null<'de, D>(deserializer: D) -> Result<MetaMapType, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<MetaMapType>::deserialize(deserializer).map(|opt| opt.unwrap_or_default())
}

// ---- OType constants (match Go's obj.OType_* constants) ----

pub const OTYPE_CLIENT: &str = "client";
pub const OTYPE_WINDOW: &str = "window";
pub const OTYPE_WORKSPACE: &str = "workspace";
pub const OTYPE_TAB: &str = "tab";
pub const OTYPE_LAYOUT: &str = "layout";
pub const OTYPE_BLOCK: &str = "block";
pub const OTYPE_TEMP: &str = "temp";

pub const VALID_OTYPES: &[&str] = &[
    OTYPE_CLIENT,
    OTYPE_WINDOW,
    OTYPE_WORKSPACE,
    OTYPE_TAB,
    OTYPE_LAYOUT,
    OTYPE_BLOCK,
    OTYPE_TEMP,
];

// ---- MetaMapType ----

/// Matches Go's `MetaMapType = map[string]any`.
pub type MetaMapType = HashMap<String, serde_json::Value>;

/// Merge `update` into `base`, matching Go's `MergeMeta` logic.
/// - Keys ending in `:*` with truthy value clear the section.
/// - `null` values delete the key.
/// - If `merge_special` is false, keys starting with `display:` are skipped.
pub fn merge_meta(base: &MetaMapType, update: &MetaMapType, merge_special: bool) -> MetaMapType {
    let mut result = base.clone();

    // First pass: handle "section:*" clear keys
    for (k, v) in update {
        if !k.ends_with(":*") {
            continue;
        }
        // Check if value is truthy (bool true)
        let is_true = matches!(v, serde_json::Value::Bool(true));
        if !is_true {
            continue;
        }
        let prefix = k.trim_end_matches(":*");
        if prefix.is_empty() {
            continue;
        }
        let prefix_colon = format!("{prefix}:");
        result.retain(|k2, _| k2 != prefix && !k2.starts_with(&prefix_colon));
    }

    // Second pass: merge regular keys
    for (k, v) in update {
        if !merge_special && k.starts_with("display:") {
            continue;
        }
        if k.ends_with(":*") {
            continue;
        }
        if v.is_null() {
            result.remove(k);
            continue;
        }
        result.insert(k.clone(), v.clone());
    }

    result
}

/// Helper to get a string value from MetaMapType.
pub fn meta_get_string(meta: &MetaMapType, key: &str, default: &str) -> String {
    meta.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or(default)
        .to_string()
}

/// Helper to get a bool value from MetaMapType.
pub fn meta_get_bool(meta: &MetaMapType, key: &str, default: bool) -> bool {
    meta.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

// ---- StoreObj trait ----

/// Rust equivalent of Go's `StoreObj` interface.
/// Every wave object has an otype, an OID, a version, and metadata.
pub trait StoreObj: Serialize + for<'de> Deserialize<'de> {
    fn get_otype() -> &'static str;
    fn get_oid(&self) -> &str;
    #[allow(dead_code)]
    fn set_oid(&mut self, oid: String);
    #[allow(dead_code)]
    fn get_version(&self) -> i64;
    fn set_version(&mut self, version: i64);
    #[allow(dead_code)]
    fn get_meta(&self) -> &MetaMapType;
    #[allow(dead_code)]
    fn set_meta(&mut self, meta: MetaMapType);

    #[allow(dead_code)]
    fn oref(&self) -> ORef {
        ORef::new(Self::get_otype(), self.get_oid())
    }
}

/// Macro that implements `StoreObj` for a struct that has standard fields:
/// `oid: String`, `version: i64`, `meta: MetaMapType`.
macro_rules! impl_wave_obj {
    ($ty:ty, $otype:expr) => {
        impl StoreObj for $ty {
            fn get_otype() -> &'static str {
                $otype
            }
            fn get_oid(&self) -> &str {
                &self.oid
            }
            fn set_oid(&mut self, oid: String) {
                self.oid = oid;
            }
            fn get_version(&self) -> i64 {
                self.version
            }
            fn set_version(&mut self, version: i64) {
                self.version = version;
            }
            fn get_meta(&self) -> &MetaMapType {
                &self.meta
            }
            fn set_meta(&mut self, meta: MetaMapType) {
                self.meta = meta;
            }
        }
    };
}

// ---- Update types ----

pub const UPDATE_TYPE_UPDATE: &str = "update";
#[allow(dead_code)]
pub const UPDATE_TYPE_DELETE: &str = "delete";

// ---- UIContext ----

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UIContext {
    #[serde(rename = "windowid")]
    pub window_id: String,
    #[serde(rename = "activetabid")]
    pub active_tab_id: String,
}

// ---- Point / WinSize / TermSize ----

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Point {
    pub x: i64,
    pub y: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WinSize {
    pub width: i64,
    pub height: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TermSize {
    pub rows: i64,
    pub cols: i64,
}

// ---- RuntimeOpts ----

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuntimeOpts {
    #[serde(default, skip_serializing_if = "is_default_term_size")]
    pub termsize: TermSize,
    #[serde(default, skip_serializing_if = "is_default_win_size")]
    pub winsize: WinSize,
}

fn is_default_term_size(ts: &TermSize) -> bool {
    ts.rows == 0 && ts.cols == 0
}
fn is_default_win_size(ws: &WinSize) -> bool {
    ws.width == 0 && ws.height == 0
}

// ---- FileDef / BlockDef ----

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FileDef {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BlockDef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<HashMap<String, FileDef>>,
    #[serde(default, skip_serializing_if = "MetaMapType::is_empty")]
    pub meta: MetaMapType,
}

// ---- StickerType ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StickerClickOpts {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sendinput: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub createblock: Option<BlockDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StickerDisplayOpts {
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub imgsrc: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub svgblob: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StickerType {
    pub stickertype: String,
    pub style: HashMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clickopts: Option<StickerClickOpts>,
    pub display: StickerDisplayOpts,
}

// ---- LayoutActionData / LeafOrderEntry ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutActionData {
    pub actiontype: String,
    pub actionid: String,
    pub blockid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nodesize: Option<u32>,
    /// Split-only: the new node's desired size as a fraction (0.0-1.0) of the
    /// TARGET node's current size, e.g. 0.5 for a 50/50 inner-direction drop,
    /// 0.2 for an outer-direction drop (matches the ghost's `leaf/5` band).
    /// The frontend applies this against the target's live `size` at split
    /// time (`layoutTree.ts:splitHorizontal/splitVertical`), which the
    /// backend cannot see. Takes precedence over `nodesize` when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nodesizefraction: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indexarr: Option<Vec<i32>>,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub magnified: bool,
    #[serde(default)]
    pub ephemeral: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub targetblockid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub position: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeafOrderEntry {
    pub nodeid: String,
    pub blockid: String,
}

// ====================================================================
// Core StoreObj types — each matches the Go struct + JSON tags exactly
// ====================================================================

/// Go: `Client` in pkg/obj/wtype.go
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Client {
    pub oid: String,
    pub version: i64,
    #[serde(default)]
    pub windowids: Vec<String>,
    #[serde(default, serialize_with = "serialize_meta_as_null_if_empty", deserialize_with = "deserialize_meta_or_null")]
    pub meta: MetaMapType,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub tosagreed: i64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hasoldhistory: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tempoid: String,
}

impl_wave_obj!(Client, OTYPE_CLIENT);

/// Go: `Window` in pkg/obj/wtype.go
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Window {
    pub oid: String,
    pub version: i64,
    #[serde(default)]
    pub workspaceid: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub isnew: bool,
    #[serde(default)]
    pub pos: Point,
    #[serde(default)]
    pub winsize: WinSize,
    #[serde(default)]
    pub lastfocusts: i64,
    /// SPEC_PILLAR1_STEP2 — the host's last-set opacity for this window
    /// (`0.0..=1.0`), so a crashed/restarted host can restore it instead
    /// of defaulting to fully opaque. `None` = never set / fully opaque.
    /// Written only via the `SetWindowOpacity` RPC (a direct store
    /// read-modify-write, not a reducer-dispatched Command — see
    /// `server::service::window::handle_window_service`); the host is
    /// the sole source of truth for the live value, this is a durable
    /// mirror.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    /// SPEC_PILLAR1_STEP3 — the window's kind (`"full_instance"` |
    /// `"subwindow"`), durable so a reproject can tell which native-window
    /// creation path to drive. `None` = unknown/legacy row; treat as
    /// `full_instance` on read (matches today's implicit default — every
    /// window was a full instance before this field existed). Written only
    /// via the `SetWindowTopology` RPC (a direct store read-modify-write,
    /// not reducer-dispatched — window kind/parent were never reducer
    /// state, same rationale as `opacity` above).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// SPEC_PILLAR1_STEP3 — the parent `Window.oid` for a `subwindow`;
    /// `None` for a `full_instance` (or an unset/legacy row). A `subwindow`
    /// with `parent_window_id: None` is a nonsensical state the
    /// `SetWindowTopology` RPC rejects at write time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_window_id: Option<String>,
    #[serde(default, serialize_with = "serialize_meta_as_null_if_empty", deserialize_with = "deserialize_meta_or_null")]
    pub meta: MetaMapType,
}

impl_wave_obj!(Window, OTYPE_WINDOW);

/// Go: `Workspace` in pkg/obj/wtype.go
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Workspace {
    pub oid: String,
    pub version: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default)]
    pub tabids: Vec<String>,
    #[serde(default)]
    pub pinnedtabids: Vec<String>,
    #[serde(default)]
    pub activetabid: String,
    #[serde(default, serialize_with = "serialize_meta_as_null_if_empty", deserialize_with = "deserialize_meta_or_null")]
    pub meta: MetaMapType,
}

impl_wave_obj!(Workspace, OTYPE_WORKSPACE);

/// Go: `WorkspaceListEntry` in pkg/obj/wtype.go
/// Used by ListWorkspaces — returns just {workspaceid, windowid}, not full workspace objects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceListEntry {
    pub workspaceid: String,
    #[serde(default)]
    pub windowid: String,
}

/// Go: `Tab` in pkg/obj/wtype.go
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Tab {
    pub oid: String,
    pub version: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub layoutstate: String,
    #[serde(default)]
    pub blockids: Vec<String>,
    #[serde(default, serialize_with = "serialize_meta_as_null_if_empty", deserialize_with = "deserialize_meta_or_null")]
    pub meta: MetaMapType,
}

impl_wave_obj!(Tab, OTYPE_TAB);

// ====================================================================
// Phase E.4.B Phase 3 — LayoutNode, LayoutNodeData, FlexDirection now
// live in agentmux-common so they can be referenced by ipc.rs Command/
// Event variants without a circular dependency. Re-exported here so
// all existing callers within agentmux-srv keep working unchanged.
// ====================================================================
pub use agentmux_common::{FlexDirection, LayoutNode, LayoutNodeData};

/// Go: `LayoutState` in pkg/obj/wtype.go
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LayoutState {
    pub oid: String,
    pub version: i64,
    /// Phase E.4.B Phase 2 — typed; was `Option<serde_json::Value>`.
    /// Wire format unchanged (serde-compat with the prior JSON shape).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rootnode: Option<LayoutNode>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub magnifiednodeid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub focusednodeid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leaforder: Option<Vec<LeafOrderEntry>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pendingbackendactions: Option<Vec<LayoutActionData>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<MetaMapType>,
}

// LayoutState has meta as Option<MetaMapType> (Go uses omitempty), so manual impl:
impl StoreObj for LayoutState {
    fn get_otype() -> &'static str {
        OTYPE_LAYOUT
    }
    fn get_oid(&self) -> &str {
        &self.oid
    }
    fn set_oid(&mut self, oid: String) {
        self.oid = oid;
    }
    fn get_version(&self) -> i64 {
        self.version
    }
    fn set_version(&mut self, version: i64) {
        self.version = version;
    }
    fn get_meta(&self) -> &MetaMapType {
        static EMPTY: std::sync::LazyLock<MetaMapType> = std::sync::LazyLock::new(MetaMapType::new);
        self.meta.as_ref().unwrap_or(&EMPTY)
    }
    fn set_meta(&mut self, meta: MetaMapType) {
        self.meta = Some(meta);
    }
}

/// Go: `Block` in pkg/obj/wtype.go
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Block {
    pub oid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub parentoref: String,
    pub version: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtimeopts: Option<RuntimeOpts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stickers: Option<Vec<StickerType>>,
    #[serde(default, serialize_with = "serialize_meta_as_null_if_empty", deserialize_with = "deserialize_meta_or_null")]
    pub meta: MetaMapType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subblockids: Option<Vec<String>>,
}

impl_wave_obj!(Block, OTYPE_BLOCK);

// ---- WaveObjUpdate ----

/// Represents an update notification for a wave object.
/// Matches Go's `WaveObjUpdate`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaveObjUpdate {
    pub updatetype: String,
    pub otype: String,
    pub oid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub obj: Option<serde_json::Value>,
}

// ---- Helpers ----

fn is_zero_i64(v: &i64) -> bool {
    *v == 0
}

/// Serialize any StoreObj to JSON bytes, including the "otype" field.
/// This matches Go's `obj.ToJson()`.
pub fn wave_obj_to_json<T: StoreObj>(obj: &T) -> Result<Vec<u8>, serde_json::Error> {
    let mut map = serde_json::to_value(obj)?;
    if let Some(m) = map.as_object_mut() {
        m.insert("otype".to_string(), serde_json::Value::String(T::get_otype().to_string()));
    }
    serde_json::to_vec(&map)
}

/// Serialize any StoreObj to a serde_json::Value, including the "otype" field.
/// This matches Go's `obj.ToJsonMap()` — used by GetObject/GetObjects responses.
pub fn wave_obj_to_value<T: StoreObj>(obj: &T) -> serde_json::Value {
    let mut map = serde_json::to_value(obj).unwrap_or_default();
    if let Some(m) = map.as_object_mut() {
        m.insert("otype".to_string(), serde_json::Value::String(T::get_otype().to_string()));
    }
    map
}

/// Deserialize JSON bytes to a specific StoreObj type.
/// Does NOT validate the otype field — caller should verify if needed.
pub fn wave_obj_from_json<T: StoreObj>(data: &[u8]) -> Result<T, serde_json::Error> {
    serde_json::from_slice(data)
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_roundtrip() {
        let client = Client {
            oid: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            version: 3,
            windowids: vec!["w1".to_string(), "w2".to_string()],
            meta: MetaMapType::new(),
            tosagreed: 1700000000000,
            ..Default::default()
        };
        let json = wave_obj_to_json(&client).unwrap();
        let parsed: Client = wave_obj_from_json(&json).unwrap();
        assert_eq!(parsed.oid, client.oid);
        assert_eq!(parsed.version, client.version);
        assert_eq!(parsed.windowids, client.windowids);
        assert_eq!(parsed.tosagreed, client.tosagreed);
    }

    #[test]
    fn test_window_roundtrip() {
        let window = Window {
            oid: "550e8400-e29b-41d4-a716-446655440001".to_string(),
            version: 1,
            workspaceid: "ws-123".to_string(),
            pos: Point { x: 100, y: 200 },
            winsize: WinSize {
                width: 1920,
                height: 1080,
            },
            lastfocusts: 1700000000000,
            meta: MetaMapType::new(),
            ..Default::default()
        };
        let json = wave_obj_to_json(&window).unwrap();
        let parsed: Window = wave_obj_from_json(&json).unwrap();
        assert_eq!(parsed.workspaceid, "ws-123");
        assert_eq!(parsed.pos.x, 100);
        assert_eq!(parsed.winsize.width, 1920);
    }

    #[test]
    fn test_workspace_roundtrip() {
        let ws = Workspace {
            oid: "ws-oid".to_string(),
            version: 2,
            name: "My Workspace".to_string(),
            tabids: vec!["t1".to_string(), "t2".to_string()],
            pinnedtabids: vec!["t0".to_string()],
            activetabid: "t1".to_string(),
            meta: MetaMapType::new(),
            ..Default::default()
        };
        let json = wave_obj_to_json(&ws).unwrap();
        let parsed: Workspace = wave_obj_from_json(&json).unwrap();
        assert_eq!(parsed.name, "My Workspace");
        assert_eq!(parsed.tabids.len(), 2);
        assert_eq!(parsed.pinnedtabids, vec!["t0"]);
    }

    #[test]
    fn test_tab_roundtrip() {
        let tab = Tab {
            oid: "tab-oid".to_string(),
            version: 1,
            name: "Tab 1".to_string(),
            layoutstate: "ls-123".to_string(),
            blockids: vec!["b1".to_string(), "b2".to_string()],
            meta: MetaMapType::new(),
        };
        let json = wave_obj_to_json(&tab).unwrap();
        let parsed: Tab = wave_obj_from_json(&json).unwrap();
        assert_eq!(parsed.name, "Tab 1");
        assert_eq!(parsed.blockids.len(), 2);
    }

    #[test]
    fn test_layout_state_roundtrip() {
        // Phase E.4.B Phase 2 — uses typed LayoutNode (was a junk JSON blob).
        let ls = LayoutState {
            oid: "ls-oid".to_string(),
            version: 1,
            rootnode: Some(LayoutNode {
                id: "node-1".into(),
                flex_direction: FlexDirection::Row,
                size: 1.0,
                children: Vec::new(),
                data: None,
                ..Default::default()
            }),
            magnifiednodeid: "node-1".to_string(),
            ..Default::default()
        };
        let json = wave_obj_to_json(&ls).unwrap();
        let parsed: LayoutState = wave_obj_from_json(&json).unwrap();
        assert_eq!(parsed.magnifiednodeid, "node-1");
        assert!(parsed.rootnode.is_some());
        assert_eq!(parsed.rootnode.as_ref().unwrap().id, "node-1");
    }

    /// Phase E.4.B Phase 2 acceptance gate: deserializing the JSON shape
    /// the frontend produces today MUST yield the typed LayoutNode, and
    /// reserializing MUST produce equivalent JSON. The shapes here cover:
    ///   1. Single-leaf root (dnd / tear-off shape)
    ///   2. A multi-pane shape (deep nesting + mixed group/leaf nodes) —
    ///      the former first-launch four-pane layout, see below
    ///   3. Edge: missing `children` array for leaves
    #[test]
    fn test_layout_node_serde_compat_with_frontend_shapes() {
        // Shape 1 — single leaf (matches dnd.rs / service.rs writers).
        let leaf_json = serde_json::json!({
            "id": "node-uuid-1",
            "data": { "blockId": "block-uuid-1" },
            "flexDirection": "row",
            "size": 1
        });
        let leaf: LayoutNode = serde_json::from_value(leaf_json).unwrap();
        assert_eq!(leaf.id, "node-uuid-1");
        assert_eq!(leaf.flex_direction, FlexDirection::Row);
        assert_eq!(leaf.size, 1.0);
        assert!(leaf.children.is_empty());
        assert_eq!(leaf.data.as_ref().unwrap().block_id, "block-uuid-1");
        // Round-trip preserves shape.
        let reserialized = serde_json::to_value(&leaf).unwrap();
        let reparsed: LayoutNode = serde_json::from_value(reserialized).unwrap();
        assert_eq!(leaf, reparsed);

        // Shape 2 — a nested row/column tree with a multi-child right
        // column. This was the first-launch four-pane layout when written;
        // the default is now three panes (wcore::default_three_pane_tree),
        // but the fixture is kept as-is: what it exercises is serde
        // round-tripping of a nested LayoutNode, not the current default.
        let four_pane_json = serde_json::json!({
            "id": "root-id",
            "flexDirection": "row",
            "size": 10,
            "children": [
                {
                    "id": "agent-id",
                    "flexDirection": "column",
                    "size": 5,
                    "data": { "blockId": "agent-block" }
                },
                {
                    "id": "right-col-id",
                    "flexDirection": "column",
                    "size": 5,
                    "children": [
                        {
                            "id": "swarm-id",
                            "flexDirection": "row",
                            "size": 4,
                            "data": { "blockId": "swarm-block" }
                        },
                        {
                            "id": "armory-id",
                            "flexDirection": "row",
                            "size": 4,
                            "data": { "blockId": "armory-block" }
                        },
                        {
                            "id": "sysinfo-id",
                            "flexDirection": "row",
                            "size": 2,
                            "data": { "blockId": "sysinfo-block" }
                        }
                    ]
                }
            ]
        });
        let root: LayoutNode = serde_json::from_value(four_pane_json).unwrap();
        assert_eq!(root.id, "root-id");
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.children[0].id, "agent-id");
        assert_eq!(root.children[0].flex_direction, FlexDirection::Column);
        assert_eq!(root.children[0].data.as_ref().unwrap().block_id, "agent-block");
        let right_col = &root.children[1];
        assert_eq!(right_col.children.len(), 3);
        assert_eq!(right_col.data, None); // group node, no leaf data
        assert_eq!(right_col.children[0].data.as_ref().unwrap().block_id, "swarm-block");
        assert_eq!(right_col.children[1].data.as_ref().unwrap().block_id, "armory-block");
        assert_eq!(right_col.children[2].data.as_ref().unwrap().block_id, "sysinfo-block");
        let reserialized = serde_json::to_value(&root).unwrap();
        let reparsed: LayoutNode = serde_json::from_value(reserialized).unwrap();
        assert_eq!(root, reparsed);

        // Shape 3 — leaf without `children` array (real on-disk blobs may
        // or may not have `"children": []`; deserializer must tolerate either).
        let leaf_no_children = serde_json::json!({
            "id": "loner",
            "flexDirection": "column",
            "size": 5,
            "data": { "blockId": "b" }
        });
        let parsed: LayoutNode = serde_json::from_value(leaf_no_children).unwrap();
        assert!(parsed.children.is_empty());
    }

    /// Phase E.4.B Phase 2 — deserializing a node WITHOUT `flexDirection`
    /// falls back to `Row` default. Defensive against older blobs.
    #[test]
    fn test_layout_node_defaults_flex_direction_to_row() {
        let no_dir = serde_json::json!({"id": "x", "size": 1});
        let parsed: LayoutNode = serde_json::from_value(no_dir).unwrap();
        assert_eq!(parsed.flex_direction, FlexDirection::Row);
    }

    /// Phase E.4.B Phase 2 (reagent P2 PR #688) — whole-number sizes
    /// serialize as integer JSON literals, not float (`10` not `10.0`).
    /// This preserves byte-equal compatibility with the prior
    /// `serde_json::json!()`-produced JSON which used integer literals.
    #[test]
    fn test_layout_node_size_serializes_as_integer_when_whole() {
        let whole = LayoutNode {
            id: "x".into(),
            flex_direction: FlexDirection::Row,
            size: 10.0,
            children: Vec::new(),
            data: None,
            ..Default::default()
        };
        let json = serde_json::to_string(&whole).unwrap();
        assert!(json.contains("\"size\":10"), "whole sizes should serialize as integer; got {}", json);
        assert!(!json.contains("\"size\":10.0"), "whole sizes should NOT include the decimal; got {}", json);

        // Fractional sizes still serialize with the decimal.
        let frac = LayoutNode {
            id: "y".into(),
            flex_direction: FlexDirection::Row,
            size: 5.5,
            children: Vec::new(),
            data: None,
            ..Default::default()
        };
        let json = serde_json::to_string(&frac).unwrap();
        assert!(json.contains("\"size\":5.5"), "fractional sizes preserve the decimal; got {}", json);

        // Round-trip works either way.
        let reparsed: LayoutNode = serde_json::from_str(&serde_json::to_string(&whole).unwrap()).unwrap();
        assert_eq!(reparsed.size, 10.0);
    }

    /// Phase E.4.B Phase 2 — `data: None` serializes ABSENT (not `"data": null`).
    /// Frontend tolerates either, but absent matches the pre-typed JSON shape.
    #[test]
    fn test_layout_node_data_skipped_when_none() {
        let group = LayoutNode {
            id: "g".into(),
            flex_direction: FlexDirection::Row,
            size: 1.0,
            children: Vec::new(),
            data: None,
            ..Default::default()
        };
        let json = serde_json::to_value(&group).unwrap();
        assert!(json.get("data").is_none(), "data: None must skip-serialize");
    }

    /// Regression test: unknown fields on LayoutNode and LayoutNodeData round-
    /// trip via the `extra: HashMap` catch-all (codex P1 PR #688 + #689).
    /// Without this, fields the frontend writes that we don't yet model would
    /// be silently dropped on deserialize→serialize cycles — a regression vs
    /// the prior `serde_json::Value` storage.
    #[test]
    fn test_layout_node_preserves_unknown_fields() {
        let json_with_extras = serde_json::json!({
            "id": "n",
            "flexDirection": "row",
            "size": 1,
            "backendLayoutVersion": 7,
            "data": {
                "blockId": "b",
                "renderHint": "compact"
            }
        });
        let node: LayoutNode = serde_json::from_value(json_with_extras).unwrap();
        assert_eq!(node.extra.get("backendLayoutVersion"), Some(&serde_json::json!(7)));
        assert_eq!(
            node.data.as_ref().unwrap().extra.get("renderHint"),
            Some(&serde_json::json!("compact"))
        );
        // Round-trip preserves both.
        let reserialized = serde_json::to_value(&node).unwrap();
        assert_eq!(reserialized.get("backendLayoutVersion"), Some(&serde_json::json!(7)));
        assert_eq!(
            reserialized.get("data").and_then(|d| d.get("renderHint")),
            Some(&serde_json::json!("compact"))
        );
        // Empty extra → no stray "extra" key on output.
        let no_extras = LayoutNode {
            id: "x".into(),
            flex_direction: FlexDirection::Row,
            size: 1.0,
            ..Default::default()
        };
        let json = serde_json::to_value(&no_extras).unwrap();
        assert!(json.get("extra").is_none());
    }

    /// Regression test (codex P1 PR #689): missing blockId in JSON must fail
    /// deserialization rather than silently default to "". An empty
    /// block_id would cause incorrect matching wherever code resolves a
    /// layout node by block id (e.g. the reducer's
    /// `find_node_id_by_block`/`LayoutDeleteNodeByBlock`).
    #[test]
    fn test_layout_node_data_missing_block_id_fails_deserialization() {
        let missing_id = serde_json::json!({ "renderHint": "compact" });
        let result = serde_json::from_value::<LayoutNodeData>(missing_id);
        assert!(result.is_err(), "missing blockId must fail deserialization");
    }

    #[test]
    fn test_block_roundtrip() {
        let block = Block {
            oid: "blk-oid".to_string(),
            version: 5,
            parentoref: "tab:parent-id".to_string(),
            meta: {
                let mut m = MetaMapType::new();
                m.insert("view".to_string(), serde_json::json!("term"));
                m
            },
            ..Default::default()
        };
        let json = wave_obj_to_json(&block).unwrap();
        let parsed: Block = wave_obj_from_json(&json).unwrap();
        assert_eq!(parsed.parentoref, "tab:parent-id");
        assert_eq!(
            parsed.meta.get("view").and_then(|v| v.as_str()),
            Some("term")
        );
    }

    #[test]
    fn test_wire_compat_go_json_client() {
        // Hardcoded JSON from Go's encoding to verify wire compatibility
        let go_json = r#"{"oid":"abc-123","otype":"client","version":2,"windowids":["w1"],"meta":{"view":"term"},"tosagreed":1700000000000}"#;
        let client: Client = serde_json::from_str(go_json).unwrap();
        assert_eq!(client.oid, "abc-123");
        assert_eq!(client.version, 2);
        assert_eq!(client.windowids, vec!["w1"]);
        assert_eq!(client.tosagreed, 1700000000000);
    }

    #[test]
    fn test_wire_compat_go_json_block() {
        let go_json = r#"{"oid":"blk-1","otype":"block","version":3,"parentoref":"tab:t1","meta":{"view":"term","cmd":"ls"}}"#;
        let block: Block = serde_json::from_str(go_json).unwrap();
        assert_eq!(block.oid, "blk-1");
        assert_eq!(block.parentoref, "tab:t1");
        assert_eq!(block.version, 3);
    }

    #[test]
    fn test_wire_compat_go_json_tab() {
        let go_json = r#"{"oid":"tab-1","otype":"tab","version":1,"name":"Shell","layoutstate":"ls-1","blockids":["b1","b2"],"meta":{}}"#;
        let tab: Tab = serde_json::from_str(go_json).unwrap();
        assert_eq!(tab.name, "Shell");
        assert_eq!(tab.blockids, vec!["b1", "b2"]);
    }

    #[test]
    fn test_wave_obj_to_json_includes_otype() {
        let client = Client {
            oid: "test".to_string(),
            version: 1,
            ..Default::default()
        };
        let json_bytes = wave_obj_to_json(&client).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&json_bytes).unwrap();
        assert_eq!(v["otype"], "client");
    }

    #[test]
    fn test_merge_meta_basic() {
        let mut base = MetaMapType::new();
        base.insert("view".into(), serde_json::json!("term"));
        base.insert("cmd".into(), serde_json::json!("ls"));

        let mut update = MetaMapType::new();
        update.insert("cmd".into(), serde_json::json!("pwd"));
        update.insert("icon".into(), serde_json::json!("star"));

        let result = merge_meta(&base, &update, true);
        assert_eq!(result["view"], "term");
        assert_eq!(result["cmd"], "pwd");
        assert_eq!(result["icon"], "star");
    }

    #[test]
    fn test_merge_meta_null_deletes() {
        let mut base = MetaMapType::new();
        base.insert("view".into(), serde_json::json!("term"));
        base.insert("cmd".into(), serde_json::json!("ls"));

        let mut update = MetaMapType::new();
        update.insert("cmd".into(), serde_json::Value::Null);

        let result = merge_meta(&base, &update, true);
        assert_eq!(result.get("view").unwrap(), "term");
        assert!(!result.contains_key("cmd"));
    }

    #[test]
    fn test_merge_meta_section_clear() {
        let mut base = MetaMapType::new();
        base.insert("frame".into(), serde_json::json!(true));
        base.insert("frame:title".into(), serde_json::json!("hello"));
        base.insert("frame:icon".into(), serde_json::json!("star"));
        base.insert("cmd".into(), serde_json::json!("ls"));

        let mut update = MetaMapType::new();
        update.insert("frame:*".into(), serde_json::json!(true));

        let result = merge_meta(&base, &update, true);
        assert!(!result.contains_key("frame"));
        assert!(!result.contains_key("frame:title"));
        assert!(!result.contains_key("frame:icon"));
        assert_eq!(result["cmd"], "ls"); // unrelated key preserved
    }

    #[test]
    fn test_merge_meta_skip_display_when_not_special() {
        let base = MetaMapType::new();
        let mut update = MetaMapType::new();
        update.insert("display:name".into(), serde_json::json!("test"));
        update.insert("view".into(), serde_json::json!("term"));

        let result = merge_meta(&base, &update, false);
        assert!(!result.contains_key("display:name"));
        assert_eq!(result["view"], "term");
    }

    #[test]
    fn test_meta_get_string() {
        let mut meta = MetaMapType::new();
        meta.insert("view".into(), serde_json::json!("term"));
        assert_eq!(meta_get_string(&meta, "view", "default"), "term");
        assert_eq!(meta_get_string(&meta, "missing", "default"), "default");
    }

    #[test]
    fn test_meta_get_bool() {
        let mut meta = MetaMapType::new();
        meta.insert("edit".into(), serde_json::json!(true));
        assert!(meta_get_bool(&meta, "edit", false));
        assert!(!meta_get_bool(&meta, "missing", false));
    }
}
