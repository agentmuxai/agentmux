// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The `Calendar` tool's read actions (G1a), named after Google's Calendar
//! MCP tools (SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md §5):
//! `list_calendars`, `list_events`, `search_events`, `free_busy`.
//!
//! Each action is a [`plan`] (the request to Google, from the agent's
//! arguments) and a [`shape`] (Google's answer, trimmed to what an agent
//! needs and capped). Neither touches the network or a token.

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::{clip, GoogleRequest, SCOPE_PREFIX};

const API: &str = "https://www.googleapis.com/calendar/v3";

/// The read actions, in the order the tool lists them.
pub const READ_ACTIONS: [&str; 4] = ["list_calendars", "list_events", "search_events", "free_busy"];

/// Results per page by default, and at most.
const DEFAULT_RESULTS: u32 = 25;
const MAX_RESULTS: u32 = 100;
/// How far ahead `list_events` and `free_busy` look when no end is given.
const DEFAULT_SPAN_DAYS: i64 = 7;
/// The longest window `free_busy` asks about (Google's own limit is wider).
const MAX_FREE_BUSY_DAYS: i64 = 62;
/// Calendars `free_busy` asks about at most.
const MAX_FREE_BUSY_CALENDARS: usize = 20;
/// An event's description, and its attendees, are cut to these.
const DESCRIPTION_CHARS: usize = 2000;
const ATTENDEES: usize = 50;

/// The arguments the read actions take. Unknown fields are ignored.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct CalendarArgs {
    /// A calendar id from `list_calendars`; default `primary`.
    #[serde(default)]
    pub calendar_id: Option<String>,
    /// RFC 3339, e.g. `2026-10-24T09:00:00Z` or `2026-10-24T09:00:00-07:00`.
    #[serde(default)]
    pub time_min: Option<String>,
    #[serde(default)]
    pub time_max: Option<String>,
    /// `search_events`: free text, as Google Calendar's search box takes it.
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub max_results: Option<u32>,
    #[serde(default)]
    pub page_token: Option<String>,
    /// An IANA time zone for the answer's times, e.g. `Europe/Paris`.
    #[serde(default)]
    pub time_zone: Option<String>,
    /// `free_busy`: calendar ids (or people's emails); default `primary`.
    #[serde(default)]
    pub calendars: Option<Vec<String>>,
}

/// The scopes an action can run with: any one of them is enough, so a
/// partial grant still runs what it covers (§7).
pub fn needs(action: &str) -> &'static [&'static str] {
    match action {
        "list_calendars" => &["calendar.calendarlist.readonly", "calendar.readonly", "calendar"],
        "list_events" | "search_events" => &["calendar.events.readonly", "calendar.events", "calendar.readonly", "calendar"],
        "free_busy" => &["calendar.events.freebusy", "calendar.freebusy", "calendar.readonly", "calendar"],
        _ => &[],
    }
}

/// Whether `granted` (full scope URLs, or bare names) covers `action`.
pub fn granted_for(action: &str, granted: &[String]) -> bool {
    needs(action)
        .iter()
        .any(|n| granted.iter().any(|g| g.strip_prefix(SCOPE_PREFIX).unwrap_or(g) == *n))
}

fn time_arg(name: &str, v: Option<&str>) -> Result<Option<DateTime<Utc>>, String> {
    match v.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) => DateTime::parse_from_rfc3339(s)
            .map(|t| Some(t.with_timezone(&Utc)))
            .map_err(|_| format!("{name} \"{s}\" isn't an RFC 3339 time; write it like 2026-10-24T09:00:00Z")),
    }
}

fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// The window an action reads: from `time_min` (default now) to `time_max`
/// (default a week later).
fn window(args: &CalendarArgs, now: DateTime<Utc>) -> Result<(DateTime<Utc>, DateTime<Utc>), String> {
    let min = time_arg("time_min", args.time_min.as_deref())?.unwrap_or(now);
    let max = time_arg("time_max", args.time_max.as_deref())?.unwrap_or(min + Duration::days(DEFAULT_SPAN_DAYS));
    if max <= min {
        return Err("time_max must be after time_min".to_string());
    }
    Ok((min, max))
}

fn calendar_url(calendar_id: &str, tail: &str) -> Result<String, String> {
    let mut url = url::Url::parse(API).map_err(|e| e.to_string())?;
    url.path_segments_mut()
        .map_err(|_| "bad Calendar API URL".to_string())?
        .push("calendars")
        .push(calendar_id)
        .push(tail);
    Ok(url.to_string())
}

fn calendar_id(args: &CalendarArgs) -> String {
    args.calendar_id.as_deref().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("primary").to_string()
}

fn page(args: &CalendarArgs, query: &mut Vec<(&'static str, String)>) {
    let n = args.max_results.unwrap_or(DEFAULT_RESULTS).clamp(1, MAX_RESULTS);
    query.push(("maxResults", n.to_string()));
    if let Some(t) = args.page_token.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        query.push(("pageToken", t.to_string()));
    }
}

/// The request to Google for a read action.
pub fn plan(action: &str, args: &CalendarArgs, now: DateTime<Utc>) -> Result<GoogleRequest, String> {
    let get = |url: String, query| GoogleRequest { method: reqwest::Method::GET, url, query, body: None, text: false };
    let tz = args.time_zone.as_deref().map(str::trim).filter(|s| !s.is_empty());
    match action {
        "list_calendars" => {
            let mut q = Vec::new();
            page(args, &mut q);
            Ok(get(format!("{API}/users/me/calendarList"), q))
        }
        "list_events" | "search_events" => {
            let (min, max) = window(args, now)?;
            let mut q = vec![
                ("timeMin", rfc3339(min)),
                ("timeMax", rfc3339(max)),
                ("singleEvents", "true".to_string()),
                ("orderBy", "startTime".to_string()),
            ];
            if action == "search_events" {
                let text = args.query.as_deref().map(str::trim).unwrap_or("");
                if text.is_empty() {
                    return Err("search_events needs query: the words to look for".to_string());
                }
                q.push(("q", clip(text, 200)));
            }
            if let Some(tz) = tz {
                q.push(("timeZone", tz.to_string()));
            }
            page(args, &mut q);
            Ok(get(calendar_url(&calendar_id(args), "events")?, q))
        }
        "free_busy" => {
            let (min, max) = window(args, now)?;
            if max - min > Duration::days(MAX_FREE_BUSY_DAYS) {
                return Err(format!("free_busy covers at most {MAX_FREE_BUSY_DAYS} days at a time"));
            }
            let ids: Vec<String> = match &args.calendars {
                Some(c) if !c.is_empty() => c.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
                _ => vec![calendar_id(args)],
            };
            if ids.len() > MAX_FREE_BUSY_CALENDARS {
                return Err(format!("free_busy asks about at most {MAX_FREE_BUSY_CALENDARS} calendars at a time"));
            }
            let mut body = json!({
                "timeMin": rfc3339(min),
                "timeMax": rfc3339(max),
                "items": ids.iter().map(|id| json!({ "id": id })).collect::<Vec<_>>(),
            });
            if let Some(tz) = tz {
                body["timeZone"] = json!(tz);
            }
            Ok(GoogleRequest { method: reqwest::Method::POST, url: format!("{API}/freeBusy"), query: Vec::new(), body: Some(body), text: false })
        }
        other => Err(format!("Calendar has no action \"{other}\"; one of {}", READ_ACTIONS.join(", "))),
    }
}

fn s<'a>(v: &'a Value, ptr: &str) -> &'a str {
    v.pointer(ptr).and_then(Value::as_str).unwrap_or("")
}

/// Copy the non-empty string fields `keys` (Google's names) of `from` into
/// `to`, under the names they map to.
fn copy(to: &mut Map<String, Value>, from: &Value, keys: &[(&str, &str)]) {
    for (src, dst) in keys {
        let v = s(from, src);
        if !v.is_empty() {
            to.insert((*dst).to_string(), json!(v));
        }
    }
}

/// An event's start or end: its `dateTime`, or `date` for an all-day event.
fn when(v: &Value) -> Value {
    match (v.get("dateTime").and_then(Value::as_str), v.get("date").and_then(Value::as_str)) {
        (Some(t), _) => json!(t),
        (None, Some(d)) => json!({ "date": d }),
        _ => Value::Null,
    }
}

fn shape_event(e: &Value) -> Value {
    let mut out = Map::new();
    copy(
        &mut out,
        e,
        &[
            ("/id", "id"),
            ("/summary", "title"),
            ("/status", "status"),
            ("/location", "location"),
            ("/organizer/email", "organizer"),
            ("/htmlLink", "link"),
            ("/hangoutLink", "meet_link"),
            ("/recurringEventId", "recurring_event_id"),
        ],
    );
    out.insert("start".into(), when(e.get("start").unwrap_or(&Value::Null)));
    out.insert("end".into(), when(e.get("end").unwrap_or(&Value::Null)));
    let desc = s(e, "/description");
    if !desc.is_empty() {
        out.insert("description".into(), json!(clip(desc, DESCRIPTION_CHARS)));
    }
    if let Some(att) = e.get("attendees").and_then(Value::as_array) {
        let list: Vec<Value> = att
            .iter()
            .take(ATTENDEES)
            .map(|a| {
                let mut m = Map::new();
                copy(&mut m, a, &[("/email", "email"), ("/displayName", "name"), ("/responseStatus", "response")]);
                if a.get("self").and_then(Value::as_bool) == Some(true) {
                    m.insert("you".into(), json!(true));
                }
                Value::Object(m)
            })
            .collect();
        out.insert("attendees".into(), json!(list));
        if att.len() > ATTENDEES {
            out.insert("attendees_not_shown".into(), json!(att.len() - ATTENDEES));
        }
    }
    Value::Object(out)
}

fn next_page(body: &Value, out: &mut Map<String, Value>) {
    let t = s(body, "/nextPageToken");
    if !t.is_empty() {
        out.insert("next_page_token".into(), json!(t));
    }
}

/// Google's answer to a read action, trimmed for the agent.
pub fn shape(action: &str, body: &Value) -> Value {
    let mut out = Map::new();
    match action {
        "list_calendars" => {
            let items: Vec<Value> = body
                .get("items")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|c| {
                            let mut m = Map::new();
                            copy(
                                &mut m,
                                c,
                                &[("/id", "id"), ("/summaryOverride", "name"), ("/summary", "calendar_name"), ("/accessRole", "access"), ("/timeZone", "time_zone")],
                            );
                            if c.get("primary").and_then(Value::as_bool) == Some(true) {
                                m.insert("primary".into(), json!(true));
                            }
                            Value::Object(m)
                        })
                        .collect()
                })
                .unwrap_or_default();
            out.insert("calendars".into(), json!(items));
            next_page(body, &mut out);
        }
        "list_events" | "search_events" => {
            let items: Vec<Value> =
                body.get("items").and_then(Value::as_array).map(|a| a.iter().map(shape_event).collect()).unwrap_or_default();
            copy(&mut out, body, &[("/summary", "calendar"), ("/timeZone", "time_zone")]);
            out.insert("events".into(), json!(items));
            next_page(body, &mut out);
        }
        "free_busy" => {
            let mut cals = Map::new();
            if let Some(c) = body.get("calendars").and_then(Value::as_object) {
                for (id, v) in c {
                    let busy: Vec<Value> = v
                        .get("busy")
                        .and_then(Value::as_array)
                        .map(|b| b.iter().map(|p| json!({ "start": s(p, "/start"), "end": s(p, "/end") })).collect())
                        .unwrap_or_default();
                    let mut m = Map::new();
                    m.insert("busy".into(), json!(busy));
                    if let Some(errs) = v.get("errors").and_then(Value::as_array).filter(|e| !e.is_empty()) {
                        let why: Vec<&str> = errs.iter().map(|e| s(e, "/reason")).collect();
                        m.insert("error".into(), json!(why.join(", ")));
                    }
                    cals.insert(id.clone(), Value::Object(m));
                }
            }
            copy(&mut out, body, &[("/timeMin", "time_min"), ("/timeMax", "time_max")]);
            out.insert("calendars".into(), Value::Object(cals));
        }
        _ => {}
    }
    Value::Object(out)
}

/// The ids a read touched, for the account's log (D7): the calendars, never
/// what was in them.
pub fn logged_ids(action: &str, args: &CalendarArgs) -> String {
    match action {
        "list_events" | "search_events" => calendar_id(args),
        "free_busy" => match &args.calendars {
            Some(c) if !c.is_empty() => c.iter().take(MAX_FREE_BUSY_CALENDARS).cloned().collect::<Vec<_>>().join(","),
            _ => calendar_id(args),
        },
        _ => String::new(),
    }
}

#[cfg(test)]
#[path = "calendar_tests.rs"]
mod tests;
