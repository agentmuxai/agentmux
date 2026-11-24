// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Approval block: the run stops and a person says yes or no
//! (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §4.2 and §6.4).
//!
//! `{ "kind": "approval", "title": "Merge the pull request", "show": [{ "label": "Pull request", "value": "{{pr.title}}" }] }`.
//! The card shows the title and each item, and marks the items that came from outside text. The title is the graph's
//! own words: it may not read outside text, so a crafted pull request cannot dress itself up as the proposed action.
//! `timeout_seconds` says how long the person has (a day when it is left out); silence rejects.
//!
//! The run waits as a Wait block does (`scheduler/wait.rs`), on a key that stays inside AgentMux. Only the person's own
//! client can answer (`server::drone_approval_handlers`). A yes ends the block with `approved: true` and who answered;
//! a no, and silence, fail it as `approval_rejected` and `approval_timeout`, so an error edge takes them.

use serde_json::{json, Value};

use crate::backend::drone_approval::ApprovalItem;
use crate::drone::data_flow::ExecutionScope;
use crate::drone::types::FlowNode;

/// How long the person has when the block does not say: a day.
pub const DEFAULT_TIMEOUT_S: u64 = 24 * 3600;
/// The most items a card shows, and the longest text of one.
const MAX_ITEMS: usize = 20;
const MAX_TEXT: usize = 2000;

/// What the card shows.
#[derive(Debug)]
pub struct Card {
    pub title: String,
    pub items: Vec<ApprovalItem>,
}

fn clip(text: String) -> String {
    if text.chars().count() <= MAX_TEXT {
        return text;
    }
    let mut cut: String = text.chars().take(MAX_TEXT).collect();
    cut.push('…');
    cut
}

/// The items `show` names, each filled in with what the run holds now.
fn items_of(node: &FlowNode, scope: &ExecutionScope) -> Result<Vec<ApprovalItem>, String> {
    let pairs: Vec<(String, String)> = match node.data.get("show") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(list)) => list
            .iter()
            .map(|e| {
                let label = e.get("label").and_then(Value::as_str).unwrap_or_default().to_string();
                let value = e.get("value").and_then(Value::as_str).ok_or("approval block: each `show` item is { label, value }")?;
                Ok((label, value.to_string()))
            })
            .collect::<Result<_, String>>()?,
        Some(Value::Object(map)) => map.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect(),
        Some(_) => return Err("approval block: `show` is a list of { label, value }".to_string()),
    };
    // A line the person has not filled in yet is not an item.
    let pairs: Vec<(String, String)> = pairs.into_iter().filter(|(l, v)| !(l.trim().is_empty() && v.trim().is_empty())).collect();
    if pairs.len() > MAX_ITEMS {
        return Err(format!("approval block: `show` has at most {MAX_ITEMS} items"));
    }
    Ok(pairs
        .into_iter()
        .map(|(label, template)| {
            let (tainted, derived) = scope.bits_of(&template);
            ApprovalItem { label, text: clip(scope.resolve(&template)), tainted: tainted || derived }
        })
        .collect())
}

/// The card for `node`, read from the run as it is now.
pub fn card(node: &FlowNode, scope: &ExecutionScope) -> Result<Card, String> {
    let raw = node.data.get("title").and_then(Value::as_str).unwrap_or_default();
    if raw.trim().is_empty() {
        return Err("approval block: `title` says what is proposed, and is empty".to_string());
    }
    let (tainted, derived) = scope.bits_of(raw);
    if tainted || derived {
        return Err("approval block: `title` may not read outside text: put it in `show`, where it is marked".to_string());
    }
    Ok(Card { title: clip(scope.resolve(raw)), items: items_of(node, scope)? })
}

/// What the block gives when the person answered: a yes is its output, a no fails it.
pub fn decide(answer: &Value) -> Result<Value, String> {
    let by = answer.get("by").and_then(Value::as_str).unwrap_or("the person");
    match answer.get("approve").and_then(Value::as_bool) {
        Some(true) => Ok(json!({ "approved": true, "by": by })),
        _ => Err(format!("approval_rejected: {by} said no")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(data: Value) -> FlowNode {
        serde_json::from_value(json!({ "id": "ap", "position": { "x": 0, "y": 0 }, "data": data })).unwrap()
    }

    #[test]
    fn the_card_marks_the_items_that_came_from_outside_text() {
        let mut scope = ExecutionScope::for_drone("d1", json!({ "title": "a pull request title" }));
        scope.outputs.insert("agent".into(), json!({ "output": "looks fine" }));
        scope.bits.insert("agent".into(), (true, false));
        let n = node(json!({
            "kind": "approval", "title": "Merge the pull request",
            "show": [{ "label": "Agent says", "value": "{{agent.output}}" }, { "label": "Fixed", "value": "the base branch" }],
        }));
        let card = card(&n, &scope).unwrap();
        assert_eq!(card.title, "Merge the pull request");
        assert_eq!(card.items.len(), 2);
        assert_eq!((card.items[0].label.as_str(), card.items[0].text.as_str(), card.items[0].tainted), ("Agent says", "looks fine", true));
        assert!(!card.items[1].tainted, "the graph's own text is clean");
    }

    #[test]
    fn a_title_that_reads_outside_text_or_says_nothing_is_refused() {
        let mut scope = ExecutionScope::for_drone("d1", json!({ "title": "x" }));
        scope.outputs.insert("agent".into(), json!({ "output": "ok" }));
        scope.bits.insert("agent".into(), (true, false));
        let spoof = card(&node(json!({ "title": "Approve {{agent.output}}" })), &scope).unwrap_err();
        assert!(spoof.contains("may not read outside text"), "{spoof}");
        assert!(card(&node(json!({ "title": "  " })), &scope).unwrap_err().contains("is empty"));
        let many: Vec<Value> = (0..=MAX_ITEMS).map(|i| json!({ "label": i.to_string(), "value": "x" })).collect();
        assert!(card(&node(json!({ "title": "t", "show": many })), &scope).unwrap_err().contains("at most"));
    }

    #[test]
    fn a_long_text_is_cut_and_an_answer_is_a_yes_or_a_rejection_that_says_who() {
        let scope = ExecutionScope::for_drone("d1", Value::Null);
        let long = "x".repeat(MAX_TEXT + 50);
        let c = card(&node(json!({ "title": "t", "show": { "Body": long } })), &scope).unwrap();
        assert_eq!(c.items[0].text.chars().count(), MAX_TEXT + 1);
        assert_eq!(decide(&json!({ "approve": true, "by": "operator" })).unwrap(), json!({ "approved": true, "by": "operator" }));
        assert_eq!(decide(&json!({ "approve": false, "by": "operator" })).unwrap_err(), "approval_rejected: operator said no");
        assert!(decide(&json!({})).unwrap_err().starts_with("approval_rejected"), "no clear yes is a no");
    }
}
