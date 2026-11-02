// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Reading a long text through a snapshot: how many characters of a text, name or value a line
//! shows (`max_text`), and matching an element with no name of its own (a paragraph) by the text it
//! holds, so a `scope` or a `find` can reach it. Split out of `snapshot.rs`.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::snapshot::{str_of, ElementQuery, TEXT_SHOWN_ELSEWHERE};

/// How many characters of one text, name or value a snapshot line shows.
pub const DEFAULT_TEXT_LIMIT: usize = 160;
/// The most an agent can ask for with `max_text`.
pub const MAX_TEXT_LIMIT: usize = 8000;
/// The text masked before it is cut, so a cut never falls inside a number.
pub(super) const MASK_WINDOW: usize = 2000;

thread_local! {
    /// The limit for the render in progress. A render is one synchronous call, so it is set around
    /// that call (`with_text_limit`) rather than passed through every line-building function.
    static TEXT_LIMIT: std::cell::Cell<usize> = const { std::cell::Cell::new(DEFAULT_TEXT_LIMIT) };
}

/// The limit for the render in progress.
pub(super) fn limit() -> usize {
    TEXT_LIMIT.with(|l| l.get())
}

/// Run `render` with each text, name and value on a snapshot line shown up to `limit` characters
/// (at least the default, at most [`MAX_TEXT_LIMIT`]), for reading a long text in full.
pub fn with_text_limit<R>(limit: Option<usize>, render: impl FnOnce() -> R) -> R {
    let wanted = limit.map_or(DEFAULT_TEXT_LIMIT, |l| l.clamp(DEFAULT_TEXT_LIMIT, MAX_TEXT_LIMIT));
    let before = TEXT_LIMIT.with(|l| l.replace(wanted));
    struct Restore(usize);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEXT_LIMIT.with(|l| l.set(self.0));
        }
    }
    let _restore = Restore(before);
    render()
}

pub(super) fn ids_of(nodes: &[Value]) -> HashMap<&str, &Value> {
    nodes.iter().filter_map(|n| Some((n.get("nodeId")?.as_str()?, n))).collect()
}

/// The most text of an element's descendants a name match reads.
const MATCH_TEXT_CHARS: usize = 4000;

/// The name a query is matched against. An element with an explicit role in the query and no
/// accessible name of its own (a paragraph, an article, a list item) is matched by the text it holds,
/// so `paragraph "the refund will"` finds the paragraph and a scope can narrow to it. Text on a page
/// that shows a secret once is never read: an element holding one is matched by its own name only.
pub(super) fn matchable_name(
    node: &Value,
    in_zone: bool,
    by_id: &HashMap<&str, &Value>,
    zones: &HashSet<&str>,
    q: &ElementQuery,
) -> String {
    let shown = super::shown_once::shown_name(node, in_zone);
    if !shown.trim().is_empty() || in_zone || q.name.trim().is_empty() || q.role().is_none() {
        return shown;
    }
    let mut text = String::new();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if text.len() >= MATCH_TEXT_CHARS {
            break;
        }
        if super::shown_once::starts_zone(n, zones) {
            return shown;
        }
        // A field's text is its value, shown (or masked) on its own line: matching on it would let a
        // query count a read-only secret out, so it is never read here.
        if TEXT_SHOWN_ELSEWHERE.contains(&str_of(n, "role")) {
            continue;
        }
        if str_of(n, "role") == "StaticText" && !n.get("ignored").and_then(Value::as_bool).unwrap_or(false) {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(str_of(n, "name").trim());
        }
        if let Some(kids) = n.get("childIds").and_then(Value::as_array) {
            stack.extend(kids.iter().rev().filter_map(|k| k.as_str().and_then(|id| by_id.get(id)).copied()));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::super::snapshot::{find, first_match, render};
    use super::*;
    use serde_json::json;

    fn n(id: &str, parent: Option<&str>, role: &str, name: &str, backend: i64, kids: &[&str]) -> Value {
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

    fn q(role: Option<&str>, name: &str) -> ElementQuery {
        ElementQuery { role: role.map(str::to_string), name: name.to_string(), exact: false }
    }

    /// A page with one long paragraph (a support reply) and a footer.
    fn long_reply(words: usize) -> Vec<Value> {
        let text = (0..words).map(|i| format!("word{i}")).collect::<Vec<_>>().join(" ");
        vec![
            n("1", None, "RootWebArea", "", 1, &["2", "5"]),
            n("2", Some("1"), "paragraph", "", 2, &["3", "4"]),
            n("3", Some("2"), "StaticText", &text, 3, &[]),
            n("4", Some("2"), "StaticText", "The refund will arrive on Friday.", 4, &[]),
            n("5", Some("1"), "paragraph", "", 5, &["6"]),
            n("6", Some("5"), "StaticText", "Unrelated footer", 6, &[]),
        ]
    }

    #[test]
    fn a_long_text_is_cut_at_160_characters_unless_more_is_asked_for() {
        let nodes = long_reply(100);
        let short = render(&nodes, &HashMap::new(), None, "");
        assert!(!short.text.contains("word99"), "{}", short.text);
        assert!(short.text.contains('…'), "{}", short.text);
        let full = with_text_limit(Some(4000), || render(&nodes, &HashMap::new(), None, ""));
        assert!(full.text.contains("word99"), "{}", full.text);
        assert!(!full.text.contains('…'), "{}", full.text);
        // The limit is for that render only.
        let after = render(&nodes, &HashMap::new(), None, "");
        assert!(!after.text.contains("word99"), "{}", after.text);
    }

    #[test]
    fn the_text_limit_is_bounded_both_ways() {
        let nodes = long_reply(3000);
        let huge = with_text_limit(Some(1_000_000), || render(&nodes, &HashMap::new(), None, ""));
        assert!(huge.text.contains('…'), "a text over the largest limit is still cut");
        let tiny = with_text_limit(Some(5), || render(&long_reply(100), &HashMap::new(), None, ""));
        assert!(tiny.text.contains("word0"), "a limit under the default is the default: {}", tiny.text);
    }

    #[test]
    fn a_long_text_still_has_a_card_number_in_it_hidden() {
        let mut nodes = long_reply(10);
        nodes[2]["name"]["value"] = json!("Your card 4111 1111 1111 1111 was charged. word1 word2");
        let s = with_text_limit(Some(4000), || render(&nodes, &HashMap::new(), None, ""));
        assert!(!s.text.contains("4111"), "{}", s.text);
    }

    #[test]
    fn a_paragraph_is_found_and_scoped_to_by_the_text_it_holds() {
        let nodes = long_reply(10);
        let query = q(Some("paragraph"), "refund will arrive");
        assert_eq!(first_match(&nodes, &query), Some(2));
        let scoped = render(&nodes, &HashMap::new(), first_match(&nodes, &query), "");
        assert!(scoped.text.contains("refund will arrive") && !scoped.text.contains("Unrelated footer"), "{}", scoped.text);
        let found = find(&nodes, &HashMap::new(), &query, "n1");
        assert_eq!(found.total, 1, "{}", found.text);
    }

    #[test]
    fn a_fields_value_is_never_read_to_match_the_container_around_it() {
        let nodes = vec![
            n("1", None, "RootWebArea", "", 1, &["2"]),
            n("2", Some("1"), "group", "", 2, &["3", "5"]),
            n("3", Some("2"), "textbox", "Recovery code", 3, &["4"]),
            n("4", Some("3"), "StaticText", "s3cr3t-value", 4, &[]),
            n("5", Some("2"), "StaticText", "Keep it safe", 5, &[]),
        ];
        assert_eq!(first_match(&nodes, &q(Some("group"), "s3cr3t")), None);
        assert_eq!(first_match(&nodes, &q(Some("group"), "keep it safe")), Some(2));
    }

    #[test]
    fn text_holding_matching_needs_a_role_so_containers_dont_all_match() {
        let nodes = long_reply(10);
        assert_eq!(find(&nodes, &HashMap::new(), &q(None, "refund will arrive"), "n1").total, 0, "no role: no container is matched by its text");
        assert_eq!(find(&nodes, &HashMap::new(), &q(Some("text"), "refund will arrive"), "n1").total, 1, "the text itself");
        assert_eq!(first_match(&nodes, &q(Some("paragraph"), "no such words")), None);
    }
}
