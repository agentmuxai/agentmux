// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The conformance file both parsers run (R12), and a bounded fuzz run of this one. The app runs the same file in
//! `frontend/app/deeplinks/parse.test.ts`.

use std::collections::BTreeMap;

use proptest::prelude::*;
use serde_json::{json, Value as Json};

use super::parse::{parse_link, Value};
use super::{place_label_key, REGISTRY};

const CONFORMANCE: &str = include_str!("../../deeplinks/conformance.json");

/// A link or a value as the file writes it: a string, or `{ prefix, repeat, count }`.
fn text_of(v: &Json) -> String {
    match v {
        Json::String(s) => s.clone(),
        Json::Object(o) => {
            let prefix = o["prefix"].as_str().unwrap();
            let repeat = o["repeat"].as_str().unwrap();
            let count = o["count"].as_u64().unwrap() as usize;
            format!("{prefix}{}", repeat.repeat(count))
        }
        other => panic!("not a link: {other}"),
    }
}

fn json_of(map: &BTreeMap<String, Value>) -> Json {
    Json::Object(
        map.iter()
            .map(|(k, v)| {
                let j = match v {
                    Value::Int(n) => json!(n),
                    Value::Text(s) => json!(s),
                };
                (k.clone(), j)
            })
            .collect(),
    )
}

fn expected_map(v: &Json) -> Json {
    Json::Object(
        v.as_object()
            .unwrap()
            .iter()
            .map(|(k, x)| (k.clone(), if x.is_object() { json!(text_of(x)) } else { x.clone() }))
            .collect(),
    )
}

fn cases(group: &str) -> Vec<Json> {
    let file: Json = serde_json::from_str(CONFORMANCE).unwrap();
    file[group].as_array().unwrap().clone()
}

#[test]
fn every_valid_link_parses_to_what_the_file_says() {
    let valid = cases("valid");
    assert!(valid.len() > 10);
    for case in valid {
        let link = text_of(&case["link"]);
        let parsed = parse_link(&link).unwrap_or_else(|e| panic!("{link}: refused with {}", e.as_str()));
        let ok = &case["ok"];
        assert_eq!(parsed.view.id, ok["view"].as_str().unwrap(), "{link}: view");
        assert_eq!(parsed.place.map(|p| p.id), ok["place"].as_str(), "{link}: place");
        assert_eq!(json_of(&parsed.values), expected_map(&ok["values"]), "{link}: values");
        assert_eq!(json_of(&parsed.params), expected_map(&ok["params"]), "{link}: params");
        assert_eq!(parsed.anchor.as_deref(), ok["anchor"].as_str(), "{link}: anchor");
        let canonical = case.get("canonical").map(text_of).unwrap_or(link.clone());
        assert_eq!(parsed.canonical, canonical, "{link}: canonical form");
    }
}

#[test]
fn every_invalid_link_is_refused_with_the_files_reason() {
    let invalid = cases("invalid");
    assert!(invalid.len() > 20);
    for case in invalid {
        let link = text_of(&case["link"]);
        let want = case["error"].as_str().unwrap();
        match parse_link(&link) {
            Ok(p) => panic!("{link}: parsed as {}, expected {want}", p.canonical),
            Err(e) => assert_eq!(e.as_str(), want, "{link}"),
        }
        assert_ne!(REGISTRY.reason(want), "the link is refused", "{want} has a sentence in the registry");
    }
}

#[test]
fn a_place_is_labelled_by_its_enum_value_or_its_own_label() {
    let section = parse_link("agentmux://settings/recording").unwrap();
    assert_eq!(place_label_key(&section), Some("settings.sections.recording"));
    let task = parse_link("agentmux://kanban/task/7").unwrap();
    assert_eq!(place_label_key(&task), Some("deeplinks.places.kanban.task"));
    assert_eq!(place_label_key(&parse_link("agentmux://help").unwrap()), None);
}

#[test]
fn a_link_never_names_a_target_or_a_connection() {
    for view in REGISTRY.views {
        for place in view.places {
            for p in place.params {
                assert!(
                    !["pane", "tab", "window", "target", "connection", "host"].contains(&p.name),
                    "{}/{} takes {}",
                    view.id,
                    place.id,
                    p.name
                );
            }
        }
    }
}

/// Characters a link is made of, weighted toward the ones that matter to the parser.
fn link_char() -> impl Strategy<Value = char> {
    prop_oneof![
        8 => prop::sample::select(vec!['a', 'e', 's', 't', 'k', '1', '0', '-', '.', '_', '~']),
        4 => prop::sample::select(vec!['/', '?', '#', '&', '=', '%']),
        2 => prop::sample::select(vec!['A', 'F', '2', 'E', 'C', '3']),
        1 => any::<char>(),
    ]
}

fn fuzz_link() -> impl Strategy<Value = String> {
    let heads = prop::sample::select(vec![
        "agentmux://settings/",
        "agentmux://kanban/",
        "agentmux://editor/",
        "agentmux://files/%2F",
        "agentmux://vault/",
        "agentmux://task/",
        "agentmux://",
        "",
    ]);
    (heads, prop::collection::vec(link_char(), 0..40)).prop_map(|(h, tail)| format!("{h}{}", tail.into_iter().collect::<String>()))
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 4000, ..ProptestConfig::default() })]

    /// The parser never panics, and an accepted link's canonical form parses to itself (R7): one written form.
    #[test]
    fn the_parser_is_total_and_canonical_forms_are_fixed_points(link in fuzz_link()) {
        if let Ok(parsed) = parse_link(&link) {
            let again = parse_link(&parsed.canonical).expect("a canonical form parses");
            prop_assert_eq!(&again.canonical, &parsed.canonical);
            prop_assert_eq!(again.view.id, parsed.view.id);
            prop_assert_eq!(again.values, parsed.values);
            prop_assert!(!parsed.canonical.chars().any(super::parse::is_control));
        }
    }
}
