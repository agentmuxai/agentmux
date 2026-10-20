// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

fn store() -> Store {
    seal::use_test_key();
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    s
}

fn parts(pairs: &[(&str, &str)]) -> Parts {
    Parts(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
}

fn ssn(label: &str, value: &str) -> NewEntry {
    NewEntry {
        kind: "ssn".into(),
        label: label.into(),
        fields: [("owner".to_string(), "Alex".to_string())].into(),
        tags: vec![],
        parts: parts(&[("value", value)]),
    }
}

#[test]
fn an_entry_is_stored_sealed_and_listed_without_its_value() {
    let s = store();
    let id = create(&s, ssn("Alex", "123-45-6789")).unwrap();
    assert_eq!(id, "ssn-alex");
    let row = s.vault_get(&id).unwrap().unwrap();
    assert!(!row.sealed.windows(11).any(|w| w == b"123-45-6789"), "the value is sealed");
    assert!(!row.fields.contains("6789") && !row.label.contains("6789"));

    let (pane, problem) = list_for_pane(&s).unwrap();
    assert!(problem.is_none());
    assert_eq!(pane[0].tail, "6789");
    assert_eq!(pane[0].parts_set, ["value"]);
    assert_eq!(pane[0].fields["owner"], "Alex");
    let shown = serde_json::to_string(&pane).unwrap();
    assert!(!shown.contains("123-45"), "the pane never gets the whole value: {shown}");

    let agent = serde_json::to_string(&list_for_agent(&s).unwrap()).unwrap();
    assert_eq!(agent, r#"[{"id":"ssn-alex","label":"Alex","kind":"ssn","section":"personal"}]"#);
}

#[test]
fn ids_are_readable_unique_and_have_no_dot() {
    let s = store();
    assert_eq!(create(&s, ssn("Alex Q. Doe", "111-11-1111")).unwrap(), "ssn-alex-q-doe");
    assert_eq!(create(&s, ssn("Alex Q. Doe", "222-22-2222")).unwrap(), "ssn-alex-q-doe-2");
    let long = create(&s, ssn(&"x".repeat(150), "333-33-3333")).unwrap();
    assert!(long.len() <= MAX_ID && !long.contains('.'));
}

#[test]
fn bad_entries_are_refused() {
    let s = store();
    let mut e = ssn("A", "1");
    e.kind = "rocket".into();
    assert!(create(&s, e).unwrap_err().contains("unknown kind"));
    assert!(create(&s, ssn(" ", "1")).unwrap_err().contains("label"));
    assert!(create(&s, ssn("A", "")).unwrap_err().contains("needs its value"));
    let mut e = ssn("A", "1");
    e.parts.0.insert("password".into(), "x".into());
    assert!(create(&s, e).unwrap_err().contains("no part"));
    let mut e = ssn("A", "1");
    e.fields.insert("site".into(), "x".into());
    assert!(create(&s, e).unwrap_err().contains("no field"));
    assert_eq!(s.vault_count().unwrap(), 0);
}

#[test]
fn a_change_keeps_the_parts_it_doesnt_name() {
    let s = store();
    let id = create(
        &s,
        NewEntry {
            kind: "card".into(),
            label: "Company card".into(),
            fields: Default::default(),
            tags: vec![],
            parts: parts(&[("number", "4111111111111111"), ("cvc", "123"), ("expiry", "12/30")]),
        },
    )
    .unwrap();
    update(
        &s,
        EntryChange {
            id: id.clone(),
            label: "Company Visa".into(),
            fields: Default::default(),
            tags: vec!["work".into()],
            parts: parts(&[("cvc", "999")]),
            clear: vec!["expiry".into()],
        },
    )
    .unwrap();
    let row = s.vault_get(&id).unwrap().unwrap();
    let opened = seal::open(&seal::test_key(), &id, &row.sealed).unwrap();
    assert_eq!(opened.get("number"), Some("4111111111111111"));
    assert_eq!(opened.get("cvc"), Some("999"));
    assert_eq!(opened.get("expiry"), None);
    assert_eq!(row.label, "Company Visa");
    assert_eq!(id, "card-company-card", "the id stays when the label changes");

    let clear_main = EntryChange { id: id.clone(), label: "x".into(), fields: Default::default(), tags: vec![], parts: Parts::default(), clear: vec!["number".into()] };
    assert!(update(&s, clear_main).is_err());
}

#[test]
fn every_change_is_in_the_activity_log_without_values() {
    let s = store();
    let id = create(&s, ssn("Alex", "123-45-6789")).unwrap();
    update(&s, EntryChange { id: id.clone(), label: "Alex".into(), fields: Default::default(), tags: vec![], parts: parts(&[("value", "987-65-4321")]), clear: vec![] }).unwrap();
    delete(&s, &id).unwrap();
    let log = activity(&s, 10).unwrap();
    assert_eq!(log.iter().map(|a| a.action.as_str()).collect::<Vec<_>>(), ["deleted", "changed", "created"]);
    assert!(log.iter().all(|a| a.actor == PERSON && a.entry_label == "Alex"));
    let text = serde_json::to_string(&log).unwrap();
    assert!(!text.contains("6789") && !text.contains("4321"));
    assert!(delete(&s, &id).is_err());
}

#[test]
fn a_reveal_returns_the_part_and_logs_the_use_without_the_value() {
    let s = store();
    let id = create(&s, ssn("Alex", "123-45-6789")).unwrap();
    assert_eq!(&*reveal(&s, &id, "value", "revealed").unwrap(), "123-45-6789");
    assert_eq!(&*reveal(&s, &id, "value", "copied").unwrap(), "123-45-6789");
    let log = activity(&s, 10).unwrap();
    assert_eq!(log.iter().map(|a| a.action.as_str()).take(2).collect::<Vec<_>>(), ["copied", "revealed"]);
    assert_eq!(log[0].target, "value");
    assert!(!serde_json::to_string(&log).unwrap().contains("6789"));
    assert!(s.vault_get(&id).unwrap().unwrap().last_used_at > 0, "the use is noted on the entry");
}

#[test]
fn a_reveal_of_a_missing_part_or_an_unknown_use_is_refused() {
    let s = store();
    let id = create(&s, ssn("Alex", "123-45-6789")).unwrap();
    assert!(reveal(&s, &id, "cvc", "revealed").unwrap_err().contains("no part"));
    assert!(reveal(&s, &id, "value", "emailed").unwrap_err().contains("unknown use"));
    assert!(reveal(&s, "nope", "value", "revealed").unwrap_err().contains("no entry"));
    let card = create(
        &s,
        NewEntry { kind: "card".into(), label: "C".into(), fields: Default::default(), tags: vec![], parts: parts(&[("number", "4111111111111111")]) },
    )
    .unwrap();
    assert!(reveal(&s, &card, "cvc", "revealed").unwrap_err().contains("no cvc saved"));
    assert_eq!(activity(&s, 10).unwrap().iter().filter(|a| a.action == "revealed").count(), 0, "a refusal isn't a use");
}
