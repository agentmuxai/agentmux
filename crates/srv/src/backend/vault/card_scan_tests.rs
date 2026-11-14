// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A card's parts appear in no plain text: the scan of SPEC_VAULT_CARD_ENTRIES_FULL_CARD_DATA_AND_AUTOFILL_2026_11_08.md
//! §7. Two cards are made the ways an agent can make them (a command's output, parts read from a page), then every
//! record and answer a person, a pane or an agent can read is searched for the number, the security code, the name, the
//! ZIP and the expiry, in the forms they may be written. The brand, the last four and the expiry date (`expires`) are
//! readable by design and are not searched for.

use agentmux_common::api_types::{CaptureFrom, CaptureInto, CaptureRule};
use zeroize::Zeroizing;

use super::capture::{self, Outcome};
use super::history::{self, Asker};
use super::*;
use crate::backend::storage::store::Store;

const NUMBER: &str = "5555555555554444";
const GROUPED: &str = "5555 5555 5555 4444";
const EXPIRY: &str = "11/31";
const CODE: &str = "737";
const NAME: &str = "Zelda Quincy";
const ZIP: &str = "60614";

fn store() -> Store {
    seal::use_test_key();
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    s
}

/// `needle` in `text` where it does not touch a letter or a digit: a three-digit code is in many a timestamp.
fn standalone(text: &str, needle: &str) -> bool {
    let bytes = text.as_bytes();
    text.match_indices(needle).any(|(at, _)| {
        let before = at.checked_sub(1).map(|i| bytes[i]);
        let after = bytes.get(at + needle.len()).copied();
        !before.is_some_and(|b| b.is_ascii_alphanumeric()) && !after.is_some_and(|b| b.is_ascii_alphanumeric())
    })
}

/// What of the card shows in `text`, by name.
fn leaks(text: &str) -> Vec<&'static str> {
    let mut found = Vec::new();
    for (name, value) in [("number", NUMBER), ("grouped number", GROUPED), ("expiry", EXPIRY), ("name", NAME), ("zip", ZIP), ("number but its last four", "555555555555")] {
        if text.contains(value) {
            found.push(name);
        }
    }
    if standalone(text, CODE) {
        found.push("security code");
    }
    found
}

const OUTPUT: &str = r#"{"pan": "5555 5555 5555 4444", "expiry": "11/31", "cvv": "737", "holder": "Zelda Quincy", "zip": "60614"}"#;

fn parts(list: &[(&str, &str)]) -> CaptureFrom {
    CaptureFrom::Parts(list.iter().map(|(k, p)| (k.to_string(), CaptureFrom::Json(p.to_string()))).collect())
}

fn rule(label: &str) -> CaptureRule {
    CaptureRule {
        into: CaptureInto { kind: Some("card".into()), label: Some(label.into()), ..Default::default() },
        from: parts(&[("number", "$.pan"), ("exp", "$.expiry"), ("cvc", "$.cvv"), ("name", "$.holder"), ("zip", "$.zip")]),
    }
}

#[test]
fn no_part_of_a_card_is_in_any_record_or_answer_however_it_was_saved() {
    // The detector finds each part where it is written, and not a code inside a longer run of digits.
    assert_eq!(leaks(&format!("{NUMBER} {GROUPED} {EXPIRY} {NAME} {ZIP} {CODE}")), ["number", "grouped number", "expiry", "name", "zip", "number but its last four", "security code"]);
    assert!(leaks("updated_at: 1794607371738, last four 4444").is_empty(), "a timestamp and the last four are not a leak");
    let s = store();

    // 1. From a command's output: the output the agent gets back, and the withheld note when a part is missing.
    let cap = capture::plan(&s, &[rule("Scan card A")], "Agent2-6", "virtual-card show").unwrap();
    let Outcome::Stored { ids, scrubber, .. } = cap.finish(&s, OUTPUT, OUTPUT, false, 0) else { panic!("stored") };
    let a = ids[0].clone();
    let printed = format!("card issued: {OUTPUT}\nagain: {GROUPED}, code {CODE}, {NAME}, {ZIP}, {EXPIRY}");
    let shown_to_the_agent = scrubber.scrub(&printed).to_string();
    assert!(leaks(&shown_to_the_agent).is_empty(), "the command's output still has {:?}: {shown_to_the_agent}", leaks(&shown_to_the_agent));

    // 2. From parts read from a page.
    let cap = capture::plan(&s, &[rule("Scan card B")], "Agent2-6", "").unwrap();
    let found = capture::card_found(
        [("number", GROUPED), ("exp", EXPIRY), ("cvc", CODE), ("name", NAME), ("zip", ZIP)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), Zeroizing::new(v.to_string())))
            .collect(),
    )
    .unwrap();
    let b = cap.store(&s, &[found]).unwrap().remove(0);

    // 3. A refusal of a part the output could not be cleaned of names the part, never the value.
    let no_cvv = OUTPUT.replace("\"cvv\": \"737\", ", "\"cvv\": \"73\", ");
    let cap = capture::plan(&s, &[rule("Scan card C")], "Agent2-6", "virtual-card show").unwrap();
    let Outcome::Withheld { why, .. } = cap.finish(&s, &no_cvv, &no_cvv, false, 0) else { panic!("withheld") };
    assert!(why.contains("too short"), "{why}");
    assert!(leaks(&why).is_empty(), "the refusal has {:?}: {why}", leaks(&why));

    // Everything a person, a pane or an agent can read afterwards.
    let mut seen: Vec<(String, String)> = vec![("a refusal".into(), why)];
    seen.push(("the activity".into(), format!("{:?}", activity(&s, 100).unwrap())));
    for id in [&a, &b] {
        seen.push(("a history".into(), format!("{:?}", history::history_for(&s, id, Asker::Person, 100).unwrap())));
        seen.push(("its versions".into(), format!("{:?}", versions::list(&s, id).unwrap())));
        let key: Result<_, String> = Ok(seal::test_key());
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 13).unwrap();
        seen.push(("Vault show".into(), format!("{:?}", inspect::show(&s, id, Some("builder"), &key, today).unwrap())));
        let fields = s.vault_get(id).unwrap().unwrap().fields;
        seen.push(("its plain fields".into(), fields));
    }
    seen.push(("Vault list for an agent".into(), format!("{:?}", list_for_agent(&s, Some("builder")).unwrap())));
    seen.push(("Vault list for the pane".into(), format!("{:?}", list_for_pane(&s).unwrap())));
    let bad: Vec<String> = seen.iter().filter(|(_, text)| !leaks(text).is_empty()).map(|(what, text)| format!("{what}: {:?}", leaks(text))).collect();
    assert!(bad.is_empty(), "a card's part is in plain text: {bad:?}");

    // The values are still there, sealed: the scan is of something that was saved.
    assert_eq!(reveal(&s, &a, "number", "revealed").unwrap().as_str(), NUMBER);
    assert_eq!(reveal(&s, &b, "zip", "revealed").unwrap().as_str(), ZIP);
}
