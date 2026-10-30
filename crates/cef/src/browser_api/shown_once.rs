// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where a page shows a secret once, in its accessibility tree
//! (docs/specs/SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §15.1).
//!
//! A one-time secret's shape ([`agentmux_common::shown_once`]) is masked only where
//! a page shows one: in a `code` or `pre` element, in a read-only field, or in the
//! row or container of a button named like "Copy". A snapshot or `find` line that
//! masks one gets a reference, so the agent can save it with `Browser` `capture`.
//! (`kbd` and `samp` have no role of their own in the tree; the screenshot scan,
//! which sees tag names, covers them.)

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use agentmux_common::shown_once;
use serde_json::{json, Value};

/// Roles whose text is shown as code.
const CODE_ROLES: &[&str] = &["code", "pre"];
/// How far up from a Copy button its container is looked for.
const MAX_CLIMB: usize = 4;

fn role(node: &Value) -> &str {
    node.get("role").and_then(|v| v.get("value")).and_then(Value::as_str).unwrap_or("")
}

fn name(node: &Value) -> &str {
    node.get("name").and_then(|v| v.get("value")).and_then(Value::as_str).unwrap_or("").trim()
}

fn is_copy_button(node: &Value) -> bool {
    matches!(role(node), "button" | "link") && name(node).to_lowercase().contains("copy")
}

/// The nodes (by id) whose subtree is the row or container of a Copy button: the
/// button's nearest ancestor that holds more than the button itself.
pub fn copy_zones(nodes: &[Value]) -> HashSet<&str> {
    let by_id: std::collections::HashMap<&str, &Value> = nodes.iter().filter_map(|n| Some((n.get("nodeId")?.as_str()?, n))).collect();
    let mut zones = HashSet::new();
    for button in nodes.iter().filter(|n| is_copy_button(n)) {
        let mut at = button;
        for _ in 0..MAX_CLIMB {
            let Some(parent) = at.get("parentId").and_then(Value::as_str).and_then(|p| by_id.get(p)) else { break };
            at = parent;
            if at.get("childIds").and_then(Value::as_array).is_some_and(|k| k.len() > 1) {
                break;
            }
        }
        if let Some(id) = at.get("nodeId").and_then(Value::as_str) {
            if !std::ptr::eq(at, button) {
                zones.insert(id);
            }
        }
    }
    zones
}

/// Whether `node`'s subtree is somewhere a page shows a secret once (given that
/// its ancestors' isn't).
pub fn starts_zone(node: &Value, zones: &HashSet<&str>) -> bool {
    CODE_ROLES.contains(&role(node)) || node.get("nodeId").and_then(Value::as_str).is_some_and(|id| zones.contains(id))
}

/// Whether one of `node`'s ancestors starts a zone: a scoped snapshot that begins
/// inside one is still in it.
pub fn under_zone(node: &Value, by_id: &std::collections::HashMap<&str, &Value>, zones: &HashSet<&str>) -> bool {
    let mut at = node;
    for _ in 0..256 {
        let Some(parent) = at.get("parentId").and_then(Value::as_str).and_then(|p| by_id.get(p)) else { return false };
        if starts_zone(parent, zones) {
            return true;
        }
        at = parent;
    }
    false
}

/// `nodes`, with each secret a zone splits over several text nodes joined back
/// together. A syntax-highlighted `<code>` gives each token a span of its own, so
/// no piece alone has a secret's shape and each would be shown. The zone's text is
/// joined into its first piece, which takes the zone element's backend id (so
/// `capture` of its reference reads the whole element), and the other pieces are
/// emptied (an empty text node shows no line). Borrowed when nothing is split.
pub fn join_split(nodes: &[Value]) -> Cow<'_, [Value]> {
    let by_id: HashMap<&str, &Value> = nodes.iter().filter_map(|n| Some((n.get("nodeId")?.as_str()?, n))).collect();
    let zones = copy_zones(nodes);
    // The first piece's id → (joined text, the zone's backend id); others → "".
    let mut joined: HashMap<String, (String, Option<i64>)> = HashMap::new();
    let mut emptied: HashSet<String> = HashSet::new();
    for root in nodes.iter().filter(|n| starts_zone(n, &zones) && !under_zone(n, &by_id, &zones)) {
        let pieces = text_pieces(root, &by_id);
        if pieces.len() < 2 {
            continue;
        }
        let whole: String = pieces.iter().map(|(_, t)| *t).collect();
        let apart: String = pieces.iter().map(|(_, t)| shown_once::mask(t)).collect();
        if !shown_once::has_shown_once(&whole) || apart == shown_once::mask(&whole) {
            continue;
        }
        joined.insert(pieces[0].0.to_string(), (whole, root.get("backendDOMNodeId").and_then(Value::as_i64)));
        emptied.extend(pieces[1..].iter().map(|(id, _)| id.to_string()));
    }
    if joined.is_empty() {
        return Cow::Borrowed(nodes);
    }
    let mut out = nodes.to_vec();
    for n in &mut out {
        let Some(id) = n.get("nodeId").and_then(Value::as_str).map(str::to_string) else { continue };
        if let Some((text, backend)) = joined.get(&id) {
            n["name"] = json!({ "type": "computedString", "value": text });
            if let Some(b) = backend {
                n["backendDOMNodeId"] = json!(b);
            }
        } else if emptied.contains(&id) {
            n["name"] = json!({ "type": "computedString", "value": "" });
        }
    }
    Cow::Owned(out)
}

/// The text nodes under `root`, in document order, with their raw text (spaces
/// kept: they separate words). A text node's own children repeat its text.
fn text_pieces<'a>(root: &'a Value, by_id: &HashMap<&str, &'a Value>) -> Vec<(&'a str, &'a str)> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if role(node) == "StaticText" {
            let text = node.get("name").and_then(|v| v.get("value")).and_then(Value::as_str).unwrap_or("");
            if let Some(id) = node.get("nodeId").and_then(Value::as_str) {
                out.push((id, text));
            }
            continue;
        }
        if let Some(kids) = node.get("childIds").and_then(Value::as_array) {
            stack.extend(kids.iter().rev().filter_map(|k| k.as_str().and_then(|id| by_id.get(id)).copied()));
        }
    }
    out
}

/// Whether a line masks a one-time secret: in its name, inside a zone, or in its
/// value, inside a zone or in a read-only field.
pub fn masks(name: &str, value: Option<&str>, in_zone: bool, readonly: bool) -> bool {
    (in_zone && shown_once::has_shown_once(name)) || ((in_zone || readonly) && value.is_some_and(shown_once::has_shown_once))
}

/// `text` with a one-time secret masked, where one is shown.
pub fn mask_where(text: &str, shown: bool) -> std::borrow::Cow<'_, str> {
    if shown {
        std::borrow::Cow::Owned(shown_once::mask(text))
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

#[cfg(test)]
#[path = "shown_once_tests.rs"]
mod tests;
