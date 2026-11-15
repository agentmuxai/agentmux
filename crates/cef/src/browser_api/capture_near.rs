// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Browser` `capture` of text that has no reference of its own, found by the label beside it (#47629).
//!
//! A secret a page reveals with an eye button is only a text node: nothing a snapshot lists holds it, so
//! `capture` had no reference to take it by. The agent names the label instead (`near`: "Client secret"),
//! and the host finds it the way a person does: the label's text in the accessibility tree, then the
//! row or cell around it (up to [`CLIMB`] levels, never a page-wide container) for the one value beside it:
//! a text node, or a read-only field's value, that has a secret's shape or the shape (prefix, named
//! shape, length) the agent gave. Buttons and links in the row are skipped: their text is a name, never the value.
//!
//! Exactly one value must be there. None, or more than one, is an error that says so (without quoting
//! anything the page holds), and the agent adds a prefix or takes a reference from `find`.
//!
//! What is found is given a reference of its own, in the pane's table, so the rest of the capture (the read,
//! the mark that hides it on the page once it is in the Vault) is the same as for any other reference. The
//! text is read from the page's raw tree: hidden text and text a snapshot would withhold are findable, which
//! is the point; the value goes to srv and never to the agent.

use std::collections::HashMap;
use std::sync::Arc;

use agentmux_common::capture_shape::CaptureShape;
use serde_json::{json, Value};

use super::act::page_url;
use super::cdp::CdpSession;
use super::snapshot::str_of;
use crate::state::AppState;

/// How many levels up from the label its row or cell is looked for, and the most nodes one can hold.
const CLIMB: usize = 3;
const ROW_NODES: usize = 60;
/// Containers that are never a row.
/// A table's or list's own container is more than a row: the value beside a label is in the label's row or cell, not
/// in another row of its table.
const NOT_A_ROW: &[&str] =
    &["navigation", "banner", "complementary", "contentinfo", "main", "region", "RootWebArea", "WebArea", "document", "table", "grid", "treegrid", "rowgroup", "list", "tree"];
/// Controls whose text is their name, not a value beside a label.
const CONTROLS: &[&str] = &["button", "link", "menuitem", "tab"];

/// Find the value beside `label` and give it a reference in the pane's table, which is returned.
pub(super) async fn resolve(state: &Arc<AppState>, cdp: &mut CdpSession, block_id: &str, label: &str, shape: Option<&CaptureShape>) -> Result<String, String> {
    let epoch = state.browser_api.ref_tables.epoch(block_id);
    let tree = cdp.call("Accessibility.getFullAXTree", json!({})).await.map_err(|e| format!("CDP Accessibility.getFullAXTree: {e}"))?;
    let nodes: Vec<Value> = tree.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default();
    let backend = find_value(&nodes, label, shape)?;
    let url = page_url(cdp).await;
    let name = format!("{}e1", state.browser_api.ref_tables.next_find_prefix(block_id));
    if !state.browser_api.ref_tables.merge_at(block_id, Some(epoch), &url, &[(name.clone(), backend)]) {
        return Err("the page closed while it was read".into());
    }
    Ok(name)
}

/// A label compared the way a person reads it: case, spacing and a trailing colon don't matter.
fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").trim_end_matches(':').trim().to_lowercase()
}

/// Whether one word of text is a value a capture would take: it has the shape the agent gave when one was
/// given, else the shape of a one-time secret; `broad` also takes a known key's shape and one odd token (what
/// a snapshot withholds), for finding a value beside a label.
pub(super) fn is_value_token(token: &str, shape: Option<&CaptureShape>, broad: bool) -> bool {
    match shape {
        Some(s) => s.matches(token),
        None => {
            agentmux_common::shown_once::looks_shown_once(token)
                || (broad && (agentmux_common::secret_shape::has_key_shape(token) || agentmux_common::secret_shape::token_like(token)))
        }
    }
}

fn kids(node: &Value) -> impl Iterator<Item = &str> {
    node.get("childIds").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str)
}

/// The backend id of the one value beside the text that reads `label`, or what is wrong.
pub(super) fn find_value(nodes: &[Value], label: &str, shape: Option<&CaptureShape>) -> Result<i64, String> {
    let by_id: HashMap<&str, usize> = nodes.iter().enumerate().filter_map(|(i, n)| Some((n.get("nodeId")?.as_str()?, i))).collect();
    let wanted = squash(label);
    let labels: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| !n.get("ignored").and_then(Value::as_bool).unwrap_or(false) && str_of(n, "role") == "StaticText" && squash(str_of(n, "name")) == wanted)
        .map(|(i, _)| i)
        .collect();
    let at = match labels.as_slice() {
        [] => return Err(format!("no text on the page reads {label:?}: use find to see what the row is called")),
        [one] => *one,
        many => return Err(format!("{} places on the page read {label:?}: name one by its ref from find", many.len())),
    };
    let mut up = at;
    for _ in 0..CLIMB {
        let Some(parent) = nodes[up].get("parentId").and_then(Value::as_str).and_then(|p| by_id.get(p)).copied() else { break };
        up = parent;
        if NOT_A_ROW.contains(&str_of(&nodes[up], "role")) {
            break;
        }
        let Some(found) = values_in(nodes, &by_id, up, at, shape) else { break };
        match found.as_slice() {
            [] => {}
            [one] => return Ok(*one),
            many => return Err(format!("{} values sit beside {label:?}: give a prefix or a shape the one you want has, or its ref from find", many.len())),
        }
    }
    Err(format!("no value found beside {label:?}: it may be hidden behind a button (press it first), or named by another label (use find)"))
}

/// The backend ids of the text nodes (and read-only fields) under `row` that hold a value, `label` and the
/// text of controls left out. `None` when the row is too big to be one.
fn values_in(nodes: &[Value], by_id: &HashMap<&str, usize>, row: usize, label: usize, shape: Option<&CaptureShape>) -> Option<Vec<i64>> {
    let mut found = Vec::new();
    let mut seen = 0;
    let mut stack = vec![row];
    while let Some(i) = stack.pop() {
        seen += 1;
        if seen > ROW_NODES {
            return None;
        }
        let node = &nodes[i];
        let role = str_of(node, "role");
        if i == label || CONTROLS.contains(&role) {
            continue;
        }
        // An ignored node says nothing itself, but what is under it may.
        let ignored = node.get("ignored").and_then(Value::as_bool).unwrap_or(false);
        let text = match role {
            _ if ignored => "",
            "StaticText" => str_of(node, "name"),
            "textbox" | "searchbox" => node.pointer("/value/value").and_then(Value::as_str).unwrap_or(""),
            _ => "",
        };
        if text.split_whitespace().any(|t| is_value_token(t, shape, true)) {
            if let Some(b) = node.get("backendDOMNodeId").and_then(Value::as_i64) {
                found.push(b);
            }
            continue;
        }
        stack.extend(kids(node).filter_map(|k| by_id.get(k).copied()));
    }
    Some(found)
}

#[cfg(test)]
#[path = "capture_near_tests.rs"]
mod tests;
