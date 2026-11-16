// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::super::signins::PERSONAL;
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

fn saved(s: &Store, user: &str, password: &str) -> VaultRow {
    let id = signins::save(s, &space(), user, password).unwrap();
    s.vault_get(&id).unwrap().unwrap()
}

#[test]
fn the_pending_password_is_not_the_entrys_until_the_site_was_sent_it() {
    let s = store();
    let row = saved(&s, "alex", "the one that works");
    let change = start(&s, &row, "made-for-the-change", "Agent1").unwrap();
    assert!(current_is(&s, &row.id, "the one that works"), "the saved password stays the one that works");
    assert_eq!(change.by, "Agent1");
    assert_eq!(confirm_sent(&s, &space(), "typed-by-hand").unwrap(), None, "any other password is the person's own");
    assert!(current_is(&s, &row.id, "the one that works"));
    assert_eq!(confirm_sent(&s, &space(), "made-for-the-change").unwrap().as_deref(), Some(row.id.as_str()));
    assert!(current_is(&s, &row.id, "made-for-the-change"));
    let kept = versions::list(&s, &row.id).unwrap();
    assert_eq!(kept.len(), 1, "the old password is kept for 7 days");
    assert_eq!((kept[0].state.as_str(), kept[0].how.as_str(), kept[0].replaced_by.as_str()), (versions::PREVIOUS, HOW, "Agent1"));
    assert_eq!(confirm_sent(&s, &space(), "made-for-the-change").unwrap(), None, "once only");
}

#[test]
fn a_change_that_was_never_sent_leaves_the_password_alone_and_a_new_one_sets_it_aside() {
    let s = store();
    let row = saved(&s, "alex", "the one that works");
    start(&s, &row, "first-try", "Agent1").unwrap();
    assert!(start(&s, &row, "second-try", "Agent1").is_ok(), "asking again replaces the change that was never sent");
    let states: Vec<String> = versions::list(&s, &row.id).unwrap().into_iter().map(|v| v.state).collect();
    assert!(states.contains(&versions::INTERRUPTED.to_string()) && states.contains(&versions::PENDING.to_string()), "{states:?}");
    assert_eq!(confirm_sent(&s, &space(), "first-try").unwrap(), None, "the one set aside is never made current by a form");
    assert!(current_is(&s, &row.id, "the one that works"));
    assert!(confirm_sent(&s, &space(), "second-try").unwrap().is_some());
}

#[test]
fn a_change_is_for_the_site_it_was_made_for_in_the_profile_it_was_made_for() {
    let s = store();
    let row = saved(&s, "alex", "the one that works");
    start(&s, &row, "made-for-the-change", "Agent1").unwrap();
    let other_site = Space { origin: "https://elsewhere.example.com".into(), ..space() };
    let other_profile = Space { profile: "profile:other".into(), ..space() };
    assert_eq!(confirm_sent(&s, &other_site, "made-for-the-change").unwrap(), None);
    assert_eq!(confirm_sent(&s, &other_profile, "made-for-the-change").unwrap(), None);
    assert!(current_is(&s, &row.id, "the one that works"));
}

#[test]
fn the_saved_sign_in_is_found_by_user_name_or_as_the_sites_only_one() {
    let s = store();
    let alex = saved(&s, "alex", "pw-a");
    assert_eq!(saved_login(&s, &space(), "", "Agent1").unwrap().id, alex.id, "the only one needs no user name");
    assert_eq!(saved_login(&s, &space(), "alex", "Agent1").unwrap().id, alex.id);
    assert!(saved_login(&s, &space(), "sam", "Agent1").unwrap_err().contains("no saved sign-in"));
    let sam = saved(&s, "sam", "pw-s");
    assert_eq!(saved_login(&s, &space(), "sam", "Agent1").unwrap().id, sam.id);
    let err = saved_login(&s, &space(), "", "Agent1").unwrap_err();
    assert!(err.contains("2 sign-ins") && err.contains("give username") && !err.contains("alex") && !err.contains("sam"), "{err}");
}

#[test]
fn a_sign_up_that_was_never_sent_is_not_a_password_to_change() {
    let s = store();
    super::super::signups::create(&s, &space(), "alex", "made-on-sign-up").unwrap();
    assert!(saved_login(&s, &space(), "alex", "Agent1").unwrap_err().contains("no saved sign-in"));
}

#[test]
fn a_change_the_site_took_makes_a_sign_in_it_had_turned_down_offered_again() {
    let s = store();
    let row = saved(&s, "alex", "old");
    let mut tags = signins::tags_of(&row);
    tags.push(signins::REFUSED.to_string());
    super::super::update(&s, EntryChange { id: row.id.clone(), label: row.label.clone(), fields: signins::fields_of(&row), tags, parts: Parts::default(), clear: Vec::new() }).unwrap();
    let row = s.vault_get(&row.id).unwrap().unwrap();
    assert!(!signins::offered(&row));
    start(&s, &row, "new", "Agent1").unwrap();
    confirm_sent(&s, &space(), "new").unwrap().unwrap();
    assert!(signins::offered(&s.vault_get(&row.id).unwrap().unwrap()));
}

#[test]
fn a_change_whose_form_was_never_filled_leaves_nothing_behind() {
    let s = store();
    let row = saved(&s, "alex", "the one that works");
    let change = start(&s, &row, "never-typed", "Agent1").unwrap();
    abandon(&s, &change).unwrap();
    assert!(versions::list(&s, &row.id).unwrap().is_empty());
    assert_eq!(confirm_sent(&s, &space(), "never-typed").unwrap(), None);
}
