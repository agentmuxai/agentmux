// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

const NUMBER: &str = "4242 4242 4242 4242";

#[test]
fn the_card_the_person_typed_is_settled_like_any_card_and_empty_parts_are_left_out() {
    let sent = format!(r#"{{"number": "{NUMBER}", "exp": "03/30", "cvc": "123", "name": "  Alex Doe ", "zip": ""}}"#);
    let (parts, _) = typed_parts(&sent).unwrap();
    let get = |k: &str| parts.0.get(k).map(String::as_str);
    assert_eq!(get("number"), Some("4242424242424242"), "the digits");
    assert_eq!((get("exp_month"), get("exp_year")), (Some("03"), Some("2030")), "the expiry text is a month and a year");
    assert_eq!(get("expiry"), None, "the older single expiry is read into them");
    assert_eq!((get("cvc"), get("name"), get("zip")), (Some("123"), Some("Alex Doe"), None), "trimmed, and an empty ZIP is no ZIP");
}

#[test]
fn a_card_form_that_sends_no_number_a_bad_month_or_something_else_is_refused_without_the_value() {
    let refused = |sent: &str| match typed_parts(sent) {
        Err(e) => e,
        Ok(_) => panic!("a card form that should be refused was taken"),
    };
    assert!(refused(r#"{"cvc": "123"}"#).contains("needs its number"));
    assert!(refused(r#"{"number": "  "}"#).contains("needs its number"));
    assert!(refused(r#"{"number": "4242 4242 4242 4242", "pin": "1234"}"#).contains("a part a card doesn't have"));
    let month = refused(r#"{"number": "4242 4242 4242 4242", "exp": "13/30"}"#);
    assert!(!month.contains("4242") && !month.contains("13/30"), "no value in {month}");
    let junk = refused("number=4242 4242 4242 4242");
    assert_eq!(junk, "the card form didn't send a card", "an unreadable text is not quoted");
}

#[test]
fn the_question_is_the_persons_own_card_form_for_the_parts_the_page_asks_for_and_never_offers_always() {
    let form = CardForm { origin: "https://js.stripe.com".into(), page: "https://shop.example".into(), grant_key: "https://js.stripe.com on https://shop.example".into(), fields: Vec::new(), names: vec!["number", "expiry", "cvc"] };
    let q = typed_question("Agent2-6", "pane-1".into(), "Alex's card", &form, "pay for the order");
    assert_eq!((q.kind.as_str(), q.needs_value, q.offer_always), ("card", true, false));
    assert_eq!((q.entry_label.as_str(), q.what.as_str()), ("Alex's card", "card"));
    assert_eq!((q.origin.as_str(), q.page.as_str()), ("https://js.stripe.com", "https://shop.example"));
    assert_eq!(q.field, "the card fields: number, expiry, cvc");
    assert!(q.entry_id.is_empty() && q.choices.is_none(), "no saved card is named or offered");
    assert!(!uses::goes_ahead(&q), "silence never fills a card");
}
