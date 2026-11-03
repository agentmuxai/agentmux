// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::super::signins::PERSONAL;
use super::super::versions;
use super::*;

fn store() -> Store {
    seal::use_test_key();
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    s
}

fn space() -> Space {
    Space { profile: PERSONAL.into(), origin: "https://shop.example.com".into(), realm: String::new(), proxy: false }
}

fn tags(s: &Store, id: &str) -> Vec<String> {
    signins::tags_of(&s.vault_get(id).unwrap().unwrap())
}

fn password(s: &Store, id: &str) -> String {
    let row = s.vault_get(id).unwrap().unwrap();
    seal::open(&seal::key(false).unwrap(), &row.id, &row.sealed).unwrap().get("password").unwrap_or("").to_string()
}

#[test]
fn a_sign_up_password_is_an_unconfirmed_entry_of_its_own_never_joined_to_a_saved_one() {
    let s = store();
    let saved = signins::save(&s, &space(), "alex", "the one that works").unwrap();
    let (made, label) = create(&s, &space(), "alex", "made-on-sign-up").unwrap();
    assert_ne!(made, saved, "a sign-up never replaces a saved sign-in, even for the same user name");
    assert_eq!(label, "shop.example.com");
    assert_eq!(password(&s, &saved), "the one that works");
    assert_eq!(password(&s, &made), "made-on-sign-up");
    assert_eq!(tags(&s, &made), [UNCONFIRMED]);
    let fields = signins::fields_of(&s.vault_get(&made).unwrap().unwrap());
    assert_eq!((fields.get("site").map(String::as_str), fields.get("profile").map(String::as_str)), (Some("https://shop.example.com"), Some(PERSONAL)));
}

#[test]
fn asking_again_replaces_the_unconfirmed_password_and_keeps_the_refused_one() {
    let s = store();
    let (made, _) = create(&s, &space(), "", "refused-by-the-site").unwrap();
    assert_eq!(replace(&s, &made, "second-try").unwrap().as_deref(), Some("shop.example.com"));
    assert_eq!(password(&s, &made), "second-try");
    assert_eq!(versions::list(&s, &made).unwrap().len(), 1, "the refused password is a previous version");
    assert_eq!(tags(&s, &made), [UNCONFIRMED], "still unconfirmed until it is sent");
}

#[test]
fn only_the_made_password_sent_confirms_the_entry_and_records_the_user_name() {
    let s = store();
    let (made, _) = create(&s, &space(), "", "made-1").unwrap();
    assert!(!confirm(&s, &made, "alex", "typed-by-hand").unwrap(), "another password sent leaves it unconfirmed");
    assert_eq!(tags(&s, &made), [UNCONFIRMED]);
    assert!(confirm(&s, &made, "alex", "made-1").unwrap());
    assert!(tags(&s, &made).is_empty());
    let fields = signins::fields_of(&s.vault_get(&made).unwrap().unwrap());
    assert_eq!(fields.get("username").map(String::as_str), Some("alex"));
    assert_eq!(password(&s, &made), "made-1");
    assert!(versions::list(&s, &made).unwrap().is_empty(), "confirming keeps the value");
    assert!(!confirm(&s, &made, "alex", "made-1").unwrap(), "once only");
    assert_eq!(replace(&s, &made, "another").unwrap(), None, "a confirmed sign-in is never replaced by a new sign-up password");
    assert_eq!(password(&s, &made), "made-1");
    assert!(!confirm(&s, "no-such-entry", "alex", "made-1").unwrap());
}
