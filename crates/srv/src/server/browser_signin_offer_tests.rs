// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::obj::Block;

/// A site of its own for each test: the one-offer-per-site limit is shared.
fn site() -> String {
    format!("https://t{}.example.com", &uuid::Uuid::new_v4().simple().to_string()[..8])
}

fn state() -> AppState {
    crate::backend::vault::seal::use_test_key();
    crate::server::tests::test_state()
}

fn pane(state: &AppState, identity: Option<&str>) -> String {
    let mut block = Block { oid: uuid::Uuid::new_v4().to_string(), ..Default::default() };
    block.meta.insert("view".into(), json!("browser"));
    if let Some(i) = identity {
        block.meta.insert(crate::server::browser_identity::IDENTITY_META_KEY.into(), json!(i));
    }
    state.mstore.insert(&mut block).unwrap();
    block.oid
}

fn shown(state: &AppState, block_id: &str) -> Value {
    state.mstore.get::<Block>(block_id).unwrap().unwrap().meta.get(OFFER_META_KEY).cloned().unwrap_or(Value::Null)
}

fn z(s: &str) -> Zeroizing<String> {
    Zeroizing::new(s.to_string())
}

fn sign_in(state: &AppState, block_id: &str, site: &str, user: &str, password: &str) {
    offer(state, block_id, site, z(user), Some(z(password)));
}

fn personal(origin: &str) -> Space {
    Space { profile: signins::PERSONAL.into(), origin: origin.into(), realm: String::new(), proxy: false }
}

#[tokio::test]
async fn an_offer_shows_once_the_page_moves_on_with_only_the_origin_and_a_masked_user() {
    let s = state();
    let site = site();
    let p = pane(&s, None);
    sign_in(&s, &p, &site, "alex@example.com", "correct horse");
    assert_eq!(shown(&s, &p), Value::Null, "not before the sign-in went through");
    settled(&s, &p);
    let offer = shown(&s, &p);
    assert_eq!((offer["origin"].as_str(), offer["user"].as_str(), offer["update"].as_bool()), (Some(site.as_str()), Some("al***"), Some(false)));
    assert!(!offer.to_string().contains("correct horse") && !offer.to_string().contains("alex@"));
}

#[tokio::test]
async fn save_keeps_it_in_the_tabs_profile_and_takes_the_offer_down() {
    let s = state();
    let site = site();
    let p = pane(&s, Some("profile:p-0123456789ab"));
    sign_in(&s, &p, &site, "alex", "correct horse");
    settled(&s, &p);
    let id = shown(&s, &p)["id"].as_str().unwrap().to_string();
    answer(&s, &p, &id, Answer::Save).unwrap();
    assert_eq!(shown(&s, &p), Value::Null);
    let work = Space { profile: "profile:p-0123456789ab".into(), ..personal(&site) };
    assert_eq!(signins::saved_as(&s.identity_store, &work, "alex", "correct horse").unwrap(), Some(true));
    assert_eq!(signins::saved_as(&s.identity_store, &personal(&site), "alex", "correct horse").unwrap(), None);
    assert!(answer(&s, &p, &id, Answer::Save).is_err(), "an offer is answered once");
}

#[tokio::test]
async fn a_late_answer_to_an_offer_that_went_leaves_the_newer_one_up() {
    let s = state();
    let site = site();
    let p = pane(&s, None);
    sign_in(&s, &p, &site, "alex", "correct horse");
    settled(&s, &p);
    let id = shown(&s, &p)["id"].as_str().unwrap().to_string();
    assert!(answer(&s, &p, "an-offer-that-went", Answer::NotNow).is_err());
    assert_eq!(shown(&s, &p)["id"].as_str(), Some(id.as_str()), "the offer still up stays up");
}

#[tokio::test]
async fn a_bar_whose_sign_in_srv_no_longer_holds_goes_at_its_first_answer() {
    let s = state();
    let p = pane(&s, None);
    // As after an srv restart: the bar is in the pane's meta, the pair is gone.
    show(&s, &p, json!({ "id": "from-before-a-restart", "origin": site(), "user": "al***", "update": false }));
    assert!(answer(&s, &p, "from-before-a-restart", Answer::NotNow).is_err());
    assert_eq!(shown(&s, &p), Value::Null);
}

#[tokio::test]
async fn a_new_sign_in_in_the_pane_takes_the_bar_of_the_one_it_replaces_down() {
    let s = state();
    let site = site();
    let p = pane(&s, None);
    sign_in(&s, &p, &site, "alex", "correct horse");
    settled(&s, &p);
    assert!(shown(&s, &p)["id"].is_string());
    sign_in(&s, &p, &site, "alex", "battery staple");
    assert_eq!(shown(&s, &p), Value::Null, "its pair is gone, so its buttons would do nothing");
}

#[tokio::test]
async fn a_wrong_passwords_update_bar_goes_when_the_saved_one_is_used_after_it() {
    let s = state();
    let site = site();
    let p = pane(&s, None);
    signins::save(&s.identity_store, &personal(&site), "alex", "correct horse").unwrap();
    sign_in(&s, &p, &site, "alex", "a wrong password");
    settled(&s, &p);
    let wrong = shown(&s, &p)["id"].as_str().unwrap().to_string();
    assert_eq!(shown(&s, &p)["update"], json!(true));
    // The person signs in again with the saved password: nothing to offer.
    sign_in(&s, &p, &site, "alex", "correct horse");
    assert_eq!(shown(&s, &p), Value::Null);
    assert!(answer(&s, &p, &wrong, Answer::Save).is_err(), "the wrong one can't be saved over the right one");
    assert_eq!(signins::saved_as(&s.identity_store, &personal(&site), "alex", "correct horse").unwrap(), Some(true));
}

#[tokio::test]
async fn incognito_never_offers() {
    let s = state();
    let site = site();
    let p = pane(&s, Some("incognito:aaaaaaaa-1111"));
    sign_in(&s, &p, &site, "alex", "correct horse");
    settled(&s, &p);
    assert_eq!(shown(&s, &p), Value::Null);
}

#[tokio::test]
async fn never_for_this_site_stops_the_offers_there_and_nowhere_else() {
    let s = state();
    let site = site();
    let p = pane(&s, None);
    sign_in(&s, &p, &site, "alex", "correct horse");
    settled(&s, &p);
    let id = shown(&s, &p)["id"].as_str().unwrap().to_string();
    answer(&s, &p, &id, Answer::Never).unwrap();
    let q = pane(&s, None);
    sign_in(&s, &q, &site, "sam", "battery staple");
    settled(&s, &q);
    assert_eq!(shown(&s, &q), Value::Null);
    let other = self::site();
    offer(&s, &q, &other, z("sam"), Some(z("battery staple")));
    settled(&s, &q);
    assert_eq!(shown(&s, &q)["origin"], other.as_str());
}

#[tokio::test]
async fn a_saved_sign_in_is_not_offered_again_and_a_new_password_is_an_update() {
    let s = state();
    let site = site();
    signins::save(&s.identity_store, &personal(&site), "alex", "correct horse").unwrap();
    let p = pane(&s, None);
    sign_in(&s, &p, &site, "alex", "correct horse");
    settled(&s, &p);
    assert_eq!(shown(&s, &p), Value::Null);
    sign_in(&s, &p, &site, "alex", "battery staple");
    settled(&s, &p);
    assert_eq!(shown(&s, &p)["update"], true);
}

#[tokio::test]
async fn a_user_name_sent_alone_is_joined_to_the_password_step() {
    let s = state();
    let site = site();
    let p = pane(&s, None);
    offer(&s, &p, &site, z("alex@example.com"), None);
    sign_in(&s, &p, &site, "", "correct horse");
    settled(&s, &p);
    assert_eq!(shown(&s, &p)["user"], "al***");
}

#[tokio::test]
async fn the_offer_goes_with_the_next_navigation_but_not_with_the_sign_in_finishing() {
    let s = state();
    let site = site();
    let p = pane(&s, None);
    sign_in(&s, &p, &site, "alex", "correct horse");
    settled(&s, &p);
    settled(&s, &p);
    assert_ne!(shown(&s, &p), Value::Null, "a redirect right after is the same sign-in");
    OFFERS.lock().held.get_mut(&p).unwrap().shown_at = Some(Instant::now() - SETTLING);
    settled(&s, &p);
    assert_eq!(shown(&s, &p), Value::Null);
}

#[tokio::test]
async fn a_site_shows_one_offer_at_a_time() {
    let s = state();
    let site = site();
    let (p, q) = (pane(&s, None), pane(&s, None));
    sign_in(&s, &p, &site, "alex", "correct horse");
    settled(&s, &p);
    sign_in(&s, &q, &site, "sam", "battery staple");
    settled(&s, &q);
    assert_eq!(shown(&s, &q), Value::Null);
}

#[test]
fn an_answer_is_save_never_or_not_now() {
    assert_eq!(Answer::parse("save"), Some(Answer::Save));
    assert_eq!(Answer::parse("never"), Some(Answer::Never));
    assert_eq!(Answer::parse("not_now"), Some(Answer::NotNow));
    assert_eq!(Answer::parse("yes"), None);
}

#[tokio::test(start_paused = true)]
async fn a_held_sign_in_and_a_lone_user_name_go_after_two_minutes_shown_or_not() {
    let s = state();
    let site = site();
    let (p, q) = (pane(&s, None), pane(&s, None));
    // A failed sign-in leaves the password field up: nothing settles, no offer shows.
    sign_in(&s, &p, &site, "alex", "correct horse");
    offer(&s, &q, &site, z("sam"), None);
    assert!(OFFERS.lock().held.contains_key(&p) && OFFERS.lock().identifiers.contains_key(&q));
    tokio::time::sleep(HOLD + Duration::from_secs(1)).await;
    assert!(!OFFERS.lock().held.contains_key(&p), "the password isn't kept past two minutes");
    assert!(!OFFERS.lock().identifiers.contains_key(&q));
}

#[tokio::test]
async fn a_failed_answer_keeps_the_sign_in_only_within_its_two_minutes() {
    let s = state();
    let site = site();
    let p = pane(&s, None);
    sign_in(&s, &p, &site, "alex", "correct horse");
    // As `answer` does: the pair is out of the map while the answer is tried.
    let held = OFFERS.lock().held.remove(&p).unwrap();
    keep_after_failure(&s, &p, held);
    assert!(OFFERS.lock().held.contains_key(&p), "a failed save keeps it, to try again");

    let mut held = OFFERS.lock().held.remove(&p).unwrap();
    held.at = Instant::now() - HOLD - Duration::from_secs(1);
    keep_after_failure(&s, &p, held);
    assert!(!OFFERS.lock().held.contains_key(&p), "its expiry has run: nothing would drop it again");
}
