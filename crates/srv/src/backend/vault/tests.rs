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
    assert_eq!(log.iter().map(|a| a.action.as_str()).collect::<Vec<_>>(), ["deleted", "replaced", "created"]);
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
    let log = activity(&s, 10).unwrap();
    assert_eq!(log[0].action, "revealed");
    assert_eq!(log[0].target, "value");
    assert!(!serde_json::to_string(&log).unwrap().contains("6789"));
    assert!(s.vault_get(&id).unwrap().unwrap().last_used_at > 0, "the use is noted on the entry");
}

// The trail never says copied when nothing was: a copy is logged when the host reports it.
#[test]
fn a_copy_is_logged_only_once_the_host_says_the_clipboard_has_it() {
    let s = store();
    let id = create(&s, ssn("Alex", "123-45-6789")).unwrap();
    assert_eq!(&*reveal(&s, &id, "value", "copied").unwrap(), "123-45-6789");
    assert!(activity(&s, 10).unwrap().iter().all(|a| a.action != "copied"), "fetching isn't copying");
    assert_eq!(s.vault_get(&id).unwrap().unwrap().last_used_at, 0);

    record_copy(&s, &id, "value", false).unwrap();
    let log = activity(&s, 10).unwrap();
    assert_eq!((log[0].action.as_str(), log[0].outcome.as_str()), ("copied", "failed"));
    assert_eq!(s.vault_get(&id).unwrap().unwrap().last_used_at, 0, "a failed copy isn't a use");

    record_copy(&s, &id, "value", true).unwrap();
    let log = activity(&s, 10).unwrap();
    assert_eq!((log[0].action.as_str(), log[0].outcome.as_str()), ("copied", "done"));
    assert!(s.vault_get(&id).unwrap().unwrap().last_used_at > 0);
    assert!(!serde_json::to_string(&log).unwrap().contains("6789"));
    assert!(record_copy(&s, "nope", "value", true).is_err());
    assert!(record_copy(&s, &id, "free text <script>", true).unwrap_err().contains("has no part"), "the part is checked against the kind");
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

fn gen(kind: &str, label: &str) -> GenerateRequest {
    GenerateRequest { kind: kind.into(), label: label.into(), why: "deploy key for the staging site".into(), fields: BTreeMap::new(), rules: Default::default(), expires: None }
}

#[test]
fn generate_makes_an_entry_whose_value_nobody_was_given() {
    let s = store();
    let made = generate_entry(&s, gen("api-key", "Staging deploy key"), "Agent2-6").unwrap();
    assert_eq!(made.kind, "api-key");
    assert!(made.made.contains("20 characters"), "{}", made.made);
    // The value is in the sealed row, with the generator's length.
    let value = reveal(&s, &made.id, "value", "revealed").unwrap();
    assert_eq!(value.chars().count(), generate::DEFAULT_LENGTH);
    // What an agent can see names it, and nothing else.
    let listed = list_for_agent(&s).unwrap();
    assert_eq!(listed, vec![VaultAgentEntry { id: made.id.clone(), label: "Staging deploy key".into(), kind: "api-key".into(), section: VaultSection::ApiKeys, expires: None }]);
    // The record names the agent, and never the value.
    let rows = activity(&s, 10).unwrap();
    let line = rows.iter().find(|r| r.action == "generated").unwrap();
    assert_eq!(line.actor, "Agent2-6");
    assert_eq!(line.target, "deploy key for the staging site");
    assert!(rows.iter().all(|r| !format!("{r:?}").contains(value.as_str())));
}

#[test]
fn generate_fills_a_logins_password_and_keeps_its_site_and_username() {
    let s = store();
    let mut req = gen("login", "Example shop");
    req.fields = [("site".to_string(), "https://shop.example.com".to_string()), ("username".to_string(), "alex@example.com".to_string())].into();
    req.rules.passwordrules = Some("minlength: 24; required: lower; required: digit; allowed: upper".into());
    let made = generate_entry(&s, req, "Agent2-6").unwrap();
    let pw = reveal(&s, &made.id, "password", "revealed").unwrap();
    assert_eq!(pw.chars().count(), 24);
    let (entries, _) = list_for_pane(&s).unwrap();
    assert_eq!(entries[0].fields.get("username").map(String::as_str), Some("alex@example.com"));
}

#[test]
fn generate_refuses_kinds_that_are_the_persons_own() {
    let s = store();
    for kind in ["ssn", "card", "bank-account", "passport"] {
        let e = generate_entry(&s, gen(kind, "x"), "Agent2-6").unwrap_err();
        assert!(e.contains("request"), "{kind}: {e}");
    }
    assert_eq!(s.vault_count().unwrap(), 0);
}

#[test]
fn generate_refuses_weak_rules_and_stores_nothing() {
    let s = store();
    let mut req = gen("login", "Weak site");
    req.rules.passwordrules = Some("allowed: digit; maxlength: 6".into());
    assert!(generate_entry(&s, req, "Agent2-6").is_err());
    assert_eq!(s.vault_count().unwrap(), 0);
}
