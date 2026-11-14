// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! Text a page hides from the person is never handed to an agent as a secret (#47628).
//!
//! A page can mask a secret on screen (dots) and keep the real text in a node it hides with CSS: opacity 0,
//! zero size, moved off the page. The accessibility tree still holds it. A snapshot leaves hidden text out
//! unless `include_hidden` is asked for, and then lists it; a hidden element inside something an agent can
//! act on is listed either way. The rule here covers every read that goes through `visibility::apply`
//! (snapshot in every mode, `find`) and the frames a snapshot reads (`frames.rs`):
//!
//! - hidden text (a text node, or the name, value or description of a hidden element) with a secret's shape
//!   (`agentmux_common::shown_once`, or a known key prefix such as `sk_` or `ghp_`) is replaced by
//!   [`WITHHELD`]; and
//! - hidden text inside or beside a small row, control or label named like a secret (whole words: secret,
//!   key, token, password, passcode, signing, credential, and compounds such as apikey), is replaced the
//!   same way, whatever its shape; and
//! - the same text is taken out of every other name, value and description that repeats it: a visible cell
//!   or button is named from its contents, and its name would carry the text its child no longer shows.
//!
//! The element stays listed, with its role, name and reference, so it can still be targeted: only its text
//! is withheld. A withheld text node gets a reference, as a secret a page shows once does, so `Browser`
//! `capture` can save it into the Vault without the agent reading it. The marker is also what a query
//! matches on, so `find` can't count a secret out a piece at a time.
//!
//! When the page's layout can't be read, nothing is known to be hidden, so every text node counts as
//! hidden: a secret's shape is withheld wherever it is, rather than shown for want of an answer.
//!
//! [`scrub`] is the shape rule alone, for the strings read from a page's own script (the state an action
//! reports, a name read for one element): there is no tree to climb, and a secret shape is withheld
//! wherever it sits.
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

/// Whole words that name a secret (a plural counts).
const SECRET_NOUNS: &[&str] = &["secret", "key", "token", "password", "passcode", "passphrase", "signing", "credential"];
/// Words that join one of [`SECRET_JOINED`] into a single word (`apikey`, `passkey`, `accesstoken`).
const SECRET_PREFIXES: &[&str] = &[
    "api", "pass", "secret", "access", "private", "signing", "auth", "bearer", "client", "master", "license", "session", "refresh", "webhook",
    "encryption", "ssh", "pgp", "gpg",
];
const SECRET_JOINED: &[&str] = &["key", "token", "secret", "password"];

/// How many ancestors up from the text a row is looked for, and the most nodes a row can hold: a
/// container with more (a sidebar, a page's whole main area) is not a row.
const CLIMB: usize = 3;
const ROW_NODES: usize = 40;
/// Containers that are never a row.
const NOT_A_ROW: &[&str] = &["navigation", "banner", "complementary", "contentinfo", "main", "region", "RootWebArea", "WebArea", "document"];

/// The shortest text taken out of other nodes' names: from a node that sits below it (an ancestor named
/// from its contents), and from any node at all.
const ECHO_BELOW: usize = 3;
const ECHO_ANYWHERE: usize = 6;

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

/// The words of a name, lowercased, split on anything that is not a letter or digit and on a lower-to-upper
/// step (`apiKey` is `api`, `key`).
fn words(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut prev_lower = false;
    for c in name.chars() {
        if !c.is_alphanumeric() {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower && !word.is_empty() {
            out.push(std::mem::take(&mut word));
        }
        prev_lower = c.is_lowercase();
        word.extend(c.to_lowercase());
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

fn is_secret_word(w: &str) -> bool {
    let base = w.strip_suffix('s').unwrap_or(w);
    SECRET_NOUNS.contains(&base) || SECRET_PREFIXES.iter().any(|p| base.strip_prefix(p).is_some_and(|rest| SECRET_JOINED.contains(&rest)))
}

/// Whether a name says its row or control holds a secret.
fn says_secret(name: &str) -> bool {
    words(name).iter().any(|w| is_secret_word(w))
}

fn ignored(node: &Value) -> bool {
    node.get("ignored").and_then(Value::as_bool).unwrap_or(false)
}

fn backend(node: &Value) -> Option<i64> {
    node.get("backendDOMNodeId").and_then(Value::as_i64)
}

fn is_text(node: &Value) -> bool {
    matches!(str_of(node, "role"), "StaticText" | "InlineTextBox")
}

/// The strings of a node an agent reads: its name, its value and its description.
const FIELDS: [&str; 3] = ["name", "value", "description"];

fn kids<'a>(node: &'a Value) -> impl Iterator<Item = &'a str> {
    node.get("childIds").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str)
}

/// What `withhold` knows about the tree it works on.
struct Tree<'a> {
    nodes: &'a [Value],
    by_id: HashMap<String, usize>,
    /// What the page hides (empty when nothing is known to be hidden, or when `unknown`).
    hidden: &'a HashSet<i64>,
    /// The layout could not be read: every text counts as hidden.
    unknown: bool,
}

impl Tree<'_> {
    fn is_hidden(&self, node: &Value) -> bool {
        self.unknown || backend(node).is_some_and(|b| self.hidden.contains(&b))
    }

    fn parent(&self, i: usize) -> Option<usize> {
        self.nodes[i].get("parentId").and_then(Value::as_str).and_then(|p| self.by_id.get(p)).copied()
    }

    /// The nodes of the subtree at `at`, at most `cap + 1` of them.
    fn subtree(&self, at: usize, cap: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut queue = VecDeque::from([at]);
        while let Some(i) = queue.pop_front() {
            out.push(i);
            if out.len() > cap {
                break;
            }
            queue.extend(kids(&self.nodes[i]).filter_map(|k| self.by_id.get(k)));
        }
        out
    }

    /// Whether the row at `at` holds a name that says secret, other than the text at `skip` and other hidden
    /// text.
    fn row_says_secret(&self, at: usize, skip: usize) -> bool {
        let inside = self.subtree(at, ROW_NODES);
        if inside.len() > ROW_NODES {
            return false;
        }
        inside.into_iter().any(|i| {
            let node = &self.nodes[i];
            i != skip && !ignored(node) && !(is_text(node) && self.is_hidden(node)) && says_secret(str_of(node, "name"))
        })
    }

    /// Whether the text at `i` is inside or beside a small row named like a secret.
    fn in_secret_row(&self, i: usize) -> bool {
        let mut at = i;
        for _ in 0..CLIMB {
            let Some(parent) = self.parent(at) else { return false };
            at = parent;
            if NOT_A_ROW.contains(&str_of(&self.nodes[at], "role")) || self.subtree(at, ROW_NODES).len() > ROW_NODES {
                return false;
            }
            if self.row_says_secret(at, i) {
                return true;
            }
        }
        false
    }
}

fn squeezed(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Replace, in `nodes`, the text the page hides (`hidden`: backend ids; every text node when `unknown`) that
/// has a secret's shape or sits in a row named like a secret, and what repeats it in another node's name,
/// value or description. Returns how many text nodes were withheld.
pub(super) fn withhold(nodes: &mut [Value], hidden: &HashSet<i64>, unknown: bool) -> usize {
    if hidden.is_empty() && !unknown {
        return 0;
    }
    let by_id: HashMap<String, usize> = nodes.iter().enumerate().filter_map(|(i, n)| Some((n.get("nodeId")?.as_str()?.to_string(), i))).collect();
    let (chosen, pieces_of) = {
        let tree = Tree { nodes, by_id: by_id.clone(), hidden, unknown };
        let mut chosen: Vec<usize> = Vec::new();
        for (i, node) in nodes.iter().enumerate() {
            if ignored(node) || !tree.is_hidden(node) {
                continue;
            }
            let shaped = FIELDS.iter().any(|f| has_key_shape(str_of(node, f)));
            let holds_text = FIELDS.iter().any(|f| !str_of(node, f).trim().is_empty());
            if shaped || (is_text(node) && holds_text && tree.in_secret_row(i)) {
                chosen.push(i);
            }
        }
        // What each withheld node said, and which nodes sit above it.
        let mut pieces_of: HashMap<usize, (Vec<String>, Vec<usize>)> = HashMap::new();
        for &i in &chosen {
            let said: Vec<String> = FIELDS.iter().map(|f| str_of(&nodes[i], f).trim().to_string()).filter(|s| !s.is_empty()).collect();
            let mut up = Vec::new();
            let mut at = i;
            while let Some(p) = tree.parent(at) {
                up.push(p);
                at = p;
            }
            pieces_of.insert(i, (said, up));
        }
        (chosen, pieces_of)
    };
    let changed = chosen.len();
    // Nodes above a withheld text: its pieces are looked for in them at any length, in a squeezed form too.
    let mut above: HashMap<usize, Vec<String>> = HashMap::new();
    let mut anywhere: Vec<String> = Vec::new();
    for (pieces, up) in pieces_of.values() {
        for p in pieces {
            if p.chars().count() >= ECHO_BELOW {
                for &a in up {
                    above.entry(a).or_default().push(p.clone());
                }
            }
            if p.chars().count() >= ECHO_ANYWHERE {
                anywhere.push(p.clone());
            }
        }
    }
    let chosen_set: HashSet<usize> = chosen.iter().copied().collect();
    for i in chosen.iter().copied() {
        for field in FIELDS {
            if !str_of(&nodes[i], field).trim().is_empty() {
                nodes[i][field]["value"] = Value::String(WITHHELD.to_string());
            }
        }
        if str_of(&nodes[i], "role") == "StaticText" {
            nodes[i][FLAG] = Value::Bool(true);
        }
        // The text pieces under a text node say the same words.
        let under: Vec<usize> = kids(&nodes[i]).filter_map(|k| by_id.get(k).copied()).collect();
        for k in under {
            if str_of(&nodes[k], "role") == "InlineTextBox" {
                nodes[k]["name"]["value"] = Value::String(String::new());
            }
        }
    }
    for (i, node) in nodes.iter_mut().enumerate() {
        if chosen_set.contains(&i) || ignored(node) {
            continue;
        }
        let near = above.get(&i);
        for field in FIELDS {
            let text = str_of(node, field).to_string();
            if text.is_empty() || text == WITHHELD {
                continue;
            }
            let mut now = text.clone();
            for p in anywhere.iter().chain(near.into_iter().flatten()) {
                now = now.replace(p.as_str(), WITHHELD);
            }
            // An ancestor's name is made from its contents, so the text may come apart (spaces, line breaks).
            if let Some(near) = near {
                let squeezed_now = squeezed(&now);
                if near.iter().any(|p| !p.is_empty() && squeezed_now.contains(&squeezed(p))) {
                    now = WITHHELD.to_string();
                }
            }
            if now != text {
                node[field]["value"] = Value::String(now);
            }
        }
    }
    changed
}

/// `text` with every word that has a secret's shape replaced by [`WITHHELD`]; the text itself when none has.
pub(super) fn scrub(text: &str) -> String {
    if !has_key_shape(text) {
        return text.to_string();
    }
    text.split_whitespace().map(|w| if has_key_shape(w) { WITHHELD } else { w }).collect::<Vec<_>>().join(" ")
}

/// [`scrub`] on every string in `value`, at any depth.
pub(super) fn scrub_value(value: &mut Value) {
    match value {
        Value::String(s) => {
            let clean = scrub(s);
            if clean != *s {
                *s = clean;
            }
        }
        Value::Array(a) => a.iter_mut().for_each(scrub_value),
        Value::Object(o) => o.values_mut().for_each(scrub_value),
        _ => {}
    }
}

#[cfg(test)]
#[path = "hidden_secrets_tests.rs"]
mod tests;
