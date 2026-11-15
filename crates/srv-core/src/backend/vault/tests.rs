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

    let agent = serde_json::to_string(&list_for_agent(&s, None).unwrap()).unwrap();
    assert_eq!(agent, r#"[{"id":"ssn-alex","label":"Alex","kind":"ssn","section":"personal","vault":"Personal"}]"#);
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
    // The whole values: the log's millisecond timestamps can hold any four digits.
    assert!(!text.contains("123-45-6789") && !text.contains("987-65-4321"));
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
    assert!(!serde_json::to_string(&log).unwrap().contains("123-45-6789"));
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
    assert!(!serde_json::to_string(&log).unwrap().contains("123-45-6789"));
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
    GenerateRequest { kind: kind.into(), label: label.into(), why: "deploy key for the staging site".into(), reason: None, fields: BTreeMap::new(), rules: Default::default(), expires: None, vault: None }
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
    let listed = list_for_agent(&s, None).unwrap();
    assert_eq!(listed, vec![VaultAgentEntry { id: made.id.clone(), label: "Staging deploy key".into(), kind: "api-key".into(), section: VaultSection::ApiKeys, vault: "Personal".into(), expires: None }]);
    // The record names the agent, and never the value.
    let rows = activity(&s, 10).unwrap();
    let line = rows.iter().find(|r| r.action == "generated").unwrap();
    assert_eq!(line.actor, "Agent2-6");
    assert_eq!(line.reason, "deploy key for the staging site");
    assert!(line.target.contains("20 characters"), "{}", line.target);
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

fn card(label: &str, p: &[(&str, &str)], f: &[(&str, &str)]) -> NewEntry {
    let mut all = vec![("number", "4111 1111 1111 1111")];
    all.extend_from_slice(p);
    NewEntry {
        kind: "card".into(),
        label: label.into(),
        fields: f.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        tags: vec![],
        parts: parts(&all),
    }
}

fn opened(s: &Store, id: &str) -> BTreeMap<String, String> {
    let row = s.vault_get(id).unwrap().unwrap();
    seal::open(&seal::test_key(), id, &row.sealed).unwrap().0.clone()
}

fn fields_of(s: &Store, id: &str) -> BTreeMap<String, String> {
    serde_json::from_str(&s.vault_get(id).unwrap().unwrap().fields).unwrap()
}

#[test]
fn a_card_is_saved_with_its_number_as_digits_and_its_brand_and_expiry_date_in_the_open() {
    let s = store();
    let id = create(&s, card("Visa", &[("exp_month", "3"), ("exp_year", "27"), ("cvc", "123"), ("zip", " 94107 ")], &[("brand", "amex")])).unwrap();
    let sealed = opened(&s, &id);
    assert_eq!(sealed["number"], "4111111111111111");
    assert_eq!((sealed["exp_month"].as_str(), sealed["exp_year"].as_str(), sealed["zip"].as_str()), ("03", "2027", "94107"));
    let fields = fields_of(&s, &id);
    assert_eq!(fields["brand"], "visa", "the brand is worked out, not taken from the caller");
    assert_eq!(fields["expires"], "2027-03-31");
    let row = s.vault_get(&id).unwrap().unwrap();
    for secret in ["4111", "123", "94107"] {
        assert!(!row.fields.contains(secret), "{secret} is in the plain fields: {}", row.fields);
    }
    let (pane, _) = list_for_pane(&s).unwrap();
    assert_eq!(pane[0].tail, "1111");
    assert!(pane[0].parts_set.iter().any(|p| p == "exp_month"));
}

#[test]
fn the_older_expiry_is_read_into_a_month_and_year_and_still_fills_as_expiry() {
    let s = store();
    let id = create(&s, card("Visa", &[("expiry", "12/2030"), ("cvc", "123")], &[])).unwrap();
    let sealed = opened(&s, &id);
    assert_eq!((sealed.get("exp_month").map(String::as_str), sealed.get("exp_year").map(String::as_str), sealed.get("expiry")), (Some("12"), Some("2030"), None));
    assert_eq!(fields_of(&s, &id)["expires"], "2030-12-31");
    let created = s.vault_get(&id).unwrap().unwrap().created_at;
    assert_eq!(value_for_fill(&s, &id, created, "expiry").unwrap().as_str(), "12/30", "a fill that asks for the older part gets it from the month and year");
    assert!(value_for_fill(&s, &id, created, "zip").is_err());
}

#[test]
fn a_card_with_an_expiry_nobody_can_read_keeps_it_as_it_was() {
    let s = store();
    let id = create(&s, card("Gift", &[("expiry", "someday")], &[])).unwrap();
    assert_eq!(opened(&s, &id)["expiry"], "someday");
    assert!(!fields_of(&s, &id).contains_key("expires"));
}

#[test]
fn a_bad_month_or_year_is_refused_and_nothing_is_saved() {
    let s = store();
    for (m, y) in [("13", "2030"), ("3", "1999")] {
        let err = create(&s, card("Visa", &[("exp_month", m), ("exp_year", y)], &[])).unwrap_err();
        assert!(err.contains("expiry"), "{err}");
    }
    assert_eq!(s.vault_count().unwrap(), 0);
}

#[test]
fn a_change_keeps_the_expiry_it_is_not_given_and_follows_a_new_number() {
    let s = store();
    let id = create(&s, card("Visa", &[("exp_month", "03"), ("exp_year", "2027")], &[])).unwrap();
    let change = |p: Parts| EntryChange { id: id.clone(), label: "Visa".into(), fields: fields_of(&s, &id), tags: vec![], parts: p, clear: vec![] };
    update(&s, change(parts(&[("number", "5555 5555 5555 4444")]))).unwrap();
    let sealed = opened(&s, &id);
    assert_eq!((sealed["number"].as_str(), sealed["exp_month"].as_str(), sealed["exp_year"].as_str()), ("5555555555554444", "03", "2027"));
    let fields = fields_of(&s, &id);
    assert_eq!((fields["brand"].as_str(), fields["expires"].as_str()), ("mastercard", "2027-03-31"));
    update(&s, change(parts(&[("exp_month", "11"), ("exp_year", "2029")]))).unwrap();
    assert_eq!(fields_of(&s, &id)["expires"], "2029-11-30");
}

#[test]
fn only_one_card_in_a_vault_is_the_default() {
    let s = store();
    let a = create(&s, card("Visa A", &[], &[("default", "yes")])).unwrap();
    assert_eq!(fields_of(&s, &a)["default"], "yes");
    let b = create(&s, card("Visa B", &[], &[("default", "yes")])).unwrap();
    assert!(!fields_of(&s, &a).contains_key("default"), "the flag moved to the newer one");
    assert_eq!(fields_of(&s, &b)["default"], "yes");
    let c = create(&s, card("Visa C", &[], &[])).unwrap();
    assert_eq!(fields_of(&s, &b)["default"], "yes", "a card saved without it takes nothing");
    let mut fields = fields_of(&s, &c);
    fields.insert("default".into(), "yes".into());
    update(&s, EntryChange { id: c.clone(), label: "Visa C".into(), fields, tags: vec![], parts: Parts::default(), clear: vec![] }).unwrap();
    assert!(!fields_of(&s, &b).contains_key("default"));
    assert_eq!(fields_of(&s, &c)["default"], "yes");
}

#[test]
fn a_default_that_is_not_yes_is_dropped() {
    let s = store();
    let id = create(&s, card("Visa", &[], &[("default", "true")])).unwrap();
    assert!(!fields_of(&s, &id).contains_key("default"));
}
