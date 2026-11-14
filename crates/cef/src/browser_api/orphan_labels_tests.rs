// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
use super::*;
use serde_json::json;
use std::collections::HashMap;

/// A node as the accessibility tree gives it.
fn node(id: &str, role: &str, name: &str, kids: &[&str]) -> Value {
    json!({ "nodeId": id, "role": { "type": "role", "value": role }, "name": { "type": "string", "value": name },
            "ignored": false, "childIds": kids, "backendDOMNodeId": id.parse::<i64>().unwrap_or(0) })
}

fn text(id: &str, words: &str) -> [Value; 1] {
    [node(id, "StaticText", words, &[])]
}

/// What the agent reads for these nodes.
fn shown(nodes: &[Value]) -> String {
    super::super::snapshot::render(&prepared(nodes), &HashMap::new(), None, "").text
}

/// The page of QTest's Stripe sign-up, as Edge's tree shows it: a label whose control is hidden, holding the
/// sentence, and a button beside it.
fn orphan_page() -> Vec<Value> {
    let mut v = vec![
        node("1", "RootWebArea", "t", &["5"]),
        node("5", "generic", "", &["6", "9"]),
        node("6", "LabelText", "", &["10", "11", "12"]),
        node("9", "button", "Yes, update", &["13"]),
    ];
    v.extend(text("10", "Did you mean "));
    v.extend(text("11", "x@y.test"));
    v.extend(text("12", "?"));
    v.extend(text("13", "Yes, update"));
    v
}

#[test]
fn a_label_no_field_holds_shows_its_sentence_beside_the_button() {
    let out = shown(&orphan_page());
    assert!(out.contains("Did you mean"), "{out}");
    assert!(out.contains("x@y.test"), "{out}");
    assert!(out.contains(r#"button "Yes, update""#), "{out}");
    let (sentence, button) = (out.find("Did you mean").unwrap(), out.find("button").unwrap());
    assert!(sentence < button, "the sentence comes before the button it asks about: {out}");
}

#[test]
fn a_label_a_field_is_named_by_is_not_repeated() {
    let mut nodes = vec![
        node("1", "RootWebArea", "t", &["5"]),
        node("5", "generic", "", &["6", "7"]),
        node("6", "LabelText", "", &["10"]),
        node("7", "textbox", "Email address", &[]),
    ];
    nodes.extend(text("10", "Email address"));
    assert!(matches!(open_orphans(&nodes), Cow::Borrowed(_)));
    let out = shown(&nodes);
    assert_eq!(out.matches("Email address").count(), 1, "{out}");
}

#[test]
fn a_label_around_its_field_is_not_repeated_whatever_the_field_name_adds() {
    let mut nodes = vec![
        node("1", "RootWebArea", "t", &["5"]),
        node("5", "generic", "", &["6"]),
        node("6", "LabelText", "", &["7", "10", "11"]),
        node("7", "checkbox", "I agree to the terms of service", &[]),
    ];
    nodes.extend(text("10", "I agree to the "));
    nodes.extend(text("11", "terms of service"));
    assert!(matches!(open_orphans(&nodes), Cow::Borrowed(_)));
    assert_eq!(shown(&nodes).matches("terms of service").count(), 1);
}

#[test]
fn a_label_around_a_field_with_a_value_is_not_repeated_for_the_value() {
    // <label>Your name <input value="Lark"></label>: the value's text is the field's, not the label's.
    let mut nodes = vec![
        node("1", "RootWebArea", "t", &["5"]),
        node("5", "generic", "", &["6"]),
        node("6", "LabelText", "", &["10", "7"]),
        node("7", "textbox", "Your name", &["11"]),
    ];
    nodes.extend(text("10", "Your name"));
    nodes.extend(text("11", "Lark"));
    assert!(matches!(open_orphans(&nodes), Cow::Borrowed(_)));
    assert_eq!(shown(&nodes).matches("Your name").count(), 1);
}

#[test]
fn a_legend_the_group_is_named_by_is_not_repeated() {
    let mut nodes = vec![
        node("1", "RootWebArea", "t", &["5"]),
        node("5", "group", "Delivery speed", &["6"]),
        node("6", "Legend", "", &["10"]),
    ];
    nodes.extend(text("10", "Delivery speed"));
    assert!(matches!(open_orphans(&nodes), Cow::Borrowed(_)));
}

#[test]
fn only_the_orphan_label_is_opened() {
    let mut nodes = orphan_page();
    nodes[1]["childIds"] = json!(["6", "9", "20"]);
    nodes.push(node("20", "LabelText", "", &["21"]));
    nodes.extend(text("21", "Email"));
    nodes.push(node("22", "textbox", "Email", &[]));
    nodes[1]["childIds"] = json!(["6", "9", "20", "22"]);
    let opened = open_orphans(&nodes);
    let roles: Vec<&str> = opened.iter().map(|n| str_of(n, "role")).collect();
    assert_eq!(roles.iter().filter(|r| **r == "LabelText").count(), 1, "{roles:?}");
    let out = shown(&nodes);
    assert_eq!(out.matches("Email").count(), 1, "{out}");
    assert!(out.contains("Did you mean"), "{out}");
}

#[test]
fn a_page_with_no_label_is_not_copied() {
    let nodes = vec![node("1", "RootWebArea", "t", &[])];
    assert!(matches!(prepared(&nodes), Cow::Borrowed(_)));
}
