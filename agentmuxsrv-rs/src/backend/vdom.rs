// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Virtual DOM types and wire protocol.
//! Port of Go's pkg/vdom/.

#![allow(dead_code)]
//!
//! Defines the VDOM element types, protocol messages for frontend-backend
//! communication, event types, and transfer format conversion utilities.
//! The reflection-based component rendering engine is deferred until
//! the sidecar is replaced; this module provides the type layer.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---- Constants ----

/// Text node tag.
pub const TEXT_TAG: &str = "#text";

/// Wave text tag (explicit text element).
pub const WAVE_TEXT_TAG: &str = "wave:text";

/// Wave null tag (empty placeholder).
pub const WAVE_NULL_TAG: &str = "wave:null";

/// Fragment tag (multiple children without wrapper).
pub const FRAGMENT_TAG: &str = "#fragment";

/// Binding tag.
pub const BIND_TAG: &str = "#bind";

/// Children prop key.
pub const CHILDREN_PROP_KEY: &str = "children";

/// Key prop key.
pub const KEY_PROP_KEY: &str = "key";

/// Ref object type.
pub const OBJECT_TYPE_REF: &str = "ref";

/// Binding object type.
pub const OBJECT_TYPE_BINDING: &str = "binding";

/// Function object type.
pub const OBJECT_TYPE_FUNC: &str = "func";

/// HTML binding prefix.
pub const HTML_BIND_PREFIX: &str = "#bind:";

/// HTML parameter prefix.
pub const HTML_PARAM_PREFIX: &str = "#param:";

/// HTML global event prefix.
pub const HTML_GLOBAL_EVENT_PREFIX: &str = "#globalevent";

/// Initial chunk size for backend updates.
pub const BACKEND_UPDATE_INITIAL_CHUNK_SIZE: usize = 50;

/// Subsequent chunk size for backend updates.
pub const BACKEND_UPDATE_CHUNK_SIZE: usize = 100;

// ---- Core Element Types ----

/// A VDOM element (in-memory representation with nested children).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VDomElem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waveid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub props: Option<HashMap<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<VDomElem>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl VDomElem {
    /// Create a text element.
    pub fn text(text: &str) -> Self {
        Self {
            tag: Some(TEXT_TAG.to_string()),
            text: Some(text.to_string()),
            ..Default::default()
        }
    }

    /// Create a null element.
    pub fn null() -> Self {
        Self {
            tag: Some(WAVE_NULL_TAG.to_string()),
            ..Default::default()
        }
    }

    /// Get the key prop, if set.
    pub fn key(&self) -> Option<&str> {
        self.props
            .as_ref()?
            .get(KEY_PROP_KEY)?
            .as_str()
    }

    /// Set the key prop.
    pub fn with_key(mut self, key: &str) -> Self {
        let props = self.props.get_or_insert_with(HashMap::new);
        props.insert(KEY_PROP_KEY.to_string(), serde_json::json!(key));
        self
    }

    /// Check if this is a text node.
    pub fn is_text(&self) -> bool {
        self.tag.as_deref() == Some(TEXT_TAG)
    }

    /// Check if this uses a wave: or w: tag.
    pub fn is_wave_tag(&self) -> bool {
        match self.tag.as_deref() {
            Some(tag) => tag.starts_with("wave:") || tag.starts_with("w:"),
            None => false,
        }
    }

    /// Check if this is a base tag (not a custom component).
    pub fn is_base_tag(&self) -> bool {
        match self.tag.as_deref() {
            Some(tag) => {
                tag.starts_with('#')
                    || tag.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                    || tag.starts_with("wave:")
                    || tag.starts_with("w:")
            }
            None => false,
        }
    }
}

/// Wire format element (children as WaveId strings, not nested).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VDomTransferElem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waveid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub props: Option<HashMap<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

// ---- Protocol Messages ----

/// Context creation request (client → server).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VDomCreateContext {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub ts: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<crate::backend::MetaMapType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<VDomTarget>,
    #[serde(default)]
    pub persist: bool,
}

/// Async initiation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VDomAsyncInitiationRequest {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub ts: i64,
    pub blockid: String,
}

/// Frontend → Backend update message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VDomFrontendUpdate {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub ts: i64,
    pub blockid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlationid: Option<String>,
    #[serde(default)]
    pub dispose: bool,
    #[serde(default)]
    pub resync: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rendercontext: Option<VDomRenderContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<Vec<VDomEvent>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statesync: Option<Vec<VDomStateSync>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refupdates: Option<Vec<VDomRefUpdate>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messages: Option<Vec<VDomMessage>>,
}

/// Backend → Frontend update message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VDomBackendUpdate {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub ts: i64,
    pub blockid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opts: Option<VDomBackendOpts>,
    #[serde(default)]
    pub haswork: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renderupdates: Option<Vec<VDomRenderUpdate>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transferelems: Option<Vec<VDomTransferElem>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statesync: Option<Vec<VDomStateSync>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refoperations: Option<Vec<VDomRefOperation>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messages: Option<Vec<VDomMessage>>,
}

// ---- Prop Types ----

/// Data binding reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VDomBinding {
    #[serde(rename = "type")]
    pub binding_type: String,
    pub bind: String,
}

impl VDomBinding {
    pub fn new(atom_name: &str) -> Self {
        Self {
            binding_type: OBJECT_TYPE_BINDING.to_string(),
            bind: atom_name.to_string(),
        }
    }
}

/// Event handler function reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VDomFunc {
    #[serde(rename = "type")]
    pub func_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stoppropagation: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preventdefault: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub globalevent: Option<String>,
    #[serde(default, rename = "#keys", skip_serializing_if = "Option::is_none")]
    pub keys: Option<Vec<String>>,
}

/// DOM element reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VDomRef {
    #[serde(rename = "type")]
    pub ref_type: String,
    pub refid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trackposition: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<VDomRefPosition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hascurrent: Option<bool>,
}

// ---- DOM Geometry ----

/// DOM bounding client rect.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DomRect {
    #[serde(default)]
    pub top: f64,
    #[serde(default)]
    pub left: f64,
    #[serde(default)]
    pub right: f64,
    #[serde(default)]
    pub bottom: f64,
    #[serde(default)]
    pub width: f64,
    #[serde(default)]
    pub height: f64,
}

/// DOM element position and size information.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VDomRefPosition {
    #[serde(default)]
    pub offsetheight: f64,
    #[serde(default)]
    pub offsetwidth: f64,
    #[serde(default)]
    pub scrollheight: f64,
    #[serde(default)]
    pub scrollwidth: f64,
    #[serde(default)]
    pub scrolltop: f64,
    #[serde(default)]
    pub scrollleft: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundingclientrect: Option<DomRect>,
}

// ---- Event Types ----

/// DOM event payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VDomEvent {
    pub waveid: String,
    pub eventtype: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub globaleventtype: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targetvalue: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targetchecked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targetname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targetid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keydata: Option<WaveKeyboardEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mousedata: Option<WavePointerData>,
}

/// Keyboard event data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaveKeyboardEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub repeat: bool,
    #[serde(default)]
    pub location: u32,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub control: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub meta: bool,
    #[serde(default)]
    pub cmd: bool,
    #[serde(default)]
    pub option: bool,
}

/// Mouse/pointer event data.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WavePointerData {
    #[serde(default)]
    pub button: i32,
    #[serde(default)]
    pub buttons: i32,
    #[serde(default, rename = "clientX")]
    pub client_x: f64,
    #[serde(default, rename = "clientY")]
    pub client_y: f64,
    #[serde(default, rename = "pageX")]
    pub page_x: f64,
    #[serde(default, rename = "pageY")]
    pub page_y: f64,
    #[serde(default, rename = "screenX")]
    pub screen_x: f64,
    #[serde(default, rename = "screenY")]
    pub screen_y: f64,
    #[serde(default, rename = "movementX")]
    pub movement_x: f64,
    #[serde(default, rename = "movementY")]
    pub movement_y: f64,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub control: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub meta: bool,
    #[serde(default)]
    pub cmd: bool,
    #[serde(default)]
    pub option: bool,
}

// ---- State & Refs ----

/// Rendering context.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VDomRenderContext {
    #[serde(default)]
    pub blockid: String,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub width: i32,
    #[serde(default)]
    pub height: i32,
    #[serde(default)]
    pub rootrefid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
}

/// Atom state synchronization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VDomStateSync {
    pub atom: String,
    pub value: serde_json::Value,
}

/// Frontend ref update (position information).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VDomRefUpdate {
    pub refid: String,
    #[serde(default)]
    pub hascurrent: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<VDomRefPosition>,
}

// ---- Backend Options ----

/// Backend options sent to frontend.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VDomBackendOpts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closeonctrlc: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub globalkeyboardevents: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub globalstyles: Option<String>,
}

// ---- Render Operations ----

/// Render update instruction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VDomRenderUpdate {
    pub updatetype: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waveid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vdomwaveid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vdom: Option<VDomElem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<i32>,
}

/// Ref operation instruction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VDomRefOperation {
    pub refid: String,
    pub op: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Vec<serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outputref: Option<String>,
}

/// Message/log output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VDomMessage {
    pub messagetype: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stacktrace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Vec<serde_json::Value>>,
}

// ---- Target Types ----

/// Rendering target specification.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VDomTarget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newblock: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub magnified: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toolbar: Option<VDomTargetToolbar>,
}

/// Toolbar target specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VDomTargetToolbar {
    pub toolbar: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<i32>,
}

// ---- Transfer Format Conversion ----

/// Convert a tree of VDomElem into a flat list of VDomTransferElem.
///
/// The transfer format replaces nested children with WaveId references,
/// allowing the wire format to be a flat array instead of a deep tree.
pub fn convert_elems_to_transfer(elems: &[VDomElem]) -> Vec<VDomTransferElem> {
    let mut result = Vec::new();
    for elem in elems {
        convert_elem_recursive(elem, &mut result);
    }
    result
}

fn convert_elem_recursive(elem: &VDomElem, out: &mut Vec<VDomTransferElem>) {
    let child_ids: Option<Vec<String>> = elem.children.as_ref().map(|children| {
        children
            .iter()
            .filter_map(|child| {
                convert_elem_recursive(child, out);
                child.waveid.clone()
            })
            .collect()
    });

    let transfer = VDomTransferElem {
        waveid: elem.waveid.clone(),
        tag: elem.tag.clone(),
        props: elem.props.clone(),
        children: child_ids,
        text: elem.text.clone(),
    };
    out.push(transfer);
}

/// Deduplicate transfer elements by WaveId (last wins).
pub fn dedup_transfer_elems(elems: Vec<VDomTransferElem>) -> Vec<VDomTransferElem> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::with_capacity(elems.len());

    // Process in reverse so last occurrence wins
    for elem in elems.into_iter().rev() {
        let id = elem.waveid.clone().unwrap_or_default();
        if id.is_empty() || seen.insert(id) {
            result.push(elem);
        }
    }

    result.reverse();
    result
}

/// Split a large backend update into chunks for transmission.
///
/// First chunk uses BACKEND_UPDATE_INITIAL_CHUNK_SIZE (50),
/// subsequent chunks use BACKEND_UPDATE_CHUNK_SIZE (100).
pub fn split_backend_update(update: VDomBackendUpdate) -> Vec<VDomBackendUpdate> {
    let transfer_elems = match update.transferelems {
        Some(ref elems) if elems.len() > BACKEND_UPDATE_INITIAL_CHUNK_SIZE => {
            update.transferelems.unwrap()
        }
        _ => return vec![update],
    };

    let mut result = Vec::new();
    let total = transfer_elems.len();

    // First chunk: initial size with all other fields
    let first_size = BACKEND_UPDATE_INITIAL_CHUNK_SIZE.min(total);
    let first_chunk = VDomBackendUpdate {
        msg_type: update.msg_type.clone(),
        ts: update.ts,
        blockid: update.blockid.clone(),
        opts: update.opts.clone(),
        haswork: update.haswork || total > first_size,
        renderupdates: update.renderupdates.clone(),
        transferelems: Some(transfer_elems[..first_size].to_vec()),
        statesync: update.statesync.clone(),
        refoperations: update.refoperations.clone(),
        messages: update.messages.clone(),
    };
    result.push(first_chunk);
    let mut offset = first_size;

    // Subsequent chunks: only transfer elems
    while offset < total {
        let end = (offset + BACKEND_UPDATE_CHUNK_SIZE).min(total);
        let chunk = VDomBackendUpdate {
            msg_type: update.msg_type.clone(),
            ts: update.ts,
            blockid: update.blockid.clone(),
            opts: None,
            haswork: end < total,
            renderupdates: None,
            transferelems: Some(transfer_elems[offset..end].to_vec()),
            statesync: None,
            refoperations: None,
            messages: None,
        };
        result.push(chunk);
        offset = end;
    }

    result
}

/// Convert a CSS property name from kebab-case to camelCase.
///
/// E.g., "background-color" → "backgroundColor", "-webkit-transform" → "WebkitTransform"
pub fn to_react_name(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut capitalize_next = false;
    let mut first = true;

    for ch in input.chars() {
        if ch == '-' {
            if first {
                // Leading dash (vendor prefix): capitalize next
                capitalize_next = true;
                first = false;
                continue;
            }
            capitalize_next = true;
        } else if capitalize_next {
            result.push(ch.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(ch);
        }
        first = false;
    }

    result
}

// ---- Tests ----

#[cfg(test)]
mod tests {
    use super::*;

    // -- VDomElem tests --

    #[test]
    fn test_text_elem() {
        let elem = VDomElem::text("hello");
        assert!(elem.is_text());
        assert_eq!(elem.text.as_deref(), Some("hello"));
        assert_eq!(elem.tag.as_deref(), Some(TEXT_TAG));
    }

    #[test]
    fn test_null_elem() {
        let elem = VDomElem::null();
        assert_eq!(elem.tag.as_deref(), Some(WAVE_NULL_TAG));
        assert!(elem.is_wave_tag());
    }

    #[test]
    fn test_elem_key() {
        let elem = VDomElem::default().with_key("my-key");
        assert_eq!(elem.key(), Some("my-key"));
    }

    #[test]
    fn test_elem_no_key() {
        let elem = VDomElem::default();
        assert_eq!(elem.key(), None);
    }

    #[test]
    fn test_is_base_tag() {
        let mut elem = VDomElem::default();
        elem.tag = Some("div".to_string());
        assert!(elem.is_base_tag());

        elem.tag = Some("#text".to_string());
        assert!(elem.is_base_tag());

        elem.tag = Some("wave:null".to_string());
        assert!(elem.is_base_tag());

        elem.tag = Some("MyComponent".to_string());
        assert!(!elem.is_base_tag());
    }

    #[test]
    fn test_is_wave_tag() {
        let mut elem = VDomElem::default();
        elem.tag = Some("wave:text".to_string());
        assert!(elem.is_wave_tag());

        elem.tag = Some("w:custom".to_string());
        assert!(elem.is_wave_tag());

        elem.tag = Some("div".to_string());
        assert!(!elem.is_wave_tag());
    }

    // -- Serde tests --

    #[test]
    fn test_vdom_elem_serde() {
        let elem = VDomElem {
            waveid: Some("abc-123".to_string()),
            tag: Some("div".to_string()),
            props: Some(HashMap::from([(
                "class".to_string(),
                serde_json::json!("container"),
            )])),
            children: Some(vec![VDomElem::text("hello")]),
            text: None,
        };

        let json = serde_json::to_string(&elem).unwrap();
        assert!(json.contains("\"waveid\":\"abc-123\""));
        assert!(json.contains("\"tag\":\"div\""));

        let parsed: VDomElem = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.waveid.as_deref(), Some("abc-123"));
        assert_eq!(parsed.children.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_vdom_elem_minimal_serde() {
        let json = r#"{"tag":"span"}"#;
        let elem: VDomElem = serde_json::from_str(json).unwrap();
        assert_eq!(elem.tag.as_deref(), Some("span"));
        assert!(elem.waveid.is_none());
        assert!(elem.children.is_none());
    }

    #[test]
    fn test_transfer_elem_serde() {
        let te = VDomTransferElem {
            waveid: Some("id-1".to_string()),
            tag: Some("div".to_string()),
            props: None,
            children: Some(vec!["id-2".to_string(), "id-3".to_string()]),
            text: None,
        };

        let json = serde_json::to_string(&te).unwrap();
        let parsed: VDomTransferElem = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.children.unwrap(), vec!["id-2", "id-3"]);
    }

    #[test]
    fn test_frontend_update_serde() {
        let json = r#"{
            "type": "frontendupdate",
            "ts": 1700000000000,
            "blockid": "block-1",
            "dispose": false,
            "resync": true,
            "events": [{"waveid": "elem-1", "eventtype": "click"}]
        }"#;

        let update: VDomFrontendUpdate = serde_json::from_str(json).unwrap();
        assert_eq!(update.msg_type, "frontendupdate");
        assert_eq!(update.blockid, "block-1");
        assert!(update.resync);
        assert_eq!(update.events.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_backend_update_serde() {
        let update = VDomBackendUpdate {
            msg_type: "backendupdate".to_string(),
            ts: 1700000000000,
            blockid: "block-1".to_string(),
            opts: Some(VDomBackendOpts {
                closeonctrlc: Some(true),
                ..Default::default()
            }),
            haswork: false,
            renderupdates: None,
            transferelems: None,
            statesync: Some(vec![VDomStateSync {
                atom: "count".to_string(),
                value: serde_json::json!(42),
            }]),
            refoperations: None,
            messages: None,
        };

        let json = serde_json::to_string(&update).unwrap();
        let parsed: VDomBackendUpdate = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.opts.unwrap().closeonctrlc, Some(true));
        assert_eq!(parsed.statesync.unwrap()[0].atom, "count");
    }

    #[test]
    fn test_keyboard_event_serde() {
        let json = r#"{
            "type": "keydown",
            "key": "Enter",
            "code": "Enter",
            "shift": false,
            "control": true,
            "alt": false,
            "meta": false,
            "cmd": false,
            "option": false,
            "repeat": false,
            "location": 0
        }"#;

        let event: WaveKeyboardEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.event_type, "keydown");
        assert_eq!(event.key, "Enter");
        assert!(event.control);
        assert!(!event.shift);
    }

    #[test]
    fn test_pointer_data_serde() {
        let json = r#"{"button":0,"buttons":1,"clientX":100.5,"clientY":200.3,"pageX":100.5,"pageY":200.3,"screenX":100.5,"screenY":200.3,"movementX":0,"movementY":0,"shift":false,"control":false,"alt":false,"meta":false,"cmd":false,"option":false}"#;

        let data: WavePointerData = serde_json::from_str(json).unwrap();
        assert_eq!(data.button, 0);
        assert!((data.client_x - 100.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_vdom_event_serde() {
        let event = VDomEvent {
            waveid: "elem-1".to_string(),
            eventtype: "onChange".to_string(),
            globaleventtype: None,
            targetvalue: Some("new value".to_string()),
            targetchecked: None,
            targetname: Some("input1".to_string()),
            targetid: None,
            keydata: None,
            mousedata: None,
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(!json.contains("targetchecked")); // None skipped
        assert!(json.contains("\"targetvalue\":\"new value\""));
    }

    #[test]
    fn test_binding_new() {
        let binding = VDomBinding::new("myAtom");
        assert_eq!(binding.binding_type, OBJECT_TYPE_BINDING);
        assert_eq!(binding.bind, "myAtom");

        let json = serde_json::to_string(&binding).unwrap();
        assert!(json.contains("\"type\":\"binding\""));
    }

    #[test]
    fn test_vdom_target_serde() {
        let target = VDomTarget {
            newblock: Some(true),
            magnified: None,
            toolbar: None,
        };

        let json = serde_json::to_string(&target).unwrap();
        assert!(json.contains("\"newblock\":true"));
        assert!(!json.contains("magnified"));

        let target2 = VDomTarget {
            newblock: None,
            magnified: None,
            toolbar: Some(VDomTargetToolbar {
                toolbar: "bottom".to_string(),
                height: Some(48),
            }),
        };

        let json2 = serde_json::to_string(&target2).unwrap();
        assert!(json2.contains("\"toolbar\":\"bottom\""));
        assert!(json2.contains("\"height\":48"));
    }

    #[test]
    fn test_render_context_serde() {
        let ctx = VDomRenderContext {
            blockid: "block-1".to_string(),
            focused: true,
            width: 800,
            height: 600,
            rootrefid: "root-ref".to_string(),
            background: Some("#000".to_string()),
        };

        let json = serde_json::to_string(&ctx).unwrap();
        let parsed: VDomRenderContext = serde_json::from_str(&json).unwrap();
        assert!(parsed.focused);
        assert_eq!(parsed.width, 800);
    }

    #[test]
    fn test_ref_operation_serde() {
        let op = VDomRefOperation {
            refid: "ref-1".to_string(),
            op: "focus".to_string(),
            params: None,
            outputref: None,
        };

        let json = serde_json::to_string(&op).unwrap();
        let parsed: VDomRefOperation = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.op, "focus");
    }

    #[test]
    fn test_message_serde() {
        let msg = VDomMessage {
            messagetype: "error".to_string(),
            message: "something failed".to_string(),
            stacktrace: Some("at line 42".to_string()),
            params: None,
        };

        let json = serde_json::to_string(&msg).unwrap();
        let parsed: VDomMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.messagetype, "error");
    }

    // -- Transfer conversion tests --

    #[test]
    fn test_convert_flat_elems() {
        let elems = vec![
            VDomElem {
                waveid: Some("id-1".to_string()),
                tag: Some("div".to_string()),
                children: None,
                ..Default::default()
            },
            VDomElem {
                waveid: Some("id-2".to_string()),
                tag: Some("span".to_string()),
                children: None,
                ..Default::default()
            },
        ];

        let transfer = convert_elems_to_transfer(&elems);
        assert_eq!(transfer.len(), 2);
        assert_eq!(transfer[0].waveid.as_deref(), Some("id-1"));
        assert_eq!(transfer[1].waveid.as_deref(), Some("id-2"));
    }

    #[test]
    fn test_convert_nested_elems() {
        let elem = VDomElem {
            waveid: Some("parent".to_string()),
            tag: Some("div".to_string()),
            children: Some(vec![
                VDomElem {
                    waveid: Some("child-1".to_string()),
                    tag: Some("span".to_string()),
                    ..Default::default()
                },
                VDomElem {
                    waveid: Some("child-2".to_string()),
                    tag: Some("p".to_string()),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        };

        let transfer = convert_elems_to_transfer(&[elem]);
        assert_eq!(transfer.len(), 3); // parent + 2 children
        // Children first (depth-first), then parent
        let parent = transfer.iter().find(|t| t.waveid.as_deref() == Some("parent")).unwrap();
        assert_eq!(
            parent.children.as_ref().unwrap(),
            &["child-1".to_string(), "child-2".to_string()]
        );
    }

    #[test]
    fn test_dedup_transfer_elems() {
        let elems = vec![
            VDomTransferElem {
                waveid: Some("id-1".to_string()),
                tag: Some("old".to_string()),
                ..Default::default()
            },
            VDomTransferElem {
                waveid: Some("id-2".to_string()),
                tag: Some("span".to_string()),
                ..Default::default()
            },
            VDomTransferElem {
                waveid: Some("id-1".to_string()),
                tag: Some("new".to_string()),
                ..Default::default()
            },
        ];

        let deduped = dedup_transfer_elems(elems);
        assert_eq!(deduped.len(), 2);
        // Last occurrence wins
        let id1 = deduped.iter().find(|t| t.waveid.as_deref() == Some("id-1")).unwrap();
        assert_eq!(id1.tag.as_deref(), Some("new"));
    }

    // -- Split backend update tests --

    #[test]
    fn test_split_small_update() {
        let update = VDomBackendUpdate {
            msg_type: "backendupdate".to_string(),
            ts: 0,
            blockid: "b1".to_string(),
            opts: None,
            haswork: false,
            renderupdates: None,
            transferelems: Some(vec![VDomTransferElem::default(); 10]),
            statesync: None,
            refoperations: None,
            messages: None,
        };

        let chunks = split_backend_update(update);
        assert_eq!(chunks.len(), 1); // Small enough, no split
    }

    #[test]
    fn test_split_large_update() {
        let update = VDomBackendUpdate {
            msg_type: "backendupdate".to_string(),
            ts: 0,
            blockid: "b1".to_string(),
            opts: Some(VDomBackendOpts::default()),
            haswork: false,
            renderupdates: None,
            transferelems: Some(vec![VDomTransferElem::default(); 200]),
            statesync: Some(vec![]),
            refoperations: None,
            messages: None,
        };

        let chunks = split_backend_update(update);
        // 200 items: 50 (initial) + 100 + 50 = 3 chunks
        assert_eq!(chunks.len(), 3);

        // First chunk has opts and statesync
        assert!(chunks[0].opts.is_some());
        assert!(chunks[0].statesync.is_some());
        assert_eq!(chunks[0].transferelems.as_ref().unwrap().len(), 50);
        assert!(chunks[0].haswork); // More to come

        // Middle chunk has only transfer elems
        assert!(chunks[1].opts.is_none());
        assert!(chunks[1].statesync.is_none());
        assert_eq!(chunks[1].transferelems.as_ref().unwrap().len(), 100);
        assert!(chunks[1].haswork);

        // Last chunk
        assert_eq!(chunks[2].transferelems.as_ref().unwrap().len(), 50);
        assert!(!chunks[2].haswork); // No more
    }

    // -- CSS name conversion --

    #[test]
    fn test_to_react_name_simple() {
        assert_eq!(to_react_name("background-color"), "backgroundColor");
        assert_eq!(to_react_name("font-size"), "fontSize");
        assert_eq!(to_react_name("margin"), "margin");
    }

    #[test]
    fn test_to_react_name_vendor_prefix() {
        assert_eq!(to_react_name("-webkit-transform"), "WebkitTransform");
        assert_eq!(to_react_name("-moz-appearance"), "MozAppearance");
    }

    #[test]
    fn test_to_react_name_no_change() {
        assert_eq!(to_react_name("color"), "color");
        assert_eq!(to_react_name("display"), "display");
    }

    // -- State sync serde --

    #[test]
    fn test_state_sync_serde() {
        let sync = VDomStateSync {
            atom: "count".to_string(),
            value: serde_json::json!(42),
        };

        let json = serde_json::to_string(&sync).unwrap();
        let parsed: VDomStateSync = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.atom, "count");
        assert_eq!(parsed.value, serde_json::json!(42));
    }

    #[test]
    fn test_state_sync_complex_value() {
        let sync = VDomStateSync {
            atom: "user".to_string(),
            value: serde_json::json!({"name": "Alice", "age": 30}),
        };

        let json = serde_json::to_string(&sync).unwrap();
        let parsed: VDomStateSync = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.value["name"], "Alice");
    }

    // -- DomRect tests --

    #[test]
    fn test_dom_rect_serde() {
        let rect = DomRect {
            top: 10.0,
            left: 20.0,
            right: 110.0,
            bottom: 60.0,
            width: 90.0,
            height: 50.0,
        };

        let json = serde_json::to_string(&rect).unwrap();
        let parsed: DomRect = serde_json::from_str(&json).unwrap();
        assert!((parsed.width - 90.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_ref_position_serde() {
        let pos = VDomRefPosition {
            offsetheight: 100.0,
            offsetwidth: 200.0,
            scrollheight: 500.0,
            scrollwidth: 200.0,
            scrolltop: 50.0,
            scrollleft: 0.0,
            boundingclientrect: Some(DomRect {
                top: 10.0,
                left: 20.0,
                ..Default::default()
            }),
        };

        let json = serde_json::to_string(&pos).unwrap();
        let parsed: VDomRefPosition = serde_json::from_str(&json).unwrap();
        assert!((parsed.scrolltop - 50.0).abs() < f64::EPSILON);
        assert!(parsed.boundingclientrect.is_some());
    }

    // -- VDomFunc serde --

    #[test]
    fn test_vdom_func_serde() {
        let func = VDomFunc {
            func_type: OBJECT_TYPE_FUNC.to_string(),
            stoppropagation: Some(true),
            preventdefault: None,
            globalevent: None,
            keys: Some(vec!["Enter".to_string(), "Escape".to_string()]),
        };

        let json = serde_json::to_string(&func).unwrap();
        assert!(json.contains("\"type\":\"func\""));
        assert!(json.contains("\"#keys\""));

        let parsed: VDomFunc = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.keys.unwrap().len(), 2);
    }

    // -- VDomRef serde --

    #[test]
    fn test_vdom_ref_serde() {
        let vref = VDomRef {
            ref_type: OBJECT_TYPE_REF.to_string(),
            refid: "ref-123".to_string(),
            trackposition: Some(true),
            position: None,
            hascurrent: Some(false),
        };

        let json = serde_json::to_string(&vref).unwrap();
        assert!(json.contains("\"type\":\"ref\""));
        assert!(json.contains("\"refid\":\"ref-123\""));
    }

    // -- Create context serde --

    #[test]
    fn test_create_context_serde() {
        let ctx = VDomCreateContext {
            msg_type: "createcontext".to_string(),
            ts: 1700000000000,
            meta: None,
            target: Some(VDomTarget {
                newblock: Some(true),
                ..Default::default()
            }),
            persist: false,
        };

        let json = serde_json::to_string(&ctx).unwrap();
        let parsed: VDomCreateContext = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.msg_type, "createcontext");
        assert!(parsed.target.unwrap().newblock.unwrap());
    }

    // -- Render update serde --

    #[test]
    fn test_render_update_types() {
        for update_type in ["root", "append", "replace", "remove", "insert"] {
            let update = VDomRenderUpdate {
                updatetype: update_type.to_string(),
                waveid: Some("id-1".to_string()),
                vdomwaveid: None,
                vdom: None,
                index: None,
            };

            let json = serde_json::to_string(&update).unwrap();
            let parsed: VDomRenderUpdate = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed.updatetype, update_type);
        }
    }
}
