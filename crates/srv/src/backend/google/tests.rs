// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use serde_json::json;

#[test]
fn calendar_read_scopes_are_the_specs_and_write_adds_events() {
    let read = Service::Calendar.scopes(GrantLevel::Read);
    assert_eq!(
        read,
        [
            "https://www.googleapis.com/auth/calendar.calendarlist.readonly",
            "https://www.googleapis.com/auth/calendar.events.readonly",
            "https://www.googleapis.com/auth/calendar.events.freebusy",
        ]
    );
    let write = Service::Calendar.scopes(GrantLevel::Write);
    assert!(read.iter().all(|s| write.contains(s)), "write includes read");
    assert!(write.contains(&"https://www.googleapis.com/auth/calendar.events".to_string()));
}

#[test]
fn googles_errors_become_one_sentence() {
    let r = |status, body| HttpReply { status, body };
    assert!(google_error(&r(401, json!({}))).contains("reconnect"));
    let scope = json!({ "error": { "message": "Request had insufficient authentication scopes.", "errors": [{ "reason": "insufficientPermissions" }] } });
    assert!(google_error(&r(403, scope)).contains("turn the service on again"));
    assert!(google_error(&r(404, json!({}))).contains("no such calendar"));
    assert_eq!(google_error(&r(500, json!({ "error": { "message": "Backend Error" } }))), "Google answered HTTP 500: Backend Error");
    assert_eq!(google_error(&r(502, json!("<html>"))), "Google answered HTTP 502");
}

#[test]
fn clip_cuts_on_characters_and_marks_the_cut() {
    assert_eq!(clip("short", 10), "short");
    assert_eq!(clip("ééééé", 3), "ééé…");
}
