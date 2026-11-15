// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A drone's API call given a Vault entry: asked once per drone, site and part, never an entry the
//! drone may not use or a part a header can't carry, and the value cleaned from what comes back.

use super::*;
use crate::backend::vault::seal;
use crate::backend::vault::uses::{self, Decision};
use crate::backend::vault::vaults::{self, VaultAgents};
use crate::backend::vault::{create, create_in, NewEntry};

const VALUE: &str = "sk_live_abcdefgh1234";
const SITE: &str = "https://api.example.com";

struct Setup {
    state: AppState,
    drone: String,
    entry: String,
}

fn new_entry(kind: &str, parts: &[(&str, &str)]) -> NewEntry {
    NewEntry {
        kind: kind.into(),
        label: format!("Acme {}", uuid::Uuid::new_v4().simple()),
        fields: Default::default(),
        tags: vec![],
        parts: seal::Parts(parts.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()),
    }
}

async fn setup() -> Setup {
    seal::use_test_key();
    let state = crate::server::tests::test_state();
    let entry = create(&state.identity_store, new_entry("api-key", &[("value", VALUE)])).unwrap();
    Setup { state, drone: format!("nightly-{}", uuid::Uuid::new_v4().simple()), entry }
}

/// Answers the drone's waiting use as the person would, and says what it showed.
async fn answer_when_asked(drone: String, decision: Decision) -> UseRequest {
    let who = actor(&drone);
    for _ in 0..200 {
        if let Some(u) = uses::uses().list().into_iter().find(|u| u.agent == who) {
            uses::uses().answer(&u.id, uses::UseAnswer::of(decision));
            return u;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the use was never asked about");
}

fn activity(s: &Setup) -> String {
    s.state.identity_store.vault_activity_list(50).unwrap().iter().map(|a| format!("{} {} {} {}", a.actor, a.action, a.target, a.outcome)).collect::<Vec<_>>().join("\n")
}

#[tokio::test]
async fn a_first_call_asks_naming_the_drone_and_the_site_and_then_gives_the_value() {
    let s = setup().await;
    let ask = tokio::spawn(answer_when_asked(s.drone.clone(), Decision::Once));
    let got = resolve(&s.state, &s.drone, &s.entry, SITE).await.ok().expect("allowed");
    let shown = ask.await.unwrap();
    assert_eq!((shown.agent.as_str(), shown.origin.as_str(), shown.kind.as_str()), (actor(&s.drone).as_str(), SITE, "api-key"));
    assert!(shown.offer_always && !shown.needs_value && shown.command.is_empty());
    assert!(shown.reason.contains(&s.drone) && shown.reason.contains(SITE), "{}", shown.reason);
    assert_eq!(got.value.as_str(), VALUE);
    // What comes back is cleaned of it, and the log says a drone's API call used it, without it.
    assert_eq!(got.scrubber.scrub(&format!("you sent {VALUE}")).as_ref(), format!("you sent [vault:{}]", s.entry));
    let log = activity(&s);
    assert!(log.contains(&format!("{} called an API {SITE} done", actor(&s.drone))) && !log.contains(VALUE), "{log}");
    // Once is once: the next call asks again.
    let ask = tokio::spawn(answer_when_asked(s.drone.clone(), Decision::Once));
    assert!(resolve(&s.state, &s.drone, &s.entry, SITE).await.is_ok());
    ask.await.unwrap();
}

#[tokio::test]
async fn always_is_one_drones_grant_for_one_site_and_asks_nothing_again_there() {
    let s = setup().await;
    let ask = tokio::spawn(answer_when_asked(s.drone.clone(), Decision::Always));
    assert!(resolve(&s.state, &s.drone, &s.entry, SITE).await.is_ok());
    ask.await.unwrap();
    let store = &s.state.identity_store;
    assert!(store.vault_grant_check(&s.entry, &grant_key(&s.drone, SITE, "value")).unwrap());
    assert!(!store.vault_grant_check(&s.entry, &grant_key("another-drone", SITE, "value")).unwrap(), "a grant is one drone's");
    assert!(!store.vault_grant_check(&s.entry, &grant_key(&s.drone, "https://api.example.org", "value")).unwrap(), "a grant is one site's");
    // Nobody answers now, and it still goes.
    let again = tokio::time::timeout(Duration::from_secs(5), resolve(&s.state, &s.drone, &s.entry, SITE)).await.expect("didn't wait for an answer");
    assert!(again.is_ok());
    // The same entry for another site, or by another drone, asks.
    let ask = tokio::spawn(answer_when_asked(s.drone.clone(), Decision::Deny));
    let other_site = resolve(&s.state, &s.drone, &s.entry, "https://api.example.org").await.err().expect("asked, and denied");
    ask.await.unwrap();
    assert!(other_site.contains("said no") && other_site.contains(NOT_SENT), "{other_site}");
    let other = format!("{}-b", s.drone);
    let ask = tokio::spawn(answer_when_asked(other.clone(), Decision::Deny));
    assert!(resolve(&s.state, &other, &s.entry, SITE).await.is_err());
    ask.await.unwrap();
}

#[tokio::test]
async fn a_no_reads_nothing_and_is_logged() {
    let s = setup().await;
    let ask = tokio::spawn(answer_when_asked(s.drone.clone(), Decision::Deny));
    let err = resolve(&s.state, &s.drone, &s.entry, SITE).await.err().expect("denied");
    ask.await.unwrap();
    assert!(err.contains("said no") && !err.contains(VALUE), "{err}");
    let log = activity(&s);
    assert!(log.contains(&format!("{} called an API {SITE} denied", actor(&s.drone))) && !log.contains(VALUE), "{log}");
    assert!(!s.state.identity_store.vault_grant_check(&s.entry, &grant_key(&s.drone, SITE, "value")).unwrap());
}

#[tokio::test]
async fn a_login_gives_its_password_and_nothing_else_of_it() {
    seal::use_test_key();
    let state = crate::server::tests::test_state();
    let login = create(&state.identity_store, new_entry("login", &[("password", "hunter2-hunter2"), ("totp", "JBSWY3DPEHPK3PXP")])).unwrap();
    let drone = format!("nightly-{}", uuid::Uuid::new_v4().simple());
    let ask = tokio::spawn(answer_when_asked(drone.clone(), Decision::Once));
    let got = resolve(&state, &drone, &login, SITE).await.ok().expect("the password");
    ask.await.unwrap();
    assert_eq!(got.value.as_str(), "hunter2-hunter2");
    // The one-time code isn't something a header carries: refused before anyone is asked.
    let err = resolve(&state, &drone, &format!("{login}.totp"), SITE).await.err().expect("refused");
    assert!(err.contains("totp") && !err.contains("JBSWY3DPEHPK3PXP"), "{err}");
    assert!(uses::uses().list().into_iter().all(|u| u.agent != actor(&drone)), "nobody was asked");
}

#[tokio::test]
async fn an_entry_that_is_not_a_key_or_a_login_is_refused_before_anyone_is_asked() {
    seal::use_test_key();
    let state = crate::server::tests::test_state();
    let note = create(&state.identity_store, new_entry("note", &[("text", "the vault door code is 424242")])).unwrap();
    let drone = format!("nightly-{}", uuid::Uuid::new_v4().simple());
    let err = resolve(&state, &drone, &note, SITE).await.err().expect("refused");
    assert!(!err.contains("424242"), "{err}");
    assert!(uses::uses().list().into_iter().all(|u| u.agent != actor(&drone)), "nobody was asked");
}

#[tokio::test]
async fn a_vault_closed_to_agents_is_not_there_for_a_drone() {
    seal::use_test_key();
    let state = crate::server::tests::test_state();
    let closed = vaults::create(&state.identity_store, "Closed", VaultAgents::None).unwrap();
    let entry = create_in(&state.identity_store, new_entry("api-key", &[("value", VALUE)]), &closed.id).unwrap();
    let drone = format!("nightly-{}", uuid::Uuid::new_v4().simple());
    let err = resolve(&state, &drone, &entry, SITE).await.err().expect("refused");
    assert!(err.contains("no Vault entry") && !err.contains(VALUE), "{err}");
    assert!(uses::uses().list().into_iter().all(|u| u.agent != actor(&drone)), "nobody was asked");
}

#[tokio::test]
async fn a_value_too_short_to_clean_from_the_response_is_not_sent() {
    seal::use_test_key();
    let state = crate::server::tests::test_state();
    let short = create(&state.identity_store, new_entry("api-key", &[("value", "abc")])).unwrap();
    let drone = format!("nightly-{}", uuid::Uuid::new_v4().simple());
    let ask = tokio::spawn(answer_when_asked(drone.clone(), Decision::Once));
    let err = resolve(&state, &drone, &short, SITE).await.err().expect("refused");
    ask.await.unwrap();
    assert!(err.contains("too short") && !err.contains("abc\""), "{err}");
}
