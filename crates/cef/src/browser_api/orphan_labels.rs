// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! A label's text that no field shows (#47619, QTest's Stripe sign-up).
//!
//! The snapshot hides the text of a `<label>` and of a field: it is the field's name or its value, and
//! it is shown on the field's own line. That is only true when a field is named by it. A page that puts
//! a sentence in a `<label for=...>` whose control is hidden, absent or `aria-hidden` (a styled
//! "Did you mean x@y.test? [Yes, update]") has no such line: the sentence went nowhere and the agent saw a
//! bare "Yes, update" button, which could change the account's address.
//!
//! [`prepared`] gives the snapshot and `find` the page's nodes with each such label turned into a plain
//! container, so its text is shown as any other text on the page.
use serde_json::Value;
use std::borrow::Cow;

use super::snapshot::str_of;

/// Roles whose own name is no field's: the text pieces and the labels themselves.
const NOT_A_FIELD: &[&str] = &["StaticText", "InlineTextBox", "LineBreak", "LabelText", "Legend", "RootWebArea", "WebArea"];

/// `nodes` with split secrets joined (see `shown_once::join_split`) and each orphan label opened.
pub fn prepared(nodes: &[Value]) -> Cow<'_, [Value]> {
    let joined = super::shown_once::join_split(nodes);
    match open_orphans(&joined) {
        Cow::Borrowed(_) => joined,
        Cow::Owned(opened) => Cow::Owned(opened),
    }
}

/// Without whitespace, so a label's pieces ("Did you mean ", "x@y.test", "?") compare with a field's name.
fn squeezed(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

fn ignored(node: &Value) -> bool {
    node.get("ignored").and_then(Value::as_bool).unwrap_or(false)
}

/// Controls a label can wrap (`<label>Name <input value=x></label>`): their own text (a value, a caption) is not
/// the label's sentence.
const CONTROLS: &[&str] = &[
    "textbox", "searchbox", "combobox", "checkbox", "radio", "switch", "spinbutton", "slider", "button", "listbox",
    "menuitemcheckbox", "menuitemradio", "ComboBoxSelect", "ComboBoxMenuButton", "PopUpButton", "TextField",
    "SearchBox", "Switch", "ToggleButton", "progressbar", "meter",
];

/// The text a label holds: its shown text pieces, squeezed, apart from what a field inside it holds.
fn label_text(label: &Value, by_id: &std::collections::HashMap<&str, &Value>) -> String {
    let mut text = String::new();
    let mut stack = vec![label];
    while let Some(n) = stack.pop() {
        if !std::ptr::eq(n, label) && CONTROLS.contains(&str_of(n, "role")) {
            continue;
        }
        if str_of(n, "role") == "StaticText" && !ignored(n) {
            text.push_str(&squeezed(str_of(n, "name")));
        }
        if let Some(kids) = n.get("childIds").and_then(Value::as_array) {
            stack.extend(kids.iter().rev().filter_map(|k| k.as_str().and_then(|id| by_id.get(id)).copied()));
        }
    }
    text
}

/// `nodes`, with every `LabelText` and `Legend` that no field's name holds turned into a `generic`
/// container. Borrowed when there is none.
pub fn open_orphans(nodes: &[Value]) -> Cow<'_, [Value]> {
    let labels: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| matches!(str_of(n, "role"), "LabelText" | "Legend") && !ignored(n))
        .map(|(i, _)| i)
        .collect();
    if labels.is_empty() {
        return Cow::Borrowed(nodes);
    }
    let by_id: std::collections::HashMap<&str, &Value> =
        nodes.iter().filter_map(|n| Some((n.get("nodeId")?.as_str()?, n))).collect();
    let names: Vec<String> = nodes
        .iter()
        .filter(|n| !ignored(n) && !NOT_A_FIELD.contains(&str_of(n, "role")))
        .map(|n| squeezed(str_of(n, "name")))
        .filter(|s| !s.is_empty())
        .collect();
    let exact: std::collections::HashSet<&str> = names.iter().map(String::as_str).collect();
    let orphans: Vec<usize> = labels
        .into_iter()
        .filter(|&i| {
            let text = label_text(&nodes[i], &by_id);
            !text.is_empty() && !exact.contains(text.as_str()) && !names.iter().any(|n| n.contains(&text))
        })
        .collect();
    if orphans.is_empty() {
        return Cow::Borrowed(nodes);
    }
    let mut opened = nodes.to_vec();
    for i in orphans {
        opened[i]["role"]["value"] = Value::String("generic".into());
    }
    Cow::Owned(opened)
}

#[cfg(test)]
#[path = "orphan_labels_tests.rs"]
mod tests;
