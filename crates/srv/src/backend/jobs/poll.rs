// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! A job on the `poll` source asks AgentMux's own state a question every so often and starts one run for each item
//! that is new in the answer (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §3.2). Polling spends no agent
//! tokens: the question is one of the App query's, answered by srv.
//!
//! The job's filter says what to ask and how often:
//!   * `query`           required: `tasks`, `jobs`, `agents` or `lan_peers`, as the App query block reads them.
//!   * `every_minutes`   required: 1 to 1440.
//!   * `holder`, `state`, `pr`   (`tasks`) and `job` (`jobs`, required: the runs of that one job), `limit` (both):
//!     the App query's own filters. An answer is cut to `limit`, which a poll leaves at the most the query gives (200
//!     tasks, 100 runs): a new item past the cut, because more than that appeared between two looks, is never seen.
//!     `agents` lists at most 20 problems.
//!   * `include_existing`   optional true: the items that are already there when the job is turned on run too.
//!     Left out, the first poll only takes note of them, so turning a job on does not start a run for everything
//!     that happened before.
//!
//! What counts as an item, and what makes it new:
//!   * `tasks`      each task, by its number.
//!   * `jobs`       each run of the named job, by its id.
//!   * `agents`     each agent that is wanted running and is not up, by its name and state: an agent that goes down
//!                  again in another way is new.
//!   * `lan_peers`  each machine, by its host name.
//!
//! The run reads `{{trigger.query}}`, `{{trigger.key}}` (the item's id) and `{{trigger.item}}` (the item as the
//! query lists it). An item is never reported twice: the job's event ids (`backend::storage::job_events`) remember
//! it for a week, and an item that is still in the answer is remembered for as long as it is there. (An item that is
//! still there when srv restarts more than a week after it was reported is reported again: its id has been forgotten.) A poll that finds
//! more than [`MAX_NEW_PER_POLL`] new items starts that many and leaves the rest for the next poll. A poll whose
//! question fails (the ledger is not ready yet, say) is skipped and asked again at the next time.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use super::definition::{event_source, trigger_kind};
use crate::backend::cron::{CronScheduler, JobEvent};
use crate::backend::storage::cron::CronJob;
use crate::backend::storage::store::Store;

/// The questions a poll may ask.
pub const QUERIES: [&str; 4] = ["tasks", "jobs", "agents", "lan_peers"];
pub const MIN_EVERY_MINUTES: u64 = 1;
pub const MAX_EVERY_MINUTES: u64 = 24 * 60;
/// The most tasks and runs the App query returns (its own caps), which is what a poll asks for unless `limit` says less.
const MAX_TASKS: u64 = 200;
const MAX_RUNS: u64 = 100;
/// The most new items one poll starts a run for.
pub const MAX_NEW_PER_POLL: usize = 20;
/// How often the poller looks at which jobs are due.
const TICK: Duration = Duration::from_secs(30);

const COMMON_KEYS: [&str; 3] = ["query", "every_minutes", "include_existing"];

/// Asks the App query `filter["query"]` with the filter's own keys and returns its answer. Supplied by the
/// binary, which has the App query; the tests supply a fake.
pub type Check = Arc<dyn Fn(Map<String, Value>) -> Pin<Box<dyn Future<Output = Result<Value, String>> + Send>> + Send + Sync>;

/// Where a query's items are in its answer, and which of their fields make one item's id.
struct Shape {
    items: &'static str,
    id: &'static [&'static str],
}

fn shape_of(query: &str) -> Option<Shape> {
    match query {
        "tasks" => Some(Shape { items: "tasks", id: &["num"] }),
        "jobs" => Some(Shape { items: "runs", id: &["id"] }),
        "agents" => Some(Shape { items: "problems", id: &["agent", "state"] }),
        "lan_peers" => Some(Shape { items: "peers", id: &["host"] }),
        _ => None,
    }
}

/// The filter keys a query takes besides the common ones.
fn own_keys(query: &str) -> &'static [&'static str] {
    match query {
        "tasks" => &["holder", "state", "pr", "limit"],
        "jobs" => &["job", "limit"],
        _ => &[],
    }
}

/// A `poll` trigger's filter: the question, how often, and the question's own filters.
pub fn validate_filter(filter: &Map<String, Value>) -> Result<(), String> {
    let query = filter.get("query").and_then(Value::as_str).unwrap_or("");
    if !QUERIES.contains(&query) {
        return Err(format!("trigger: a poll filter names its query, one of {}", QUERIES.join(", ")));
    }
    let own = own_keys(query);
    if let Some(key) = filter.keys().find(|k| !COMMON_KEYS.contains(&k.as_str()) && !own.contains(&k.as_str())) {
        let mut has: Vec<&str> = COMMON_KEYS.to_vec();
        has.extend(own);
        return Err(format!("trigger: a poll of {query} has no filter key {key:?}; it has {}", has.join(", ")));
    }
    match filter.get("every_minutes").and_then(Value::as_u64) {
        Some(n) if (MIN_EVERY_MINUTES..=MAX_EVERY_MINUTES).contains(&n) => {}
        _ => return Err(format!("trigger: a poll filter has every_minutes, a whole number of {MIN_EVERY_MINUTES} to {MAX_EVERY_MINUTES}")),
    }
    if filter.get("include_existing").is_some_and(|v| !v.is_boolean()) {
        return Err("trigger: include_existing is true or false".to_string());
    }
    for key in ["holder", "state", "pr", "job"] {
        if filter.get(key).is_some_and(|v| !v.is_string()) {
            return Err(format!("trigger: poll filter {key:?} is text"));
        }
    }
    if query == "jobs" && !filter.get("job").and_then(Value::as_str).is_some_and(|j| !j.trim().is_empty()) {
        // Without `job` the query lists only the newest run of each job, so a job that finishes two runs between looks would lose one.
        return Err("trigger: a poll of jobs has job, the name or id of the job whose runs it watches".to_string());
    }
    if filter.get("limit").is_some_and(|v| !v.as_u64().is_some_and(|n| (1..=200).contains(&n))) {
        return Err("trigger: poll filter \"limit\" is a whole number of 1 to 200".to_string());
    }
    Ok(())
}

/// What a poll job asks, read from its filter.
#[derive(Debug, PartialEq)]
struct Plan {
    query: String,
    every_ms: i64,
    include_existing: bool,
    /// What the check is asked: the query and its own filters, without how often.
    ask: Map<String, Value>,
    /// Changes when the question does, so a job that is edited starts again from what is there.
    signature: String,
}

fn plan_of(job: &CronJob) -> Option<Plan> {
    if !job.enabled || job.trigger_kind != trigger_kind::EVENT {
        return None;
    }
    let t = &job.trigger_event;
    if t.get("source").and_then(Value::as_str) != Some(event_source::POLL) {
        return None;
    }
    let filter = t.get("filter").and_then(Value::as_object)?;
    validate_filter(filter).ok()?;
    let mut ask = filter.clone();
    ask.remove("every_minutes");
    // Ask for the most the query gives: an answer is cut to `limit`, and a new item past the cut is not seen.
    match filter.get("query").and_then(Value::as_str) {
        Some("tasks") => {
            ask.entry("limit").or_insert(json!(MAX_TASKS));
        }
        Some("jobs") => {
            ask.entry("limit").or_insert(json!(MAX_RUNS));
        }
        _ => {}
    }
    let include_existing = ask.remove("include_existing").and_then(|v| v.as_bool()).unwrap_or(false);
    let signature = hex::encode(&Sha256::digest(Value::Object(ask.clone()).to_string().as_bytes())[..6]);
    Some(Plan {
        query: filter.get("query").and_then(Value::as_str)?.to_string(),
        every_ms: filter.get("every_minutes").and_then(Value::as_u64)? as i64 * 60_000,
        include_existing,
        ask,
        signature,
    })
}

/// A field of an item as the text an id is made of.
fn part(item: &Value, field: &str) -> Option<String> {
    match item.get(field)? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// The items of an answer, each with its id. An item that lacks an id field is not one.
fn items_of(query: &str, answer: &Value) -> Vec<(String, Value)> {
    let Some(shape) = shape_of(query) else { return Vec::new() };
    let Some(items) = answer.get(shape.items).and_then(Value::as_array) else { return Vec::new() };
    let mut seen = HashSet::new();
    items
        .iter()
        .filter_map(|item| {
            let parts = shape.id.iter().map(|f| part(item, f)).collect::<Option<Vec<_>>>()?;
            let id = parts.join(":");
            seen.insert(id.clone()).then(|| (id, item.clone()))
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn items_of_for_test(query: &str, answer: &Value) -> Vec<(String, Value)> {
    items_of(query, answer)
}

/// The id under which a job's first look is kept with its event ids.
fn baseline_marker(signature: &str) -> String {
    format!("poll:baseline:{signature}")
}

#[cfg(test)]
pub(crate) fn marker_of_for_test(job: &CronJob) -> Option<String> {
    plan_of(job).map(|p| baseline_marker(&p.signature))
}

fn event_id(query: &str, id: &str) -> String {
    format!("poll:{query}:{id}")
}

/// What one job's poll remembers between times.
struct JobPoll {
    signature: String,
    next_due_ms: i64,
    /// The ids in the last answer that have been looked at.
    seen: HashSet<String>,
    baselined: bool,
}

/// The poller's memory: one entry per poll job.
#[derive(Default)]
pub struct PollState {
    jobs: HashMap<String, JobPoll>,
}

/// Asks the question of every poll job that is due and starts a run for each new item. Returns how many runs it
/// started or queued. `now` is a parameter so a test can step the clock.
pub async fn poll_due(scheduler: &CronScheduler, all: &[CronJob], check: &Check, state: &mut PollState, now: i64) -> usize {
    // The job's event ids live with the run ledger (what the scheduler admits events against), which is not always the
    // store the jobs are listed from: the baseline and the items are claimed where `fire_event` looks.
    let Some(ledger) = scheduler.runs().cloned() else { return 0 };
    let store = ledger.store().clone();
    let polling: Vec<(&CronJob, Plan)> = all.iter().filter_map(|j| plan_of(j).map(|p| (j, p))).collect();
    state.jobs.retain(|id, _| polling.iter().any(|(j, _)| &j.id == id));
    let mut offered = 0;
    for (job, plan) in &polling {
        let st = state.jobs.entry(job.id.clone()).or_insert_with(|| JobPoll { signature: plan.signature.clone(), next_due_ms: now, seen: HashSet::new(), baselined: false });
        if st.signature != plan.signature {
            *st = JobPoll { signature: plan.signature.clone(), next_due_ms: now, seen: HashSet::new(), baselined: false };
        }
        if now < st.next_due_ms {
            continue;
        }
        st.next_due_ms = now + plan.every_ms;
        let answer = match check(plan.ask.clone()).await {
            Ok(answer) => answer,
            Err(e) => {
                tracing::warn!(job_id = %job.id, query = %plan.query, error = %e, "jobs: a poll's question failed; asking again next time");
                continue;
            }
        };
        let items = items_of(&plan.query, &answer);
        let present: HashSet<String> = items.iter().map(|(id, _)| id.clone()).collect();
        let marker = baseline_marker(&plan.signature);
        if !plan.include_existing {
            // The marker is an id like the others and would go with the week's sweep: each look moves it forward, so
            // a restart after a long stop still knows this is not the first look.
            let _ = store.job_event_touch(&job.id, &marker, now);
        }
        if !st.baselined {
            if !plan.include_existing {
                // The first poll of a job takes note of what is there, once.
                // The marker and what is there are claimed together, so the marker is never seen without them.
                let ids: Vec<String> = items.iter().map(|(id, _)| event_id(&plan.query, id)).collect();
                match store.job_event_claim_baseline(&job.id, &marker, &ids, now) {
                    Ok(true) => {
                        st.baselined = true;
                        st.seen = present;
                        continue;
                    }
                    Ok(false) => {}
                    Err(e) => {
                        // Not knowing whether this is the first poll must not start a run for everything that is there.
                        tracing::warn!(job_id = %job.id, error = %e, "jobs: a poll couldn't record its first look; trying again next time");
                        continue;
                    }
                }
            }
            st.baselined = true;
        }
        let mut started = 0;
        for (id, item) in items {
            if st.seen.contains(&id) {
                continue;
            }
            if started >= MAX_NEW_PER_POLL {
                break;
            }
            started += 1;
            st.seen.insert(id.clone());
            let input = json!({ "query": plan.query, "key": id, "item": item });
            let fired = scheduler.fire_event(job, JobEvent { id: event_id(&plan.query, &id), source: event_source::POLL.to_string(), input }).await;
            // An item the job already heard of, or that a cap or the policy refused, starts nothing.
            offered += usize::from(fired.run_id.is_some());
        }
        st.seen.retain(|id| present.contains(id));
    }
    offered
}

/// Start the poller. `check` answers the App query.
pub fn spawn(jobs: Arc<Store>, scheduler: Arc<CronScheduler>, check: Check) {
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    tokio::spawn(async move {
        let mut state = PollState::default();
        loop {
            tokio::time::sleep(TICK).await;
            let listing = jobs.clone();
            let all = tokio::task::spawn_blocking(move || listing.cron_list_enabled()).await.ok().and_then(Result::ok).unwrap_or_default();
            poll_due(&scheduler, &all, &check, &mut state, agentmux_common::time::now_ms()).await;
        }
    });
}
