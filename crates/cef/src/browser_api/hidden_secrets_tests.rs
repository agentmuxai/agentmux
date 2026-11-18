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
    for label in ["Webhook signing secret", "API key", "Access token", "Password", "Private key", "apiKey", "Client secrets"] {
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
    for key in [concat!("sk_live", "_abcdefgh12"), "whsec_abcdefgh12", concat!("ghp", "_abcdefghij12"), concat!("AKI", "AABCDEFGHIJKLMN12")] {
        assert!(has_key_shape(key), "{key}");
    }
    for word in ["Account settings", "skip to content", "keyboard", "2026-11-14"] {
        assert!(!has_key_shape(word), "{word}");
    }
}

#[test]
fn nothing_is_changed_when_nothing_is_hidden() {
    let mut nodes = row("Webhook signing secret", "abc123");
    assert_eq!(withhold(&mut nodes, &HashSet::new(), false), 0);
    assert!(shown(&nodes, None).text.contains("abc123"), "visible text is the page's own, shown to the person too");
}

/// A visible cell and a visible button, each named from its contents, with the secret in a node the page hides
/// inside it (backend 9 and 10).
fn named_from_contents() -> Vec<Value> {
    vec![
        n("1", None, "RootWebArea", "Settings", 1, &["2", "5"]),
        n("2", Some("1"), "cell", &format!("Signing {SECRET}"), 2, &["3", "4"]),
        n("3", Some("2"), "StaticText", "Signing ", 3, &[]),
        n("4", Some("2"), "StaticText", SECRET, 9, &[]),
        n("5", Some("1"), "button", &format!("Copy {SECRET}"), 5, &["6", "7"]),
        n("6", Some("5"), "StaticText", "Copy ", 6, &[]),
        n("7", Some("5"), "StaticText", SECRET, 10, &[]),
    ]
}

#[test]
fn a_visible_ancestor_named_from_its_contents_does_not_carry_the_text() {
    for include_hidden in [true, false] {
        let mut nodes = named_from_contents();
        visibility::apply(&mut nodes, &layout(&[9, 10]), include_hidden);
        for scope in [None, Some(2), Some(5)] {
            let out = shown(&nodes, scope);
            assert!(!out.text.contains(SECRET), "include_hidden={include_hidden} scope={scope:?}: {}", out.text);
        }
        assert!(shown(&nodes, None).text.contains("button"), "the button stays listed");
        let by_secret = ElementQuery { role: None, name: "xK9fQ2".into(), exact: false };
        assert_eq!(snapshot::find(&nodes, &HashMap::new(), &by_secret, "").total, 0);
    }
}

#[test]
fn text_that_comes_apart_in_an_ancestors_name_is_still_withheld() {
    // The ancestor's name has the pieces joined by a space the text itself does not have.
    let mut nodes = vec![
        n("1", None, "RootWebArea", "Settings", 1, &["2"]),
        n("2", Some("1"), "cell", "Key sk_live_ abcdefgh12", 2, &["3", "4"]),
        n("3", Some("2"), "StaticText", "sk_live_", 9, &[]),
        n("4", Some("2"), "StaticText", "abcdefgh12", 10, &[]),
    ];
    visibility::apply(&mut nodes, &layout(&[9, 10]), true);
    let out = shown(&nodes, None);
    assert!(!out.text.contains("abcdefgh12") && !out.text.contains("sk_live_"), "{}", out.text);
}

#[test]
fn a_secret_text_repeated_in_another_name_is_withheld_there_too() {
    let mut nodes = row("Endpoint", SECRET);
    nodes[1]["childIds"] = json!(["3", "4", "5", "6", "7"]);
    nodes.push(n("7", Some("2"), "link", &format!("Open {SECRET} in a new tab"), 7, &[]));
    visibility::apply(&mut nodes, &layout(&[9]), true);
    assert!(!shown(&nodes, None).text.contains(SECRET));
}

#[test]
fn only_whole_words_name_a_secret() {
    for name in ["API keys", "apiKey", "Signing secret", "Access token", "Passkeys", "Passwords", "Credentials", "private_key", "Show key"] {
        assert!(says_secret(name), "{name}");
    }
    for name in ["Keyboard shortcuts", "Hotkeys", "Monkey business", "Turkey", "Keynote", "Tokenize", "Private repositories", "Settings"] {
        assert!(!says_secret(name), "{name}");
    }
    let mut nodes = row("Keyboard shortcuts", "abc123");
    visibility::apply(&mut nodes, &layout(&[9]), true);
    assert!(shown(&nodes, None).text.contains("abc123"), "a row about the keyboard holds no secret");
}

/// A settings page: a sidebar with a link named like a secret, and a main area with a collapsed menu and
/// other hidden text of its own (backends 20..=24).
fn settings_page() -> Vec<Value> {
    let mut v = vec![
        n("1", None, "RootWebArea", "Settings", 1, &["2", "10"]),
        n("2", Some("1"), "navigation", "Sidebar", 2, &["3", "4", "5", "6"]),
        n("3", Some("2"), "link", "Profile", 3, &[]),
        n("4", Some("2"), "link", "API keys", 4, &[]),
        n("5", Some("2"), "link", "Billing", 5, &[]),
        n("6", Some("2"), "StaticText", "Hidden sidebar hint", 20, &[]),
        n("10", Some("1"), "main", "", 10, &["11", "12", "13", "14"]),
        n("11", Some("10"), "generic", "", 11, &["15", "16"]),
        n("15", Some("11"), "StaticText", "Language", 15, &[]),
        n("16", Some("11"), "StaticText", "Collapsed menu: English", 21, &[]),
        n("12", Some("10"), "generic", "", 12, &["17"]),
        n("17", Some("12"), "StaticText", "Closed select option: Deutsch", 22, &[]),
        n("13", Some("10"), "generic", "", 13, &["18"]),
        n("18", Some("13"), "StaticText", "Tooltip text for theme", 23, &[]),
        n("14", Some("10"), "generic", "", 14, &["19"]),
        n("19", Some("14"), "StaticText", "Skip to content", 24, &[]),
    ];
    // A page's worth of other nodes, so the main area is no row.
    for i in 0..50 {
        v.push(n(&format!("9{i}"), Some("10"), "StaticText", &format!("Line {i}"), 100 + i, &[]));
        v[6]["childIds"].as_array_mut().unwrap().push(json!(format!("9{i}")));
    }
    v
}

#[test]
fn a_nav_link_named_like_a_secret_does_not_withhold_the_pages_other_hidden_text() {
    let hidden: HashSet<i64> = (20..=24).collect();
    let mut nodes = settings_page();
    assert_eq!(withhold(&mut nodes, &hidden, false), 0, "a navigation is no row, and the main area is too big to be one");
    let mut nodes = settings_page();
    visibility::apply(&mut nodes, &Layout { hidden: hidden.clone(), ..Layout::default() }, true);
    let out = shown(&nodes, None);
    for kept in ["Collapsed menu: English", "Closed select option: Deutsch", "Tooltip text for theme", "Skip to content"] {
        assert!(out.text.contains(kept), "{kept} is still listed: {}", out.text);
    }
}

#[test]
fn when_the_layout_cannot_be_read_a_secret_shape_is_withheld_wherever_it_is() {
    let mut nodes = row("Endpoint", SECRET);
    let layout = Layout { unknown: true, ..Layout::default() };
    visibility::apply(&mut nodes, &layout, true);
    let out = shown(&nodes, None);
    assert!(!out.text.contains(SECRET) && out.text.contains(WITHHELD), "{}", out.text);
}

#[test]
fn hidden_text_in_a_frame_of_the_page_is_read_from_every_document() {
    let doc = |backend: i64, opacity: usize| {
        json!({
            "nodes": { "parentIndex": [-1], "backendNodeId": [backend] },
            "layout": { "nodeIndex": [0], "styles": [[opacity, 2, 2]], "bounds": [[0.0, 0.0, 10.0, 10.0]] }
        })
    };
    let result = json!({ "strings": ["1", "0", "visible"], "documents": [doc(1, 0), doc(7, 1)] });
    assert_eq!(visibility::hidden_ids(&result), Some(HashSet::from([7])));
    assert_eq!(visibility::hidden_ids(&json!({})), None);
}

#[test]
fn what_a_page_script_read_is_scrubbed_of_secret_shapes() {
    assert_eq!(scrub("Copy sk_live_abcdefgh12 now"), format!("Copy {WITHHELD} now"));
    assert_eq!(scrub("Account settings"), "Account settings");
    let mut state = json!({ "tag": "div", "value": "token ghp_abcdefghij12", "checked": true, "options": ["a", "xK9fQ2mLp7ZaB3cD4eF5gH6"] });
    scrub_value(&mut state);
    assert_eq!(state["value"], json!(format!("token {WITHHELD}")));
    assert_eq!(state["checked"], json!(true));
    assert_eq!(state["options"][0], json!("a"));
    assert!(!state.to_string().contains("xK9fQ2"));
}

#[test]
fn a_pem_header_and_an_access_key_id_are_shaped_and_ordinary_words_are_not() {
    // The header is put together here so no file holds a whole one.
    let pem_header = format!("{}BEGIN {} KEY{}", "-".repeat(5), "PRIVATE", "-".repeat(5));
    for key in [pem_header.as_str(), concat!("AKI", "AIOSFODNN7EXAMPLE"), concat!("ASI", "AIOSFODNN7EXAMPLE")] {
        assert!(has_key_shape(key), "{key}");
    }
    for word in ["Asia/Kolkata", "Asiatic", "akiakiakiakia", "Australia/Sydney"] {
        assert!(!has_key_shape(word), "{word}");
    }
}

#[test]
fn an_unreadable_layout_withholds_a_shape_only_and_leaves_help_text_near_a_password_field() {
    let mut nodes = vec![
        n("1", None, "RootWebArea", "Sign in", 1, &["2"]),
        n("2", Some("1"), "generic", "", 2, &["3", "4", "5"]),
        n("3", Some("2"), "textbox", "Password", 3, &[]),
        n("4", Some("2"), "StaticText", "At least eight characters", 4, &[]),
        n("5", Some("2"), "StaticText", SECRET, 5, &[]),
    ];
    visibility::apply(&mut nodes, &Layout { unknown: true, ..Layout::default() }, true);
    let out = shown(&nodes, None);
    assert!(out.text.contains("At least eight characters"), "{}", out.text);
    assert!(!out.text.contains(SECRET), "{}", out.text);
}

#[test]
fn a_phrase_a_row_withheld_is_not_taken_out_of_the_pages_visible_controls() {
    // A hidden menu item in a secret row ("Roll key"), and a visible button elsewhere with the same name.
    let mut nodes = vec![
        n("1", None, "RootWebArea", "Settings", 1, &["2", "6"]),
        n("2", Some("1"), "generic", "", 2, &["3", "4", "5"]),
        n("3", Some("2"), "StaticText", "Signing key", 3, &[]),
        n("4", Some("2"), "StaticText", "Roll key", 9, &[]),
        n("5", Some("2"), "StaticText", "Reveal", 10, &[]),
        n("6", Some("1"), "button", "Roll key", 6, &[]),
    ];
    visibility::apply(&mut nodes, &layout(&[9, 10]), true);
    let out = shown(&nodes, None);
    assert!(out.text.contains(r#"button "Roll key""#), "the visible control keeps its name: {}", out.text);
    assert_eq!(out.text.matches(WITHHELD).count(), 2, "the two hidden menu items are withheld: {}", out.text);
}

#[test]
fn an_odd_shaped_value_a_row_withheld_is_still_taken_out_of_a_siblings_description() {
    let mut nodes = row("Signing key", "q7Zp-93xaLm2Rt");
    nodes[5]["description"] = json!({ "type": "computedString", "value": "Copies q7Zp-93xaLm2Rt to the clipboard" });
    visibility::apply(&mut nodes, &layout(&[9]), true);
    let said = nodes[5]["description"]["value"].as_str().unwrap_or("");
    assert!(!said.contains("q7Zp") && said.contains("to the clipboard"), "{said}");
    assert!(!shown(&nodes, None).text.contains("q7Zp"));
}

#[test]
fn an_unreadable_layout_withholds_an_odd_token_in_a_secret_row_and_keeps_a_phrase_beside_a_password_field() {
    let unknown = Layout { unknown: true, ..Layout::default() };
    // The odd-shaped value in a "Signing key" row, with no key shape of its own.
    let mut nodes = row("Signing key", "q7Zp-93xaLm2Rt");
    visibility::apply(&mut nodes, &unknown, true);
    let out = shown(&nodes, None);
    assert!(!out.text.contains("q7Zp") && out.text.contains(WITHHELD), "{}", out.text);
    // Help text beside a password field is a phrase, not a token.
    let mut nodes = row("Password", "At least eight characters");
    visibility::apply(&mut nodes, &unknown, true);
    assert!(shown(&nodes, None).text.contains("At least eight characters"));
    // With the layout read, the same odd token is withheld as before.
    let mut nodes = row("Signing key", "q7Zp-93xaLm2Rt");
    visibility::apply(&mut nodes, &layout(&[9]), true);
    assert!(!shown(&nodes, None).text.contains("q7Zp"));
}

#[test]
fn an_answer_with_an_approval_request_in_it_is_left_as_the_person_is_shown_it() {
    let ask = json!({ "needs_approval": { "what": "Click \"Save\"", "fields": [["Key", "ghp_abcdefghij12", true]] } });
    assert_eq!(scrub_answer(ask.clone()), ask);
    let answered = scrub_answer(json!({ "value": "ghp_abcdefghij12", "checked": true }));
    assert_eq!(answered["value"], json!(WITHHELD));
    assert_eq!(answered["checked"], json!(true));
}

#[test]
fn a_cell_whose_only_content_is_the_withheld_text_has_the_reference_for_capture() {
    // A table cell named from its one hidden text: the marker is the cell's name, and the cell's line has a ref.
    let mut nodes = vec![
        n("1", None, "RootWebArea", "Settings", 1, &["2"]),
        n("2", Some("1"), "cell", SECRET, 2, &["3"]),
        n("3", Some("2"), "StaticText", SECRET, 9, &[]),
    ];
    visibility::apply(&mut nodes, &layout(&[9]), true);
    let out = shown(&nodes, None);
    assert!(!out.text.contains(SECRET) && out.text.contains(WITHHELD), "{}", out.text);
    assert!(out.refs.iter().any(|(_, b)| *b == 2), "the cell has a reference: {:?} {}", out.refs, out.text);
}

/// A settings row after its eye button was pressed: the label, the secret in plain sight (backend 4), a Hide button.
fn revealed(label: &str, text: &str) -> Vec<Value> {
    vec![
        n("1", None, "RootWebArea", "Settings", 1, &["2"]),
        n("2", Some("1"), "generic", "", 2, &["3", "4", "5"]),
        n("3", Some("2"), "StaticText", label, 3, &[]),
        n("4", Some("2"), "StaticText", text, 4, &[]),
        n("5", Some("2"), "button", "Hide", 5, &[]),
    ]
}

#[test]
fn a_secret_revealed_in_plain_sight_is_listed_unread_with_a_reference() {
    let mut nodes = revealed("Client secret", SECRET);
    visibility::apply(&mut nodes, &layout(&[]), false);
    let out = shown(&nodes, None);
    assert!(!out.text.contains(SECRET), "{}", out.text);
    assert!(out.text.contains(UNREAD), "the row says there is an unread secret: {}", out.text);
    assert!(out.refs.iter().any(|(_, b)| *b == 4), "capture can name it: {:?}", out.refs);
    // `find` lists it by the marker and never matches the secret a piece at a time.
    let by_marker = ElementQuery { role: Some("text".into()), name: "unread secret".into(), exact: false };
    assert_eq!(snapshot::find(&nodes, &HashMap::new(), &by_marker, "").total, 1);
    let by_piece = ElementQuery { role: Some("text".into()), name: "xK9fQ2".into(), exact: false };
    assert_eq!(snapshot::find(&nodes, &HashMap::new(), &by_piece, "").total, 0);
}

#[test]
fn plain_text_that_is_no_secret_or_in_no_secret_row_is_left_alone() {
    // An odd token in a row about something else (an order number), and a word in a secret row.
    for (label, text) in [("Order reference", SECRET), ("API key", "Production"), ("API key", "Created 3 days ago")] {
        let mut nodes = revealed(label, text);
        visibility::apply(&mut nodes, &layout(&[]), false);
        let out = shown(&nodes, None);
        assert!(out.text.contains(text) && !out.text.contains(UNREAD), "{label}: {}", out.text);
    }
}

#[test]
fn a_name_made_from_the_revealed_text_does_not_carry_it() {
    let mut nodes = revealed("Client secret", SECRET);
    nodes[1]["name"]["value"] = json!(format!("Client secret {SECRET} Hide"));
    visibility::apply(&mut nodes, &layout(&[]), false);
    let out = shown(&nodes, None);
    assert!(!out.text.contains(SECRET), "{}", out.text);
}

#[test]
fn a_tooltip_that_repeats_a_withheld_secret_does_not_carry_it_into_an_icon_buttons_name() {
    let token = "q7Zp-93xaLm2Rt";
    let mut nodes = row("Signing key", token);
    nodes[5]["name"]["value"] = json!("");
    let layout = Layout { hidden: HashSet::from([9]), hints: HashMap::from([(6, format!("title \"{token}\""))]), ..Layout::default() };
    visibility::apply(&mut nodes, &layout, true);
    let out = shown(&nodes, None);
    assert!(!out.text.contains(token), "{}", out.text);
    assert!(out.text.contains("no name"), "the button is still named for what it is: {}", out.text);
}
