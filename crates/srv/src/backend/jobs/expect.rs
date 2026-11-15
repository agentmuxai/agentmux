// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! A job on the `expect` source is a dead man's switch: it starts a run when a job it watches has **not** had a good
//! run lately (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §3.2). Silence is the event: "the nightly report
//! did not come" is the failure nobody sees, and this is how someone does.
//!
//! The job's filter says what is expected:
//!   * `job`             required: the name or id of the job whose runs are looked at.
//!   * `within_minutes`  required: 1 to 43200 (30 days). The watched job should have a good run that ended within
//!                       this long ago.
//!   * `every_minutes`   how often to look, 1 to 1440, default 5.
//!
//! A good run is a finished one whose outcome is `ok` or `findings` (it did its work). A run that failed, or ended
//! with an unknown outcome, is not one. While a run of the watched job is still going, nothing fires: it may yet
//! succeed in time.
//!
//! When the last good run is older than `within_minutes` (or there is none, or no job has that name), the job's
//! run starts **once for that gap**: the event is keyed by the last good run, so a restart, or the next look, does not
//! repeat it, and a good run followed by a new gap is a new event. The run reads `{{trigger.expected}}` (the watched
//! job), `{{trigger.within_minutes}}`, `{{trigger.last_ok_ms}}` (null when there has never been one),
//! `{{trigger.job_found}}` and `{{trigger.last_seen}}` (the newest run, if any).
//!
//! The local form checks this machine's run ledger, so it cannot fire while the machine is off; the cloud form that
//! can is a later phase.

use std::collections::HashMap;

use serde_json::{json, Map, Value};

use super::definition::{event_source, trigger_kind};
use super::poll::Check;
use crate::backend::cron::{CronScheduler, JobEvent};
use crate::backend::storage::cron::CronJob;

pub const MAX_WITHIN_MINUTES: u64 = 30 * 24 * 60;
pub const DEFAULT_EVERY_MINUTES: u64 = 5;
const MAX_EVERY_MINUTES: u64 = 24 * 60;
const KEYS: [&str; 3] = ["job", "within_minutes", "every_minutes"];
/// How many of the watched job's newest runs are read.
const RUNS_READ: u64 = 100;

/// An `expect` trigger's filter: the job, how recent a good run must be, and how often to look.
pub fn validate_filter(filter: &Map<String, Value>) -> Result<(), String> {
    if let Some(key) = filter.keys().find(|k| !KEYS.contains(&k.as_str())) {
        return Err(format!("trigger: expect has no filter key {key:?}; it has {}", KEYS.join(", ")));
    }
    if !filter.get("job").and_then(Value::as_str).is_some_and(|j| !j.trim().is_empty()) {
        return Err("trigger: an expect filter has job, the name or id of the job it watches".to_string());
    }
    match filter.get("within_minutes").and_then(Value::as_u64) {
        Some(n) if (1..=MAX_WITHIN_MINUTES).contains(&n) => {}
        _ => return Err(format!("trigger: an expect filter has within_minutes, a whole number of 1 to {MAX_WITHIN_MINUTES}")),
    }
    if filter.get("every_minutes").is_some_and(|v| !v.as_u64().is_some_and(|n| (1..=MAX_EVERY_MINUTES).contains(&n))) {
        return Err(format!("trigger: expect every_minutes is a whole number of 1 to {MAX_EVERY_MINUTES}"));
    }
    Ok(())
}

/// What an expect job watches, read from its filter.
#[derive(Debug, PartialEq)]
struct Plan {
    job: String,
    within_ms: i64,
    every_ms: i64,
}

fn plan_of(job: &CronJob) -> Option<Plan> {
    if !job.enabled || job.trigger_kind != trigger_kind::EVENT {
        return None;
    }
    let t = &job.trigger_event;
    if t.get("source").and_then(Value::as_str) != Some(event_source::EXPECT) {
        return None;
    }
    let filter = t.get("filter").and_then(Value::as_object)?;
    validate_filter(filter).ok()?;
    Some(Plan {
        job: filter.get("job").and_then(Value::as_str)?.trim().to_string(),
        within_ms: filter.get("within_minutes").and_then(Value::as_u64)? as i64 * 60_000,
        every_ms: filter.get("every_minutes").and_then(Value::as_u64).unwrap_or(DEFAULT_EVERY_MINUTES) as i64 * 60_000,
    })
}

/// A run that did its work.
fn is_good(run: &Value) -> bool {
    run.get("state").and_then(Value::as_str) == Some("finished") && matches!(run.get("outcome").and_then(Value::as_str), Some("ok" | "findings"))
}

fn is_open(run: &Value) -> bool {
    matches!(run.get("state").and_then(Value::as_str), Some("scheduled" | "pending" | "running"))
}

fn ended_or_started(run: &Value) -> i64 {
    run.get("ended_ms").or_else(|| run.get("started_ms")).and_then(Value::as_i64).unwrap_or(0)
}

/// What the watched job's runs say at `now`: nothing to do, or the event to start.
#[derive(Debug, PartialEq)]
pub(crate) enum Verdict {
    /// A good run is recent enough, or one is still going.
    Fine,
    /// The gap: the event's id and input.
    Missed { id: String, input: Value },
}

pub(crate) fn judge(plan_job: &str, within_minutes: u64, answer: &Value, now: i64) -> Verdict {
    let found = answer.get("found").and_then(Value::as_bool).unwrap_or(false);
    let runs: Vec<&Value> = answer.get("runs").and_then(Value::as_array).map(|r| r.iter().collect()).unwrap_or_default();
    if answer.get("ambiguous").and_then(Value::as_bool).unwrap_or(false) {
        // Two jobs share the name: it is not known which is meant, and a guess would be a false alarm.
        return Verdict::Fine;
    }
    let within_ms = within_minutes as i64 * 60_000;
    if runs.iter().any(|r| is_open(r) && now - r.get("started_ms").and_then(Value::as_i64).unwrap_or(now) < within_ms) {
        return Verdict::Fine;
    }
    let last_ok = runs.iter().filter(|r| is_good(r)).map(|r| ended_or_started(r)).max();
    if last_ok.is_some_and(|ok| now - ok <= within_ms) {
        return Verdict::Fine;
    }
    let newest = runs.iter().max_by_key(|r| r.get("started_ms").and_then(Value::as_i64).unwrap_or(0));
    let last_seen = newest.map(|r| json!({ "id": r.get("id"), "state": r.get("state"), "outcome": r.get("outcome"), "started_ms": r.get("started_ms"), "ended_ms": r.get("ended_ms") })).unwrap_or(Value::Null);
    let key = last_ok.map_or("never".to_string(), |ms| ms.to_string());
    Verdict::Missed {
        id: format!("expect:{plan_job}:{key}"),
        input: json!({ "expected": plan_job, "within_minutes": within_minutes, "last_ok_ms": last_ok, "job_found": found, "last_seen": last_seen }),
    }
}

/// The expect poller's memory: when each job looks next.
#[derive(Default)]
pub struct ExpectState {
    next_due_ms: HashMap<String, i64>,
}

/// Looks at the watched job of every expect job that is due and starts a run for each gap. Returns how many runs it
/// started or queued. `check` answers the App query's `jobs`.
pub async fn expect_due(scheduler: &CronScheduler, all: &[CronJob], check: &Check, state: &mut ExpectState, now: i64) -> usize {
    let watching: Vec<(&CronJob, Plan)> = all.iter().filter_map(|j| plan_of(j).map(|p| (j, p))).collect();
    state.next_due_ms.retain(|id, _| watching.iter().any(|(j, _)| &j.id == id));
    let mut started = 0;
    for (job, plan) in &watching {
        let due = state.next_due_ms.entry(job.id.clone()).or_insert(now);
        if now < *due {
            continue;
        }
        *due = now + plan.every_ms;
        let mut ask = Map::new();
        ask.insert("query".into(), json!("jobs"));
        ask.insert("job".into(), json!(plan.job));
        ask.insert("limit".into(), json!(RUNS_READ));
        let answer = match check(ask).await {
            Ok(answer) => answer,
            Err(e) => {
                tracing::warn!(job_id = %job.id, watched = %plan.job, error = %e, "jobs: an expect couldn't read the runs; looking again next time");
                continue;
            }
        };
        let within_minutes = (plan.within_ms / 60_000) as u64;
        if let Verdict::Missed { id, input } = judge(&plan.job, within_minutes, &answer, now) {
            let fired = scheduler.fire_event(job, JobEvent { id, source: event_source::EXPECT.to_string(), input }).await;
            started += usize::from(fired.run_id.is_some());
        }
    }
    started
}
