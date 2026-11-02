// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::obj::Block;
use crate::backend::vault::signins::{Space, PERSONAL};

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

fn listed(state: &AppState, block_id: &str) -> Value {
    state.mstore.get::<Block>(block_id).unwrap().unwrap().meta.get(SIGNINS_META_KEY).cloned().unwrap_or(Value::Null)
}

fn space(profile: &str, origin: &str) -> Space {
    Space { profile: profile.into(), origin: origin.into(), realm: String::new(), proxy: false }
}

#[test]
fn a_sign_in_is_typed_only_over_https_or_on_this_computer() {
    for ok in ["https://accounts.example.com", "http://localhost", "http://localhost:3000", "http://127.0.0.1:8080", "http://[::1]:5173"] {
        assert!(fillable(ok), "{ok}");
    }
    for bad in ["http://accounts.example.com", "http://localhost.example.com", "http://203.0.113.4", "ftp://localhost", ""] {
        assert!(!fillable(bad), "{bad}");
    }
}

#[tokio::test]
async fn a_form_lists_the_profiles_sign_ins_for_exactly_its_site_and_a_navigation_takes_them_down() {
    let s = state();
    let site = format!("https://t{}.example.com", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let id = signins::save(&s.identity_store, &space(PERSONAL, &site), "alex@example.com", "correct horse").unwrap();
    signins::save(&s.identity_store, &space("profile:p-0123456789ab", &site), "sam", "battery staple").unwrap();
    signins::save(&s.identity_store, &space(PERSONAL, "https://other.example.com"), "kim", "staple battery").unwrap();
    let p = pane(&s, None);
    form(&s, &p, &site);
    let list = listed(&s, &p);
    assert_eq!(list.as_array().map(Vec::len), Some(1), "{list}");
    assert_eq!((list[0]["entry"].as_str(), list[0]["origin"].as_str(), list[0]["user"].as_str()), (Some(id.as_str()), Some(site.as_str()), Some("al***")));
    assert!(!list.to_string().contains("correct horse"));
    navigated(&s, &p);
    assert_eq!(listed(&s, &p), Value::Null);
}

#[tokio::test]
async fn a_page_over_plain_http_or_in_incognito_lists_nothing() {
    let s = state();
    signins::save(&s.identity_store, &space(PERSONAL, "http://intranet.example.com"), "alex", "correct horse").unwrap();
    let p = pane(&s, None);
    form(&s, &p, "http://intranet.example.com");
    assert_eq!(listed(&s, &p), Value::Null);
    let q = pane(&s, Some("incognito:aaaaaaaa-1111"));
    form(&s, &q, "https://accounts.example.com");
    assert_eq!(listed(&s, &q), Value::Null);
}

#[tokio::test]
async fn a_fill_takes_only_a_sign_in_of_this_profile_for_a_site_the_page_shows() {
    let s = state();
    let store = &s.identity_store;
    let site = "https://accounts.example.com".to_string();
    let mine = signins::save(store, &space(PERSONAL, &site), "alex", "correct horse").unwrap();
    let work = signins::save(store, &space("profile:p-0123456789ab", &site), "sam", "battery staple").unwrap();
    let elsewhere = signins::save(store, &space(PERSONAL, "https://evil.example.net"), "alex", "staple battery").unwrap();
    let (origin, user, password, _) = signins::for_fill(store, &mine, PERSONAL, std::slice::from_ref(&site)).unwrap();
    assert_eq!((origin.as_str(), user.as_str(), password.as_str()), (site.as_str(), "alex", "correct horse"));
    assert!(signins::for_fill(store, &work, PERSONAL, std::slice::from_ref(&site)).is_err(), "another profile's");
    assert!(signins::for_fill(store, &elsewhere, PERSONAL, std::slice::from_ref(&site)).is_err(), "another site's");
    assert!(signins::for_fill(store, &mine, PERSONAL, &[]).is_err(), "no form on the page");
}

#[tokio::test]
async fn an_http_sign_in_is_not_a_form_sign_in() {
    let s = state();
    let store = &s.identity_store;
    let site = "https://intranet.example.com".to_string();
    signins::save(store, &Space { realm: "Admin".into(), ..space(PERSONAL, &site) }, "alex", "correct horse").unwrap();
    assert!(signins::offered_for(store, PERSONAL, std::slice::from_ref(&site)).unwrap().is_empty());
}

#[tokio::test]
async fn every_list_and_form_is_cleared_when_the_host_registers() {
    let s = state();
    let site = format!("https://t{}.example.com", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    signins::save(&s.identity_store, &space(PERSONAL, &site), "alex@example.com", "correct horse").unwrap();
    let (live, stale) = (pane(&s, None), pane(&s, None));
    form(&s, &live, &site);
    // As after srv restarts: the meta is saved with the pane, the forms are gone.
    form(&s, &stale, &site);
    FORMS.lock().remove(&stale);
    assert!(listed(&s, &stale).is_array());
    reset_lists(&s);
    assert!(listed(&s, &stale).is_null(), "a key button for a page srv knows nothing of goes");
    // A host that restarted has new pages and doesn't know to say they moved on.
    assert!(listed(&s, &live).is_null() && !FORMS.lock().contains_key(&live), "forms the old host reported go too");
}

#[tokio::test]
async fn a_sign_in_saved_or_deleted_while_its_page_is_open_shows_in_the_list_at_once() {
    let s = state();
    let site = format!("https://t{}.example.com", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let p = pane(&s, None);
    form(&s, &p, &site);
    assert!(listed(&s, &p).is_null(), "nothing saved yet");
    let id = signins::save(&s.identity_store, &space(PERSONAL, &site), "alex@example.com", "correct horse").unwrap();
    crate::server::vault_handlers::publish(&s);
    assert_eq!(listed(&s, &p)[0]["entry"], json!(id));
    crate::backend::vault::delete(&s.identity_store, &id).unwrap();
    crate::server::vault_handlers::publish(&s);
    assert!(listed(&s, &p).is_null(), "a deleted sign-in goes from the menu");
}
