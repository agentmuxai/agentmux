// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! Text a page hides from the person is never handed to an agent as a secret (#47628).
//!
//! A page can mask a secret on screen (dots) and keep the real text in a node it hides with CSS: opacity 0,
//! zero size, moved off the page. The accessibility tree still holds it. A snapshot leaves hidden text out
//! unless `include_hidden` is asked for, and then lists it; a hidden element inside something an agent can
//! act on is listed either way. The rule here covers every read that goes through
//! `visibility::apply`, snapshot in every mode and `find` alike:
//!
//! - hidden text (a text node, or the name, value or description of a hidden element) with a secret's shape
//!   (`agentmux_common::shown_once`, or a known key prefix such as `sk_` or `ghp_`) is replaced by
//!   [`WITHHELD`]; and
//! - hidden text inside or beside a row, control or label named like a secret (secret, key, token,
//!   password, signing, credential, private) is replaced the same way, whatever its shape.
//!
//! The element stays listed, with its role, name and reference, so it can still be targeted: only its text
//! is withheld. A withheld text node gets a reference, as a secret a page shows once does, so `Browser`
//! `capture` can save it into the Vault without the agent reading it. The marker is also what a query
//! matches on, so `find` can't count a secret out a piece at a time.
//!
//! Not covered here: a `UI screenshot` shows what the person sees, and a page that shows a secret on
//! screen shows it in the picture, as it does to the person (password fields are blacked out by their own
//! guard). The pane Log records the agent's own actions, never page text.

use std::collections::{HashMap, HashSet, VecDeque};

use serde_json::Value;

use super::snapshot::str_of;

/// What an agent reads in place of withheld hidden text.
pub const WITHHELD: &str = "[hidden text withheld: it may be a secret; use Browser capture to save it]";

/// Set on a text node whose text was withheld, so its line gets a reference.
const FLAG: &str = "agentmuxWithheld";

/// Whether `node`'s text was withheld.
pub(super) fn is_withheld(node: &Value) -> bool {
    node.get(FLAG).is_some()
}

/// A name that says a row or control holds a secret.
const SECRET_WORDS: &[&str] = &["secret", "key", "token", "password", "passcode", "passphrase", "signing", "credential", "private"];

/// How many ancestors up from the text a row is looked for, and how many nodes of it are read.
const CLIMB: usize = 4;
const ROW_NODES: usize = 80;

/// Prefixes of keys, tokens and signing secrets that are short enough to miss the one-time shape.
const KEY_PREFIXES: &[&str] = &[
    "sk_", "rk_", "pk_", "whsec_", "ghp_", "gho_", "ghs_", "github_pat_", "xoxb-", "xoxp-", "xoxa-", "akia", "asia", "-----begin", "eyj",
];

fn has_key_shape(text: &str) -> bool {
    agentmux_common::shown_once::has_shown_once(text)
        || text.split_whitespace().any(|t| {
            let lower = t.to_ascii_lowercase();
            t.len() >= 12 && KEY_PREFIXES.iter().any(|p| lower.starts_with(p))
        })
}

fn says_secret(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SECRET_WORDS.iter().any(|w| lower.contains(w))
}

fn ignored(node: &Value) -> bool {
    node.get("ignored").and_then(Value::as_bool).unwrap_or(false)
}

fn backend(node: &Value) -> Option<i64> {
    node.get("backendDOMNodeId").and_then(Value::as_i64)
}

/// The strings of a node an agent reads: its name, its value and its description.
const FIELDS: [&str; 3] = ["name", "value", "description"];

/// Whether the row `at` (an ancestor of hidden text) holds a name that says secret: its own, or one of the
/// first nodes below it that is not itself hidden text.
fn row_says_secret(nodes: &[Value], by_id: &HashMap<String, usize>, hidden: &HashSet<i64>, at: usize) -> bool {
    let mut queue = VecDeque::from([at]);
    let mut seen = 0;
    while let Some(i) = queue.pop_front() {
        seen += 1;
        if seen > ROW_NODES {
            break;
        }
        let node = &nodes[i];
        let is_hidden_text = str_of(node, "role") == "StaticText" && backend(node).is_some_and(|b| hidden.contains(&b));
        if !is_hidden_text && !ignored(node) && says_secret(str_of(node, "name")) {
            return true;
        }
        for k in node.get("childIds").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            queue.extend(by_id.get(k));
        }
    }
    false
}

/// Whether hidden text at `i` is inside or beside a row named like a secret.
fn in_secret_row(nodes: &[Value], by_id: &HashMap<String, usize>, hidden: &HashSet<i64>, i: usize) -> bool {
    let mut at = i;
    for _ in 0..CLIMB {
        let Some(&parent) = nodes[at].get("parentId").and_then(Value::as_str).and_then(|p| by_id.get(p)) else { return false };
        at = parent;
        if row_says_secret(nodes, by_id, hidden, at) {
            return true;
        }
    }
    false
}

/// Replace, in `nodes`, the text the page hides (`hidden`: backend ids) that has a secret's shape or sits in
/// a row named like a secret. Returns how many nodes were changed.
pub(super) fn withhold(nodes: &mut [Value], hidden: &HashSet<i64>) -> usize {
    if hidden.is_empty() {
        return 0;
    }
    let by_id: HashMap<String, usize> = nodes.iter().enumerate().filter_map(|(i, n)| Some((n.get("nodeId")?.as_str()?.to_string(), i))).collect();
    let mut chosen: Vec<usize> = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        if ignored(node) || !backend(node).is_some_and(|b| hidden.contains(&b)) {
            continue;
        }
        let is_text = matches!(str_of(node, "role"), "StaticText" | "InlineTextBox");
        let shaped = FIELDS.iter().any(|f| has_key_shape(str_of(node, f)));
        let holds_text = FIELDS.iter().any(|f| !str_of(node, f).trim().is_empty());
        if shaped || (is_text && holds_text && in_secret_row(nodes, &by_id, hidden, i)) {
            chosen.push(i);
        }
    }
    let changed = chosen.len();
    for i in chosen {
        for field in FIELDS {
            if !str_of(&nodes[i], field).trim().is_empty() {
                nodes[i][field]["value"] = Value::String(WITHHELD.to_string());
            }
        }
        if str_of(&nodes[i], "role") == "StaticText" {
            nodes[i][FLAG] = Value::Bool(true);
        }
        // The text pieces under a text node say the same words.
        let kids: Vec<usize> = nodes[i].get("childIds").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).filter_map(|k| by_id.get(k).copied()).collect();
        for k in kids {
            if str_of(&nodes[k], "role") == "InlineTextBox" {
                nodes[k]["name"]["value"] = Value::String(String::new());
            }
        }
    }
    changed
}

#[cfg(test)]
#[path = "hidden_secrets_tests.rs"]
mod tests;
