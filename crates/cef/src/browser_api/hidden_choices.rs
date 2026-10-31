// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Inputs the page hides, listed in a snapshot anyway: checkboxes and radios
//! behind a label drawn as the box, and file inputs behind an upload button
//! (shown as `file input (hidden)`). The accessibility tree leaves such an input
//! out, so it would get no reference.

use serde_json::{json, Value};

use super::act::{backend_of, call, isolated_context};
use super::cdp::CdpSession;

/// The property a hidden file input's node carries, so its line can say so.
pub(super) const HIDDEN_FILE_PROP: &str = "agentmuxHiddenFile";

/// A checkbox or radio input the page hides (`display: none`) behind a
/// `<label>` drawn as the box, a common styling pattern. The accessibility
/// tree leaves such an input out, so it would get no reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenChoice {
    pub input_backend: i64,
    pub label_backend: i64,
    /// A hidden `<input type=file>`, not a checkbox or radio: it is listed as a
    /// `file input (hidden)` that `set_files` fills.
    pub file: bool,
    pub radio: bool,
    /// The label's text, or the input's own aria-label.
    pub name: String,
    pub checked: bool,
    pub disabled: bool,
}

/// Add each hidden choice to `nodes` as a checkbox or radio node, named by its
/// label, carrying the input's backend id, first inside its label's node (or
/// last in the root when the label isn't in the tree). It then gets a line and
/// a reference like any checkbox; `check` and `click` reach the input through
/// its label (`act.rs`'s `CLICK_POINT`).
pub fn add_hidden_choices(nodes: &mut Vec<Value>, choices: &[HiddenChoice]) {
    for (i, c) in choices.iter().enumerate() {
        let id = format!("agentmux-hidden-choice-{i}");
        let holder = nodes
            .iter()
            .position(|n| n.get("backendDOMNodeId").and_then(|v| v.as_i64()) == Some(c.label_backend));
        let (holder, first) = match holder {
            Some(h) => (h, true),
            None => match nodes.iter().position(|n| n.get("parentId").is_none()) {
                Some(root) => (root, false),
                None => continue,
            },
        };
        let parent = nodes[holder].get("nodeId").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let kids = nodes[holder]
            .as_object_mut()
            .map(|o| o.entry("childIds").or_insert_with(|| json!([])));
        if let Some(Value::Array(kids)) = kids {
            if first {
                kids.insert(0, json!(id));
            } else {
                kids.push(json!(id));
            }
        }
        let mut properties = if c.file {
            vec![json!({ "name": HIDDEN_FILE_PROP, "value": { "type": "boolean", "value": true } })]
        } else {
            vec![json!({ "name": "checked", "value": { "type": "tristate", "value": if c.checked { "true" } else { "false" } } })]
        };
        if c.disabled {
            properties.push(json!({ "name": "disabled", "value": { "type": "boolean", "value": true } }));
        }
        nodes.push(json!({
            "nodeId": id,
            "parentId": parent,
            "ignored": false,
            "role": { "type": "role", "value": if c.file { "button" } else if c.radio { "radio" } else { "checkbox" } },
            "name": { "type": "computedString", "value": c.name },
            "properties": properties,
            "backendDOMNodeId": c.input_backend,
            "childIds": [],
        }));
    }
}

/// The most hidden checkboxes and radios one frame's snapshot adds.
const MAX_HIDDEN_CHOICES: usize = 200;

/// Checkbox and radio inputs with no box of their own whose label has one,
/// each followed by that label: the inputs a page hides behind a label drawn
/// as the box.
const HIDDEN_CHOICES: &str = r#"(() => {
  const out = [];
  for (const el of document.querySelectorAll("input[type=checkbox], input[type=radio]")) {
    if (out.length >= 2 * MAX) break;
    if (el.getClientRects().length > 0) continue;
    const label = Array.from(el.labels || []).find(l => l.getClientRects().length > 0);
    if (label) out.push(el, label);
  }
  // Hidden file inputs too: each goes with its visible label, else the nearest
  // visible element around it (the drop zone), else the page.
  for (const el of document.querySelectorAll("input[type=file]")) {
    if (out.length >= 2 * MAX) break;
    if (el.getClientRects().length > 0) continue;
    let holder = Array.from(el.labels || []).find(l => l.getClientRects().length > 0);
    for (let n = el.parentElement; !holder && n && n !== document.documentElement; n = n.parentElement) {
      if (n.getClientRects().length > 0) holder = n;
    }
    out.push(el, holder || document.body);
  }
  return out;
})()"#;

/// A hidden choice's state and name, read from the input with its label as
/// the argument. The label's own text, without that of controls nested in it.
const CHOICE_FACTS: &str = r#"function (label) {
  const file = String(this.type).toLowerCase() === "file";
  const c = label.cloneNode(true);
  c.querySelectorAll("select, input, textarea, button, option").forEach(n => n.remove());
  // A file input's holder may be a whole drop zone: name it by its own label.
  const own = Array.from(this.labels || []).map(l => l.textContent).join(" ");
  const text = file ? (own || "Choose file") : c.textContent;
  const name = (this.getAttribute("aria-label") || text).replace(/\s+/g, " ").trim().slice(0, 200);
  return { file, radio: String(this.type).toLowerCase() === "radio", checked: !!this.checked, disabled: !!this.disabled, name };
}"#;

/// The hidden checkboxes and radios in `frame_id` (or the main frame), for
/// [`add_hidden_choices`]. Best effort: a failed lookup adds none,
/// and the page reads as it did before.
pub(super) async fn hidden_choices(cdp: &mut CdpSession, frame_id: Option<&str>) -> Vec<HiddenChoice> {
    const GROUP: &str = "agentmux-hidden-choices";
    let mut out = Vec::new();
    let Ok(ctx) = isolated_context(cdp, frame_id).await else { return out };
    let expression = HIDDEN_CHOICES.replace("2 * MAX", &(2 * MAX_HIDDEN_CHOICES).to_string());
    let listed = call(
        cdp,
        "Runtime.evaluate",
        json!({ "expression": expression, "contextId": ctx, "objectGroup": GROUP, "returnByValue": false }),
    )
    .await;
    let Some(list) = listed.ok().and_then(|v| v.pointer("/result/objectId").and_then(|o| o.as_str()).map(str::to_string)) else {
        return out;
    };
    let props = call(cdp, "Runtime.getProperties", json!({ "objectId": list, "ownProperties": true })).await;
    let mut elements: Vec<(usize, String)> = props
        .ok()
        .and_then(|p| p.get("result").and_then(|r| r.as_array()).cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|p| {
            let index = p.get("name")?.as_str()?.parse::<usize>().ok()?;
            Some((index, p.pointer("/value/objectId")?.as_str()?.to_string()))
        })
        .collect();
    elements.sort_by_key(|(i, _)| *i);
    for pair in elements.chunks_exact(2) {
        let (input, label) = (&pair[0].1, &pair[1].1);
        let (Some(input_backend), Some(label_backend)) = (backend_of(cdp, input).await, backend_of(cdp, label).await) else {
            continue;
        };
        let facts = call(
            cdp,
            "Runtime.callFunctionOn",
            json!({ "objectId": input, "functionDeclaration": CHOICE_FACTS, "arguments": [{ "objectId": label }], "returnByValue": true }),
        )
        .await;
        let Some(f) = facts.ok().and_then(|v| v.pointer("/result/value").cloned()) else { continue };
        out.push(HiddenChoice {
            input_backend,
            label_backend,
            file: f.get("file").and_then(Value::as_bool).unwrap_or(false),
            radio: f.get("radio").and_then(Value::as_bool).unwrap_or(false),
            name: f.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
            checked: f.get("checked").and_then(Value::as_bool).unwrap_or(false),
            disabled: f.get("disabled").and_then(Value::as_bool).unwrap_or(false),
        });
    }
    let _ = cdp.call("Runtime.releaseObjectGroup", json!({ "objectGroup": GROUP })).await;
    out
}
