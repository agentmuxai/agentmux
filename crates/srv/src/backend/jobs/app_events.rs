// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! App events: something inside AgentMux happened, and a job on the `app_event` source starts a run
//! (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §3.2, §3.2.1).
//!
//! An observer on the broker reads each published event and keeps only the few it names (a turn that ended, an
//! agent that failed); AgentMux's own start is the one more, said once; it does nothing else under the broker's locks but queue. A task then gives the event its
//! typed fields, finds the enabled jobs whose filter it matches and starts each one's run through
//! [`CronScheduler::fire_event`], which records it, applies policy, caps, the delay and the rate limit, and drops
//! an event the job already heard. The run reads the fields as `{{trigger.*}}`.
//!
//! A job's filter names the event (`"event": "agent.turn_ended"`) and may name any of that event's fields
//! (`"agent": "<agent id>"`, `"outcome": "completed"`); an event matches when every entry equals its field.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::definition::{event_source, trigger_kind};
use crate::backend::blockcontroller::health::{TriggerKind, TurnEnd, TurnTrigger};
use crate::backend::cron::{CronScheduler, JobEvent};
use crate::backend::mps::{Broker, MuxEvent, EVENT_AGENT_FAILURE, EVENT_AGENT_TURN};
use crate::backend::obj::{meta_get_string, Block};
use crate::backend::storage::store::Store;

/// The events a job can name, as its filter's `event` says them.
pub mod name {
    /// An agent's turn, as its ledger reports it, is over.
    pub const AGENT_TURN_ENDED: &str = "agent.turn_ended";
    /// An agent's process failed (a limit, a sign-in, a crash).
    pub const AGENT_CRASHED: &str = "agent.crashed";
    /// AgentMux (srv) has started; `upgraded` says whether this start made an upgrade.
    pub const APP_STARTED: &str = "app.started";
    pub const ALL: [&str; 3] = [AGENT_TURN_ENDED, AGENT_CRASHED, APP_STARTED];
}

/// The fields an event carries besides its name: what a filter may name, and all a run reads of it.
pub fn fields_of(event: &str) -> Option<&'static [&'static str]> {
    match event {
        name::AGENT_TURN_ENDED => Some(&["agent", "outcome", "cost_usd"]),
        name::AGENT_CRASHED => Some(&["agent", "reason"]),
        name::APP_STARTED => Some(&["version", "upgraded", "upgraded_from"]),
        _ => None,
    }
}

/// An `app_event` trigger's filter names one of the events, and only fields that event has.
pub fn validate_filter(filter: &Map<String, Value>) -> Result<(), String> {
    let Some(event) = filter.get("event").and_then(Value::as_str) else {
        return Err(format!("trigger: an app_event filter names its event, one of {}", name::ALL.join(", ")));
    };
    let Some(fields) = fields_of(event) else {
        return Err(format!("trigger: app event {event:?} is not one of {}", name::ALL.join(", ")));
    };
    for key in filter.keys().filter(|k| k.as_str() != "event") {
        if !fields.contains(&key.as_str()) {
            return Err(format!("trigger: {event} has no field {key:?}; it has {}", fields.join(", ")));
        }
    }
    Ok(())
}

/// What the observer keeps of a broker event: enough to find the agent later, off the broker's locks.
#[derive(Debug, Clone, PartialEq)]
pub enum Raw {
    TurnEnded { block_id: String, turn_id: u64, ended_at_ms: u64, outcome: &'static str, cost_usd: f64 },
    Crashed { block_id: String, reason: String },
    Started { version: String, upgraded_from: String, at_ms: i64 },
}

/// The fields of a published ledger that matter here.
#[derive(Deserialize)]
struct Ledger {
    block_id: String,
    turn_id: u64,
    #[serde(default)]
    ended_at_ms: Option<u64>,
    #[serde(default)]
    end: Option<TurnEnd>,
    #[serde(default)]
    trigger: Option<TurnTrigger>,
    #[serde(default)]
    cost_usd: f64,
}

/// The broker event as an app event, or `None` for one no job can name. AgentMux's own hidden turn (the memory
/// reinjection) is not an agent's turn; a turn still running or settling has no end yet.
pub fn raw_of(ev: &MuxEvent) -> Option<Raw> {
    if ev.event == EVENT_AGENT_TURN {
        let l = Ledger::deserialize(ev.data.as_ref()?).ok()?;
        let outcome = match l.end? {
            TurnEnd::Completed => "completed",
            TurnEnd::Exited => "errored",
        };
        if l.trigger.is_some_and(|t| t.kind == TriggerKind::System) {
            return None;
        }
        return Some(Raw::TurnEnded {
            block_id: l.block_id,
            turn_id: l.turn_id,
            ended_at_ms: l.ended_at_ms.unwrap_or(0),
            outcome,
            cost_usd: l.cost_usd,
        });
    }
    if ev.event == EVENT_AGENT_FAILURE {
        let block_id = ev.scopes.iter().find_map(|s| s.strip_prefix("block:"))?.to_string();
        let reason = ev.data.as_ref()?.get("code")?.as_str()?.to_string();
        if reason.is_empty() {
            return None;
        }
        return Some(Raw::Crashed { block_id, reason });
    }
    None
}

/// An app event with its fields: what [`fire_matching`] matches jobs against and hands to the run.
#[derive(Debug, Clone, PartialEq)]
pub struct AppEvent {
    /// Unique per occurrence: a repeat of it starts nothing.
    pub id: String,
    pub name: &'static str,
    /// `event` and the event's typed fields.
    pub input: Map<String, Value>,
}

/// The agent a block runs: its id, or none for a block this machine does not know.
fn agent_of(store: &Store, block_id: &str) -> String {
    let block = store.get::<Block>(block_id).ok().flatten();
    block.map(|b| meta_get_string(&b.meta, "agentId", "")).unwrap_or_default()
}

/// Gives a raw event its fields. Reads the store: off the async workers.
pub fn event_of(raw: &Raw, store: &Store, now_ms: i64) -> AppEvent {
    let mut input = Map::new();
    match raw {
        Raw::TurnEnded { block_id, turn_id, ended_at_ms, outcome, cost_usd } => {
            input.insert("event".into(), json!(name::AGENT_TURN_ENDED));
            input.insert("agent".into(), json!(agent_of(store, block_id)));
            input.insert("outcome".into(), json!(outcome));
            input.insert("cost_usd".into(), json!(cost_usd));
            // A turn rejoined by a held message ends again later: that end is a new one.
            AppEvent { id: format!("turn:{block_id}:{turn_id}:{ended_at_ms}"), name: name::AGENT_TURN_ENDED, input }
        }
        Raw::Crashed { block_id, reason } => {
            input.insert("event".into(), json!(name::AGENT_CRASHED));
            input.insert("agent".into(), json!(agent_of(store, block_id)));
            input.insert("reason".into(), json!(reason));
            AppEvent { id: format!("crash:{block_id}:{reason}:{now_ms}"), name: name::AGENT_CRASHED, input }
        }
        Raw::Started { version, upgraded_from, at_ms } => {
            input.insert("event".into(), json!(name::APP_STARTED));
            input.insert("version".into(), json!(version));
            input.insert("upgraded".into(), json!(!upgraded_from.is_empty()));
            input.insert("upgraded_from".into(), json!(upgraded_from));
            AppEvent { id: format!("started:{version}:{at_ms}"), name: name::APP_STARTED, input }
        }
    }
}

/// Whether a job listens for this event: an enabled event job on `app_event` whose filter every entry the event meets.
pub fn listens(job: &crate::backend::storage::cron::CronJob, event: &AppEvent) -> bool {
    if !job.enabled || job.trigger_kind != trigger_kind::EVENT {
        return false;
    }
    let t = &job.trigger_event;
    if t.get("source").and_then(Value::as_str) != Some(event_source::APP_EVENT) {
        return false;
    }
    let Some(filter) = t.get("filter").and_then(Value::as_object) else { return false };
    filter.get("event").and_then(Value::as_str) == Some(event.name)
        && filter.iter().all(|(key, want)| event.input.get(key).is_some_and(|have| same(have, want)))
}

/// A filter value equals a field: text and true or false by value, numbers as numbers.
fn same(have: &Value, want: &Value) -> bool {
    match (have, want) {
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        _ => have == want,
    }
}

/// Starts the run of every job that listens for `event`. Returns how many jobs it was offered to. Reads the
/// store: the job list is read off the async workers.
pub async fn fire_matching(scheduler: &CronScheduler, jobs: &Arc<Store>, event: &AppEvent) -> usize {
    let listing = jobs.clone();
    let all = tokio::task::spawn_blocking(move || listing.cron_list_enabled()).await.ok().and_then(Result::ok).unwrap_or_default();
    let mut offered = 0;
    for job in all.iter().filter(|j| listens(j, event)) {
        offered += 1;
        let _ = scheduler
            .fire_event(job, JobEvent { id: event.id.clone(), source: event_source::APP_EVENT.to_string(), input: Value::Object(event.input.clone()) })
            .await;
    }
    offered
}

/// The most events waiting to be looked at: past it the newest are dropped, as a stuck consumer must not grow without bound.
const QUEUE: usize = 1024;

/// Start following the broker. `mstore` knows the agents' blocks, `jobs` the jobs.
pub fn spawn(broker: &Arc<Broker>, mstore: Arc<Store>, jobs: Arc<Store>, scheduler: Arc<CronScheduler>) {
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Raw>(QUEUE);
    // This start is the first event: a job that names `app.started` runs once for it, whatever else follows.
    if let Some(start) = crate::backend::upgrade_notice::this_start() {
        let _ = tx.try_send(Raw::Started { version: start.version, upgraded_from: start.upgraded_from.unwrap_or_default(), at_ms: start.at_ms });
    }
    // The observer runs inside `Broker::publish`, under its locks: it only parses and queues.
    broker.add_observer(Arc::new(move |ev: &MuxEvent| {
        if let Some(raw) = raw_of(ev) {
            let _ = tx.try_send(raw);
        }
    }));
    tokio::spawn(async move {
        while let Some(raw) = rx.recv().await {
            let now = agentmux_common::time::now_ms();
            let store = mstore.clone();
            let Ok(event) = tokio::task::spawn_blocking(move || event_of(&raw, &store, now)).await else { continue };
            fire_matching(&scheduler, &jobs, &event).await;
        }
    });
}
