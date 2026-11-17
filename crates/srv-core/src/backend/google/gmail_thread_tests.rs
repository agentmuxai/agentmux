// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use serde_json::json;

use super::*;
use crate::backend::google::gmail_write::DraftView;

fn message(from: &str, to: &str, cc: &str) -> Value {
    let mut headers = vec![json!({ "name": "From", "value": from }), json!({ "name": "To", "value": to }), json!({ "name": "Subject", "value": "Invoice 42" })];
    if !cc.is_empty() {
        headers.push(json!({ "name": "Cc", "value": cc }));
    }
    json!({ "id": "m", "payload": { "headers": headers } })
}

fn view(to: &str, cc: &str) -> DraftView {
    let mut headers = vec![json!({ "name": "To", "value": to })];
    if !cc.is_empty() {
        headers.push(json!({ "name": "Cc", "value": cc }));
    }
    DraftView::of(&json!({ "id": "r-1", "message": { "id": "m1", "threadId": "t1", "payload": { "mimeType": "text/plain", "headers": headers } } }))
}

fn thread() -> ThreadContext {
    context_of(&json!({ "messages": [message("Dana Vendor <Dana@Vendor.com>", "me@example.com", ""), message("me@example.com", "dana@vendor.com", "\"Lee, Sam\" <sam@vendor.com>")] }))
}

#[test]
fn addresses_come_from_the_angle_brackets_and_a_name_is_never_one() {
    assert_eq!(addresses_in("Dana <Dana@Vendor.com>, bob@example.com"), ["dana@vendor.com", "bob@example.com"]);
    assert_eq!(addresses_in("\"Lee, Sam\" <sam@vendor.com>; Al <al@x.org>"), ["sam@vendor.com", "al@x.org"]);
    assert_eq!(addresses_in("\"ceo@bank.com\" <mallory@evil.com>"), ["mallory@evil.com"], "a name that spells an address is not that address");
    assert!(addresses_in("undisclosed-recipients:;").is_empty());
    assert!(addresses_in("").is_empty());
}

#[test]
fn a_thread_lists_everyone_who_wrote_or_received_in_it_once() {
    let t = thread();
    assert_eq!((t.messages, t.subject.as_str()), (2, "Invoice 42"));
    assert_eq!(t.people, ["dana@vendor.com", "me@example.com", "sam@vendor.com"]);
}

#[test]
fn a_recipient_in_the_thread_gets_no_warning_and_the_thread_is_named() {
    let note = ThreadNote::of(&view("Dana <dana@vendor.com>", ""), &thread(), "me@example.com");
    assert_eq!((note.warning, note.strangers), (None, false));
    assert_eq!(note.line.as_deref(), Some("Replies in the thread: Invoice 42 (2 messages, with dana@vendor.com, sam@vendor.com)"));
    // Any of To, Cc or Bcc being in the thread is enough.
    assert!(ThreadNote::of(&view("stranger@example.net", "sam@vendor.com"), &thread(), "me@example.com").warning.is_none());
}

#[test]
fn a_draft_for_a_stranger_in_the_thread_is_warned_about() {
    let note = ThreadNote::of(&view("stranger@example.net", "other@example.net"), &thread(), "me@example.com");
    assert_eq!(note.warning.as_deref(), Some(NO_PARTICIPANT));
    assert!(note.strangers && note.line.is_some());
}

#[test]
fn a_long_thread_lists_five_people_and_counts_the_rest() {
    let to: Vec<String> = (0..8).map(|i| format!("p{i}@example.com")).collect();
    let t = context_of(&json!({ "messages": [message("me@example.com", &to.join(", "), "")] }));
    let line = ThreadNote::of(&view("p0@example.com", ""), &t, "me@example.com").line.unwrap();
    assert!(line.contains("(1 message, with p0@example.com, p1@example.com, p2@example.com, p3@example.com, p4@example.com and 3 more)"), "{line}");
}

#[test]
fn an_unreadable_thread_is_said_so_without_claiming_a_stranger() {
    let note = ThreadNote::unchecked();
    assert!(note.warning.unwrap().contains("could not be read") && note.line.is_none() && !note.strangers);
}

#[test]
fn the_question_puts_the_warning_first_and_the_thread_after_the_recipients() {
    let v = view("stranger@example.net", "");
    let note = ThreadNote::of(&v, &thread(), "me@example.com");
    let (_, text) = crate::backend::google::gmail_write::send_question_in("Naki", "me@example.com", &v, &note);
    assert!(text.starts_with(NO_PARTICIPANT), "{text}");
    let (to, thread_line, subject) = (text.find("To: stranger").unwrap(), text.find("Replies in the thread: Invoice 42").unwrap(), text.find("Subject:").unwrap());
    assert!(to < thread_line && thread_line < subject, "{text}");
    let (_, plain) = crate::backend::google::gmail_write::send_question("Naki", "me@example.com", &v);
    assert!(!plain.contains("Replies in the thread") && plain.starts_with("Naki wants to send"), "{plain}");
}

#[test]
fn the_thread_does_not_change_the_digest() {
    let a = view("dana@vendor.com", "");
    let mut b = a.clone();
    b.thread_id = "another-thread".to_string();
    assert_eq!(a.digest(), b.digest(), "the thread is shown, not part of what the person approves by digest");
}
