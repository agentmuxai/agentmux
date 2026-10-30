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

use std::collections::HashSet;

use agentmux_common::shown_once;
use serde_json::Value;

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
