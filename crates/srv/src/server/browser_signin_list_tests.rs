// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The list of saved sign-ins under a login field: which rows, in which order, filtered by what, what the
//! pane is told, and what a key or a pick does.

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use axum::extract::{Path, State as AxState};
use axum::routing::post;
use axum::{Json, Router};

use super::*;
use crate::backend::obj::Block;
use crate::backend::vault::seal;
use crate::backend::vault::signins::{Space, PERSONAL};

#[derive(Default)]
struct Host {
    seen: StdMutex<Vec<(String, Value)>>,
}

async fn host_route(AxState(h): AxState<Arc<Host>>, Path(route): Path<String>, Json(body): Json<Value>) -> Json<Value> {
    h.seen.lock().unwrap().push((route.clone(), body));
    Json(json!({ "ok": true, "data": { "filled": true, "username_filled": true } }))
}

struct Setup {
    state: AppState,
    host: Arc<Host>,
    block: String,
    site: String,
}

/// A browser pane of `identity` showing a sign-in form of a site of its own, and the host's stand-in.
async fn setup(identity: Option<&str>) -> Setup {
    seal::use_test_key();
    let state = crate::server::tests::test_state();
    let host = Arc::new(Host::default());
    let app = Router::new().route("/agentmux/browser/:route", post(host_route)).with_state(host.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    *state.host_ipc.lock().await = Some(crate::server::state::HostIpc { port, token: "host-token".into() });
    let mut block = Block { oid: uuid::Uuid::new_v4().to_string(), ..Default::default() };
    block.meta.insert("view".into(), json!("browser"));
    if let Some(i) = identity {
        block.meta.insert(crate::server::browser_identity::IDENTITY_META_KEY.into(), json!(i));
    }
    state.mstore.insert(&mut block).unwrap();
    let site = format!("https://s{}.example.com", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    crate::server::browser_signin_fill::form(&state, &block.oid, &site);
    Setup { state, host, block: block.oid, site }
}

fn save(s: &Setup, user: &str) -> String {
    let space = Space { profile: PERSONAL.into(), origin: s.site.clone(), realm: String::new(), proxy: false };
    signins::save(&s.state.identity_store, &space, user, "correct horse").unwrap()
}

/// What the pane is told, as it reads it.
fn meta(s: &Setup) -> Value {
    s.state.mstore.get::<Block>(&s.block).unwrap().unwrap().meta.get(LIST_META_KEY).cloned().unwrap_or(Value::Null)
}

fn users(m: &Value) -> Vec<String> {
    m["rows"].as_array().map(|r| r.iter().map(|x| x["user"].as_str().unwrap_or("").to_string()).collect()).unwrap_or_default()
}

#[tokio::test]
async fn a_password_field_lists_the_sites_sign_ins_most_recently_used_first_then_by_user_name() {
    let s = setup(None).await;
    let (bob, alice, carol) = (save(&s, "bob@example.com"), save(&s, "alice@example.com"), save(&s, "carol@example.com"));
    // bob and alice were never used and tie; carol was used last.
    s.state.identity_store.vault_touch(&carol, agentmux_common::time::now_ms()).unwrap();
    let answer = open(&s.state, &s.block, &s.site, "password", "");
    assert_eq!(answer, Answer { shown: true, cursor: false });
    let m = meta(&s);
    assert_eq!(users(&m), ["ca***", "al***", "bo***"]);
    assert_eq!(m["rows"][0]["entry"], json!(carol));
    assert_eq!((m["rows"][1]["entry"].clone(), m["rows"][2]["entry"].clone()), (json!(alice), json!(bob)));
    assert_eq!((m["role"].as_str(), m["origin"].as_str(), m["strong"].as_bool()), (Some("password"), Some(s.site.as_str()), Some(false)));
}

#[tokio::test]
async fn the_pane_is_told_masked_user_names_and_never_a_full_one_nor_a_password() {
    let s = setup(None).await;
    save(&s, "alice@example.com");
    open(&s.state, &s.block, &s.site, "username", "");
    let text = meta(&s).to_string();
    assert!(!text.contains("alice@example.com") && !text.contains("correct horse"), "{text}");
    assert!(text.contains("al***"));
}

#[tokio::test]
async fn a_user_name_field_filters_by_what_is_typed_as_a_case_insensitive_prefix() {
    let s = setup(None).await;
    save(&s, "Alice@example.com");
    save(&s, "alan@example.com");
    save(&s, "bob@example.com");
    open(&s.state, &s.block, &s.site, "username", "AL");
    let shown = users(&meta(&s));
    assert_eq!(shown.len(), 2, "alice and alan, not bob: {shown:?}");
    assert!(shown.iter().all(|u| u.to_lowercase().starts_with("al")), "{shown:?}");
    // A prefix is not a substring.
    assert_eq!(open(&s.state, &s.block, &s.site, "username", "ice"), Answer::default());
    assert_eq!(meta(&s), Value::Null, "nothing matches: no list at all");
    // A password field is not filtered by anything.
    assert!(open(&s.state, &s.block, &s.site, "password", "zzz").shown);
    assert_eq!(users(&meta(&s)).len(), 3);
}

#[tokio::test]
async fn a_field_for_a_new_password_offers_a_strong_password_and_no_saved_sign_in() {
    let s = setup(None).await;
    save(&s, "alice@example.com");
    assert_eq!(open(&s.state, &s.block, &s.site, "newpassword", ""), Answer { shown: true, cursor: false });
    let m = meta(&s);
    assert_eq!((m["strong"].as_bool(), m["rows"].as_array().map(Vec::len)), (Some(true), Some(0)));
}

#[tokio::test]
async fn no_sign_in_for_the_site_and_no_strong_password_is_no_list() {
    let s = setup(None).await;
    assert_eq!(open(&s.state, &s.block, &s.site, "password", ""), Answer::default());
    assert_eq!(meta(&s), Value::Null);
    assert_eq!(open(&s.state, &s.block, &s.site, "nonsense", ""), Answer::default(), "an unknown role is no list");
}

#[tokio::test]
async fn an_incognito_tab_and_a_site_that_isnt_https_get_no_list() {
    let incognito = setup(Some("incognito:jar-3")).await;
    save(&incognito, "alice@example.com");
    assert_eq!(open(&incognito.state, &incognito.block, &incognito.site, "password", ""), Answer::default());
    assert_eq!(open(&incognito.state, &incognito.block, &incognito.site, "newpassword", ""), Answer::default(), "not even a strong password: it would be kept nowhere");
    let plain = setup(None).await;
    assert_eq!(open(&plain.state, &plain.block, "http://shop.example.com", "password", ""), Answer::default());
}

#[tokio::test]
async fn the_keys_move_the_selection_and_escape_closes() {
    let s = setup(None).await;
    save(&s, "alice@example.com");
    save(&s, "bob@example.com");
    open(&s.state, &s.block, &s.site, "password", "");
    assert_eq!(meta(&s)["cursor"], Value::Null);
    assert_eq!(key(&s.state, &s.block, "ArrowDown"), Answer { shown: true, cursor: true });
    assert_eq!(meta(&s)["cursor"], json!(0));
    key(&s.state, &s.block, "ArrowDown");
    assert_eq!(meta(&s)["cursor"], json!(1));
    key(&s.state, &s.block, "ArrowDown");
    assert_eq!(meta(&s)["cursor"], json!(0), "Down from the last wraps");
    key(&s.state, &s.block, "ArrowUp");
    assert_eq!(meta(&s)["cursor"], json!(1), "Up from the first wraps");
    assert_eq!(key(&s.state, &s.block, "Escape"), Answer::default());
    assert_eq!(meta(&s), Value::Null);
    assert_eq!(key(&s.state, &s.block, "ArrowDown"), Answer::default(), "a key with no list does nothing");
}

#[tokio::test]
async fn up_with_nothing_selected_starts_at_the_last_row() {
    let s = setup(None).await;
    save(&s, "alice@example.com");
    save(&s, "bob@example.com");
    open(&s.state, &s.block, &s.site, "password", "");
    key(&s.state, &s.block, "ArrowUp");
    assert_eq!(meta(&s)["cursor"], json!(1));
}

#[tokio::test]
async fn enter_with_a_row_selected_types_that_sign_in_and_closes_the_list() {
    let s = setup(None).await;
    save(&s, "alice@example.com");
    open(&s.state, &s.block, &s.site, "password", "");
    // Enter with nothing selected is not a pick.
    assert_eq!(key(&s.state, &s.block, "Enter"), Answer { shown: true, cursor: false });
    assert!(s.host.seen.lock().unwrap().is_empty());
    key(&s.state, &s.block, "ArrowDown");
    assert_eq!(key(&s.state, &s.block, "Enter"), Answer::default());
    assert_eq!(meta(&s), Value::Null);
    let mut typed = None;
    for _ in 0..80 {
        typed = s.host.seen.lock().unwrap().iter().find(|(r, _)| r == "signin_fill").map(|(_, b)| b.clone());
        if typed.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let typed = typed.expect("the host was asked to type the picked sign-in");
    assert_eq!((typed["origin"].as_str(), typed["username"].as_str(), typed["password"].as_str()), (Some(s.site.as_str()), Some("alice@example.com"), Some("correct horse")));
}

#[tokio::test]
async fn tab_picks_the_selected_row_too_and_a_mouse_pick_closes_the_list() {
    let s = setup(None).await;
    let alice = save(&s, "alice@example.com");
    open(&s.state, &s.block, &s.site, "password", "");
    key(&s.state, &s.block, "ArrowDown");
    assert_eq!(key(&s.state, &s.block, "Tab"), Answer::default());
    open(&s.state, &s.block, &s.site, "password", "");
    assert!(meta(&s) != Value::Null);
    // A click on a row is `browser.signin_fill`, which closes the list as it starts.
    crate::server::browser_signin_fill::fill(&s.state, &s.block, &alice).await.unwrap();
    assert_eq!(meta(&s), Value::Null);
}

#[tokio::test]
async fn reopening_for_the_same_rows_keeps_the_selection_and_other_rows_clear_it() {
    let s = setup(None).await;
    save(&s, "alice@example.com");
    save(&s, "bob@example.com");
    open(&s.state, &s.block, &s.site, "password", "");
    key(&s.state, &s.block, "ArrowDown");
    assert_eq!(open(&s.state, &s.block, &s.site, "password", ""), Answer { shown: true, cursor: true });
    open(&s.state, &s.block, &s.site, "username", "al");
    assert_eq!(meta(&s)["cursor"], Value::Null, "different rows, nothing selected");
}

#[tokio::test]
async fn a_new_page_closes_the_list() {
    let s = setup(None).await;
    save(&s, "alice@example.com");
    open(&s.state, &s.block, &s.site, "password", "");
    navigated(&s.state, &s.block);
    assert_eq!(meta(&s), Value::Null);
    assert_eq!(close(&s.state, &s.block), Answer::default());
}

#[test]
fn only_booleans_go_back_to_the_host() {
    assert_eq!(Answer { shown: true, cursor: false }.to_json(), json!({ "shown": true, "cursor": false }));
}
