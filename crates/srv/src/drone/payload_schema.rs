// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a drone can read of what started it (`{{trigger.*}}`), field by field, and which of it is outside text
//! (Drone real-world jobs spec §3.6).
//!
//! Every trigger kind publishes a schema: each field is **clean** (a number, a flag, a value from a short list, an id
//! AgentMux made) or **tainted** (a text somebody outside the person's own design wrote: a title, a log, a body, a
//! file name). The canvas lists the fields to complete `{{trigger.…}}` and marks a reference to a tainted one.
//!
//! This is what the graph can be told about its input; it is not what keeps a run safe. The engine still treats the
//! whole trigger as outside text (`ExecutionScope::bits_of`) until the taint rules of phase 4 read these schemas, so
//! a mistake here shows a wrong mark and never lets a value through.

use serde::Serialize;
use serde_json::{Map, Value};

use crate::backend::jobs::definition::event_source;
use crate::backend::jobs::{app_events, poll};

/// One field a run reads as `{{trigger.<path>}}`.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct PayloadField {
    /// The dotted path under `trigger`, `*` for "any field the sender chose".
    pub path: String,
    /// `text`, `number`, `flag`, `id`, `choice`, or `any`.
    pub kind: String,
    /// True for a typed value; false for outside text.
    pub clean: bool,
    /// One line for the person: what it is.
    pub note: String,
}

fn field(path: &str, kind: &str, clean: bool, note: &str) -> PayloadField {
    PayloadField { path: path.to_string(), kind: kind.to_string(), clean, note: note.to_string() }
}

/// What every run of a drone job has, whatever started it. These are set by AgentMux, never by the sender.
fn framing() -> Vec<PayloadField> {
    vec![
        field("kind", "choice", true, "how the run started: schedule, manual, retry or event"),
        field("run_id", "id", true, "this run's id in the ledger"),
        field("fired_at_ms", "number", true, "when the run started"),
        field("scheduled_for_ms", "number", true, "the slot a timer run was due for"),
        field("job.id", "id", true, "the job that started the run"),
        field("job.name", "text", false, "the job's name"),
    ]
}

/// The fields of the event named in an `app_event` filter, as `app_events::event_of` builds them.
fn app_event_fields(event: &str) -> Vec<PayloadField> {
    let mut out = vec![field("event", "choice", true, "the event's name")];
    for name in app_events::fields_of(event).unwrap_or(&[]) {
        out.push(match (event, *name) {
            (_, "agent") => field("agent", "id", true, "the agent's name"),
            (_, "outcome") => field("outcome", "choice", true, "completed, errored or interrupted"),
            (_, "cost_usd") => field("cost_usd", "number", true, "what the turn cost"),
            (_, "reason") => field("reason", "text", false, "why the agent stopped, as the failure says it"),
            (_, "version") => field("version", "text", true, "the version AgentMux started as"),
            (_, "upgraded") => field("upgraded", "flag", true, "whether this start made an upgrade"),
            (_, "upgraded_from") => field("upgraded_from", "text", true, "the version it upgraded from, empty if none"),
            // A field added to an event but not yet described here: outside text until someone says otherwise.
            (_, other) => field(other, "any", false, "not described yet"),
        });
    }
    out
}

/// The fields of one poll item, by query. `item` is one entry of the list the check answered with.
fn poll_fields(query: &str) -> Vec<PayloadField> {
    let mut out = vec![field("query", "choice", true, "which check found it")];
    match query {
        "tasks" => out.extend([
            field("key", "id", true, "the task's number"),
            field("item.num", "number", true, "the task's number"),
            field("item.state", "choice", true, "where the task stands"),
            field("item.holder", "id", true, "the agent that holds it"),
            field("item.title", "text", false, "the task's title"),
            field("item.pr", "text", false, "the pull request link"),
            field("item.branch", "text", false, "the branch name"),
        ]),
        "jobs" => out.extend([
            field("key", "id", true, "the run's id"),
            field("item.id", "id", true, "the run's id"),
            field("item.job_id", "id", true, "the job's id"),
            field("item.state", "choice", true, "the run's state"),
            field("item.outcome", "choice", true, "how the run ended"),
            field("item.trigger", "choice", true, "how the run started"),
            field("item.started_ms", "number", true, "when it started"),
            field("item.ended_ms", "number", true, "when it ended"),
            field("item.cost_usd", "number", true, "what it cost"),
            field("item.job", "text", false, "the job's name"),
            field("item.summary", "text", false, "the run's summary line"),
        ]),
        "agents" => out.extend([
            field("key", "id", true, "the agent and its state"),
            field("item.agent", "id", true, "the agent's name"),
            field("item.state", "choice", true, "the agent's state"),
            field("item.detail", "text", false, "what the state says"),
        ]),
        "lan_peers" => out.extend([
            field("key", "text", false, "the peer's host name, as the peer says it"),
            field("item", "any", false, "what the peer reports about itself"),
        ]),
        // A site's list: nothing in it is AgentMux's own.
        _ => out.extend([field("key", "text", false, "the item's id, as the site gives it"), field("item", "any", false, "the item, as the site gives it")]),
    }
    out
}

/// The fields a run reads, for a job started by `source` (an `event_source`, or `None` for a timer or a hand run) with
/// the trigger's own `filter`. The framing fields come first.
pub fn fields_of(source: Option<&str>, filter: &Map<String, Value>) -> Vec<PayloadField> {
    let mut out = framing();
    match source {
        Some(event_source::APP_EVENT) => out.extend(app_event_fields(filter.get("event").and_then(Value::as_str).unwrap_or(""))),
        Some(event_source::POLL) => out.extend(poll_fields(filter.get("query").and_then(Value::as_str).unwrap_or(""))),
        Some(event_source::FILE_WATCH) => out.extend([
            field("root", "text", true, "the folder the job watches"),
            field("count", "number", true, "how many paths changed"),
            field("change", "choice", true, "created, modified, removed or mixed"),
            field("paths", "any", false, "the changed paths: names are chosen by whoever wrote the files"),
            field("first", "text", false, "the first changed path"),
        ]),
        Some(event_source::WEBHOOK_LOCAL) => out.push(field("*", "any", false, "the JSON body: every field is outside text")),
        Some(event_source::EXPECT) => out.extend([
            field("expected", "id", true, "the job that was expected"),
            field("within_minutes", "number", true, "how long it had"),
            field("last_ok_ms", "number", true, "when it last ended well"),
            field("job_found", "flag", true, "whether the job exists"),
            field("last_seen.id", "id", true, "the newest run's id"),
            field("last_seen.state", "choice", true, "the newest run's state"),
            field("last_seen.outcome", "choice", true, "the newest run's outcome"),
            field("last_seen.started_ms", "number", true, "when it started"),
            field("last_seen.ended_ms", "number", true, "when it ended"),
        ]),
        // A timer has only the framing. A run started by hand or by another agent reads what that sender gave.
        _ => out.push(field("*", "any", false, "what the sender gave: outside text")),
    }
    out
}

/// Whether `{{trigger.<path>}}` is outside text under `fields`: it is unless the path, or a parent of it, is a clean
/// field. A path no field names is outside text (it can only come from the sender).
pub fn is_tainted(fields: &[PayloadField], path: &str) -> bool {
    let under = |p: &str, of: &str| p == of || p.strip_prefix(of).is_some_and(|rest| rest.starts_with('.'));
    match fields.iter().filter(|f| f.path != "*" && under(path, &f.path)).max_by_key(|f| f.path.len()) {
        Some(f) => !f.clean,
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn filter(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap_or_default()
    }

    fn clean_paths(fields: &[PayloadField]) -> Vec<&str> {
        fields.iter().filter(|f| f.clean).map(|f| f.path.as_str()).collect()
    }

    #[test]
    fn every_run_has_the_framing_fields_clean_but_the_job_name() {
        let f = fields_of(None, &Map::new());
        for p in ["kind", "run_id", "fired_at_ms", "scheduled_for_ms", "job.id"] {
            assert!(!is_tainted(&f, p), "{p} is made by AgentMux");
        }
        assert!(is_tainted(&f, "job.name"));
    }

    #[test]
    fn a_hand_run_or_timer_reads_only_outside_text_besides_the_framing() {
        let f = fields_of(None, &Map::new());
        assert!(is_tainted(&f, "run"), "a field the sender chose");
        assert!(is_tainted(&f, "run.conclusion"));
    }

    #[test]
    fn an_app_event_describes_every_field_the_event_carries() {
        for event in app_events::name::ALL {
            let f = fields_of(Some(event_source::APP_EVENT), &filter(json!({ "event": event })));
            for name in app_events::fields_of(event).expect("a listed event has fields") {
                let found = f.iter().find(|x| x.path == *name).unwrap_or_else(|| panic!("{event}: {name} missing"));
                assert_ne!(found.note, "not described yet", "{event}.{name} needs its own line");
            }
        }
    }

    #[test]
    fn a_crash_reason_is_outside_text_and_the_agent_and_outcome_are_not() {
        let f = fields_of(Some(event_source::APP_EVENT), &filter(json!({ "event": "agent.crashed" })));
        assert!(is_tainted(&f, "reason"));
        assert!(!is_tainted(&f, "agent"));
        let t = fields_of(Some(event_source::APP_EVENT), &filter(json!({ "event": "agent.turn_ended" })));
        assert!(!is_tainted(&t, "outcome") && !is_tainted(&t, "cost_usd"));
    }

    #[test]
    fn a_task_poll_keeps_the_title_apart_from_the_number() {
        let f = fields_of(Some(event_source::POLL), &filter(json!({ "query": "tasks" })));
        assert!(!is_tainted(&f, "item.num") && !is_tainted(&f, "key"));
        assert!(is_tainted(&f, "item.title"));
        assert!(is_tainted(&f, "item"), "the item whole has the title in it");
        assert!(is_tainted(&f, "item.other"), "a field nobody described");
    }

    #[test]
    fn a_site_poll_is_all_outside_text_but_the_query() {
        let f = fields_of(Some(event_source::POLL), &filter(json!({ "query": "api", "url": "https://x.test/a" })));
        assert_eq!(clean_paths(&f).iter().filter(|p| !["kind", "run_id", "fired_at_ms", "scheduled_for_ms", "job.id"].contains(p)).collect::<Vec<_>>(), vec![&"query"]);
        assert!(is_tainted(&f, "item.title") && is_tainted(&f, "key"));
    }

    #[test]
    fn a_webhook_body_and_file_names_are_outside_text() {
        let w = fields_of(Some(event_source::WEBHOOK_LOCAL), &Map::new());
        assert!(is_tainted(&w, "anything"));
        let fw = fields_of(Some(event_source::FILE_WATCH), &Map::new());
        assert!(is_tainted(&fw, "paths") && is_tainted(&fw, "first"));
        assert!(!is_tainted(&fw, "count") && !is_tainted(&fw, "change"));
    }

    #[test]
    fn a_child_path_follows_the_nearest_field_that_names_it() {
        let f = fields_of(Some(event_source::EXPECT), &Map::new());
        assert!(!is_tainted(&f, "last_seen.outcome"));
        assert!(is_tainted(&f, "last_seen"), "the whole object is not a clean field of its own");
    }

    #[test]
    fn every_poll_query_has_a_schema() {
        for q in poll::QUERIES {
            let f = fields_of(Some(event_source::POLL), &filter(json!({ "query": q })));
            assert!(f.iter().any(|x| x.path == "key"), "{q} has a key");
        }
    }
}
