// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use agentmux_common::shown_once::SHOWN_ONCE;
use serde_json::{json, Value};

use super::super::snapshot::{find, render, DomFacts, ElementQuery};

const SECRET: &str = "Xy8Q~abcDEF1234ghijKLmnoPQRstu.vwxYZ0123";
const CLIENT_ID: &str = "0f4c2a1e-9b7d-4e3a-8c6f-1a2b3c4d5e6f";

fn n(id: &str, parent: Option<&str>, role: &str, name: &str, backend: Option<i64>, kids: &[&str]) -> Value {
    let mut v = json!({
        "nodeId": id,
        "role": { "type": "role", "value": role },
        "name": { "type": "computedString", "value": name },
        "childIds": kids,
        "ignored": false,
    });
    if let Some(p) = parent {
        v["parentId"] = json!(p);
    }
    if let Some(b) = backend {
        v["backendDOMNodeId"] = json!(b);
    }
    v
}

fn field(id: &str, parent: &str, backend: i64, value: &str, readonly: bool) -> Value {
    let mut v = n(id, Some(parent), "textbox", "Client secret", Some(backend), &[]);
    v["value"] = json!({ "type": "string", "value": value });
    v["properties"] = json!([{ "name": "readonly", "value": { "type": "boolean", "value": readonly } }]);
    v
}

fn text_input(backends: &[i64]) -> HashMap<i64, DomFacts> {
    backends.iter().map(|b| (*b, DomFacts { local_name: "input".into(), attrs: HashMap::new(), label: String::new() })).collect()
}

/// A console page that has just made a client secret: shown in code, in a row with
/// a Copy button, and in a read-only field; its id (a GUID) next to it; and the same
/// shape in an ordinary paragraph.
fn console() -> Vec<Value> {
    vec![
        n("1", None, "RootWebArea", "New client secret", Some(1), &["2", "4", "7", "9", "11"]),
        n("2", Some("1"), "code", "", Some(2), &["3"]),
        n("3", Some("2"), "StaticText", SECRET, Some(3), &[]),
        n("4", Some("1"), "row", "", Some(4), &["5", "6"]),
        n("5", Some("4"), "StaticText", &format!("Value {SECRET}"), Some(5), &[]),
        n("6", Some("4"), "button", "Copy to clipboard", Some(6), &[]),
        n("7", Some("1"), "generic", "", Some(7), &["8"]),
        field("8", "7", 8, SECRET, true),
        n("9", Some("1"), "paragraph", "", Some(9), &["10"]),
        n("10", Some("9"), "StaticText", &format!("Client ID {CLIENT_ID} and NotASecretButLongIdentifierName"), Some(10), &[]),
        n("11", Some("1"), "generic", "", Some(11), &["12"]),
        field("12", "11", 12, "AnEditableValue1234567", false),
    ]
}

#[test]
fn a_secret_shown_once_is_masked_and_given_a_reference_to_capture_it() {
    let s = render(&console(), &text_input(&[8, 12]), None, "");
    assert!(!s.text.contains(SECRET), "{}", s.text);
    for line in s.text.lines().filter(|l| l.contains(SHOWN_ONCE)) {
        assert!(line.contains("[ref="), "a masked line can be captured: {line}");
    }
    assert_eq!(s.text.matches(SHOWN_ONCE).count(), 3, "code, Copy row and read-only field: {}", s.text);
    let refs: Vec<i64> = s.refs.iter().map(|(_, b)| *b).collect();
    assert!(refs.contains(&3) && refs.contains(&5) && refs.contains(&8), "{refs:?}");
}

#[test]
fn ids_and_text_outside_where_a_page_shows_a_secret_stay_visible() {
    let s = render(&console(), &text_input(&[8, 12]), None, "");
    assert!(s.text.contains(CLIENT_ID), "{}", s.text);
    assert!(s.text.contains("NotASecretButLongIdentifierName"), "an ordinary paragraph: {}", s.text);
    assert!(s.text.contains("AnEditableValue1234567"), "an editable field the person types in: {}", s.text);
}

#[test]
fn a_snapshot_scoped_to_the_masked_line_keeps_it_masked() {
    for backend in [3, 5] {
        let s = render(&console(), &text_input(&[8, 12]), Some(backend), "");
        assert!(s.text.contains(SHOWN_ONCE) && !s.text.contains(SECRET), "scope {backend}: {}", s.text);
    }
}

#[test]
fn find_masks_it_too() {
    let q = ElementQuery { role: Some("text".into()), name: "value".into(), ..Default::default() };
    let found = find(&console(), &HashMap::new(), &q, "");
    assert!(found.text.contains(SHOWN_ONCE) && !found.text.contains(SECRET), "{}", found.text);
    assert_eq!(found.refs.len(), 1, "{}", found.text);
}

#[test]
fn a_copy_buttons_container_is_its_nearest_ancestor_with_more_than_the_button() {
    // The button sits in a wrapper of its own; the row above it holds the secret.
    let nodes = vec![
        n("1", None, "RootWebArea", "", Some(1), &["2", "6"]),
        n("2", Some("1"), "row", "", Some(2), &["3", "4"]),
        n("3", Some("2"), "StaticText", SECRET, Some(3), &[]),
        n("4", Some("2"), "generic", "", Some(4), &["5"]),
        n("5", Some("4"), "button", "Copy", Some(5), &[]),
        n("6", Some("1"), "StaticText", SECRET, Some(6), &[]),
    ];
    let zones = super::copy_zones(&nodes);
    assert!(zones.contains("2") && !zones.contains("1"), "{zones:?}");
    let s = render(&nodes, &HashMap::new(), None, "");
    assert_eq!(s.text.matches(SHOWN_ONCE).count(), 1, "only the one in the row: {}", s.text);
}
