// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A dropdown the Vault filled: what a snapshot or a find may say of its options
//! (docs/specs/SPEC_VAULT_CARD_ENTRIES_FULL_CARD_DATA_AND_AUTOFILL_2026_11_08.md §6).
//!
//! A card's expiry month or year chosen in a `<select>` leaves the select marked
//! (`data-agentmux-vault`); the select's own line says it holds a secret, but its options
//! are lines of their own, and `[selected]` on one of them would say the choice. So an option
//! of a marked dropdown shows no selection, and the snapshot and find routes look up the
//! facts of the dropdown each option belongs to.

use std::collections::HashMap;

use serde_json::Value;

use super::act::{describe_markup, MAX_DESCRIBES};
use super::cdp::CdpSession;
use super::snapshot::{self, secret_reason, str_of, DomFacts};
use super::text_read::ids_of;

/// The dropdown (a combobox or listbox) an option belongs to.
fn option_owner<'a>(by_id: &HashMap<&str, &'a Value>, node: &Value) -> Option<&'a Value> {
    let mut at = node;
    for _ in 0..6 {
        let parent = at.get("parentId").and_then(Value::as_str).and_then(|id| by_id.get(id)).copied()?;
        if matches!(str_of(parent, "role"), "combobox" | "listbox") {
            return Some(parent);
        }
        at = parent;
    }
    None
}

/// The backend ids of the dropdowns the options among `among` belong to: their facts say whether
/// the Vault filled the choice.
pub(super) fn option_owners(all: &[Value], among: &[&Value]) -> Vec<i64> {
    let by_id = ids_of(all);
    let mut out: Vec<i64> = Vec::new();
    for n in among.iter().filter(|n| str_of(n, "role") == "option") {
        if let Some(b) = option_owner(&by_id, n).and_then(|o| o.get("backendDOMNodeId")?.as_i64()) {
            if !out.contains(&b) {
                out.push(b);
            }
        }
    }
    out
}

/// An option of a dropdown the Vault filled (its `<select>` is marked) shows no `[selected]`: that
/// would tell the agent the choice, which is the card's expiry month or year.
pub(super) fn choice_is_secret(by_id: &HashMap<&str, &Value>, facts: &HashMap<i64, DomFacts>, node: &Value) -> bool {
    str_of(node, "role") == "option"
        && option_owner(by_id, node)
            .and_then(|o| o.get("backendDOMNodeId")?.as_i64())
            .and_then(|b| facts.get(&b))
            .and_then(secret_reason)
            .is_some()
}

/// What the page says about the fields of `scope` (the whole page without one), by backend id: the
/// markup that keeps a secret field's value, and a Vault-filled dropdown's choice, out of a snapshot.
pub(super) async fn describe_scope(cdp: &mut CdpSession, nodes: &[Value], scope: Option<i64>) -> HashMap<i64, DomFacts> {
    let mut facts = HashMap::new();
    let role_of = |n: &Value| n.pointer("/role/value").and_then(|v| v.as_str()).unwrap_or("").to_string();
    // A scoped snapshot renders only its subtree, so it looks up facts only there.
    let in_scope: Vec<&Value> = match scope {
        Some(b) => snapshot::subtree(nodes, b),
        None => nodes.iter().collect(),
    };
    let wanted = in_scope
        .iter()
        .copied()
        .filter(|n| snapshot::VALUE_ROLES.contains(&role_of(n).as_str()))
        .chain(in_scope.iter().copied().filter(|n| role_of(n) == "button"));
    for n in wanted {
        if facts.len() >= MAX_DESCRIBES {
            break;
        }
        if let Some(b) = n.get("backendDOMNodeId").and_then(|v| v.as_i64()) {
            if let Some(f) = describe_markup(cdp, b).await {
                let name = n.pointer("/name/value").and_then(|v| v.as_str()).unwrap_or("");
                facts.insert(b, f.with_label(name));
            }
        }
    }
    // The dropdown an option is in says whether its choice was filled from the Vault.
    add_option_owners(cdp, nodes, &in_scope, &mut facts, MAX_DESCRIBES).await;
    facts
}

/// Adds the facts of the dropdowns that the options among `among` belong to, up to `cap` facts in all.
pub(super) async fn add_option_owners(cdp: &mut CdpSession, nodes: &[Value], among: &[&Value], facts: &mut HashMap<i64, DomFacts>, cap: usize) {
    for b in option_owners(nodes, among) {
        if facts.len() < cap && !facts.contains_key(&b) {
            if let Some(f) = describe_markup(cdp, b).await {
                facts.insert(b, f);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(id: &str, parent: Option<&str>, role: &str, name: &str, backend: i64, kids: &[&str]) -> Value {
        let mut v = json!({
            "nodeId": id,
            "role": { "type": "role", "value": role },
            "name": { "type": "computedString", "value": name },
            "childIds": kids,
            "ignored": false,
            "backendDOMNodeId": backend,
        });
        if let Some(p) = parent {
            v["parentId"] = json!(p);
        }
        v
    }

    fn select_facts(attrs: &[(&str, &str)]) -> HashMap<i64, DomFacts> {
        let attrs = attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        HashMap::from([(30, DomFacts { local_name: "select".into(), attrs, label: String::new() })])
    }

    fn expiry_month_dropdown() -> Vec<Value> {
        let mut march = node("4", Some("3"), "option", "March", 40, &[]);
        march["properties"] = json!([{ "name": "selected", "value": { "type": "boolean", "value": true } }]);
        vec![
            node("1", None, "RootWebArea", "Page", 1, &["3"]),
            node("3", Some("1"), "combobox", "Expiry month", 30, &["4", "5"]),
            march,
            node("5", Some("3"), "option", "April", 50, &[]),
        ]
    }

    #[test]
    fn a_dropdown_the_vault_filled_shows_no_selected_option() {
        let nodes = expiry_month_dropdown();
        // Not the Vault's: the choice shows, as it always has.
        let plain_select = select_facts(&[("name", "exp_month")]);
        assert!(snapshot::render(&nodes, &plain_select, Some(30), "").text.contains("[selected]"));
        // Filled from the Vault (the select is marked): the choice is the card's expiry, so it is not shown.
        let filled = select_facts(&[("name", "exp_month"), ("data-agentmux-vault", "1")]);
        let shown = snapshot::render(&nodes, &filled, Some(30), "").text;
        assert!(!shown.contains("[selected]"), "{shown}");
        assert!(shown.contains("March") && shown.contains("April"), "the options are still there to act on: {shown}");
        // An option found by itself reads the same.
        let by_id = ids_of(&nodes);
        assert!(choice_is_secret(&by_id, &filled, &nodes[2]));
        assert!(!choice_is_secret(&by_id, &plain_select, &nodes[2]));
        assert!(!choice_is_secret(&by_id, &filled, &nodes[1]), "only an option's selection is hidden");
        // The routes fetch the dropdown's facts for every option they list.
        let options: Vec<&Value> = nodes.iter().filter(|x| str_of(x, "role") == "option").collect();
        assert_eq!(option_owners(&nodes, &options), vec![30]);
    }
}
