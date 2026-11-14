// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
use super::super::snapshot::{self, ElementQuery};
use super::super::visibility::{self, Layout};
use super::*;
use serde_json::json;

const SECRET: &str = "xK9fQ2mLp7ZaB3cD4eF5gH6";

fn n(id: &str, parent: Option<&str>, role: &str, name: &str, backend: i64, kids: &[&str]) -> Value {
    let mut v = json!({
        "nodeId": id,
        "role": { "type": "role", "value": role },
        "name": { "type": "computedString", "value": name },
        "childIds": kids,
        "backendDOMNodeId": backend,
        "ignored": false,
    });
    if let Some(p) = parent {
        v["parentId"] = json!(p);
    }
    v
}

/// A settings row: a label, the secret masked on screen as dots, the real text in a node the page hides (backend
/// 9), and a button to show it. `label` is what the row is called.
fn row(label: &str, hidden_text: &str) -> Vec<Value> {
    vec![
        n("1", None, "RootWebArea", "Settings", 1, &["2"]),
        n("2", Some("1"), "generic", "", 2, &["3", "4", "5", "6"]),
        n("3", Some("2"), "StaticText", label, 3, &[]),
        n("4", Some("2"), "StaticText", "••••••••••••", 4, &[]),
        n("5", Some("2"), "StaticText", hidden_text, 9, &[]),
        n("6", Some("2"), "button", "Show", 6, &[]),
    ]
}

fn layout(hidden: &[i64]) -> Layout {
    Layout { hidden: hidden.iter().copied().collect(), ..Layout::default() }
}

fn shown(nodes: &[Value], scope: Option<i64>) -> snapshot::Snapshot {
    snapshot::render(nodes, &HashMap::new(), scope, "")
}

#[test]
fn a_secret_shaped_hidden_text_is_never_listed_with_or_without_include_hidden_or_scope() {
    for include_hidden in [true, false] {
        let mut nodes = row("Endpoint", SECRET);
        visibility::apply(&mut nodes, &layout(&[9]), include_hidden);
        for scope in [None, Some(2)] {
            let out = shown(&nodes, scope);
            assert!(!out.text.contains(SECRET), "include_hidden={include_hidden} scope={scope:?}: {}", out.text);
        }
        if include_hidden {
            let out = shown(&nodes, None);
            assert!(out.text.contains(WITHHELD), "the marker is there: {}", out.text);
            assert!(out.text.contains("button"), "the button stays listed: {}", out.text);
            assert!(out.refs.iter().any(|(_, b)| *b == 9), "the withheld text has a reference, for capture: {:?}", out.refs);
        }
    }
}

#[test]
fn find_neither_lists_nor_matches_a_withheld_text() {
    let mut nodes = row("Endpoint", SECRET);
    visibility::apply(&mut nodes, &layout(&[9]), true);
    let by_secret = ElementQuery { role: Some("text".into()), name: "xK9fQ2".into(), exact: false };
    assert_eq!(snapshot::find(&nodes, &HashMap::new(), &by_secret, "").total, 0, "no prefix of the secret matches");
    let by_marker = ElementQuery { role: Some("text".into()), name: "withheld".into(), exact: false };
    let found = snapshot::find(&nodes, &HashMap::new(), &by_marker, "");
    assert_eq!(found.total, 1);
    assert!(!found.text.contains(SECRET), "{}", found.text);
}

#[test]
fn hidden_text_in_a_row_named_like_a_secret_is_withheld_whatever_its_shape() {
    for label in ["Webhook signing secret", "API key", "Access token", "Password", "Private value"] {
        let mut nodes = row(label, "abc123");
        visibility::apply(&mut nodes, &layout(&[9]), true);
        let out = shown(&nodes, None);
        assert!(!out.text.contains("abc123") && out.text.contains(WITHHELD), "{label}: {}", out.text);
    }
}

#[test]
fn ordinary_hidden_text_is_still_listed_with_include_hidden() {
    // A collapsed menu: hidden text in a row that says nothing of secrets.
    let mut nodes = row("Navigation", "Account settings");
    let cut = visibility::apply(&mut nodes, &layout(&[9]), true);
    assert_eq!(cut, 0);
    let out = shown(&nodes, None);
    assert!(out.text.contains("Account settings") && !out.text.contains(WITHHELD), "{}", out.text);
    // Without include_hidden it is left out, as before.
    let mut nodes = row("Navigation", "Account settings");
    assert_eq!(visibility::apply(&mut nodes, &layout(&[9]), false), 1);
    assert!(!shown(&nodes, None).text.contains("Account settings"));
}

#[test]
fn a_hidden_element_with_a_key_in_its_name_stays_listed_without_the_key() {
    let mut nodes = vec![
        n("1", None, "RootWebArea", "Settings", 1, &["2"]),
        n("2", Some("1"), "button", &format!("Copy {SECRET}"), 9, &[]),
    ];
    visibility::apply(&mut nodes, &layout(&[9]), true);
    let out = shown(&nodes, None);
    assert!(!out.text.contains(SECRET) && out.text.contains("button"), "{}", out.text);
    assert!(out.refs.iter().any(|(_, b)| *b == 9));
}

#[test]
fn known_key_prefixes_count_even_when_short() {
    for key in ["sk_live_abcdefgh12", "whsec_abcdefgh12", "ghp_abcdefghij12", "AKIAABCDEFGHIJ12"] {
        assert!(has_key_shape(key), "{key}");
    }
    for word in ["Account settings", "skip to content", "keyboard", "2026-11-14"] {
        assert!(!has_key_shape(word), "{word}");
    }
}

#[test]
fn nothing_is_changed_when_nothing_is_hidden() {
    let mut nodes = row("Webhook signing secret", "abc123");
    assert_eq!(withhold(&mut nodes, &HashSet::new()), 0);
    assert!(shown(&nodes, None).text.contains("abc123"), "visible text is the page's own, shown to the person too");
}
