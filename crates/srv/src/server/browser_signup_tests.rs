// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::obj::Block;
use crate::backend::vault::{seal, versions};

fn state() -> AppState {
    seal::use_test_key();
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

fn offered(state: &AppState, block_id: &str) -> Value {
    state.mstore.get::<Block>(block_id).unwrap().unwrap().meta.get(SIGNUP_META_KEY).cloned().unwrap_or(Value::Null)
}

/// A site no other test uses: the Vault is shared by the tests running beside this one.
fn site() -> String {
    format!("https://s{}.example.com", &uuid::Uuid::new_v4().simple().to_string()[..8])
}

fn field(origin: &str) -> Field {
    Field { origin: origin.to_string(), ..Default::default() }
}

/// The entries kept for `origin`: (id, user name, tags, password).
fn kept(state: &AppState, origin: &str) -> Vec<(String, String, Vec<String>, String)> {
    let store = &state.identity_store;
    store
        .vault_list()
        .unwrap()
        .into_iter()
        .filter_map(|r| {
            let fields: std::collections::BTreeMap<String, String> = serde_json::from_str(&r.fields).unwrap_or_default();
            (fields.get("site").map(String::as_str) == Some(origin)).then(|| {
                let tags: Vec<String> = serde_json::from_str(&r.tags).unwrap_or_default();
                let password = seal::open(&seal::key(false).unwrap(), &r.id, &r.sealed).unwrap().get("password").unwrap_or("").to_string();
                (r.id.clone(), fields.get("username").cloned().unwrap_or_default(), tags, password)
            })
        })
        .collect()
}

#[tokio::test]
async fn focusing_a_new_password_field_offers_a_password_until_the_page_moves_on() {
    let s = state();
    let (p, origin) = (pane(&s, None), site());
    focused(&s, &p, field(&origin));
    assert_eq!(offered(&s, &p), json!({ "origin": origin }));
    navigated(&s, &p);
    assert_eq!(offered(&s, &p), Value::Null);
}

#[tokio::test]
async fn nothing_is_offered_in_incognito_or_on_a_page_over_plain_http() {
    let s = state();
    let p = pane(&s, None);
    focused(&s, &p, field("http://shop.example.com"));
    assert_eq!(offered(&s, &p), Value::Null);
    let q = pane(&s, Some("incognito:aaaaaaaa-1111"));
    focused(&s, &q, field(&site()));
    assert_eq!(offered(&s, &q), Value::Null);
    assert!(make(&s, &q).await.is_err());
}

#[test]
fn the_password_keeps_to_the_fields_limits() {
    let at_most = Field { maxlength: Some(16), ..field("https://a.example.com") };
    assert_eq!(new_password(&at_most).unwrap().chars().count(), 16);
    let at_least = Field { minlength: Some(40), ..field("https://a.example.com") };
    assert!(new_password(&at_least).unwrap().chars().count() >= 40);
    let digit = Field { passwordrules: Some("required: digit; allowed: lower, upper;".into()), ..field("https://a.example.com") };
    assert!(new_password(&digit).unwrap().chars().any(|c| c.is_ascii_digit()));
    let unreadable = Field { passwordrules: Some("required: [".into()), maxlength: Some(16), ..field("https://a.example.com") };
    assert_eq!(new_password(&unreadable).unwrap().chars().count(), 16, "rules it can't read are dropped, the length kept");
    let too_short = Field { maxlength: Some(8), ..field("https://a.example.com") };
    assert!(new_password(&too_short).is_err(), "never a weak password because a field is short");
}

#[tokio::test]
async fn the_password_is_in_the_vault_before_the_page_is_given_it_and_asking_again_replaces_it() {
    let s = state();
    let (p, origin) = (pane(&s, None), site());
    focused(&s, &p, Field { username: "alex".into(), ..field(&origin) });
    // No host in a test: the fill fails, after the password is kept.
    let e = make(&s, &p).await.unwrap_err();
    assert!(e.contains("saved in the Vault"), "{e}");
    let first = kept(&s, &origin);
    assert_eq!(first.len(), 1);
    let (id, user, tags, password) = first[0].clone();
    assert_eq!((user.as_str(), tags.as_slice()), ("alex", [signups::UNCONFIRMED.to_string()].as_slice()));
    assert!(password.chars().count() >= generate::DEFAULT_LENGTH);
    assert!(!offered(&s, &p).to_string().contains(&password), "the pane's meta never holds the password");
    let _ = make(&s, &p).await;
    let again = kept(&s, &origin);
    assert_eq!(again.len(), 1, "the site refused it: the same entry gets the new one");
    assert_eq!(again[0].0, id);
    assert_ne!(again[0].3, password);
    assert_eq!(versions::list(&s.identity_store, &id).unwrap().len(), 1, "the refused password is kept as a version");
}

#[tokio::test]
async fn a_password_typed_over_the_made_one_and_saved_is_never_replaced_by_the_next_strong_password() {
    let s = state();
    let (p, origin) = (pane(&s, None), site());
    focused(&s, &p, Field { username: "alex".into(), ..field(&origin) });
    let _ = make(&s, &p).await;
    let (id, ..) = kept(&s, &origin)[0].clone();
    // The person typed their own password over it, sent the form and chose Save.
    sent(&s, &p, &origin, Zeroizing::new("alex".into()), Zeroizing::new("the persons own".into()));
    let space = Space { profile: crate::backend::vault::signins::PERSONAL.into(), origin: origin.clone(), realm: String::new(), proxy: false };
    assert_eq!(crate::backend::vault::signins::save(&s.identity_store, &space, "alex", "the persons own").unwrap(), id);
    let saved = kept(&s, &origin);
    assert_eq!((saved[0].2.len(), saved[0].3.as_str()), (0, "the persons own"), "saved by the person: no longer unconfirmed");
    let _ = make(&s, &p).await;
    let after = kept(&s, &origin);
    assert_eq!(after.len(), 2, "the next strong password is an entry of its own");
    let mine = after.iter().find(|e| e.0 == id).unwrap();
    assert_eq!(mine.3, "the persons own", "the password the person sent is untouched");
}

#[tokio::test]
async fn the_made_password_sent_confirms_its_entry_and_one_typed_by_hand_does_not() {
    let s = state();
    let (p, origin) = (pane(&s, None), site());
    focused(&s, &p, field(&origin));
    let _ = make(&s, &p).await;
    let (id, _, _, password) = kept(&s, &origin)[0].clone();
    sent(&s, &p, &origin, Zeroizing::new("alex".into()), Zeroizing::new("typed by hand".into()));
    assert_eq!(kept(&s, &origin)[0].2, [signups::UNCONFIRMED.to_string()]);
    sent(&s, &p, "https://other.example.com", Zeroizing::new("alex".into()), Zeroizing::new(password.clone()));
    assert_eq!(kept(&s, &origin)[0].2, [signups::UNCONFIRMED.to_string()], "a form of another site confirms nothing");
    sent(&s, &p, &origin, Zeroizing::new("alex".into()), Zeroizing::new(password.clone()));
    let (now_id, user, tags, now_password) = kept(&s, &origin)[0].clone();
    assert_eq!((now_id, user.as_str(), tags.len(), now_password), (id, "alex", 0, password));
}
