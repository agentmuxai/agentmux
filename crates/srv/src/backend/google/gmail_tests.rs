// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use base64::Engine as _;
use serde_json::json;

use super::*;

fn b64(s: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s)
}

fn args(v: serde_json::Value) -> GmailArgs {
    serde_json::from_value(v).unwrap()
}

fn q<'a>(r: &'a GoogleRequest, k: &str) -> Option<&'a str> {
    r.query.iter().find(|(n, _)| *n == k).map(|(_, v)| v.as_str())
}

#[test]
fn a_search_asks_for_threads_with_the_words_and_a_capped_page() {
    let r = plan("search_threads", &args(json!({ "query": "from:alice newer_than:7d", "max_results": 500, "page_token": "tok" }))).unwrap();
    assert_eq!(r.url, "https://gmail.googleapis.com/gmail/v1/users/me/threads");
    assert_eq!(q(&r, "q"), Some("from:alice newer_than:7d"));
    assert_eq!(q(&r, "maxResults"), Some("25"));
    assert_eq!(q(&r, "pageToken"), Some("tok"));
    let plain = plan("search_threads", &args(json!({}))).unwrap();
    assert_eq!(q(&plain, "maxResults"), Some("10"));
    assert!(q(&plain, "q").is_none());
}

#[test]
fn a_thread_is_read_in_full_by_id_and_a_bad_id_is_refused() {
    let r = plan("get_thread", &args(json!({ "thread_id": "18c2f0a1b2c3d4e5" }))).unwrap();
    assert_eq!(r.url, "https://gmail.googleapis.com/gmail/v1/users/me/threads/18c2f0a1b2c3d4e5");
    assert_eq!(q(&r, "format"), Some("full"));
    assert!(plan("get_thread", &args(json!({}))).unwrap_err().contains("thread_id"));
    assert!(plan("get_thread", &args(json!({ "thread_id": "../drafts" }))).unwrap_err().contains("isn't a Gmail id"));
}

#[test]
fn an_action_the_tool_does_not_have_is_refused_with_the_list() {
    let e = plan("send_message", &args(json!({}))).unwrap_err();
    assert!(e.contains("search_threads") && e.contains("list_labels"), "{e}");
}

#[test]
fn a_summary_asks_for_three_headers_only() {
    let r = summary_plan("abc123").unwrap();
    assert_eq!(q(&r, "format"), Some("metadata"));
    let headers: Vec<_> = r.query.iter().filter(|(n, _)| *n == "metadataHeaders").map(|(_, v)| v.as_str()).collect();
    assert_eq!(headers, ["From", "Subject", "Date"]);
}

#[test]
fn readonly_covers_everything_and_metadata_only_the_labels() {
    let g = |s: &[&str]| s.iter().map(|x| format!("{SCOPE_PREFIX}{x}")).collect::<Vec<_>>();
    for a in READ_ACTIONS {
        assert!(granted_for(a, &g(&["gmail.readonly"])), "{a}");
        assert!(granted_for(a, &g(&["gmail.modify"])), "{a}");
        assert!(!granted_for(a, &g(&["gmail.compose"])), "{a}: compose alone reads nothing");
        assert!(!granted_for(a, &[]));
    }
    assert!(granted_for("list_labels", &g(&["gmail.metadata"])));
    assert!(!granted_for("search_threads", &g(&["gmail.metadata"])));
}

#[test]
fn a_search_answer_carries_who_wrote_each_thread_and_the_next_page() {
    let list = json!({ "threads": [{ "id": "t1", "snippet": "Hi  there\n friend" }, { "id": "t2", "snippet": "" }], "nextPageToken": "next" });
    let summary = json!({ "messages": [
        { "payload": { "headers": [{ "name": "Subject", "value": "Lunch?" }, { "name": "from", "value": "Alice <a@example.com>" }] } },
        { "payload": { "headers": [{ "name": "Date", "value": "Mon, 26 Oct 2026 12:00:00 +0000" }] } }
    ] });
    let out = shape_search(&list, &[summary, serde_json::Value::Null]);
    assert_eq!(out["threads"][0]["subject"], "Lunch?");
    assert_eq!(out["threads"][0]["from"], "Alice <a@example.com>");
    assert_eq!(out["threads"][0]["last_date"], "Mon, 26 Oct 2026 12:00:00 +0000");
    assert_eq!(out["threads"][0]["messages"], 2);
    assert_eq!(out["threads"][0]["snippet"], "Hi there friend");
    assert!(out["threads"][1].get("subject").is_none(), "a thread whose summary failed still lists");
    assert_eq!(out["next_page_token"], "next");
}

#[test]
fn a_thread_shows_each_message_with_its_text_and_attachments_listed_not_fetched() {
    let thread = json!({ "id": "t1", "messages": [{
        "id": "m1", "labelIds": ["INBOX", "UNREAD"],
        "payload": {
            "mimeType": "multipart/mixed",
            "headers": [{ "name": "From", "value": "Bob <b@example.com>" }, { "name": "To", "value": "me@example.com" }, { "name": "Subject", "value": "Report" }],
            "parts": [
                { "mimeType": "multipart/alternative", "parts": [
                    { "mimeType": "text/plain", "body": { "data": b64("Hello,\r\n\r\n\r\n\r\nsee attached.\r\n") } },
                    { "mimeType": "text/html", "body": { "data": b64("<p>Hello</p>") } }
                ] },
                { "mimeType": "application/pdf", "filename": "report.pdf", "body": { "size": 4096, "attachmentId": "att1" } }
            ]
        }
    }] });
    let out = shape("get_thread", &thread);
    let m = &out["messages"][0];
    assert_eq!(m["from"], "Bob <b@example.com>");
    assert_eq!(m["subject"], "Report");
    assert_eq!(m["labels"], json!(["INBOX", "UNREAD"]));
    assert_eq!(m["body"], "Hello,\n\nsee attached.");
    assert_eq!(m["attachments"][0], json!({ "filename": "report.pdf", "mime_type": "application/pdf", "size": 4096 }));
    assert!(!out.to_string().contains("att1"), "no attachment id or bytes in the answer");
}

#[test]
fn a_mail_with_only_html_is_read_as_text_without_scripts_or_styles() {
    let html = "<html><style>p{color:red}</style><body><p>Hi &amp; welcome</p><script>steal()</script><div>Line two</div></body></html>";
    let thread = json!({ "id": "t", "messages": [{ "id": "m", "payload": { "mimeType": "text/html", "body": { "data": b64(html) } } }] });
    let body = shape("get_thread", &thread)["messages"][0]["body"].as_str().unwrap().to_string();
    assert!(body.contains("Hi & welcome") && body.contains("Line two"), "{body}");
    assert!(!body.contains("steal") && !body.contains("color:red") && !body.contains('<'), "{body}");
}

#[test]
fn a_long_message_and_a_long_thread_are_cut() {
    let big = "x".repeat(BODY_CHARS + 500);
    let msg = |id: &str| json!({ "id": id, "payload": { "mimeType": "text/plain", "body": { "data": b64(&big) } } });
    let thread = json!({ "id": "t", "messages": (0..(MESSAGES + 5)).map(|i| msg(&format!("m{i}"))).collect::<Vec<_>>() });
    let out = shape("get_thread", &thread);
    let msgs = out["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), MESSAGES);
    assert_eq!(out["messages_not_shown"], 5);
    assert_eq!(msgs[0]["body_cut"], true);
    let total: usize = msgs.iter().map(|m| m["body"].as_str().unwrap().chars().count()).sum();
    assert!(total <= THREAD_CHARS + MESSAGES, "{total}");
}

#[test]
fn labels_are_listed_by_id_name_and_type() {
    let out = shape("list_labels", &json!({ "labels": [{ "id": "INBOX", "name": "INBOX", "type": "system" }, { "id": "Label_1", "name": "Receipts", "type": "user", "messagesTotal": 9 }] }));
    assert_eq!(out["labels"][1], json!({ "id": "Label_1", "name": "Receipts", "type": "user" }));
}

#[test]
fn the_log_names_the_thread_and_never_the_search_words() {
    assert_eq!(logged_ids("get_thread", &args(json!({ "thread_id": " t1 " }))), "t1");
    assert_eq!(logged_ids("search_threads", &args(json!({ "query": "from:my-lawyer" }))), "");
}
