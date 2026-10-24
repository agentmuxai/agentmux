// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-24T09:00:00Z").unwrap().with_timezone(&Utc)
}

fn q<'a>(req: &'a GoogleRequest, k: &str) -> Option<&'a str> {
    req.query.iter().find(|(n, _)| *n == k).map(|(_, v)| v.as_str())
}

#[test]
fn list_events_reads_the_primary_calendar_for_a_week_from_now() {
    let req = plan("list_events", &CalendarArgs::default(), now()).unwrap();
    assert_eq!(req.method, reqwest::Method::GET);
    assert_eq!(req.url, "https://www.googleapis.com/calendar/v3/calendars/primary/events");
    assert_eq!(q(&req, "timeMin"), Some("2026-10-24T09:00:00Z"));
    assert_eq!(q(&req, "timeMax"), Some("2026-10-31T09:00:00Z"));
    assert_eq!(q(&req, "singleEvents"), Some("true"));
    assert_eq!(q(&req, "maxResults"), Some("25"));
    assert_eq!(q(&req, "q"), None);
}

#[test]
fn a_calendar_id_is_escaped_into_the_path_and_results_are_capped() {
    let args = CalendarArgs {
        calendar_id: Some("team/ops#1@group.calendar.google.com".into()),
        time_min: Some("2026-10-24T00:00:00-07:00".into()),
        max_results: Some(5000),
        page_token: Some("p2".into()),
        ..Default::default()
    };
    let req = plan("list_events", &args, now()).unwrap();
    assert_eq!(
        req.url,
        "https://www.googleapis.com/calendar/v3/calendars/team%2Fops%231@group.calendar.google.com/events"
    );
    assert_eq!(q(&req, "timeMin"), Some("2026-10-24T07:00:00Z"), "kept as UTC");
    assert_eq!(q(&req, "maxResults"), Some("100"));
    assert_eq!(q(&req, "pageToken"), Some("p2"));
}

#[test]
fn bad_arguments_are_refused_with_what_to_write() {
    let bad_time = CalendarArgs { time_min: Some("tomorrow".into()), ..Default::default() };
    assert!(plan("list_events", &bad_time, now()).unwrap_err().contains("2026-10-24T09:00:00Z"));
    let backwards = CalendarArgs { time_max: Some("2026-10-23T00:00:00Z".into()), ..Default::default() };
    assert!(plan("list_events", &backwards, now()).unwrap_err().contains("after"));
    assert!(plan("search_events", &CalendarArgs::default(), now()).unwrap_err().contains("query"));
    assert!(plan("create_event", &CalendarArgs::default(), now()).unwrap_err().contains("list_calendars"));
    let long = CalendarArgs { time_max: Some("2027-01-30T00:00:00Z".into()), ..Default::default() };
    assert!(plan("free_busy", &long, now()).unwrap_err().contains("62 days"));
}

#[test]
fn search_events_sends_the_words_and_free_busy_posts_its_calendars() {
    let args = CalendarArgs { query: Some("  dentist ".into()), ..Default::default() };
    assert_eq!(q(&plan("search_events", &args, now()).unwrap(), "q"), Some("dentist"));

    let args = CalendarArgs { calendars: Some(vec!["primary".into(), "ana@example.com".into()]), time_zone: Some("Europe/Paris".into()), ..Default::default() };
    let req = plan("free_busy", &args, now()).unwrap();
    assert_eq!((req.method.as_str(), req.url.as_str()), ("POST", "https://www.googleapis.com/calendar/v3/freeBusy"));
    let body = req.body.unwrap();
    assert_eq!(body["items"], serde_json::json!([{ "id": "primary" }, { "id": "ana@example.com" }]));
    assert_eq!(body["timeZone"], "Europe/Paris");
    assert_eq!(logged_ids("free_busy", &args), "primary,ana@example.com");
}

#[test]
fn an_action_runs_on_any_scope_that_covers_it() {
    let g = |s: &[&str]| s.iter().map(|n| format!("https://www.googleapis.com/auth/{n}")).collect::<Vec<_>>();
    let read_only_events = g(&["openid", "calendar.events.readonly"]);
    assert!(granted_for("list_events", &read_only_events));
    assert!(granted_for("search_events", &read_only_events));
    assert!(!granted_for("list_calendars", &read_only_events), "partly granted");
    assert!(!granted_for("free_busy", &read_only_events));
    assert!(granted_for("list_calendars", &g(&["calendar.readonly"])));
    assert!(!granted_for("nope", &g(&["calendar"])));
}

#[test]
fn events_are_trimmed_to_what_an_agent_needs() {
    let long = "x".repeat(5000);
    let attendees: Vec<Value> = (0..60).map(|i| json!({ "email": format!("p{i}@example.com"), "responseStatus": "accepted" })).collect();
    let body = json!({
        "summary": "me@example.com",
        "timeZone": "America/Los_Angeles",
        "nextPageToken": "n2",
        "etag": "\"drop me\"",
        "items": [
            {
                "id": "e1", "summary": "Standup", "status": "confirmed", "htmlLink": "https://calendar.google.com/e1",
                "start": { "dateTime": "2026-10-24T09:30:00-07:00" }, "end": { "dateTime": "2026-10-24T09:45:00-07:00" },
                "organizer": { "email": "boss@example.com" }, "description": long, "attendees": attendees,
                "creator": { "email": "drop@example.com" }, "iCalUID": "drop"
            },
            { "id": "e2", "summary": "Holiday", "start": { "date": "2026-10-26" }, "end": { "date": "2026-10-27" },
              "attendees": [{ "email": "me@example.com", "self": true, "responseStatus": "needsAction" }] }
        ]
    });
    let out = shape("list_events", &body);
    assert_eq!(out["calendar"], "me@example.com");
    assert_eq!(out["next_page_token"], "n2");
    let e1 = &out["events"][0];
    assert_eq!(e1["title"], "Standup");
    assert_eq!(e1["start"], "2026-10-24T09:30:00-07:00");
    assert_eq!(e1["organizer"], "boss@example.com");
    assert_eq!(e1["description"].as_str().unwrap().chars().count(), 2001);
    assert_eq!(e1["attendees"].as_array().unwrap().len(), 50);
    assert_eq!(e1["attendees_not_shown"], 10);
    assert!(e1.get("iCalUID").is_none() && e1.get("creator").is_none() && out.get("etag").is_none());
    let e2 = &out["events"][1];
    assert_eq!(e2["start"], json!({ "date": "2026-10-26" }));
    assert_eq!(e2["attendees"][0], json!({ "email": "me@example.com", "response": "needsAction", "you": true }));
}

#[test]
fn calendars_and_busy_times_are_trimmed_too() {
    let list = shape(
        "list_calendars",
        &json!({ "items": [
            { "id": "me@example.com", "summary": "Me", "primary": true, "accessRole": "owner", "timeZone": "UTC", "etag": "x" },
            { "id": "c2", "summary": "Team", "summaryOverride": "My team", "accessRole": "reader" }
        ] }),
    );
    assert_eq!(list["calendars"][0], json!({ "id": "me@example.com", "calendar_name": "Me", "primary": true, "access": "owner", "time_zone": "UTC" }));
    assert_eq!(list["calendars"][1]["name"], "My team");

    let fb = shape(
        "free_busy",
        &json!({ "timeMin": "a", "timeMax": "b", "calendars": {
            "primary": { "busy": [{ "start": "s1", "end": "e1" }] },
            "ana@example.com": { "busy": [], "errors": [{ "domain": "global", "reason": "notFound" }] }
        } }),
    );
    assert_eq!(fb["calendars"]["primary"]["busy"], json!([{ "start": "s1", "end": "e1" }]));
    assert_eq!(fb["calendars"]["ana@example.com"]["error"], "notFound");
}
