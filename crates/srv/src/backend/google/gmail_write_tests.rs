// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use base64::Engine as _;
use serde_json::json;

use super::*;

fn args(v: serde_json::Value) -> GmailArgs {
    serde_json::from_value(v).unwrap()
}

fn b64(s: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s)
}

/// The text of the mail a draft request carries.
fn raw_of(req: &GoogleRequest) -> String {
    let raw = req.body.as_ref().unwrap().pointer("/message/raw").and_then(|v| v.as_str()).unwrap();
    String::from_utf8(base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(raw).unwrap()).unwrap()
}

#[test]
fn a_draft_is_a_mime_message_with_the_recipients_subject_and_a_base64_body() {
    let req = draft_plan("create_draft", &args(json!({ "to": ["alice@example.com", "Bob Smith <bob@example.org>"], "cc": ["c@example.com"], "bcc": ["d@example.com"], "subject": "Lunch", "body": "Hi Alice,\nnoon?" }))).unwrap();
    assert_eq!(req.method, reqwest::Method::POST);
    assert_eq!(req.url, "https://gmail.googleapis.com/gmail/v1/users/me/drafts");
    let mail = raw_of(&req);
    assert!(mail.starts_with("MIME-Version: 1.0\r\nTo: alice@example.com, Bob Smith <bob@example.org>\r\nCc: c@example.com\r\nBcc: d@example.com\r\nSubject: Lunch\r\n"), "{mail}");
    assert!(!mail.contains("\r\nFrom:"), "Gmail writes the account's own From");
    let body = mail.split("\r\n\r\n").nth(1).unwrap().replace("\r\n", "");
    assert_eq!(String::from_utf8(base64::engine::general_purpose::STANDARD.decode(body).unwrap()).unwrap(), "Hi Alice,\nnoon?");
}

#[test]
fn an_update_replaces_the_draft_by_id_and_a_reply_names_its_thread() {
    let req = draft_plan("update_draft", &args(json!({ "draft_id": "r-123", "thread_id": "18c2", "to": ["a@example.com"], "body": "ok" }))).unwrap();
    assert_eq!(req.method, reqwest::Method::PUT);
    assert_eq!(req.url, "https://gmail.googleapis.com/gmail/v1/users/me/drafts/r-123");
    assert_eq!(req.body.as_ref().unwrap().pointer("/message/threadId").unwrap(), "18c2");
    assert!(draft_plan("update_draft", &args(json!({ "to": ["a@example.com"] }))).unwrap_err().contains("draft_id"));
}

#[test]
fn a_non_ascii_subject_is_encoded_in_words_and_cannot_carry_a_header() {
    let req = draft_plan("create_draft", &args(json!({ "to": ["a@example.com"], "subject": "Déjeuner à midi ? Très tôt, vraiment, on se retrouve au café près de la gare" }))).unwrap();
    let mail = raw_of(&req);
    let subject: Vec<&str> = mail.split("\r\n").skip_while(|l| !l.starts_with("Subject:")).take_while(|l| l.starts_with("Subject:") || l.starts_with(' ')).collect();
    assert!(subject.len() >= 2 && subject.iter().all(|l| l.contains("=?UTF-8?B?")), "{subject:?}");
    assert!(subject.iter().all(|l| l.len() <= 76), "{subject:?}");
    // A line break in the subject or an address would add a header (Bcc, say): refused.
    assert!(outgoing(&args(json!({ "to": ["a@example.com"], "subject": "Hi\r\nBcc: x@evil.test" }))).is_err());
    assert!(outgoing(&args(json!({ "to": ["a@example.com"], "subject": "Hi\nBcc: x@evil.test" }))).is_err());
    assert!(outgoing(&args(json!({ "to": ["a@example.com\r\nBcc: x@evil.test"] }))).is_err());
}

#[test]
fn only_plain_addresses_are_accepted() {
    for ok in ["a@example.com", "first.last+tag@sub.example.co", "Ann O'Neil <ann@example.com>"] {
        assert!(outgoing(&args(json!({ "to": [ok] }))).is_ok(), "{ok}");
    }
    for bad in ["", "no-at-sign", "a@b", "a@@example.com", "a b@example.com", "a@example.com, b@example.com", "<a@example.com> x", "A \"Q\" <a@example.com>", "a@example.com;b@example.com", "Ünï <a@example.com>", "a@-.com."] {
        assert!(outgoing(&args(json!({ "to": [bad] }))).is_err(), "{bad:?} should be refused");
    }
}

#[test]
fn a_mail_is_bounded_and_not_empty() {
    assert!(outgoing(&args(json!({}))).unwrap_err().contains("at least"));
    let many: Vec<String> = (0..51).map(|i| format!("p{i}@example.com")).collect();
    assert!(outgoing(&args(json!({ "to": many }))).unwrap_err().contains("at most 50"));
    assert!(outgoing(&args(json!({ "to": ["a@example.com"], "body": "x".repeat(100_001) }))).unwrap_err().contains("body"));
    assert!(outgoing(&args(json!({ "subject": "x".repeat(301) }))).unwrap_err().contains("subject"));
    assert!(outgoing(&args(json!({ "subject": "just a subject" }))).is_ok(), "a draft may be started without a recipient");
}

#[test]
fn labels_are_changed_by_id_and_the_trash_label_is_not_one_to_add() {
    let req = label_plan(&args(json!({ "thread_id": "t1", "add_label_ids": ["STARRED", "Label_12"], "remove_label_ids": ["UNREAD"] }))).unwrap();
    assert_eq!(req.url, "https://gmail.googleapis.com/gmail/v1/users/me/threads/t1/modify");
    assert_eq!(req.body.unwrap(), json!({ "addLabelIds": ["STARRED", "Label_12"], "removeLabelIds": ["UNREAD"] }));
    assert!(label_plan(&args(json!({ "thread_id": "t1", "add_label_ids": ["TRASH"] }))).unwrap_err().contains("trash_thread"));
    assert!(label_plan(&args(json!({ "thread_id": "t1" }))).unwrap_err().contains("add_label_ids or remove_label_ids"));
    assert!(label_plan(&args(json!({ "thread_id": "t1", "add_label_ids": ["a/b"] }))).unwrap_err().contains("isn't a label id"));
    // Taking a thread out of the trash is allowed.
    assert!(label_plan(&args(json!({ "thread_id": "t1", "remove_label_ids": ["TRASH"] }))).is_ok());
}

#[test]
fn send_and_trash_are_post_requests_by_id() {
    let send = send_plan("r-9").unwrap();
    assert_eq!((send.method, send.url.as_str()), (reqwest::Method::POST, "https://gmail.googleapis.com/gmail/v1/users/me/drafts/send"));
    assert_eq!(send.body.unwrap(), json!({ "id": "r-9" }));
    assert!(send_plan("../x").is_err());
    let trash = trash_plan(&args(json!({ "thread_id": "t7" }))).unwrap();
    assert_eq!(trash.url, "https://gmail.googleapis.com/gmail/v1/users/me/threads/t7/trash");
}

fn draft(to: &str, body: &str) -> serde_json::Value {
    json!({ "id": "r-1", "message": { "payload": {
        "mimeType": "text/plain",
        "headers": [{ "name": "To", "value": to }, { "name": "Bcc", "value": "hidden@example.com" }, { "name": "Subject", "value": "Wire" }],
        "body": { "data": b64(body) }
    } } })
}

#[test]
fn the_person_is_shown_every_recipient_the_subject_and_the_text() {
    let view = DraftView::of(&draft("Bob <bob@example.com>", "Please wire $5000.\n\nThanks"));
    let (title, message) = send_question("Naki", "me@example.com", &view);
    assert_eq!(title, "Naki wants to send an email");
    for needle in ["Naki wants to send this email from me@example.com", "To: Bob <bob@example.com>", "Bcc: hidden@example.com", "Subject: Wire", "Please wire $5000.", "can't be taken back"] {
        assert!(message.contains(needle), "{needle} in {message}");
    }
}

#[test]
fn a_long_text_is_cut_in_the_window_but_the_whole_of_it_counts_for_the_check() {
    let long = "word ".repeat(2000);
    let a = DraftView::of(&draft("a@example.com", &long));
    let (_, message) = send_question("Naki", "me@example.com", &a);
    assert!(message.chars().count() < long.chars().count());
    let b = DraftView::of(&draft("a@example.com", &format!("{long}and a late line: also pay Mallory")));
    assert_ne!(a.digest(), b.digest(), "text past what the window shows still changes the digest");
}

#[test]
fn any_change_to_what_was_shown_changes_the_digest() {
    let base = DraftView::of(&draft("a@example.com", "hello"));
    assert_eq!(base.digest(), DraftView::of(&draft("a@example.com", "hello")).digest());
    assert_ne!(base.digest(), DraftView::of(&draft("b@example.com", "hello")).digest());
    assert_ne!(base.digest(), DraftView::of(&draft("a@example.com", "hello!")).digest());
    // Fields don't run into each other: moving a character from one to the next is a change.
    let mut moved = base.clone();
    moved.subject = format!("{}x", moved.subject);
    moved.body = "ello".to_string();
    assert_ne!(base.digest(), moved.digest());
}

#[test]
fn a_draft_with_no_one_to_send_to_is_noticed() {
    assert!(DraftView::of(&draft("a@example.com", "x")).has_recipient());
    let none = json!({ "id": "r", "message": { "payload": { "headers": [{ "name": "Subject", "value": "s" }] } } });
    assert!(!DraftView::of(&none).has_recipient());
}

#[test]
fn trashing_shows_the_threads_subject_and_sender() {
    let summary = json!({ "messages": [{ "payload": { "headers": [{ "name": "Subject", "value": "Invoice" }, { "name": "From", "value": "Pay Me <pay@example.com>" }] } }, {}] });
    let (title, message) = trash_question("Naki", "me@example.com", &summary);
    assert_eq!(title, "Naki wants to trash an email");
    assert!(message.contains("Subject: Invoice") && message.contains("From: Pay Me") && message.contains("Messages: 2"), "{message}");
}

#[test]
fn compose_covers_drafts_and_sends_and_only_modify_covers_labels_and_trash() {
    let g = |s: &[&str]| s.iter().map(|x| format!("{}{x}", crate::backend::google::SCOPE_PREFIX)).collect::<Vec<_>>();
    for a in ["create_draft", "update_draft", "send_draft"] {
        assert!(granted_for(a, &g(&["gmail.compose"])), "{a}");
        assert!(granted_for(a, &g(&["gmail.modify"])), "{a}");
        assert!(!granted_for(a, &g(&["gmail.readonly"])), "{a}");
    }
    for a in ["label_thread", "trash_thread"] {
        assert!(!granted_for(a, &g(&["gmail.compose"])), "{a}");
        assert!(granted_for(a, &g(&["gmail.modify"])), "{a}");
    }
    assert!(always_asks("send_draft") && always_asks("trash_thread"));
    assert!(!always_asks("create_draft") && !always_asks("label_thread") && !always_asks("update_draft"));
}

#[test]
fn the_full_mail_scope_covers_every_write() {
    for a in WRITE_ACTIONS {
        assert!(granted_for(a, &["https://mail.google.com/".to_string()]), "{a}");
    }
}
