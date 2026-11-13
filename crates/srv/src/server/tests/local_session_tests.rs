// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// A code entered and confirmed: the grant the page collects.
fn signed_in(store: &SessionStore, now: Instant) -> Grant {
    let code = store.mint_code(now);
    let entered = store.enter(&code, now).expect("a fresh code is accepted");
    assert!(store.decide(&code, true, now));
    match store.collect(&entered.handle, now) {
        Collected::Granted(g) => g,
        _ => panic!("a confirmed session is handed over"),
    }
}

#[test]
fn a_used_code_is_refused_on_a_second_entry() {
    let store = SessionStore::default();
    let now = Instant::now();
    let code = store.mint_code(now);
    assert!(store.enter(&code, now).is_some());
    assert!(store.enter(&code, now).is_none());
}

#[test]
fn a_wrong_code_makes_nothing_and_a_code_expires_after_a_minute() {
    let store = SessionStore::default();
    let now = Instant::now();
    assert!(store.enter("not-a-code", now).is_none());
    let code = store.mint_code(now);
    assert!(store.enter(&code, now + CODE_LIFE + secs(1)).is_none());
    let code = store.mint_code(now);
    assert!(store.enter(&code, now + CODE_LIFE).is_some());
}

#[test]
fn a_pending_session_has_no_access_on_any_route_class() {
    let store = SessionStore::default();
    let now = Instant::now();
    let code = store.mint_code(now);
    let entered = store.enter(&code, now).unwrap();
    for class in [RouteClass::Both, RouteClass::CookieOnly] {
        // Nothing the page, the code or the phrase holds is a credential.
        for secret in [entered.handle.as_str(), code.as_str(), entered.phrase.as_str()] {
            assert!(!store.admit(Some(secret), Some(secret), class, now));
            assert!(!store.admit(Some(secret), None, class, now));
        }
    }
    assert_eq!(store.session_count(now), 0);
    assert!(matches!(store.collect(&entered.handle, now), Collected::Waiting));
}

#[test]
fn confirming_makes_only_the_session_of_that_code() {
    let store = SessionStore::default();
    let now = Instant::now();
    let (a, b) = (store.mint_code(now), store.mint_code(now));
    let (ea, eb) = (store.enter(&a, now).unwrap(), store.enter(&b, now).unwrap());
    assert!(store.decide(&a, true, now));
    assert!(matches!(store.collect(&eb.handle, now), Collected::Waiting), "b was not confirmed");
    assert_eq!(store.session_count(now), 1);
    assert!(matches!(store.collect(&ea.handle, now), Collected::Granted(_)));
    // Only the code open minted names a session: nothing else finds one to answer.
    assert!(!store.decide("another code", true, now));
    assert!(!store.decide(&eb.handle, true, now));
    assert_eq!(store.session_count(now), 1);
}

#[test]
fn the_cookie_and_the_token_must_name_one_session() {
    let store = SessionStore::default();
    let now = Instant::now();
    let a = signed_in(&store, now);
    let b = signed_in(&store, now);
    assert!(store.admit(Some(&a.cookie), Some(&a.token), RouteClass::Both, now));
    assert!(store.admit(Some(&b.cookie), Some(&b.token), RouteClass::Both, now));
    assert!(!store.admit(Some(&a.cookie), Some(&b.token), RouteClass::Both, now), "cookie from A with token from B");
    assert!(!store.admit(Some(&b.cookie), Some(&a.token), RouteClass::Both, now));
    assert!(!store.admit(Some(&a.cookie), None, RouteClass::Both, now), "a cookie alone");
    assert!(!store.admit(None, Some(&a.token), RouteClass::Both, now), "a token alone");
    assert!(!store.admit(Some(&a.token), Some(&a.cookie), RouteClass::Both, now), "swapped");
    assert!(store.admit(Some(&a.cookie), None, RouteClass::CookieOnly, now), "the one class that takes the cookie alone");
    assert!(!store.admit(None, Some(&a.token), RouteClass::CookieOnly, now));
}

#[test]
fn a_grant_is_handed_over_once_and_a_discard_ends_the_wait() {
    let store = SessionStore::default();
    let now = Instant::now();
    let code = store.mint_code(now);
    let entered = store.enter(&code, now).unwrap();
    assert!(store.decide(&code, true, now));
    assert!(matches!(store.collect(&entered.handle, now), Collected::Granted(_)));
    assert!(matches!(store.collect(&entered.handle, now), Collected::Refused), "the handle is spent");

    let code = store.mint_code(now);
    let entered = store.enter(&code, now).unwrap();
    assert_eq!(store.code_state(&code, now), CodeState::Entered { phrase: entered.phrase.clone() });
    assert!(store.decide(&code, false, now));
    assert_eq!(store.code_state(&code, now), CodeState::Gone);
    assert!(matches!(store.collect(&entered.handle, now), Collected::Refused));
    assert!(!store.decide(&code, true, now), "an answered code cannot be answered again");
    assert_eq!(store.session_count(now), 1, "only the first sign-in is live");
}

#[test]
fn a_pending_session_dies_after_a_minute_and_takes_an_uncollected_grant_with_it() {
    let store = SessionStore::default();
    let now = Instant::now();
    let code = store.mint_code(now);
    let entered = store.enter(&code, now).unwrap();
    assert!(matches!(store.collect(&entered.handle, now + PENDING_LIFE + secs(1)), Collected::Refused));
    assert!(!store.decide(&code, true, now + PENDING_LIFE + secs(1)));

    let code = store.mint_code(now);
    let entered = store.enter(&code, now).unwrap();
    assert!(store.decide(&code, true, now + secs(30)));
    assert_eq!(store.session_count(now + secs(30)), 1);
    assert!(matches!(store.collect(&entered.handle, now + PENDING_LIFE + secs(1)), Collected::Refused));
    assert_eq!(store.session_count(now + PENDING_LIFE + secs(1)), 0, "nobody collected it, so nobody holds it");
}

#[test]
fn the_code_state_tells_open_what_happened_to_its_code() {
    let store = SessionStore::default();
    let now = Instant::now();
    let code = store.mint_code(now);
    assert_eq!(store.code_state(&code, now), CodeState::Unused);
    assert_eq!(store.code_state("someone else's", now), CodeState::Gone);
    assert_eq!(store.code_state(&code, now + CODE_LIFE + secs(1)), CodeState::Gone);
    let code = store.mint_code(now);
    let entered = store.enter(&code, now).unwrap();
    assert_eq!(store.code_state(&code, now), CodeState::Entered { phrase: entered.phrase });
}

#[test]
fn a_session_ends_after_twelve_idle_hours_and_each_use_slides_the_limit() {
    let store = SessionStore::default();
    let now = Instant::now();
    let g = signed_in(&store, now);
    let hour = |n: u64| secs(n * 3600);
    assert!(store.admit(Some(&g.cookie), Some(&g.token), RouteClass::Both, now + hour(11)));
    assert!(store.admit(Some(&g.cookie), Some(&g.token), RouteClass::Both, now + hour(22)), "slid from hour 11");
    assert!(!store.admit(Some(&g.cookie), Some(&g.token), RouteClass::Both, now + hour(35)), "13 idle hours");
    assert_eq!(store.session_count(now + hour(35)), 0);
}

#[test]
fn revoking_ends_every_session_and_every_pending_sign_in() {
    let store = SessionStore::default();
    let now = Instant::now();
    let (a, b) = (signed_in(&store, now), signed_in(&store, now));
    let code = store.mint_code(now);
    let entered = store.enter(&code, now).unwrap();
    assert_eq!(store.revoke_all(), 2);
    assert!(!store.admit(Some(&a.cookie), Some(&a.token), RouteClass::Both, now));
    assert!(!store.admit(Some(&b.cookie), Some(&b.token), RouteClass::CookieOnly, now));
    assert!(matches!(store.collect(&entered.handle, now), Collected::Refused));
    assert!(store.enter(&store.mint_code(now), now).is_some(), "a new sign-in works after a logout");
}

#[test]
fn held_codes_and_pending_sessions_are_capped() {
    let store = SessionStore::default();
    let now = Instant::now();
    let first = store.mint_code(now);
    for i in 1..=MAX_HELD as u64 {
        store.mint_code(now + Duration::from_millis(i));
    }
    let later = now + Duration::from_millis(MAX_HELD as u64);
    assert!(store.enter(&first, later).is_none(), "the oldest code went first");
}

#[test]
fn failures_are_counted_over_the_last_minute() {
    let store = SessionStore::default();
    let now = Instant::now();
    assert_eq!(store.note_failure(now), 1);
    assert_eq!(store.note_failure(now + secs(30)), 2);
    assert_eq!(store.note_failure(now + secs(70)), 2, "the first is a minute old");
}

#[test]
fn the_check_phrase_words_are_1024_distinct_short_words() {
    let words: std::collections::HashSet<String> = (0..1024).map(phrase_word).collect();
    assert_eq!(words.len(), 1024);
    assert!(words.iter().all(|w| w.len() == 4 && w.chars().all(|c| c.is_ascii_lowercase())));
    let phrase = check_phrase();
    let (a, b) = phrase.split_once('-').expect("two words");
    assert!(words.contains(a) && words.contains(b), "{phrase}");
}

#[test]
fn secrets_are_256_random_bits_and_never_repeat() {
    let a = random_secret();
    assert_eq!(a.len(), 64);
    assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_ne!(a, random_secret());
}
