// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Calendar call end to end, with Google, the token endpoint and the
//! keychain stubbed.

use std::sync::Mutex;

use super::*;
use crate::identity::oauth_tokens::tests::{oauth_account, MemSecrets, StubHttp};
use crate::identity::oauth_tokens::HttpReply;

use super::super::GoogleRequest;

#[derive(Default)]
struct StubGoogle {
    replies: Mutex<Vec<HttpReply>>,
    seen: Mutex<Vec<(GoogleRequest, String)>>,
}

impl StubGoogle {
    fn answering(status: u16, body: Value) -> Self {
        StubGoogle { replies: Mutex::new(vec![HttpReply { status, body }]), ..Default::default() }
    }
    fn seen(&self) -> Vec<(GoogleRequest, String)> {
        self.seen.lock().unwrap().clone()
    }
}

impl GoogleHttp for StubGoogle {
    async fn send(&self, req: &GoogleRequest, token: &str) -> Result<HttpReply, String> {
        self.seen.lock().unwrap().push((req.clone(), token.to_string()));
        let mut r = self.replies.lock().unwrap();
        assert!(!r.is_empty(), "an unexpected Google call");
        Ok(r.remove(0))
    }
}

const READ: &[&str] = &["calendar.calendarlist.readonly", "calendar.events.readonly", "calendar.events.freebusy"];

struct World {
    store: Arc<Store>,
    def: AgentDefinition,
    secrets: MemSecrets,
}

/// An agent `def-1`, bound to the Google account `acc-1` (me@example.com)
/// whose token was granted `scopes`, with Calendar at `level`.
fn world(scopes: &[&str], level: Option<GrantLevel>, bound: bool) -> World {
    let store = Arc::new(Store::open_in_memory().unwrap());
    store.apply_identity_schema_for_tests().unwrap();
    let mut def = crate::backend::storage::agents::test_agent_def("def-1", "Naki", "claude", "host", 1, "");
    store.agent_def_insert(&mut def).unwrap();
    let mut acct = oauth_account("acc-1", "google", "cid");
    acct.context["email"] = json!("me@example.com");
    acct.context["granted_scopes"] = json!(scopes.iter().map(|s| format!("https://www.googleapis.com/auth/{s}")).collect::<Vec<_>>());
    store.identity_upsert(&acct).unwrap();
    if bound {
        store.agent_identity_link("def-1", "acc-1", "google").unwrap();
    }
    store.google_grant_set("def-1", "acc-1", "calendar", level, 1).unwrap();
    let far = chrono::Utc::now().timestamp() + 3600;
    let secrets = MemSecrets::with("acc-1", &json!({ "access_token": "a-live", "refresh_token": "r", "expires_at": far }).to_string());
    World { store, def, secrets }
}

async fn call(w: &World, google: &StubGoogle, action: &str, args: CalendarArgs, account: Option<&str>) -> Result<Value, String> {
    let oauth = StubHttp::default();
    let deps = CallDeps { google, oauth: &oauth, secrets: &w.secrets, mstore: &w.store, id_store: &w.store, identity_store: &w.store };
    calendar_read(&deps, &w.def, action, &args, account, chrono::Utc::now()).await
}

#[tokio::test]
async fn a_granted_read_calls_google_with_the_accounts_token_and_is_logged() {
    let w = world(READ, Some(GrantLevel::Read), true);
    let google = StubGoogle::answering(200, json!({ "items": [{ "id": "e1", "summary": "Standup" }] }));
    let out = call(&w, &google, "list_events", CalendarArgs::default(), None).await.unwrap();
    assert_eq!(out["account"], "me@example.com");
    assert_eq!(out["result"]["events"][0]["title"], "Standup");
    let seen = google.seen();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].0.url.ends_with("/calendars/primary/events"));
    assert_eq!(seen[0].1, "a-live", "srv's token, never the agent's");
    let log = w.store.google_calls("acc-1", 10).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!((log[0].agent_id.as_str(), log[0].action.as_str(), log[0].ids.as_str(), log[0].outcome.as_str()), ("def-1", "list_events", "primary", "ok"));
    assert_eq!(log[0].agent_name, "Naki");
}

#[tokio::test]
async fn without_a_bound_account_or_a_grant_nothing_reaches_google() {
    let google = StubGoogle::default();
    let unbound = world(READ, Some(GrantLevel::Read), false);
    let e = call(&unbound, &google, "list_events", CalendarArgs::default(), None).await.unwrap_err();
    assert!(e.contains("no Google account is bound to you"), "{e}");

    let off = world(READ, None, true);
    let e = call(&off, &google, "list_events", CalendarArgs::default(), None).await.unwrap_err();
    assert!(e.contains("me@example.com's Calendar is off for you"), "{e}");
    assert!(e.contains("Connectors"), "says who can turn it on: {e}");

    let granted = world(READ, Some(GrantLevel::Read), true);
    let e = call(&granted, &google, "list_events", CalendarArgs::default(), Some("other@example.com")).await.unwrap_err();
    assert!(e.contains("not other@example.com"), "{e}");
    assert!(google.seen().is_empty());
}

#[tokio::test]
async fn a_partly_granted_token_runs_only_what_it_covers() {
    let w = world(&["calendar.events.readonly"], Some(GrantLevel::Read), true);
    let google = StubGoogle::default();
    let e = call(&w, &google, "free_busy", CalendarArgs::default(), None).await.unwrap_err();
    assert!(e.contains("partly granted"), "{e}");
    assert!(google.seen().is_empty());
    let log = w.store.google_calls("acc-1", 10).unwrap();
    assert!(log[0].outcome.starts_with("error: "), "{:?}", log[0]);
}

#[tokio::test]
async fn googles_refusal_comes_back_as_a_sentence_and_is_logged() {
    let w = world(READ, Some(GrantLevel::Read), true);
    let google = StubGoogle::answering(404, json!({ "error": { "message": "Not Found" } }));
    let args = CalendarArgs { calendar_id: Some("gone".into()), ..Default::default() };
    let e = call(&w, &google, "list_events", args, None).await.unwrap_err();
    assert!(e.contains("no such calendar"), "{e}");
    let log = w.store.google_calls("acc-1", 10).unwrap();
    assert_eq!(log[0].ids, "gone");
    assert!(log[0].outcome.contains("no such calendar"));
}

#[tokio::test]
async fn an_unknown_action_is_refused_before_anything_is_read() {
    let w = world(READ, Some(GrantLevel::Read), true);
    let google = StubGoogle::default();
    let e = call(&w, &google, "delete_event", CalendarArgs::default(), None).await.unwrap_err();
    assert!(e.contains("one of list_calendars"), "{e}");
    assert!(w.store.google_calls("acc-1", 10).unwrap().is_empty());
}
