// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Gmail write end to end, with Google, the token endpoint, the keychain and the person's
//! answer stubbed.

use std::sync::{Arc, Mutex};

use base64::Engine as _;
use serde_json::json;

use super::*;
use crate::backend::storage::store::Store;
use crate::identity::oauth_tokens::tests::{oauth_account, MemSecrets, StubHttp};
use crate::identity::oauth_tokens::HttpReply;

use super::super::GoogleRequest;

#[derive(Default)]
struct StubGoogle {
    replies: Mutex<Vec<HttpReply>>,
    seen: Mutex<Vec<(GoogleRequest, String)>>,
}

impl StubGoogle {
    fn new(list: Vec<(u16, Value)>) -> Self {
        StubGoogle { replies: Mutex::new(list.into_iter().map(|(status, body)| HttpReply { status, body }).collect()), ..Default::default() }
    }
    fn seen(&self) -> Vec<(GoogleRequest, String)> {
        self.seen.lock().unwrap().clone()
    }
}

impl GoogleHttp for StubGoogle {
    async fn send(&self, req: &GoogleRequest, token: &str) -> Result<HttpReply, String> {
        self.seen.lock().unwrap().push((req.clone(), token.to_string()));
        let mut r = self.replies.lock().unwrap();
        assert!(!r.is_empty(), "an unexpected Google call: {} {}", req.method, req.url);
        Ok(r.remove(0))
    }
}

/// Answers every question the same way and remembers what was asked.
struct Person {
    answer: Result<Answer, String>,
    asked: Mutex<Vec<(String, String, String, String)>>,
}

impl Person {
    fn says(answer: Answer) -> Self {
        Person { answer: Ok(answer), asked: Mutex::new(Vec::new()) }
    }
    fn asked(&self) -> Vec<(String, String, String, String)> {
        self.asked.lock().unwrap().clone()
    }
}

impl Approver for Person {
    async fn ask(&self, _def: &AgentDefinition, title: String, message: String, yes: &str, no: &str) -> Result<Answer, String> {
        self.asked.lock().unwrap().push((title, message, yes.to_string(), no.to_string()));
        self.answer.clone()
    }
}

struct World {
    store: Arc<Store>,
    def: AgentDefinition,
    secrets: MemSecrets,
}

const WRITE_SCOPES: &[&str] = &["gmail.readonly", "gmail.compose", "gmail.modify"];

/// An agent `def-1`, bound to the account `acc-1` (me@example.com) whose token was granted `scopes`,
/// with Gmail at `level`.
fn world(scopes: &[&str], level: Option<GrantLevel>) -> World {
    let store = Arc::new(Store::open_in_memory().unwrap());
    store.apply_identity_schema_for_tests().unwrap();
    let mut def = crate::backend::storage::agents::test_agent_def("def-1", "Naki", "claude", "host", 1, "");
    store.agent_def_insert(&mut def).unwrap();
    let mut acct = oauth_account("acc-1", "google", "cid");
    acct.context["email"] = json!("me@example.com");
    acct.context["granted_scopes"] = json!(scopes.iter().map(|s| format!("https://www.googleapis.com/auth/{s}")).collect::<Vec<_>>());
    store.identity_upsert(&acct).unwrap();
    store.agent_identity_link("def-1", "acc-1", "google").unwrap();
    store.google_grant_set("def-1", "acc-1", "gmail", level, 1).unwrap();
    let far = chrono::Utc::now().timestamp() + 3600;
    let secrets = MemSecrets::with("acc-1", &json!({ "access_token": "a-live", "refresh_token": "r", "expires_at": far }).to_string());
    World { store, def, secrets }
}

async fn run(w: &World, google: &StubGoogle, person: &Person, action: &str, args: serde_json::Value) -> Result<Value, String> {
    let oauth = StubHttp::default();
    let deps = CallDeps { google, oauth: &oauth, secrets: &w.secrets, mstore: &w.store, id_store: &w.store, identity_store: &w.store };
    let args: GmailArgs = serde_json::from_value(args).unwrap();
    gmail_write(&deps, person, &w.def, action, &args, None, chrono::Utc::now()).await
}

fn stored_draft(to: &str, body: &str) -> Value {
    let data = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(body);
    json!({ "id": "r-1", "message": { "payload": { "mimeType": "text/plain", "headers": [{ "name": "To", "value": to }, { "name": "Subject", "value": "Hello" }], "body": { "data": data } } } })
}

fn log(w: &World) -> Vec<crate::backend::storage::google_grants::GoogleCall> {
    w.store.google_calls("acc-1", 100).unwrap()
}

#[tokio::test]
async fn a_draft_is_created_without_asking_and_logged_without_its_text() {
    let w = world(WRITE_SCOPES, Some(GrantLevel::Write));
    let google = StubGoogle::new(vec![(200, json!({ "id": "r-1", "message": { "id": "m1", "threadId": "t1" } }))]);
    let person = Person::says(Answer::No);
    let out = run(&w, &google, &person, "create_draft", json!({ "to": ["a@example.com"], "subject": "Secret plan", "body": "the vault code is 1234" })).await.unwrap();
    assert_eq!(out["account"], "me@example.com");
    assert_eq!(out["result"]["draft_id"], "r-1");
    assert!(person.asked().is_empty(), "a draft sends nothing, so nobody is asked");
    let seen = google.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].1, "a-live", "srv's token, never the agent's");
    assert!(seen[0].0.url.ends_with("/users/me/drafts"));
    let lines = log(&w);
    assert_eq!((lines[0].action.as_str(), lines[0].outcome.as_str()), ("create_draft", "ok"));
    assert!(!format!("{lines:?}").contains("vault code"), "the mail stays out of the log");
}

#[tokio::test]
async fn a_read_grant_cannot_write_and_a_token_without_the_scope_cannot_either() {
    let google = StubGoogle::new(vec![]);
    let person = Person::says(Answer::Yes);
    let read_only = world(WRITE_SCOPES, Some(GrantLevel::Read));
    let e = run(&read_only, &google, &person, "create_draft", json!({ "to": ["a@example.com"] })).await.unwrap_err();
    assert!(e.contains("read Gmail but not write it") && e.contains("Connectors"), "{e}");
    let no_scope = world(&["gmail.readonly"], Some(GrantLevel::Write));
    let e = run(&no_scope, &google, &person, "send_draft", json!({ "draft_id": "r-1" })).await.unwrap_err();
    assert!(e.contains("hasn't granted the Gmail permission send_draft needs"), "{e}");
    let compose_only = world(&["gmail.readonly", "gmail.compose"], Some(GrantLevel::Write));
    let e = run(&compose_only, &google, &person, "trash_thread", json!({ "thread_id": "t1" })).await.unwrap_err();
    assert!(e.contains("hasn't granted"), "{e}");
    let off = world(WRITE_SCOPES, None);
    assert!(run(&off, &google, &person, "create_draft", json!({ "to": ["a@example.com"] })).await.unwrap_err().contains("Gmail is off for you"));
    assert!(google.seen().is_empty() && person.asked().is_empty());
}

#[tokio::test]
async fn an_approved_send_shows_the_draft_asks_once_checks_it_again_and_sends() {
    let w = world(WRITE_SCOPES, Some(GrantLevel::Write));
    let google = StubGoogle::new(vec![
        (200, stored_draft("bob@example.com", "See you at noon")),
        (200, stored_draft("bob@example.com", "See you at noon")),
        (200, json!({ "id": "m9", "threadId": "t9" })),
    ]);
    let person = Person::says(Answer::Yes);
    let out = run(&w, &google, &person, "send_draft", json!({ "draft_id": "r-1" })).await.unwrap();
    assert_eq!((out["result"]["sent"].as_bool(), out["result"]["message_id"].as_str()), (Some(true), Some("m9")));
    let asked = person.asked();
    assert_eq!(asked.len(), 1);
    assert!(asked[0].1.contains("To: bob@example.com") && asked[0].1.contains("See you at noon"), "{}", asked[0].1);
    assert_eq!((asked[0].2.as_str(), asked[0].3.as_str()), ("Send", "Don't send"));
    let seen = google.seen();
    assert_eq!(seen.len(), 3);
    assert!(seen[0].0.url.ends_with("/drafts/r-1") && seen[1].0.url.ends_with("/drafts/r-1"));
    assert_eq!((seen[2].0.method.as_str(), seen[2].0.url.as_str()), ("POST", "https://gmail.googleapis.com/gmail/v1/users/me/drafts/send"));
    assert_eq!(log(&w)[0].ids, "r-1");
}

#[tokio::test]
async fn a_no_or_no_answer_sends_nothing() {
    for (answer, words) in [(Answer::No, "said no"), (Answer::Unanswered, "didn't answer")] {
        let w = world(WRITE_SCOPES, Some(GrantLevel::Write));
        let google = StubGoogle::new(vec![(200, stored_draft("bob@example.com", "x"))]);
        let person = Person::says(answer);
        let e = run(&w, &google, &person, "send_draft", json!({ "draft_id": "r-1" })).await.unwrap_err();
        assert!(e.contains(words) && e.contains("was not done"), "{e}");
        assert_eq!(google.seen().len(), 1, "only the read of the draft: nothing was sent");
        assert!(log(&w)[0].outcome.starts_with("error: "));
    }
}

#[tokio::test]
async fn no_window_to_ask_in_means_nothing_is_sent() {
    let w = world(WRITE_SCOPES, Some(GrantLevel::Write));
    let google = StubGoogle::new(vec![(200, stored_draft("bob@example.com", "x"))]);
    let person = Person { answer: Err("no AgentMux window is connected to ask the user in".into()), asked: Mutex::new(Vec::new()) };
    let e = run(&w, &google, &person, "send_draft", json!({ "draft_id": "r-1" })).await.unwrap_err();
    assert!(e.contains("no AgentMux window"), "{e}");
    assert_eq!(google.seen().len(), 1);
}

#[tokio::test]
async fn a_draft_changed_after_the_yes_is_not_sent() {
    let w = world(WRITE_SCOPES, Some(GrantLevel::Write));
    let google = StubGoogle::new(vec![
        (200, stored_draft("bob@example.com", "Lunch at noon?")),
        (200, stored_draft("mallory@example.com", "Lunch at noon?")),
    ]);
    let person = Person::says(Answer::Yes);
    let e = run(&w, &google, &person, "send_draft", json!({ "draft_id": "r-1" })).await.unwrap_err();
    assert!(e.contains("changed after the person approved"), "{e}");
    assert_eq!(google.seen().len(), 2, "no send request was made");
}

#[tokio::test]
async fn a_draft_with_no_recipient_is_not_put_to_the_person() {
    let w = world(WRITE_SCOPES, Some(GrantLevel::Write));
    let none = json!({ "id": "r-1", "message": { "payload": { "mimeType": "text/plain", "headers": [{ "name": "Subject", "value": "s" }] } } });
    let google = StubGoogle::new(vec![(200, none)]);
    let person = Person::says(Answer::Yes);
    let e = run(&w, &google, &person, "send_draft", json!({ "draft_id": "r-1" })).await.unwrap_err();
    assert!(e.contains("no recipient"), "{e}");
    assert!(person.asked().is_empty());
}

#[tokio::test]
async fn twenty_sends_a_day_and_ten_questions_an_hour_are_the_most() {
    let google = StubGoogle::new(vec![]);
    let person = Person::says(Answer::Yes);
    let line = |action: &str, outcome: &str, at: i64| crate::backend::storage::google_grants::GoogleCall {
        at,
        agent_id: "def-1".into(),
        agent_name: "Naki".into(),
        service: "gmail".into(),
        action: action.into(),
        ids: "r".into(),
        outcome: outcome.into(),
    };
    let now = chrono::Utc::now().timestamp_millis();
    let w = world(WRITE_SCOPES, Some(GrantLevel::Write));
    for i in 0..20 {
        w.store.google_call_log("acc-1", &line("send_draft", "ok", now - 2 * 3_600_000 - i)).unwrap();
    }
    let e = run(&w, &google, &person, "send_draft", json!({ "draft_id": "r-1" })).await.unwrap_err();
    assert!(e.contains("20 mails"), "{e}");

    let w = world(WRITE_SCOPES, Some(GrantLevel::Write));
    for i in 0..10 {
        w.store.google_call_log("acc-1", &line("trash_thread", "error: the person said no", now - 60_000 - i)).unwrap();
    }
    let e = run(&w, &google, &person, "trash_thread", json!({ "thread_id": "t1" })).await.unwrap_err();
    assert!(e.contains("10 times in the last hour"), "{e}");
    assert!(google.seen().is_empty() && person.asked().is_empty());
}

#[tokio::test]
async fn trashing_asks_with_the_threads_subject_then_moves_it_to_the_trash() {
    let w = world(WRITE_SCOPES, Some(GrantLevel::Write));
    let headers = json!({ "messages": [{ "payload": { "headers": [{ "name": "Subject", "value": "Old invoice" }, { "name": "From", "value": "Shop <shop@example.com>" }] } }] });
    let google = StubGoogle::new(vec![(200, headers), (200, json!({ "id": "t1" }))]);
    let person = Person::says(Answer::Yes);
    let out = run(&w, &google, &person, "trash_thread", json!({ "thread_id": "t1" })).await.unwrap();
    assert_eq!(out["result"]["trashed"], true);
    let asked = person.asked();
    assert!(asked[0].1.contains("Old invoice") && asked[0].1.contains("shop@example.com"), "{}", asked[0].1);
    assert_eq!((asked[0].2.as_str(), asked[0].3.as_str()), ("Move to trash", "Keep it"));
    assert!(google.seen()[1].0.url.ends_with("/threads/t1/trash"));
    assert_eq!(log(&w)[0].ids, "t1");

    let google = StubGoogle::new(vec![(200, json!({ "messages": [] }))]);
    let person = Person::says(Answer::No);
    assert!(run(&w, &google, &person, "trash_thread", json!({ "thread_id": "t1" })).await.is_err());
    assert_eq!(google.seen().len(), 1, "a no trashes nothing");
}

#[tokio::test]
async fn labels_change_without_asking_but_never_to_the_trash() {
    let w = world(WRITE_SCOPES, Some(GrantLevel::Write));
    let google = StubGoogle::new(vec![(200, json!({ "id": "t1" }))]);
    let person = Person::says(Answer::No);
    let out = run(&w, &google, &person, "label_thread", json!({ "thread_id": "t1", "add_label_ids": ["STARRED"], "remove_label_ids": ["UNREAD"] })).await.unwrap();
    assert_eq!(out["result"]["done"], true);
    assert!(person.asked().is_empty());
    let e = run(&w, &google, &person, "label_thread", json!({ "thread_id": "t1", "add_label_ids": ["TRASH"] })).await.unwrap_err();
    assert!(e.contains("trash_thread"), "{e}");
    assert_eq!(google.seen().len(), 1);
}

#[tokio::test]
async fn an_unknown_write_action_is_refused_before_anything_is_read() {
    let w = world(WRITE_SCOPES, Some(GrantLevel::Write));
    let e = run(&w, &StubGoogle::new(vec![]), &Person::says(Answer::Yes), "send_message", json!({})).await.unwrap_err();
    assert!(e.contains("one of create_draft"), "{e}");
    assert!(log(&w).is_empty());
}

#[tokio::test]
async fn five_hundred_reads_do_not_push_the_sends_out_of_the_count() {
    let w = world(WRITE_SCOPES, Some(GrantLevel::Write));
    let now = chrono::Utc::now().timestamp_millis();
    let line = |agent: &str, action: &str, outcome: &str, at: i64| crate::backend::storage::google_grants::GoogleCall {
        at,
        agent_id: agent.into(),
        agent_name: agent.into(),
        service: "gmail".into(),
        action: action.into(),
        ids: String::new(),
        outcome: outcome.into(),
    };
    for i in 0..20 {
        w.store.google_call_log("acc-1", &line("def-1", "send_draft", "ok", now - 2 * 3_600_000 - i)).unwrap();
    }
    // Newer lines, many more than the old window of 500: reads by this agent, anything by another.
    for i in 0..600 {
        w.store.google_call_log("acc-1", &line("def-1", "search_threads", "ok", now - 1000 + i)).unwrap();
        w.store.google_call_log("acc-1", &line("other", "send_draft", "ok", now - 1000 + i)).unwrap();
    }
    let e = run(&w, &StubGoogle::new(vec![]), &Person::says(Answer::Yes), "send_draft", json!({ "draft_id": "r-1" })).await.unwrap_err();
    assert!(e.contains("20 mails"), "{e}");
    // Another agent's sends and a failed one of this agent's are not this agent's sent mails.
    let w2 = world(WRITE_SCOPES, Some(GrantLevel::Write));
    for i in 0..30 {
        w2.store.google_call_log("acc-1", &line("other", "send_draft", "ok", now - 5000 - i)).unwrap();
        w2.store.google_call_log("acc-1", &line("def-1", "send_draft", "error: no", now - 5000 - i)).unwrap();
    }
    assert_eq!(w2.store.google_calls_count("acc-1", "def-1", "gmail", &["send_draft"], now - 86_400_000, CallsCounted::Ok).unwrap(), 0);
}

#[tokio::test]
async fn questions_asked_at_once_count_against_the_limit_before_any_is_logged() {
    let w = world(WRITE_SCOPES, Some(GrantLevel::Write));
    let oauth = StubHttp::default();
    let google = StubGoogle::new(vec![]);
    let deps = CallDeps { google: &google, oauth: &oauth, secrets: &w.secrets, mstore: &w.store, id_store: &w.store, identity_store: &w.store };
    let access = access_for(deps.mstore, deps.id_store, deps.identity_store, &w.def, Service::Gmail, None).unwrap();
    let now = chrono::Utc::now();
    // Ten are waiting for the person; the eleventh is refused, and when one ends there is room again.
    let held: Vec<_> = (0..ASKS_PER_HOUR).map(|_| reserve(&deps, &access, &w.def, now, false).unwrap()).collect();
    let e = reserve(&deps, &access, &w.def, now, false).err().unwrap();
    assert!(e.contains("10 times in the last hour"), "{e}");
    drop(held);
    assert!(reserve(&deps, &access, &w.def, now, false).is_ok());
}

#[tokio::test]
async fn a_drafts_stored_text_is_loaded_before_the_person_is_asked() {
    let w = world(WRITE_SCOPES, Some(GrantLevel::Write));
    let stored = json!({ "id": "r-1", "message": { "id": "m-1", "payload": { "mimeType": "text/plain",
        "headers": [{ "name": "To", "value": "bob@example.com" }, { "name": "Subject", "value": "Long" }],
        "body": { "attachmentId": "ATT1", "size": 90000 } } } });
    let data = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("The full text of a long mail");
    let google = StubGoogle::new(vec![
        (200, stored.clone()),
        (200, json!({ "data": data, "size": 28 })),
        (200, stored),
        (200, json!({ "data": data, "size": 28 })),
        (200, json!({ "id": "m9", "threadId": "t9" })),
    ]);
    let person = Person::says(Answer::Yes);
    let out = run(&w, &google, &person, "send_draft", json!({ "draft_id": "r-1" })).await.unwrap();
    assert_eq!(out["result"]["sent"], true);
    assert!(person.asked()[0].1.contains("The full text of a long mail"), "{}", person.asked()[0].1);
}

#[tokio::test]
async fn a_draft_whose_text_cannot_be_loaded_is_not_put_to_the_person() {
    let w = world(WRITE_SCOPES, Some(GrantLevel::Write));
    let stored = json!({ "id": "r-1", "message": { "id": "m-1", "payload": { "mimeType": "text/plain",
        "headers": [{ "name": "To", "value": "bob@example.com" }], "body": { "attachmentId": "ATT1", "size": 9000000 } } } });
    let google = StubGoogle::new(vec![(200, stored), (500, json!({}))]);
    let person = Person::says(Answer::Yes);
    let e = run(&w, &google, &person, "send_draft", json!({ "draft_id": "r-1" })).await.unwrap_err();
    assert!(e.contains("too large to show the person"), "{e}");
    assert!(person.asked().is_empty());
}

#[tokio::test]
async fn a_draft_replying_in_a_thread_gets_the_threads_headers() {
    let w = world(WRITE_SCOPES, Some(GrantLevel::Write));
    let headers = json!({ "messages": [{ "payload": { "headers": [{ "name": "Message-ID", "value": "<a@mail.example>" }, { "name": "Subject", "value": "Lunch?" }] } }] });
    let google = StubGoogle::new(vec![(200, headers), (200, json!({ "id": "r-2", "message": { "threadId": "t1" } }))]);
    let out = run(&w, &google, &Person::says(Answer::No), "create_draft", json!({ "thread_id": "t1", "to": ["a@example.com"], "body": "Noon" })).await.unwrap();
    assert_eq!(out["result"]["thread_id"], "t1");
    let seen = google.seen();
    assert!(seen[0].0.url.ends_with("/threads/t1") && seen[1].0.url.ends_with("/users/me/drafts"));
    let raw = seen[1].0.body.as_ref().unwrap().pointer("/message/raw").and_then(|v| v.as_str()).unwrap();
    let mail = String::from_utf8(base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(raw).unwrap()).unwrap();
    assert!(mail.contains("In-Reply-To: <a@mail.example>") && mail.contains("Subject: Re: Lunch?"), "{mail}");
    // A thread with nothing to reply to is said so, and no draft is made.
    let none = StubGoogle::new(vec![(200, json!({ "messages": [] }))]);
    let e = run(&w, &none, &Person::says(Answer::No), "create_draft", json!({ "thread_id": "t1", "to": ["a@example.com"] })).await.unwrap_err();
    assert!(e.contains("no Message-ID"), "{e}");
    assert_eq!(none.seen().len(), 1);
}

#[tokio::test]
async fn only_calls_that_put_a_question_to_the_person_count_toward_the_hourly_limit() {
    let w = world(WRITE_SCOPES, Some(GrantLevel::Write));
    let now = chrono::Utc::now().timestamp_millis();
    let line = |outcome: &str, i: i64| crate::backend::storage::google_grants::GoogleCall {
        at: now - 1000 - i,
        agent_id: "def-1".into(),
        agent_name: "Naki".into(),
        service: "gmail".into(),
        action: "send_draft".into(),
        ids: String::new(),
        outcome: outcome.into(),
    };
    // Twenty calls that never showed a window: refused for the limit, a bad id, no recipient, a failed read.
    for i in 0..20 {
        let why = ["error: you have asked the person 10 times in the last hour", "error: \"x\" isn't a Gmail id", "error: the draft has no recipient", "error: Google answered HTTP 500"][i % 4];
        w.store.google_call_log("acc-1", &line(why, i as i64)).unwrap();
    }
    let count = |w: &World| w.store.google_calls_count("acc-1", "def-1", "gmail", &["send_draft", "trash_thread"], now - 3_600_000, CallsCounted::Asked).unwrap();
    assert_eq!(count(&w), 0);
    // A yes, a no and no answer did.
    for (i, o) in ["ok", "error: the person said no, so sending was not done", "error: the person didn't answer, so sending was not done"].into_iter().enumerate() {
        w.store.google_call_log("acc-1", &line(o, 100 + i as i64)).unwrap();
    }
    assert_eq!(count(&w), 3);
    // So an agent that keeps retrying a typo is never locked out, and one that is asked ten times is.
    let e = run(&w, &StubGoogle::new(vec![]), &Person::says(Answer::Yes), "send_draft", json!({ "draft_id": "../x" })).await.unwrap_err();
    assert!(e.contains("isn't a Gmail id"), "{e}");
}

#[tokio::test]
async fn a_failure_after_the_yes_counts_as_a_question_asked() {
    let w = world(WRITE_SCOPES, Some(GrantLevel::Write));
    // The draft changes after the yes: a window was shown, so the line counts toward the hourly limit.
    let google = StubGoogle::new(vec![(200, stored_draft("bob@example.com", "one")), (200, stored_draft("bob@example.com", "two"))]);
    let e = run(&w, &google, &Person::says(Answer::Yes), "send_draft", json!({ "draft_id": "r-1" })).await.unwrap_err();
    assert!(e.starts_with("the person said yes, but the draft changed"), "{e}");
    // So does a Google error on the send itself, and on the trash.
    let google = StubGoogle::new(vec![(200, stored_draft("bob@example.com", "x")), (200, stored_draft("bob@example.com", "x")), (500, json!({}))]);
    let e = run(&w, &google, &Person::says(Answer::Yes), "send_draft", json!({ "draft_id": "r-1" })).await.unwrap_err();
    assert!(e.starts_with("the person said yes, but "), "{e}");
    let google = StubGoogle::new(vec![(200, json!({ "messages": [] })), (500, json!({}))]);
    let e = run(&w, &google, &Person::says(Answer::Yes), "trash_thread", json!({ "thread_id": "t1" })).await.unwrap_err();
    assert!(e.starts_with("the person said yes, but "), "{e}");
    let asked = w.store.google_calls_count("acc-1", "def-1", "gmail", &["send_draft", "trash_thread"], 0, CallsCounted::Asked).unwrap();
    assert_eq!(asked, 3);
}
