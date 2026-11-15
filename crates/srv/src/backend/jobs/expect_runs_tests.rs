// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Expect: the dead man's switch, a run starting when a watched job has had no good run lately
//! (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §3.2).

use super::*;
use crate::backend::cron::ActionOutcome;
use crate::backend::jobs::definition::trigger_kind;
use crate::backend::jobs::expect::{self, ExpectState, Verdict};
use crate::backend::jobs::poll::Check;
use crate::backend::storage::cron::CronJob;

fn filter(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    v.as_object().unwrap().clone()
}

#[test]
fn an_expect_filter_names_the_job_and_how_recent_a_good_run_must_be() {
    assert!(expect::validate_filter(&filter(json!({ "job": "nightly", "within_minutes": 1500 }))).is_ok());
    assert!(expect::validate_filter(&filter(json!({ "job": "nightly", "within_minutes": 43200, "every_minutes": 1440 }))).is_ok());
    for bad in [
        json!({ "within_minutes": 5 }),
        json!({ "job": "  ", "within_minutes": 5 }),
        json!({ "job": 3, "within_minutes": 5 }),
        json!({ "job": "n" }),
        json!({ "job": "n", "within_minutes": 0 }),
        json!({ "job": "n", "within_minutes": 43201 }),
        json!({ "job": "n", "within_minutes": 5.5 }),
        json!({ "job": "n", "within_minutes": 5, "every_minutes": 0 }),
        json!({ "job": "n", "within_minutes": 5, "every_minutes": 1441 }),
        json!({ "job": "n", "within_minutes": 5, "state": "x" }),
    ] {
        assert!(expect::validate_filter(&filter(bad.clone())).is_err(), "{bad}");
    }
    let stray = expect::validate_filter(&filter(json!({ "job": "n", "within_minutes": 5, "agent": "x" }))).unwrap_err();
    assert!(stray.contains("no filter key \"agent\"") && stray.contains("job, within_minutes, every_minutes"), "{stray}");
}

const MIN: i64 = 60_000;
const NOW: i64 = 1_800_000_000_000;

fn run(id: &str, state: &str, outcome: &str, started_min_ago: i64, ended_min_ago: Option<i64>) -> serde_json::Value {
    let mut r = json!({ "id": id, "job_id": "j1", "job": "nightly", "state": state, "outcome": outcome, "started_ms": NOW - started_min_ago * MIN });
    if let Some(e) = ended_min_ago {
        r["ended_ms"] = json!(NOW - e * MIN);
    }
    r
}

fn answer(runs: Vec<serde_json::Value>) -> serde_json::Value {
    json!({ "found": true, "count": runs.len(), "runs": runs })
}

#[test]
fn a_recent_good_run_or_one_still_going_is_fine_and_a_gap_is_one_event_keyed_by_the_last_good_run() {
    let fine = |runs, within| expect::judge("nightly", within, &answer(runs), NOW) == Verdict::Fine;
    assert!(fine(vec![run("r1", "finished", "ok", 70, Some(60))], 1500), "a good run an hour ago");
    assert!(fine(vec![run("r1", "finished", "findings", 70, Some(60))], 1500), "findings is a job that did its work");
    assert!(fine(vec![run("r0", "finished", "ok", 3000, Some(2990)), run("r1", "running", "", 20, None)], 1500), "an old good run, but one is still going");
    assert!(!fine(vec![run("r0", "finished", "ok", 3000, Some(2990)), run("r1", "running", "", 2000, None)], 1500), "a run stuck for longer than the window does not hold it off");

    let Verdict::Missed { id, input } = expect::judge("nightly", 1500, &answer(vec![run("r2", "finished", "failed", 100, Some(90)), run("r1", "finished", "ok", 2000, Some(1990))]), NOW) else {
        panic!("a failed run is not a good one");
    };
    assert_eq!(id, format!("expect:nightly:{}", NOW - 1990 * MIN), "keyed by the last good run");
    assert_eq!(input["expected"], json!("nightly"));
    assert_eq!(input["within_minutes"], json!(1500));
    assert_eq!(input["last_ok_ms"], json!(NOW - 1990 * MIN));
    assert_eq!(input["job_found"], json!(true));
    assert_eq!(input["last_seen"]["id"], json!("r2"), "what was last seen: the newest run, the failed one");
    assert_eq!(input["last_seen"]["outcome"], json!("failed"));

    let Verdict::Missed { id, input } = expect::judge("nightly", 60, &answer(vec![run("r1", "finished", "unknown", 100, Some(90))]), NOW) else { panic!("an unknown outcome is not good") };
    assert_eq!((id.as_str(), input["last_ok_ms"].is_null()), ("expect:nightly:never", true));

    let Verdict::Missed { id, input } = expect::judge("nightly", 60, &json!({ "found": false, "count": 0, "runs": [] }), NOW) else { panic!("no such job is a gap too") };
    assert_eq!((id.as_str(), input["job_found"].clone(), input["last_seen"].is_null()), ("expect:nightly:never", json!(false), true));

    assert_eq!(expect::judge("nightly", 60, &json!({ "found": false, "ambiguous": true, "count": 0, "runs": [] }), NOW), Verdict::Fine, "two jobs share the name: no guess");
}

fn expect_job(rig: &Rig, id: &str, ask: serde_json::Value) -> CronJob {
    let mut job = test_job(id, "", "", "", "");
    job.created_by = BY_OPERATOR.to_string();
    job.trigger_kind = trigger_kind::EVENT.to_string();
    job.trigger_event = json!({ "source": "expect", "filter": ask, "max_per_hour": 1000 });
    job.action_kind = "drone".to_string();
    job.action = json!({ "kind": "drone", "drone_id": "d-1" });
    rig.store.cron_create(&job).unwrap();
    job
}

fn fake(answer: &Arc<Mutex<serde_json::Value>>, asked: &Arc<Mutex<Vec<serde_json::Value>>>) -> Check {
    let (answer, asked) = (answer.clone(), asked.clone());
    Arc::new(move |ask| {
        asked.lock().unwrap().push(serde_json::Value::Object(ask));
        let reply = answer.lock().unwrap().clone();
        Box::pin(async move { if reply.is_null() { Err("not ready".to_string()) } else { Ok(reply) } })
    })
}

/// A gap starts one run, however many times it is looked at, and again only after a good run and a new gap.
#[tokio::test(flavor = "multi_thread")]
async fn a_gap_starts_one_run_and_a_good_run_followed_by_a_new_gap_starts_another() {
    let rig = rig();
    let inputs: Arc<Mutex<Vec<serde_json::Value>>> = Arc::default();
    let (ledger_for_runs, sink) = (rig.ledger.clone(), inputs.clone());
    rig.sched.install_actions(Arc::new(move |_, run_id| {
        sink.lock().unwrap().push(ledger_for_runs.event_input(&run_id).unwrap_or(serde_json::Value::Null));
        ledger_for_runs.drone_attached(&run_id, &format!("dr-{run_id}"), &Value::Null, agentmux_common::time::now_ms());
        ledger_for_runs.drone_finished(&run_id, true, &Default::default(), "");
        Box::pin(async { ActionOutcome::Drone })
    }));
    let job = expect_job(&rig, "j-expect", json!({ "job": "nightly", "within_minutes": 60, "every_minutes": 5 }));
    let reply = Arc::new(Mutex::new(serde_json::Value::Null));
    let asked = Arc::new(Mutex::new(Vec::new()));
    let check = fake(&reply, &asked);
    let all = [job];
    let mut state = ExpectState::default();
    let t0 = agentmux_common::time::now_ms();
    let at = |min: i64| t0 + min * MIN;

    *reply.lock().unwrap() = json!({ "found": true, "count": 1, "runs": [{ "id": "r1", "state": "finished", "outcome": "ok", "started_ms": at(0) - 30 * MIN, "ended_ms": at(0) - 29 * MIN }] });
    assert_eq!(expect::expect_due(&rig.sched, &all, &check, &mut state, at(0)).await, 0, "a good run 29 minutes ago");
    assert_eq!(*asked.lock().unwrap(), [json!({ "query": "jobs", "job": "nightly", "limit": 100 })]);
    assert_eq!(expect::expect_due(&rig.sched, &all, &check, &mut state, at(3)).await, 0, "not due: not looked at");
    assert_eq!(asked.lock().unwrap().len(), 1);

    assert_eq!(expect::expect_due(&rig.sched, &all, &check, &mut state, at(40)).await, 1, "the hour is up (r1 ended at -29)");
    assert_eq!(expect::expect_due(&rig.sched, &all, &check, &mut state, at(45)).await, 0, "the same gap, looked at again: nothing more");
    assert_eq!(expect::expect_due(&rig.sched, &all, &check, &mut state, at(50)).await, 0);
    let mut fresh = ExpectState::default();
    assert_eq!(expect::expect_due(&rig.sched, &all, &check, &mut fresh, at(55)).await, 0, "and after a restart too");
    let first = inputs.lock().unwrap().clone();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0]["expected"], json!("nightly"));
    assert_eq!(first[0]["last_ok_ms"], json!(at(0) - 29 * MIN));

    *reply.lock().unwrap() = json!({ "found": true, "count": 2, "runs": [
        { "id": "r2", "state": "finished", "outcome": "ok", "started_ms": at(60), "ended_ms": at(61) },
        { "id": "r1", "state": "finished", "outcome": "ok", "started_ms": at(0) - 30 * MIN, "ended_ms": at(0) - 29 * MIN } ] });
    assert_eq!(expect::expect_due(&rig.sched, &all, &check, &mut state, at(65)).await, 0, "it came after all");
    assert_eq!(expect::expect_due(&rig.sched, &all, &check, &mut state, at(130)).await, 1, "a new gap since r2 is a new event");
    assert_eq!(inputs.lock().unwrap().len(), 2);

    *reply.lock().unwrap() = serde_json::Value::Null;
    assert_eq!(expect::expect_due(&rig.sched, &all, &check, &mut state, at(300)).await, 0, "a failed read is not a gap");
}

#[test]
fn the_last_good_run_said_on_its_own_counts_when_the_listing_holds_only_failures() {
    let mut a = answer(vec![run("f1", "finished", "failed", 10, Some(9)), run("f2", "finished", "failed", 20, Some(19))]);
    a["last_ok"] = run("old", "finished", "ok", 70, Some(60));
    assert_eq!(expect::judge("nightly", 1500, &a, NOW), Verdict::Fine, "a good run an hour ago, behind the failures");
    let Verdict::Missed { id, input } = expect::judge("nightly", 30, &a, NOW) else { panic!("an hour is past 30 minutes") };
    assert_eq!(id, format!("expect:nightly:{}", NOW - 60 * MIN), "keyed by it, not by 'never'");
    assert_eq!(input["last_ok_ms"], json!(NOW - 60 * MIN));
    a["last_ok"] = serde_json::Value::Null;
    assert!(matches!(expect::judge("nightly", 30, &a, NOW), Verdict::Missed { id, .. } if id == "expect:nightly:never"), "none at all");
}

/// A gap's id is forgotten after a week like any event id; each look moves it forward while the gap lasts, so the
/// same gap does not start a second run.
#[tokio::test(flavor = "multi_thread")]
async fn a_gap_that_lasts_past_the_weekly_sweep_does_not_start_a_second_run() {
    use crate::backend::storage::job_events::KEEP_MS;
    let rig = rig();
    let ledger_for_runs = rig.ledger.clone();
    rig.sched.install_actions(Arc::new(move |_, run_id| {
        ledger_for_runs.drone_attached(&run_id, &format!("dr-{run_id}"), &Value::Null, agentmux_common::time::now_ms());
        ledger_for_runs.drone_finished(&run_id, true, &Default::default(), "");
        Box::pin(async { ActionOutcome::Drone })
    }));
    let job = expect_job(&rig, "j-week", json!({ "job": "nightly", "within_minutes": 60, "every_minutes": 60 }));
    let reply = Arc::new(Mutex::new(json!({ "found": true, "count": 0, "runs": [], "last_ok": null })));
    let check = fake(&reply, &Arc::default());
    let all = [job];
    let mut state = ExpectState::default();
    let t0 = agentmux_common::time::now_ms();
    let day = 24 * 60 * MIN;
    assert_eq!(expect::expect_due(&rig.sched, &all, &check, &mut state, t0).await, 1, "the gap");
    assert_eq!(expect::expect_due(&rig.sched, &all, &check, &mut state, t0 + 6 * day).await, 0, "still the same gap, looked at on day 6");
    rig.store.job_events_forget_before(t0 + 8 * day - KEEP_MS).unwrap();
    assert_eq!(expect::expect_due(&rig.sched, &all, &check, &mut state, t0 + 8 * day).await, 0, "the week's sweep did not take the gap's id");
    assert_eq!(rig.runs("j-week").len(), 1);
}
