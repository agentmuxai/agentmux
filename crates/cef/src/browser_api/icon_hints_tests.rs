// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use serde_json::json;

/// One element of a fixture page: its parent, tag, attributes and (for a text node) its text.
struct El {
    parent: i64,
    tag: &'static str,
    attrs: Vec<(&'static str, &'static str)>,
    text: Option<&'static str>,
}

fn el(parent: i64, tag: &'static str, attrs: &[(&'static str, &'static str)]) -> El {
    El { parent, tag, attrs: attrs.to_vec(), text: None }
}

/// A `DOMSnapshot.captureSnapshot` result for `els`; the backend id of element `i` is `10 + i`.
fn dom(els: &[El]) -> Value {
    let mut strings: Vec<String> = Vec::new();
    let mut id = |s: &str| -> i64 {
        let at = strings.iter().position(|x| x == s).unwrap_or_else(|| {
            strings.push(s.to_string());
            strings.len() - 1
        });
        at as i64
    };
    let names: Vec<i64> = els.iter().map(|e| id(e.tag)).collect();
    let values: Vec<i64> = els.iter().map(|e| e.text.map_or(-1, &mut id)).collect();
    let attrs: Vec<Vec<i64>> = els.iter().map(|e| e.attrs.iter().flat_map(|(k, v)| [id(k), id(v)]).collect()).collect();
    json!({
        "strings": strings,
        "documents": [{ "nodes": {
            "parentIndex": els.iter().map(|e| e.parent).collect::<Vec<_>>(),
            "backendNodeId": (0..els.len() as i64).map(|i| 10 + i).collect::<Vec<_>>(),
            "nodeName": names, "nodeValue": values, "attributes": attrs,
        } }],
    })
}

#[test]
fn an_icon_button_is_named_by_the_icon_in_it() {
    let page = dom(&[
        el(-1, "div", &[]),
        // 1: a button holding an SVG that uses a symbol named for its icon.
        el(0, "button", &[("class", "btn btn-icon")]),
        el(1, "svg", &[]),
        el(2, "use", &[("href", "/static/icons.svg#icon-copy")]),
        // 4: an icon font's crossed-out eye.
        el(0, "button", &[("class", "btn fa fa-eye-slash")]),
        // 5: an eye, in a child span's class.
        el(0, "button", &[]),
        el(5, "span", &[("class", "mdi mdi-eye-outline")]),
        // 7: a button with no icon and no tooltip.
        el(0, "button", &[("class", "btn btn-primary")]),
    ]);
    let got = hints(&page);
    assert_eq!(got.get(&11).map(String::as_str), Some("icon: copy"));
    assert_eq!(got.get(&14).map(String::as_str), Some("icon: eye-off"));
    assert_eq!(got.get(&15).map(String::as_str), Some("icon: eye"));
    assert!(!got.contains_key(&17), "a button the page says nothing about gets no guess");
}

#[test]
fn a_tooltip_the_page_gives_is_quoted_and_wins_over_the_icon() {
    let page = dom(&[
        el(-1, "div", &[]),
        el(0, "button", &[("data-original-title", "Roll key"), ("class", "icon-refresh")]),
        el(0, "a", &[("role", "button"), ("data-tooltip", "  Show   the value ")]),
        el(0, "button", &[("data-tooltip", "Copy sk_live_abcdefgh12345 now")]),
    ]);
    let got = hints(&page);
    assert_eq!(got.get(&11).map(String::as_str), Some("title \"Roll key\""));
    assert_eq!(got.get(&12).map(String::as_str), Some("title \"Show the value\""));
    let scrubbed = got.get(&13).unwrap();
    assert!(!scrubbed.contains("sk_live"), "a tooltip is page text, and a secret's shape is never quoted: {scrubbed}");
}

#[test]
fn an_svg_title_is_a_tooltip() {
    let mut text = el(3, "#text", &[]);
    text.text = Some("Reveal secret");
    let page = dom(&[el(-1, "div", &[]), el(0, "button", &[]), el(1, "svg", &[]), el(2, "title", &[]), text]);
    assert_eq!(hints(&page).get(&11).map(String::as_str), Some("title \"Reveal secret\""));
}

fn node(id: &str, role: &str, name: &str, backend: i64) -> Value {
    json!({ "nodeId": id, "role": { "value": role }, "name": { "value": name }, "childIds": [], "ignored": false, "backendDOMNodeId": backend })
}

#[test]
fn only_a_control_with_no_name_is_given_one() {
    let hints = HashMap::from([(11, "icon: copy".to_string()), (12, "icon: eye".to_string()), (13, "icon: eye".to_string())]);
    let mut nodes = vec![node("1", "button", "", 11), node("2", "button", "Reveal", 12), node("3", "heading", "", 13), node("4", "button", "", 99)];
    assert_eq!(mark(&mut nodes, &hints), 1);
    assert_eq!(nodes[0]["name"]["value"], "(no name; icon: copy)");
    assert_eq!(nodes[1]["name"]["value"], "Reveal", "a name the page gave stays");
    assert_eq!(nodes[2]["name"]["value"], "", "only controls");
    assert_eq!(nodes[3]["name"]["value"], "", "nothing is known of it");
}

#[test]
fn a_description_the_page_links_is_the_hint_before_the_icon() {
    let hints = HashMap::from([(11, "icon: refresh".to_string())]);
    let mut button = node("1", "button", "", 11);
    button["description"] = json!({ "value": "Rotate this key" });
    let mut nodes = vec![button];
    mark(&mut nodes, &hints);
    assert_eq!(nodes[0]["name"]["value"], "(no name; title \"Rotate this key\")");
}
