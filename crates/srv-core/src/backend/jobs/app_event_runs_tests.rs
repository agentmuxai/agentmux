// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! App events: which broker events a job can name, how a filter is read, and a job's run starting when the broker
//! reports the event (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §3.2.1).

use super::*;
use crate::backend::cron::ActionOutcome;
use crate::backend::jobs::app_events::{self, name, AppEvent, Raw};
use crate::backend::jobs::definition::trigger_kind;
use crate::backend::mps::EVENT_AGENT_FAILURE;
use crate::backend::obj::{Block, MetaMapType};
use crate::backend::storage::cron::CronJob;

fn broker_event(event: &str, scopes: &[&str], data: serde_json::Value) -> MuxEvent {
    MuxEvent { event: event.to_string(), scopes: scopes.iter().map(|s| s.to_string()).collect(), sender: String::new(), persist: 1, data: Some(data) }
}

fn ledger(end: Option<&str>, trigger: Option<&str>) -> serde_json::Value {
    let mut l = json!({ "block_id": "b1", "turn_id": 7, "seq": 3, "started_at_ms": 5, "passes": 1, "active": end.is_none(), "cost_usd": 0.25 });
    if let Some(end) = end {
        l["end"] = json!(end);
        l["ended_at_ms"] = json!(900);
    }
    if let Some(kind) = trigger {
        l["trigger"] = json!({ "kind": kind });
    }
    l
}

#[test]
fn a_turn_that_ended_is_an_event_and_one_still_running_or_the_systems_own_is_not() {
    let ended = |data| app_events::raw_of(&broker_event(EVENT_AGENT_TURN, &["block:b1"], data));
    assert_eq!(
        ended(ledger(Some("completed"), Some("user"))),
        Some(Raw::TurnEnded { block_id: "b1".into(), turn_id: 7, ended_at_ms: 900, outcome: "completed", cost_usd: 0.25 })
    );
    assert!(matches!(ended(ledger(Some("exited"), None)), Some(Raw::TurnEnded { outcome: "errored", .. })), "an exit ends the turn badly");
    assert_eq!(ended(ledger(None, Some("user"))), None, "a turn still going has no end yet");
    assert_eq!(ended(ledger(Some("completed"), Some("system"))), None, "AgentMux's own hidden turn is no agent's");
}

#[test]
fn a_failure_is_an_event_when_it_says_where_and_why_and_nothing_else_is() {
    let failed = |scopes: &[&str], data| app_events::raw_of(&broker_event(EVENT_AGENT_FAILURE, scopes, data));
    assert_eq!(failed(&["block:b9"], json!({ "code": "usage_limit" })), Some(Raw::Crashed { block_id: "b9".into(), reason: "usage_limit".into() }));
    assert_eq!(failed(&[], json!({ "code": "auth" })), None, "no pane to name");
    assert_eq!(failed(&["block:b9"], json!({ "code": "" })), None);
    assert_eq!(failed(&["block:b9"], json!({})), None);
    assert_eq!(app_events::raw_of(&broker_event("sysinfo", &[], json!({ "cpu": 1 }))), None, "an event no job can name");
}

fn filter(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    v.as_object().unwrap().clone()
}

#[test]
fn a_filter_names_an_event_and_only_that_events_fields() {
    assert!(app_events::validate_filter(&filter(json!({ "event": "agent.turn_ended", "agent": "a", "outcome": "completed" }))).is_ok());
    assert!(app_events::validate_filter(&filter(json!({ "event": "agent.crashed", "reason": "auth" }))).is_ok());
    let missing = app_events::validate_filter(&filter(json!({ "agent": "a" }))).unwrap_err();
    assert!(missing.contains("names its event") && missing.contains("agent.turn_ended"), "{missing}");
    assert!(app_events::validate_filter(&filter(json!({ "event": "agent.exploded" }))).unwrap_err().contains("is not one of"));
    let stray = app_events::validate_filter(&filter(json!({ "event": "agent.crashed", "outcome": "completed" }))).unwrap_err();
    assert!(stray.contains("has no field \"outcome\"") && stray.contains("agent, reason"), "{stray}");
}

fn app_job(rig: &Rig, id: &str, filter: serde_json::Value, edit: impl FnOnce(&mut CronJob)) -> CronJob {
    let mut job = test_job(id, "", "", "", "");
    job.created_by = BY_OPERATOR.to_string();
    job.trigger_kind = trigger_kind::EVENT.to_string();
    job.trigger_event = json!({ "source": "app_event", "filter": filter });
    job.action_kind = "drone".to_string();
    job.action = json!({ "kind": "drone", "drone_id": "d-1" });
    edit(&mut job);
    rig.store.cron_create(&job).unwrap();
    job
}

fn occurred(name: &'static str, fields: serde_json::Value) -> AppEvent {
    let mut input = filter(fields);
    input.insert("event".into(), json!(name));
    AppEvent { id: "e1".into(), name, input }
}

#[test]
fn a_job_listens_when_every_filter_entry_is_met_and_it_is_an_enabled_app_event_job() {
    let rig = rig();
    let ended = occurred(name::AGENT_TURN_ENDED, json!({ "agent": "uid-a", "outcome": "completed", "cost_usd": 2 }));
    let hears = |f: serde_json::Value| app_events::listens(&app_job(&rig, &format!("j{}", rand_id()), f, |_| {}), &ended);
    assert!(hears(json!({ "event": "agent.turn_ended" })), "the event alone");
    assert!(hears(json!({ "event": "agent.turn_ended", "agent": "uid-a", "outcome": "completed" })));
    assert!(hears(json!({ "event": "agent.turn_ended", "cost_usd": 2.0 })), "a number is a number");
    assert!(!hears(json!({ "event": "agent.turn_ended", "agent": "uid-b" })), "another agent's");
    assert!(!hears(json!({ "event": "agent.turn_ended", "outcome": "errored" })));
    assert!(!hears(json!({ "event": "agent.crashed" })), "another event");
    assert!(!hears(json!({ "event": "agent.turn_ended", "nonsense": true })), "a field the event does not have");

    let mut off = app_job(&rig, "j-off", json!({ "event": "agent.turn_ended" }), |j| j.enabled = false);
    assert!(!app_events::listens(&off, &ended), "a job that is off");
    off.enabled = true;
    off.trigger_event = json!({ "source": "file_watch", "filter": { "event": "agent.turn_ended" } });
    assert!(!app_events::listens(&off, &ended), "another source");
    off.trigger_event = json!({ "source": "app_event", "filter": { "event": "agent.turn_ended" } });
    off.trigger_kind = trigger_kind::MANUAL.to_string();
    assert!(!app_events::listens(&off, &ended), "not an event job");
}

fn rand_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::SeqCst)
}

fn agent_block(mstore: &Store, agent_id: &str) -> String {
    let oid = uuid::Uuid::new_v4().to_string();
    let mut meta = MetaMapType::new();
    meta.insert("view".to_string(), json!("agent"));
    meta.insert("agentId".to_string(), json!(agent_id));
    let mut block = Block { oid: oid.clone(), parentoref: String::new(), version: 1, runtimeopts: None, stickers: None, meta, subblockids: None };
    mstore.insert(&mut block).unwrap();
    oid
}

async fn until(what: &str, mut ok: impl FnMut() -> bool) {
    for _ in 0..100 {
        if ok() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }
    panic!("{what}");
}

/// The broker reports an agent's turn: the job that names that agent starts a run reading the event's fields; a job
/// that names another agent does not; the same turn reported twice starts one run.
#[tokio::test(flavor = "multi_thread")]
async fn the_broker_reporting_a_turn_starts_the_run_of_the_job_that_names_it() {
    let rig = rig();
    let dir = tempfile::tempdir().unwrap();
    let mstore = Arc::new(Store::open(&dir.path().join("mstore.db")).unwrap());
    let block = agent_block(&mstore, "uid-watched");
    let mine = app_job(&rig, "j-mine", json!({ "event": "agent.turn_ended", "agent": "uid-watched" }), |_| {});
    let other = app_job(&rig, "j-other", json!({ "event": "agent.turn_ended", "agent": "uid-else" }), |_| {});
    let any = app_job(&rig, "j-any", json!({ "event": "agent.crashed" }), |_| {});
    let started: Arc<Mutex<Vec<String>>> = Arc::default();
    let (sink, ledger_for_runs) = (started.clone(), rig.ledger.clone());
    rig.sched.install_actions(Arc::new(move |job, run_id| {
        sink.lock().unwrap().push(job.id.clone());
        ledger_for_runs.drone_attached(&run_id, &format!("dr-{run_id}"), &Value::Null, agentmux_common::time::now_ms());
        Box::pin(async { ActionOutcome::Drone })
    }));
    app_events::spawn(&rig.broker, mstore, rig.store.clone(), rig.sched.clone());

    let mut l = ledger(Some("completed"), Some("agent"));
    l["block_id"] = json!(block);
    let publish = || rig.broker.publish(broker_event(EVENT_AGENT_TURN, &[&format!("block:{block}")], l.clone()));
    publish();
    until("the job's run", || rig.runs("j-mine").len() == 1).await;
    publish();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    assert_eq!(*started.lock().unwrap(), ["j-mine"], "only the job that names this agent and this event");
    assert_eq!(rig.runs("j-mine").len(), 1, "the turn reported twice is one run");
    assert!(rig.runs("j-other").is_empty() && rig.runs("j-any").is_empty());
    let run = rig.runs("j-mine").remove(0);
    assert_eq!((run.trigger.as_str(), run.triggered_by.as_str()), ("event", "event:app_event"));
    assert_eq!(
        rig.ledger.event_input(&run.id),
        Some(json!({ "event": "agent.turn_ended", "agent": "uid-watched", "outcome": "completed", "cost_usd": 0.25 })),
        "the run reads the event's fields"
    );
    let _ = (mine, other, any);
}

#[test]
fn srv_starting_is_an_event_that_says_whether_it_made_an_upgrade() {
    assert!(app_events::validate_filter(&filter(json!({ "event": "app.started", "upgraded": true }))).is_ok());
    let stray = app_events::validate_filter(&filter(json!({ "event": "app.started", "agent": "a" }))).unwrap_err();
    assert!(stray.contains("has no field \"agent\"") && stray.contains("version, upgraded, upgraded_from"), "{stray}");

    let dir = tempfile::tempdir().unwrap();
    let mstore = Store::open(&dir.path().join("mstore.db")).unwrap();
    let up = app_events::event_of(&Raw::Started { version: "0.60.29".into(), upgraded_from: "0.60.28".into(), at_ms: 7 }, &mstore, 9);
    assert_eq!(up.id, "started:0.60.29:7");
    assert_eq!(up.input, filter(json!({ "event": "app.started", "version": "0.60.29", "upgraded": true, "upgraded_from": "0.60.28" })));
    let plain = app_events::event_of(&Raw::Started { version: "0.60.29".into(), upgraded_from: String::new(), at_ms: 8 }, &mstore, 9);
    assert_eq!(plain.input["upgraded"], json!(false));

    let rig = rig();
    let hears = |f: serde_json::Value, e: &AppEvent| app_events::listens(&app_job(&rig, &format!("j{}", rand_id()), f, |_| {}), e);
    assert!(hears(json!({ "event": "app.started", "upgraded": true }), &up), "an upgrade");
    assert!(!hears(json!({ "event": "app.started", "upgraded": true }), &plain), "a plain restart is not one");
    assert!(hears(json!({ "event": "app.started" }), &plain), "any start");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_upgrade_starts_the_job_that_names_it_once_with_the_versions() {
    let rig = rig();
    let dir = tempfile::tempdir().unwrap();
    let mstore = Store::open(&dir.path().join("mstore.db")).unwrap();
    let job = app_job(&rig, "j-up", json!({ "event": "app.started", "upgraded": true }), |j| j.trigger_event["delay"] = json!(0));
    let ledger_for_runs = rig.ledger.clone();
    rig.sched.install_actions(Arc::new(move |_, run_id| {
        ledger_for_runs.drone_attached(&run_id, &format!("dr-{run_id}"), &Value::Null, agentmux_common::time::now_ms());
        Box::pin(async { ActionOutcome::Drone })
    }));
    let event = app_events::event_of(&Raw::Started { version: "0.60.29".into(), upgraded_from: "0.60.28".into(), at_ms: 7 }, &mstore, 9);
    assert_eq!(app_events::fire_matching(&rig.sched, &rig.store, &event).await, 1);
    assert_eq!(app_events::fire_matching(&rig.sched, &rig.store, &event).await, 1, "offered again");
    until("the run", || rig.runs("j-up").len() == 1).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(rig.runs("j-up").len(), 1, "the same start is one run");
    let run = rig.runs("j-up").remove(0);
    assert_eq!(
        rig.ledger.event_input(&run.id),
        Some(json!({ "event": "app.started", "version": "0.60.29", "upgraded": true, "upgraded_from": "0.60.28" }))
    );
    let _ = job;
}
