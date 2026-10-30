// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A closed dropdown with many options shows `[N options]` in a snapshot, not
//! one line per option (docs/specs/PLAN_BROWSER_PANE_FORM_FILLING_FIXES_2026_10_17.md
//! §3.5: two state dropdowns printed 59 options each on every snapshot, about
//! 120 lines per step). `select` by label doesn't need the lines: it reads the
//! element's options itself.

use std::collections::HashMap;

use serde_json::Value;

/// A dropdown with this many options or fewer lists them all.
pub(super) const MAX_LISTED_OPTIONS: usize = 12;

/// The number of options of a closed dropdown whose options the snapshot
/// should leave out, or `None` to list them as usual: not a dropdown, an
/// expanded one, the root of a scoped snapshot (the agent asked to read it),
/// or one with few options.
pub(super) fn collapsed_option_count(
    node: &Value,
    role: &str,
    expanded: bool,
    is_scope_root: bool,
    by_id: &HashMap<&str, &Value>,
) -> Option<usize> {
    if role != "combobox" || expanded || is_scope_root {
        return None;
    }
    let count = option_count(node, by_id, 0);
    (count > MAX_LISTED_OPTIONS).then_some(count)
}

fn option_count(node: &Value, by_id: &HashMap<&str, &Value>, depth: usize) -> usize {
    // A dropdown's options are its grandchildren at most; the cap guards a
    // malformed tree.
    if depth > 6 {
        return 0;
    }
    let own = usize::from(node.pointer("/role/value").and_then(|v| v.as_str()) == Some("option"));
    let below: usize = node
        .get("childIds")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|id| id.as_str().and_then(|id| by_id.get(id)))
        .map(|child| option_count(child, by_id, depth + 1))
        .sum();
    own + below
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(id: &str, role: &str, kids: Vec<String>) -> Value {
        json!({ "nodeId": id, "role": { "value": role }, "childIds": kids })
    }

    /// A combobox `c` over a popup `p` holding `n` options.
    fn dropdown(n: usize) -> Vec<Value> {
        let ids: Vec<String> = (0..n).map(|i| format!("o{i}")).collect();
        let mut nodes = vec![node("c", "combobox", vec!["p".to_string()]), node("p", "MenuListPopup", ids.clone())];
        nodes.extend(ids.iter().map(|id| node(id, "option", vec![])));
        nodes
    }

    fn count(nodes: &[Value], expanded: bool, scope_root: bool) -> Option<usize> {
        let by_id: HashMap<&str, &Value> = nodes.iter().map(|n| (n["nodeId"].as_str().unwrap(), n)).collect();
        collapsed_option_count(&nodes[0], "combobox", expanded, scope_root, &by_id)
    }

    #[test]
    fn a_closed_long_dropdown_collapses_to_its_count() {
        assert_eq!(count(&dropdown(59), false, false), Some(59));
        assert_eq!(count(&dropdown(13), false, false), Some(13));
    }

    #[test]
    fn a_short_dropdown_lists_its_options() {
        assert_eq!(count(&dropdown(12), false, false), None);
        assert_eq!(count(&dropdown(3), false, false), None);
    }

    #[test]
    fn an_expanded_dropdown_or_a_scoped_one_lists_its_options() {
        assert_eq!(count(&dropdown(59), true, false), None);
        assert_eq!(count(&dropdown(59), false, true), None);
    }

    #[test]
    fn only_a_combobox_collapses() {
        let nodes = dropdown(59);
        let by_id: HashMap<&str, &Value> = nodes.iter().map(|n| (n["nodeId"].as_str().unwrap(), n)).collect();
        assert_eq!(collapsed_option_count(&nodes[0], "listbox", false, false, &by_id), None);
    }
}
