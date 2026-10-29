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


const DRIVE: &[&str] = &["drive.file"];

/// [`world`], with Drive at `level` instead of Calendar.
fn drive_world(scopes: &[&str], level: Option<GrantLevel>) -> World {
    let w = world(scopes, None, true);
    w.store.google_grant_set("def-1", "acc-1", "drive", level, 1).unwrap();
    w
}

async fn drive_call(w: &World, google: &StubGoogle, action: &str, args: DriveArgs) -> Result<Value, String> {
    let oauth = StubHttp::default();
    let deps = CallDeps { google, oauth: &oauth, secrets: &w.secrets, mstore: &w.store, id_store: &w.store, identity_store: &w.store };
    drive_read(&deps, &w.def, action, &args, None, chrono::Utc::now()).await
}

fn two_replies(first: Value, second: HttpReply) -> StubGoogle {
    StubGoogle { replies: Mutex::new(vec![HttpReply { status: 200, body: first }, second]), ..Default::default() }
}

#[tokio::test]
async fn a_drive_search_uses_the_accounts_token_and_is_logged() {
    let w = drive_world(DRIVE, Some(GrantLevel::Read));
    let google = StubGoogle::answering(200, json!({ "files": [{ "id": "f1", "name": "Plan", "mimeType": "text/plain" }] }));
    let out = drive_call(&w, &google, "search_files", DriveArgs { query: Some("plan".into()), ..Default::default() }).await.unwrap();
    assert_eq!(out["account"], "me@example.com");
    assert_eq!(out["result"]["files"][0]["name"], "Plan");
    assert_eq!(google.seen()[0].1, "a-live");
    let log = w.store.google_calls("acc-1", 10).unwrap();
    assert_eq!((log[0].service.as_str(), log[0].action.as_str(), log[0].ids.as_str(), log[0].outcome.as_str()), ("drive", "search_files", "", "ok"));
}

#[tokio::test]
async fn reading_a_google_doc_asks_for_its_metadata_then_its_text() {
    let w = drive_world(DRIVE, Some(GrantLevel::Read));
    let meta = json!({ "id": "d1", "name": "Plan", "mimeType": "application/vnd.google-apps.document" });
    let google = two_replies(meta, HttpReply { status: 200, body: json!({ "text": "Step one", "truncated": false }) });
    let out = drive_call(&w, &google, "read_file_content", DriveArgs { file_id: Some("d1".into()), ..Default::default() }).await.unwrap();
    assert_eq!(out["result"]["content"], "Step one");
    assert_eq!(out["result"]["file"]["name"], "Plan");
    let seen = google.seen();
    assert_eq!(seen.len(), 2);
    assert!(!seen[0].0.text && seen[1].0.text);
    assert!(seen[1].0.url.ends_with("/files/d1/export"));
    assert_eq!(seen[1].1, "a-live");
    assert_eq!(w.store.google_calls("acc-1", 10).unwrap()[0].ids, "d1");
}

#[tokio::test]
async fn a_pdf_is_refused_after_the_metadata_and_a_file_not_picked_says_how_to_get_it() {
    let w = drive_world(DRIVE, Some(GrantLevel::Read));
    let google = StubGoogle::answering(200, json!({ "id": "p1", "name": "a.pdf", "mimeType": "application/pdf" }));
    let e = drive_call(&w, &google, "read_file_content", DriveArgs { file_id: Some("p1".into()), ..Default::default() }).await.unwrap_err();
    assert!(e.contains("can't be read as text"), "{e}");
    assert_eq!(google.seen().len(), 1, "no download was attempted");

    let google = StubGoogle::answering(404, json!({ "error": { "message": "File not found: x" } }));
    let e = drive_call(&w, &google, "get_file_metadata", DriveArgs { file_id: Some("x".into()), ..Default::default() }).await.unwrap_err();
    assert!(e.contains("pick_files") && e.contains("picked"), "{e}");
}

#[tokio::test]
async fn drive_needs_its_own_grant_and_its_own_scope() {
    let google = StubGoogle::default();
    let off = drive_world(DRIVE, None);
    let e = drive_call(&off, &google, "search_files", DriveArgs::default()).await.unwrap_err();
    assert!(e.contains("Drive is off for you"), "{e}");
    // A Calendar grant is not a Drive grant, and a Calendar-only token can't search Drive.
    let calendar_only = world(READ, Some(GrantLevel::Read), true);
    let e = drive_call(&calendar_only, &google, "search_files", DriveArgs::default()).await.unwrap_err();
    assert!(e.contains("Drive is off for you"), "{e}");
    let no_scope = drive_world(READ, Some(GrantLevel::Read));
    let e = drive_call(&no_scope, &google, "search_files", DriveArgs::default()).await.unwrap_err();
    assert!(e.contains("hasn't granted the Drive permission"), "{e}");
    assert!(google.seen().is_empty());
}

const MAIL: &[&str] = &["gmail.readonly"];

/// [`world`], with Gmail at `level` instead of Calendar.
fn gmail_world(scopes: &[&str], level: Option<GrantLevel>) -> World {
    let w = world(scopes, None, true);
    w.store.google_grant_set("def-1", "acc-1", "gmail", level, 1).unwrap();
    w
}

async fn gmail_call(w: &World, google: &StubGoogle, action: &str, args: GmailArgs) -> Result<Value, String> {
    let oauth = StubHttp::default();
    let deps = CallDeps { google, oauth: &oauth, secrets: &w.secrets, mstore: &w.store, id_store: &w.store, identity_store: &w.store };
    gmail_read(&deps, &w.def, action, &args, None, chrono::Utc::now()).await
}

fn replies(list: Vec<(u16, Value)>) -> StubGoogle {
    StubGoogle { replies: Mutex::new(list.into_iter().map(|(status, body)| HttpReply { status, body }).collect()), ..Default::default() }
}

#[tokio::test]
async fn a_mail_search_lists_threads_then_asks_for_each_ones_headers_and_logs_no_search_words() {
    let w = gmail_world(MAIL, Some(GrantLevel::Read));
    let summary = |subject: &str| json!({ "messages": [{ "payload": { "headers": [{ "name": "Subject", "value": subject }] } }] });
    let google = replies(vec![
        (200, json!({ "threads": [{ "id": "t1", "snippet": "a" }, { "id": "t2", "snippet": "b" }] })),
        (200, summary("First")),
        (500, json!({})),
    ]);
    let args = GmailArgs { query: Some("from:my-lawyer".into()), ..Default::default() };
    let out = gmail_call(&w, &google, "search_threads", args).await.unwrap();
    assert_eq!(out["account"], "me@example.com");
    assert_eq!(out["result"]["threads"][0]["subject"], "First");
    assert!(out["result"]["threads"][1].get("subject").is_none(), "a thread whose headers failed is still listed");
    let seen = google.seen();
    assert_eq!(seen.len(), 3);
    assert!(seen[0].0.url.ends_with("/users/me/threads") && seen[1].0.url.ends_with("/threads/t1") && seen[2].0.url.ends_with("/threads/t2"));
    assert!(seen.iter().all(|(_, t)| t == "a-live"), "srv's token, never the agent's");
    let log = w.store.google_calls("acc-1", 10).unwrap();
    assert_eq!((log[0].service.as_str(), log[0].action.as_str(), log[0].ids.as_str(), log[0].outcome.as_str()), ("gmail", "search_threads", "", "ok"));
    assert!(!format!("{log:?}").contains("lawyer"), "the search words stay out of the log");
}

#[tokio::test]
async fn reading_a_thread_logs_its_id_and_a_missing_one_says_so() {
    let w = gmail_world(MAIL, Some(GrantLevel::Read));
    let google = replies(vec![(200, json!({ "id": "t9", "messages": [] }))]);
    let out = gmail_call(&w, &google, "get_thread", GmailArgs { thread_id: Some("t9".into()), ..Default::default() }).await.unwrap();
    assert_eq!(out["result"]["id"], "t9");
    assert_eq!(w.store.google_calls("acc-1", 10).unwrap()[0].ids, "t9");

    let google = replies(vec![(404, json!({ "error": { "message": "Requested entity was not found." } }))]);
    let e = gmail_call(&w, &google, "get_thread", GmailArgs { thread_id: Some("gone".into()), ..Default::default() }).await.unwrap_err();
    assert!(e.contains("no such thread"), "{e}");
}

#[tokio::test]
async fn gmail_needs_its_own_grant_and_its_own_scope() {
    let google = StubGoogle::default();
    let off = gmail_world(MAIL, None);
    let e = gmail_call(&off, &google, "list_labels", GmailArgs::default()).await.unwrap_err();
    assert!(e.contains("Gmail is off for you"), "{e}");
    // A Drive grant is not a Gmail grant; compose alone reads nothing.
    let drive_only = drive_world(DRIVE, Some(GrantLevel::Read));
    let e = gmail_call(&drive_only, &google, "list_labels", GmailArgs::default()).await.unwrap_err();
    assert!(e.contains("Gmail is off for you"), "{e}");
    let compose_only = gmail_world(&["gmail.compose"], Some(GrantLevel::Read));
    let e = gmail_call(&compose_only, &google, "search_threads", GmailArgs::default()).await.unwrap_err();
    assert!(e.contains("hasn't granted the Gmail permission"), "{e}");
    let e = gmail_call(&gmail_world(MAIL, Some(GrantLevel::Read)), &google, "send_message", GmailArgs::default()).await.unwrap_err();
    assert!(e.contains("one of search_threads"), "{e}");
    assert!(google.seen().is_empty());
}

#[tokio::test]
async fn a_gmail_api_that_is_not_enabled_in_the_clients_project_says_what_to_do() {
    let w = gmail_world(MAIL, Some(GrantLevel::Read));
    let google = replies(vec![(403, json!({ "error": { "message": "Gmail API has not been used in project 123 before or it is disabled." } }))]);
    let e = gmail_call(&w, &google, "list_labels", GmailArgs::default()).await.unwrap_err();
    assert!(e.contains("Gmail API isn't turned on"), "{e}");
}

#[tokio::test]
async fn a_long_mails_stored_text_is_fetched_with_the_thread() {
    use base64::Engine as _;
    let w = gmail_world(MAIL, Some(GrantLevel::Read));
    let data = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("The whole long mail.");
    let thread = json!({ "id": "t1", "messages": [{ "id": "m1", "payload": { "mimeType": "text/plain", "body": { "attachmentId": "ATT1", "size": 5 } } }] });
    let google = replies(vec![(200, thread), (200, json!({ "data": data, "size": 20 }))]);
    let out = gmail_call(&w, &google, "get_thread", GmailArgs { thread_id: Some("t1".into()), ..Default::default() }).await.unwrap();
    assert_eq!(out["result"]["messages"][0]["body"], "The whole long mail.");
    let seen = google.seen();
    assert_eq!(seen.len(), 2);
    assert!(seen[1].0.url.ends_with("/messages/m1/attachments/ATT1"));
}
