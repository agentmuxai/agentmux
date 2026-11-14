// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! An event job through the scheduler and the ledger: one run per event, a repeat starting nothing, policy and Pause all
//! skipping a fire into a row, and the event's input held for the run that starts
//! (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §3.2, §3.7).

use super::*;
use crate::backend::cron::{ActionOutcome, JobEvent};
use crate::backend::jobs::definition::{trigger_kind, Overlap};
use crate::backend::storage::cron::CronJob;
use crate::backend::storage::job_runs::{infra, reason, state};

/// An event job (a drone on the `app_event` source) stored with `edit` applied.
fn event_job(rig: &Rig, id: &str, edit: impl FnOnce(&mut CronJob)) -> CronJob {
    let mut job = test_job(id, "", "", "", "");
    job.created_by = BY_OPERATOR.to_string();
    job.trigger_kind = trigger_kind::EVENT.to_string();
    job.trigger_event = json!({ "source": "app_event", "filter": {} });
    job.action_kind = "drone".to_string();
    job.action = json!({ "kind": "drone", "drone_id": "d-1" });
    edit(&mut job);
    rig.store.cron_create(&job).unwrap();
    job
}

fn event(id: &str, input: Value) -> JobEvent {
    JobEvent { id: id.to_string(), source: "app_event".to_string(), input }
}

/// A drone runner that records the runs it was asked to start, and links each to a Drone run as the real one does.
fn drone_runner(rig: &Rig) -> Arc<Mutex<Vec<String>>> {
    let started: Arc<Mutex<Vec<String>>> = Arc::default();
    let (sink, ledger) = (started.clone(), rig.ledger.clone());
    rig.sched.install_actions(Arc::new(move |_job, run_id| {
        sink.lock().unwrap().push(run_id.clone());
        ledger.drone_attached(&run_id, &format!("dr-{run_id}"), &Value::Null, agentmux_common::time::now_ms());
        Box::pin(async { ActionOutcome::Drone })
    }));
    started
}

fn rows(rig: &Rig, job: &str) -> Vec<(String, String, String)> {
    let mut runs = rig.runs(job);
    runs.reverse();
    runs.into_iter().map(|r| (r.trigger, r.state, r.reason)).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_event_starts_one_drone_run_and_a_repeat_of_it_starts_none() {
    let rig = rig();
    let job = event_job(&rig, "j-ev", |_| {});
    let started = drone_runner(&rig);

    let first = rig.sched.fire_event(&job, event("e1", json!({ "outcome": "failed" }))).await;
    let run_id = first.run_id.expect("a run");
    let row = rig.store.job_run_get(&run_id).unwrap().unwrap();
    assert_eq!((row.trigger.as_str(), row.triggered_by.as_str(), row.state.as_str()), ("event", "event:app_event", state::RUNNING));
    assert_eq!(row.scheduled_for_ms, None, "an event has no slot");
    assert_eq!(*started.lock().unwrap(), [run_id.clone()]);
    assert_eq!(rig.ledger.event_input(&run_id), Some(json!({ "outcome": "failed" })), "held for the run");
    rig.ledger.drone_finished(&run_id, true, &Default::default(), "");
    assert_eq!(rig.ledger.event_input(&run_id), None, "and let go when the run is over for good");

    let again = rig.sched.fire_event(&job, event("e1", json!({ "outcome": "failed" }))).await;
    assert_eq!((again.run_id, again.outcome, again.waiting), (None, None, None));
    assert_eq!(started.lock().unwrap().len(), 1, "the same event starts nothing");
    assert_eq!(rig.runs("j-ev").len(), 1, "and leaves no row");
    assert_eq!(rig.fire_count("j-ev"), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_event_that_overlap_policy_refuses_is_a_skipped_row_folded_into_the_last() {
    let rig = rig();
    let job = event_job(&rig, "j-skip", |_| {});
    let started = drone_runner(&rig);
    let first = rig.sched.fire_event(&job, event("e1", json!({}))).await.run_id.unwrap();

    for id in ["e2", "e3"] {
        let skipped = rig.sched.fire_event(&job, event(id, json!({}))).await;
        assert_eq!((skipped.run_id, skipped.outcome), (None, None));
    }
    assert_eq!(started.lock().unwrap().len(), 1);
    let history: Vec<_> = rows(&rig, "j-skip");
    assert_eq!(history, [("event".into(), "running".into(), "".into()), ("event".into(), "skipped".into(), "overlap".into())]);
    let skip = rig.runs("j-skip").into_iter().find(|r| r.state == state::SKIPPED).unwrap();
    assert_eq!(skip.skip_count, 2, "both skipped events are counted in one row");

    rig.ledger.drone_finished(&first, true, &Default::default(), "");
    assert!(rig.sched.fire_event(&job, event("e4", json!({}))).await.run_id.is_some(), "free again");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_buffered_event_waits_with_its_input_and_starts_when_the_run_ends() {
    let rig = rig();
    let job = event_job(&rig, "j-buf", |j| j.policy.overlap = Overlap::BufferOne);
    let started = drone_runner(&rig);
    let first = rig.sched.fire_event(&job, event("e1", json!({ "n": 1 }))).await.run_id.unwrap();

    let waiting = rig.sched.fire_event(&job, event("e2", json!({ "n": 2 }))).await;
    assert_eq!(waiting.waiting, Some(reason::OVERLAP));
    let second = waiting.run_id.unwrap();
    assert_eq!(started.lock().unwrap().len(), 1, "it has not started");

    rig.ledger.drone_finished(&first, true, &Default::default(), "");
    assert_eq!(rig.sched.run_policy_pass().await, 1);
    assert_eq!(*started.lock().unwrap(), [first.clone(), second.clone()]);
    assert_eq!(rig.ledger.event_input(&second), Some(json!({ "n": 2 })), "its own input, not the first's");
}

#[tokio::test(flavor = "multi_thread")]
async fn pause_all_skips_an_event_and_a_paused_or_non_event_job_ignores_it() {
    let rig = rig();
    let job = event_job(&rig, "j-pause", |_| {});
    let started = drone_runner(&rig);

    rig.ledger.set_paused_all(true, "operator").unwrap();
    assert_eq!(rig.sched.fire_event(&job, event("e1", json!({}))).await.run_id, None);
    assert_eq!(rows(&rig, "j-pause"), [("event".into(), "skipped".into(), reason::PAUSED_ALL.into())]);
    rig.ledger.set_paused_all(false, "operator").unwrap();

    let mut paused = job.clone();
    paused.enabled = false;
    assert_eq!(rig.sched.fire_event(&paused, event("e2", json!({}))).await.run_id, None);
    let mut timer = job.clone();
    timer.trigger_kind = trigger_kind::EVERY.to_string();
    assert_eq!(rig.sched.fire_event(&timer, event("e3", json!({}))).await.run_id, None);
    assert!(started.lock().unwrap().is_empty());
    assert_eq!(rig.runs("j-pause").len(), 1, "an ignored event leaves no row, and is not remembered");
    assert!(rig.sched.fire_event(&job, event("e2", json!({}))).await.run_id.is_some(), "the event the paused job ignored is new to it");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_retry_of_an_events_run_reads_the_same_event() {
    let rig = rig();
    let job = event_job(&rig, "j-retry", |j| j.policy.retries = 2);
    drone_runner(&rig);
    let first = rig.sched.fire_event(&job, event("e1", json!({ "n": 7 }))).await.run_id.unwrap();

    rig.ledger.turn_finished(&first, infra::INTERRUPTED, None, "interrupted");
    let retry = rig.runs("j-retry").into_iter().find(|r| r.trigger == "retry").expect("a retry row");
    assert_eq!(retry.links["from_event"], json!(true), "the retry knows it must have the event");
    assert_eq!(rig.ledger.event_input(&retry.id), Some(json!({ "n": 7 })), "the retry took the event over");
    assert_eq!(rig.ledger.event_input(&first), None, "and the first attempt no longer holds it");

    // Its own retry is an event retry too.
    rig.ledger.turn_finished(&retry.id, infra::INTERRUPTED, None, "interrupted");
    let again = rig.runs("j-retry").into_iter().find(|r| r.links["retry_of"] == json!(retry.id)).expect("a second retry");
    assert_eq!((again.links["from_event"].clone(), rig.ledger.event_input(&again.id)), (json!(true), Some(json!({ "n": 7 }))));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_delayed_event_is_a_scheduled_row_that_starts_when_due_with_its_event() {
    let rig = rig();
    let job = event_job(&rig, "j-delay", |j| j.trigger_event = json!({ "source": "app_event", "filter": {}, "delay": 30 }));
    let started = drone_runner(&rig);

    let before = agentmux_common::time::now_ms();
    let fired = rig.sched.fire_event(&job, event("e1", json!({ "n": 7 }))).await;
    let run_id = fired.run_id.expect("a row");
    assert_eq!((fired.outcome.is_none(), fired.waiting), (true, Some(reason::DELAY_WAIT)), "it did not start");
    let row = rig.store.job_run_get(&run_id).unwrap().unwrap();
    assert_eq!((row.trigger.as_str(), row.state.as_str(), row.reason.as_str()), ("event", state::SCHEDULED, reason::DELAY_WAIT));
    let due = row.links["due_at_ms"].as_i64().unwrap();
    assert!(due >= before + 30 * 60_000 && due <= agentmux_common::time::now_ms() + 30 * 60_000, "due at event time + 30 minutes");
    assert_eq!(rig.ledger.event_input(&run_id), Some(json!({ "n": 7 })), "its data is kept for it");
    let after_restart = RunLedger::new(rig.store.clone(), None, Arc::new(Broker::new()));
    assert_eq!(after_restart.event_input(&run_id), Some(json!({ "n": 7 })), "and a restart does not lose it");

    assert_eq!(rig.sched.run_policy_pass().await, 0, "not due yet");
    assert!(started.lock().unwrap().is_empty());

    let again = rig.sched.fire_event(&job, event("e1", json!({ "n": 7 }))).await;
    assert_eq!((again.run_id, again.waiting), (None, None), "a repeat of the event is still nothing");
    assert_eq!(rig.runs("j-delay").len(), 1);

    rig.store.job_run_add_links(&run_id, &json!({ "due_at_ms": before - 1 })).unwrap();
    assert_eq!(rig.sched.run_policy_pass().await, 1, "due: it starts");
    assert_eq!(*started.lock().unwrap(), [run_id.clone()]);
    assert_eq!(rig.store.job_run_get(&run_id).unwrap().unwrap().state, state::RUNNING);
    assert_eq!(rig.ledger.event_input(&run_id), Some(json!({ "n": 7 })), "and reads its event");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_delay_holds_the_decision_until_due_and_a_job_paused_meanwhile_skips_it() {
    let rig = rig();
    let job = event_job(&rig, "j-delay2", |j| j.trigger_event = json!({ "source": "app_event", "filter": {}, "delay": 5 }));
    let started = drone_runner(&rig);
    // Overlap is not asked at event time: two events in the delay window both wait, each with its own data.
    let first = rig.sched.fire_event(&job, event("e1", json!({ "n": 1 }))).await.run_id.unwrap();
    let second = rig.sched.fire_event(&job, event("e2", json!({ "n": 2 }))).await.run_id.unwrap();
    assert_ne!(first, second);
    assert_eq!((rig.ledger.event_input(&first), rig.ledger.event_input(&second)), (Some(json!({ "n": 1 })), Some(json!({ "n": 2 }))));

    rig.store.cron_set_enabled(&job.id, false).unwrap();
    for id in [&first, &second] {
        rig.store.job_run_add_links(id, &json!({ "due_at_ms": 1 })).unwrap();
    }
    rig.sched.run_policy_pass().await;
    assert!(started.lock().unwrap().is_empty(), "a job paused while its event waited does not run it");
    let skipped: Vec<_> = rig.runs("j-delay2").into_iter().map(|r| (r.state, r.reason, r.skip_count)).collect();
    assert_eq!(skipped, [(state::SKIPPED.to_string(), reason::PAUSED.to_string(), 2)], "both are skipped, counted in one row");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_event_id_older_than_the_retention_is_forgotten_when_the_next_event_is_heard() {
    let rig = rig();
    let job = event_job(&rig, "j-prune", |_| {});
    drone_runner(&rig);
    assert!(rig.store.job_event_claim(&job.id, "ancient", "", 1).unwrap());
    assert!(rig.sched.fire_event(&job, event("fresh", json!({}))).await.run_id.is_some());
    assert!(rig.store.job_event_claim(&job.id, "ancient", "", 1).unwrap(), "its id was pruned, so it is new again");
    assert!(!rig.store.job_event_claim(&job.id, "fresh", "", 2).unwrap(), "a recent id is kept");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_waiting_runs_input_outlives_the_event_ids_pruning() {
    use crate::backend::storage::job_events::{INPUT_KEEP_MS, KEEP_MS};
    let rig = rig();
    let job = event_job(&rig, "j-keep", |_| {});
    drone_runner(&rig);
    let now = agentmux_common::time::now_ms();
    // A run held back by a long delay: its event is older than the ids are kept, its input is still needed.
    rig.store.job_event_input_put("held-run", &job.id, &json!({ "n": 1 }), now - KEEP_MS - 1000).unwrap();
    rig.store.job_event_input_put("ancient-run", &job.id, &json!({ "n": 2 }), now - INPUT_KEEP_MS - 1000).unwrap();
    assert!(rig.sched.fire_event(&job, event("fresh", json!({}))).await.run_id.is_some(), "a new event prunes");
    assert_eq!(rig.ledger.event_input("held-run"), Some(json!({ "n": 1 })), "an input a delay may still need stays");
    assert_eq!(rig.ledger.event_input("ancient-run"), None, "one older than the longest delay plus the ids' week goes");
}
