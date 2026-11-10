// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Elements a page makes clickable with a script, given a reference in a snapshot.
//!
//! A `<div>` with a click handler has no role, so the accessibility tree calls it `generic` and a
//! snapshot gave it no reference: an agent had nothing to pass to `click` (App Privacy's "Set Up ..."
//! boxes on App Store Connect, a `<button role="tablist">`). Blink knows which elements respond to a
//! click, and `DOMSnapshot.captureSnapshot` (already asked for once per snapshot, `visibility.rs`) says
//! so per node in `isClickable`. Those nodes get the role `clickable` and a name from their text, then
//! the snapshot treats them like any other control: a reference, `find` by name, and `click` through
//! the same checks as a button's (a committing click still asks the person).
//!
//! Not every clickable node gets one, or a page would be a wall of references: not the page's own
//! root or body (where frameworks delegate every click), not a node inside a control that has a
//! reference, not one holding a control that has one, not one inside another that got one (the outer
//! one wins, and a click on it lands on whatever is under its middle), and not a container of
//! many elements. At most [`MAX_MARKED`] per snapshot.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

/// The role a marked node is shown with.
pub(super) const CLICKABLE_ROLE: &str = "clickable";

/// A clickable node with more accessibility nodes than this under it is a container, not a control.
const MAX_SUBTREE: usize = 60;

/// At most this many nodes are marked in one snapshot.
const MAX_MARKED: usize = 80;

/// The longest name given to a marked node, in characters.
const MAX_NAME: usize = 80;

/// Nodes that are never marked: where a page delegates every click.
const ROOTS: [&str; 3] = ["#document", "HTML", "BODY"];

/// The backend ids of the nodes Blink says respond to a click, from a `DOMSnapshot.captureSnapshot`
/// result. Only the top document: a frame's content is read separately.
pub(super) fn clickable_backend_ids(result: &Value) -> HashSet<i64> {
    let mut out = HashSet::new();
    let (Some(strings), Some(doc)) = (result.get("strings").and_then(Value::as_array), result.pointer("/documents/0")) else {
        return out;
    };
    let ints = |p: &str| -> Vec<i64> { doc.pointer(p).and_then(Value::as_array).map(|a| a.iter().map(|v| v.as_i64().unwrap_or(-1)).collect()).unwrap_or_default() };
    let (backends, names) = (ints("/nodes/backendNodeId"), ints("/nodes/nodeName"));
    let is_root = |i: usize| {
        names
            .get(i)
            .and_then(|n| usize::try_from(*n).ok())
            .and_then(|n| strings.get(n))
            .and_then(Value::as_str)
            .is_some_and(|s| ROOTS.contains(&s))
    };
    for i in ints("/nodes/isClickable/index") {
        let Ok(i) = usize::try_from(i) else { continue };
        if let Some(&b) = backends.get(i) {
            if b > 0 && !is_root(i) {
                out.insert(b);
            }
        }
    }
    out
}

fn role_of(node: &Value) -> &str {
    node.pointer("/role/value").and_then(Value::as_str).unwrap_or("")
}

fn backend_of(node: &Value) -> Option<i64> {
    node.get("backendDOMNodeId").and_then(Value::as_i64)
}

fn child_ids(node: &Value) -> impl Iterator<Item = &str> {
    node.get("childIds").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str)
}

/// What the page shows inside `root`: the text of its text nodes, one space apart, clipped.
fn text_under(nodes: &[Value], by_id: &HashMap<String, usize>, root: usize) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut stack = vec![root];
    while let Some(i) = stack.pop() {
        let node = &nodes[i];
        if role_of(node) == "StaticText" {
            if let Some(t) = node.pointer("/name/value").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()) {
                words.push(t.to_string());
            }
        }
        // Children are pushed last first, so they come off in page order.
        let kids: Vec<usize> = child_ids(node).filter_map(|k| by_id.get(k).copied()).collect();
        stack.extend(kids.into_iter().rev());
    }
    let joined = words.join(" ");
    if joined.chars().count() <= MAX_NAME {
        return joined;
    }
    let mut cut: String = joined.chars().take(MAX_NAME).collect();
    cut.push('…');
    cut
}

/// Give `clickable` nodes a reference-taking role. Returns how many were marked.
pub(super) fn mark(nodes: &mut [Value], clickable: &HashSet<i64>, hidden: &HashSet<i64>) -> usize {
    if clickable.is_empty() {
        return 0;
    }
    let by_id: HashMap<String, usize> = nodes.iter().enumerate().filter_map(|(i, n)| Some((n.get("nodeId")?.as_str()?.to_string(), i))).collect();
    // Pre-order from the roots, so an ancestor is decided before what is under it; and for each
    // node how many accessibility nodes are under it, and whether any of them is a control.
    let mut order = Vec::with_capacity(nodes.len());
    let mut stack: Vec<usize> = (0..nodes.len()).rev().filter(|&i| nodes[i].get("parentId").is_none()).collect();
    while let Some(i) = stack.pop() {
        order.push(i);
        let kids: Vec<usize> = child_ids(&nodes[i]).filter_map(|k| by_id.get(k).copied()).collect();
        stack.extend(kids.into_iter().rev());
    }
    let mut size = vec![1usize; nodes.len()];
    let mut holds_control = vec![false; nodes.len()];
    for &i in order.iter().rev() {
        let kids: Vec<usize> = child_ids(&nodes[i]).filter_map(|k| by_id.get(k).copied()).collect();
        for c in kids {
            size[i] += size[c];
            holds_control[i] |= holds_control[c] || super::snapshot::is_actionable_role(role_of(&nodes[c]));
        }
    }
    // Whether the node, or something above it, is a control or was marked: nothing below is marked.
    let mut covered = vec![false; nodes.len()];
    let mut marked: Vec<(usize, String)> = Vec::new();
    for &i in &order {
        let node = &nodes[i];
        let own_control = super::snapshot::is_actionable_role(role_of(node));
        covered[i] |= own_control;
        let candidate = !covered[i]
            && marked.len() < MAX_MARKED
            && node.get("ignored").and_then(Value::as_bool) != Some(true)
            && !holds_control[i]
            && size[i] <= MAX_SUBTREE
            && backend_of(node).is_some_and(|b| clickable.contains(&b) && !hidden.contains(&b));
        if candidate {
            covered[i] = true;
            let own = node.pointer("/name/value").and_then(Value::as_str).map(str::trim).unwrap_or("");
            let name = if own.is_empty() { text_under(nodes, &by_id, i) } else { own.to_string() };
            marked.push((i, name));
        }
        if covered[i] {
            for k in child_ids(&nodes[i]).filter_map(|k| by_id.get(k).copied()).collect::<Vec<_>>() {
                covered[k] = true;
            }
        }
    }
    let n = marked.len();
    for (i, name) in marked {
        nodes[i]["role"] = json!({ "type": "role", "value": CLICKABLE_ROLE });
        nodes[i]["name"] = json!({ "type": "computedString", "value": name });
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An accessibility node: `(id, parent, role, name, backend, children)`.
    fn ax(id: &str, parent: Option<&str>, role: &str, name: &str, backend: i64, kids: &[&str]) -> Value {
        let mut v = json!({
            "nodeId": id, "role": { "value": role }, "name": { "value": name },
            "childIds": kids, "ignored": false, "backendDOMNodeId": backend,
        });
        if let Some(p) = parent {
            v["parentId"] = json!(p);
        }
        v
    }

    fn roles(nodes: &[Value]) -> Vec<String> {
        nodes.iter().map(|n| role_of(n).to_string()).collect()
    }

    /// App Store Connect's App Privacy page, cut down: a box with a click handler holding text, a
    /// `<button role=tablist>` with a handler, a real button, and a card whose button already has a reference.
    fn page() -> Vec<Value> {
        vec![
            ax("1", None, "RootWebArea", "", 1, &["2", "5", "8", "10"]),
            ax("2", Some("1"), "generic", "", 2, &["3", "4"]),
            ax("3", Some("2"), "StaticText", "Data Collection", 3, &[]),
            ax("4", Some("2"), "StaticText", "Set Up", 4, &[]),
            ax("5", Some("1"), "tablist", "", 5, &["6"]),
            ax("6", Some("5"), "StaticText", "Ratings", 6, &[]),
            ax("7", Some("1"), "button", "Save", 7, &[]),
            ax("8", Some("1"), "generic", "", 8, &["9", "11"]),
            ax("9", Some("8"), "StaticText", "Card", 9, &[]),
            ax("10", Some("1"), "generic", "", 10, &["12"]),
            ax("11", Some("8"), "button", "Open", 11, &[]),
            ax("12", Some("10"), "StaticText", "Not clickable text", 12, &[]),
        ]
    }

    #[test]
    fn a_div_with_a_click_handler_and_a_tablist_button_get_the_clickable_role_with_their_text() {
        let mut nodes = page();
        let n = mark(&mut nodes, &HashSet::from([2, 5, 7]), &HashSet::new());
        assert_eq!(n, 2);
        assert_eq!(role_of(&nodes[1]), "clickable");
        assert_eq!(nodes[1]["name"]["value"], "Data Collection Set Up");
        assert_eq!(role_of(&nodes[4]), "clickable");
        assert_eq!(nodes[4]["name"]["value"], "Ratings");
        assert_eq!(role_of(&nodes[6]), "button", "a control keeps its own role");
    }

    #[test]
    fn a_card_holding_a_control_and_a_box_the_page_does_not_click_are_left_alone() {
        let mut nodes = page();
        mark(&mut nodes, &HashSet::from([8]), &HashSet::new());
        let r = roles(&nodes);
        assert_eq!(r[7], "generic", "its button has the reference");
        assert_eq!(r[9], "generic", "no handler, no reference");
    }

    #[test]
    fn the_outer_clickable_wins_and_what_is_inside_a_control_is_not_marked() {
        let mut nodes = vec![
            ax("1", None, "RootWebArea", "", 1, &["2", "4"]),
            ax("2", Some("1"), "generic", "", 2, &["3"]),
            ax("3", Some("2"), "generic", "Inner box", 3, &[]),
            ax("4", Some("1"), "link", "Docs", 4, &["5"]),
            ax("5", Some("4"), "generic", "icon", 5, &[]),
        ];
        let n = mark(&mut nodes, &HashSet::from([2, 3, 5]), &HashSet::new());
        assert_eq!(n, 1);
        assert_eq!(roles(&nodes), vec!["RootWebArea", "clickable", "generic", "link", "generic"]);
    }

    #[test]
    fn a_hidden_node_an_ignored_node_and_a_container_of_many_elements_are_not_marked() {
        let mut kids: Vec<Value> = vec![ax("1", None, "RootWebArea", "", 1, &["2", "3", "4"])];
        kids.push(ax("2", Some("1"), "generic", "", 2, &[]));
        let mut ignored = ax("3", Some("1"), "generic", "", 3, &[]);
        ignored["ignored"] = json!(true);
        kids.push(ignored);
        let ids: Vec<String> = (0..MAX_SUBTREE + 2).map(|i| format!("c{i}")).collect();
        let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        kids.push(ax("4", Some("1"), "generic", "", 4, &id_refs));
        for (i, id) in ids.iter().enumerate() {
            kids.push(ax(id, Some("4"), "StaticText", "x", 100 + i as i64, &[]));
        }
        assert_eq!(mark(&mut kids, &HashSet::from([2, 3, 4]), &HashSet::from([2])), 0);
    }

    #[test]
    fn a_name_the_page_gave_is_kept_and_a_long_text_is_clipped() {
        let mut nodes = vec![
            ax("1", None, "RootWebArea", "", 1, &["2", "4"]),
            ax("2", Some("1"), "generic", "Set Up Data Types", 2, &["3"]),
            ax("3", Some("2"), "StaticText", "ignored here", 3, &[]),
            ax("4", Some("1"), "generic", "", 4, &["5"]),
            ax("5", Some("4"), "StaticText", &"word ".repeat(40), 5, &[]),
        ];
        mark(&mut nodes, &HashSet::from([2, 4]), &HashSet::new());
        assert_eq!(nodes[1]["name"]["value"], "Set Up Data Types");
        let long = nodes[3]["name"]["value"].as_str().unwrap();
        assert_eq!(long.chars().count(), MAX_NAME + 1);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn at_most_a_fixed_number_of_nodes_are_marked_in_one_snapshot() {
        let ids: Vec<String> = (0..MAX_MARKED + 20).map(|i| format!("n{i}")).collect();
        let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let mut nodes = vec![ax("1", None, "RootWebArea", "", 1, &id_refs)];
        for (i, id) in ids.iter().enumerate() {
            nodes.push(ax(id, Some("1"), "generic", &format!("Row {i}"), 100 + i as i64, &[]));
        }
        let all: HashSet<i64> = (0..(MAX_MARKED + 20) as i64).map(|i| 100 + i).collect();
        assert_eq!(mark(&mut nodes, &all, &HashSet::new()), MAX_MARKED);
    }

    /// A `DOMSnapshot.captureSnapshot` result: `(node name, backend id, clickable)` per node.
    fn dom(nodes: &[(&str, i64, bool)]) -> Value {
        let mut strings: Vec<String> = Vec::new();
        let mut id = |s: &str| -> usize {
            strings.iter().position(|x| x == s).unwrap_or_else(|| {
                strings.push(s.to_string());
                strings.len() - 1
            })
        };
        let names: Vec<usize> = nodes.iter().map(|n| id(n.0)).collect();
        json!({
            "strings": strings,
            "documents": [{ "nodes": {
                "nodeName": names,
                "backendNodeId": nodes.iter().map(|n| n.1).collect::<Vec<_>>(),
                "isClickable": { "index": nodes.iter().enumerate().filter(|(_, n)| n.2).map(|(i, _)| i).collect::<Vec<_>>() },
            } }],
        })
    }

    #[test]
    fn blinks_clickable_nodes_are_read_without_the_page_root_and_body() {
        let r = dom(&[("#document", 1, true), ("HTML", 2, true), ("BODY", 3, true), ("DIV", 4, true), ("SPAN", 5, false), ("DIV", 6, true)]);
        assert_eq!(clickable_backend_ids(&r), HashSet::from([4, 6]));
    }

    #[test]
    fn a_result_that_cannot_be_read_marks_nothing() {
        assert!(clickable_backend_ids(&json!({})).is_empty());
        assert!(clickable_backend_ids(&json!({ "strings": [], "documents": [{}] })).is_empty());
    }
}
