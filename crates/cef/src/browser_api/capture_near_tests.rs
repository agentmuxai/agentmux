// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

const SECRET: &str = "xK9fQ2mLp7ZaB3cD4eF5gH6";

fn n(id: &str, parent: Option<&str>, role: &str, name: &str, backend: i64, kids: &[&str]) -> Value {
    let mut v = json!({
        "nodeId": id, "role": { "type": "role", "value": role }, "name": { "type": "computedString", "value": name },
        "childIds": kids, "backendDOMNodeId": backend, "ignored": false,
    });
    if let Some(p) = parent {
        v["parentId"] = json!(p);
    }
    v
}

/// A settings page: a row with a label, the revealed secret as plain text (backend 4), a Hide and a Copy button.
fn page(label: &str, value: &str) -> Vec<Value> {
    vec![
        n("1", None, "RootWebArea", "Settings", 1, &["2"]),
        n("2", Some("1"), "generic", "", 2, &["3", "4", "5", "6"]),
        n("3", Some("2"), "StaticText", label, 3, &[]),
        n("4", Some("2"), "StaticText", value, 4, &[]),
        n("5", Some("2"), "button", "Hide", 5, &[]),
        n("6", Some("2"), "button", "Copy", 6, &[]),
    ]
}

#[test]
fn the_text_beside_a_label_is_found_without_a_reference() {
    assert_eq!(find_value(&page("Client secret", SECRET), "Client secret", None), Ok(4));
    // How a person reads the label: case, spacing and the colon don't matter.
    assert_eq!(find_value(&page("Client  Secret:", SECRET), "client secret", None), Ok(4));
}

#[test]
fn a_prefix_picks_a_value_with_no_secret_shape() {
    let plain = "abc-def-ghi";
    assert!(find_value(&page("Token", plain), "Token", None).unwrap_err().contains("no value found"));
    assert_eq!(find_value(&page("Token", plain), "Token", Some("abc-")), Ok(4));
    assert!(find_value(&page("Token", plain), "Token", Some("xyz-")).is_err());
}

#[test]
fn the_text_of_a_button_beside_the_label_is_never_the_value() {
    let mut nodes = page("Key", "hidden behind the eye");
    nodes[4]["name"]["value"] = json!(SECRET);
    let err = find_value(&nodes, "Key", None).unwrap_err();
    assert!(err.contains("no value found") && !err.contains(SECRET), "{err}");
}

#[test]
fn a_value_in_a_read_only_field_beside_the_label_is_found() {
    let mut nodes = page("API key", "");
    nodes[3] = n("4", Some("2"), "textbox", "", 4, &[]);
    nodes[3]["value"] = json!({ "type": "string", "value": SECRET });
    assert_eq!(find_value(&nodes, "API key", None), Ok(4));
}

#[test]
fn two_values_or_two_labels_are_refused_without_quoting_the_page() {
    let mut nodes = page("Secret", SECRET);
    nodes.push(n("7", Some("2"), "StaticText", "Zb8Yc7Xd6We5Vf4Ug3Th2Si1", 7, &[]));
    nodes[1]["childIds"] = json!(["3", "4", "5", "6", "7"]);
    let err = find_value(&nodes, "Secret", None).unwrap_err();
    assert!(err.contains("2 values") && err.contains("prefix") && !err.contains(SECRET), "{err}");
    assert_eq!(find_value(&nodes, "Secret", Some("Zb8")), Ok(7), "a prefix chooses");
    let mut twice = page("Secret", SECRET);
    twice.push(n("8", Some("1"), "StaticText", "Secret", 8, &[]));
    twice[0]["childIds"] = json!(["2", "8"]);
    assert!(find_value(&twice, "Secret", None).unwrap_err().contains("2 places"));
    assert!(find_value(&page("Other", SECRET), "Secret", None).unwrap_err().contains("no text on the page reads"));
}

#[test]
fn a_value_in_another_row_is_not_beside_the_label() {
    // The label's row has nothing; the page's main area has a value two rows away: not taken.
    let nodes = vec![
        n("1", None, "RootWebArea", "Settings", 1, &["2", "5"]),
        n("2", Some("1"), "main", "", 2, &["3", "6"]),
        n("3", Some("2"), "generic", "", 3, &["4"]),
        n("4", Some("3"), "StaticText", "Secret", 4, &[]),
        n("6", Some("2"), "StaticText", SECRET, 6, &[]),
        n("5", Some("1"), "generic", "", 5, &[]),
    ];
    assert!(find_value(&nodes, "Secret", None).unwrap_err().contains("no value found"));
}
