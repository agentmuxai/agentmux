// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::store::Store;
use crate::backend::vault::{reveal, seal};

fn store() -> Store {
    seal::use_test_key();
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    s
}

fn refs(list: &[(&str, &str)]) -> BTreeMap<String, String> {
    list.iter().map(|(k, r)| (k.to_string(), r.to_string())).collect()
}

fn read(key: &str, value: &str, origin: &str) -> (String, Zeroizing<String>, String) {
    (key.to_string(), Zeroizing::new(value.to_string()), origin.to_string())
}

fn card_as(label: &str) -> CaptureAs {
    CaptureAs { label: label.into(), ..Default::default() }
}

fn opened(s: &Store, id: &str, part: &str) -> String {
    reveal(s, id, part, "revealed").unwrap().to_string()
}

#[test]
fn the_request_makes_a_new_card_and_says_what_is_missing() {
    let s = store();
    let all = refs(&[("number", "e1"), ("exp", "e2"), ("cvc", "e3"), ("name", "e4"), ("zip", "e5")]);
    let rule = card_rule(&all, Some(&card_as("Virtual Visa"))).unwrap();
    assert!(capture::plan(&s, &[rule], "Agent2-6", "").is_ok());

    assert!(card_rule(&all, None).unwrap_err().contains("where the card goes"));
    let other = CaptureAs { kind: Some("api-key".into()), label: "x".into(), ..Default::default() };
    assert!(card_rule(&all, Some(&other)).unwrap_err().contains("card is for a card"));
    assert!(card_rule(&refs(&[("number", "  ")]), Some(&card_as("C"))).unwrap_err().contains("card.number needs the ref"));

    let plan_err = |list: &[(&str, &str)]| capture::plan(&s, &[card_rule(&refs(list), Some(&card_as("C"))).unwrap()], "Agent2-6", "").unwrap_err();
    assert!(plan_err(&[("cvc", "e3")]).contains("needs the card's \"number\""));
    assert!(plan_err(&[("number", "e1"), ("pin", "e9")]).contains("no \"pin\""));
    assert!(plan_err(&[("number", "e1"), ("exp", "e2"), ("exp_year", "e3")]).contains("not both"));
}

#[test]
fn the_parts_must_all_be_on_one_site_the_host_named() {
    let ok = gathered(vec![read("number", "4242 4242 4242 4242", "https://bank.example"), read("cvc", "123", "https://bank.example")]).unwrap();
    assert_eq!(ok.1, "https://bank.example");
    assert_eq!(ok.0.len(), 2);

    let two = gathered(vec![read("number", "4242 4242 4242 4242", "https://bank.example"), read("cvc", "123", "https://evil.example")]).unwrap_err();
    assert!(two.contains("more than one site") && two.contains("nothing was saved"), "{two}");
    assert!(!two.contains("4242") && !two.contains("123"), "no value in {two}");
    assert!(gathered(vec![read("number", "4242424242424242", "")]).unwrap_err().contains("can't tell which site"));
}

#[test]
fn a_card_read_from_a_page_is_one_settled_entry_with_every_part_sealed() {
    let s = store();
    let all = refs(&[("number", "e1"), ("exp_month", "e2"), ("exp_year", "e3"), ("cvc", "e4"), ("name", "e5"), ("zip", "e6")]);
    let plan = capture::plan(&s, &[card_rule(&all, Some(&card_as("Virtual Visa"))).unwrap()], "Agent2-6", "").unwrap();
    let reads = vec![
        read("number", "4242 4242 4242 4242", "https://cards.example"),
        read("exp_month", "3", "https://cards.example"),
        read("exp_year", "30", "https://cards.example"),
        read("cvc", "123", "https://cards.example"),
        read("name", "Alex Doe", "https://cards.example"),
        read("zip", "94107", "https://cards.example"),
    ];
    let (values, origin) = gathered(reads).unwrap();
    let given = CaptureAs { label: "Virtual Visa".into(), fields: [("notes".to_string(), "from the bank page".to_string())].into(), ..Default::default() };
    let (id, label) = store_card(&s, plan, values, &origin, Some(&given)).unwrap();
    assert_eq!(label, "Virtual Visa");
    assert_eq!(s.vault_list().unwrap().iter().filter(|r| r.kind == "card").count(), 1, "one entry from six elements");
    assert_eq!(opened(&s, &id, "number"), "4242424242424242");
    assert_eq!((opened(&s, &id, "exp_month").as_str(), opened(&s, &id, "exp_year").as_str()), ("03", "2030"));
    assert_eq!((opened(&s, &id, "cvc").as_str(), opened(&s, &id, "name").as_str(), opened(&s, &id, "zip").as_str()), ("123", "Alex Doe", "94107"));
    let fields: BTreeMap<String, String> = serde_json::from_str(&s.vault_get(&id).unwrap().unwrap().fields).unwrap();
    assert_eq!((fields.get("brand").map(String::as_str), fields.get("expires").map(String::as_str)), (Some("visa"), Some("2030-03-31")));
    assert_eq!(fields.get("notes").map(String::as_str), Some("from the bank page"));
}

#[test]
fn a_card_without_a_number_or_with_an_empty_part_is_not_made() {
    let s = store();
    let plan = || capture::plan(&s, &[card_rule(&refs(&[("number", "e1")]), Some(&card_as("C"))).unwrap()], "Agent2-6", "").unwrap();
    let only_cvc: BTreeMap<String, Zeroizing<String>> = [("cvc".to_string(), Zeroizing::new("123".to_string()))].into();
    assert!(store_card(&s, plan(), only_cvc, "https://cards.example", None).unwrap_err().contains("needs its number"));
    let blank: BTreeMap<String, Zeroizing<String>> = [("number".to_string(), Zeroizing::new("   ".to_string()))].into();
    assert!(store_card(&s, plan(), blank, "https://cards.example", None).unwrap_err().contains("is empty"));
    assert!(s.vault_list().unwrap().iter().all(|r| r.kind != "card"), "nothing was stored");
}
