// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Mutex;

use super::follower::{on_turn, Armed, Effect, TurnSeen};
use super::*;
use crate::backend::cron::{test_job, CronScheduler, Slot};
use crate::backend::mps::{MuxEvent, EVENT_AGENT_TURN};

const MIN: i64 = 60_000;
const HOUR: i64 = 60 * MIN;

fn turn(turn_id: i64, started: i64, ended: Option<i64>, end: Option<&str>, cost: f64) -> TurnSeen {
    TurnSeen {
        block_id: "b1".into(),
        turn_id,
        started_at_ms: started,
        ended_at_ms: ended,
        end: end.map(str::to_string),
        cost_usd: cost,
        output_tokens: 120,
        steps: 3,
    }
}

fn armed(at: i64) -> Armed {
    Armed { run_id: "r1".into(), armed_at_ms: at, turn_id: None, last: None }
}

// ── the follower's rules ────────────────────────────────────────────────

#[test]
fn a_turn_the_fire_started_is_bound_whole_and_ends_with_its_cost() {
    let mut a = armed(1_000);
    assert_eq!(on_turn(&mut a, &turn(1_001, 1_001, None, None, 0.0)), vec![Effect::Bind { turn_id: 1_001, partial: false }]);
    assert!(on_turn(&mut a, &turn(1_001, 1_001, None, None, 0.1)).is_empty(), "a running update changes nothing");
    let end = on_turn(&mut a, &turn(1_001, 1_001, Some(9_000), Some("completed"), 0.3));
    assert_eq!(
        end,
        vec![Effect::Finish {
            infra: infra::OK,
            cost: Some(RunCost { cost_usd: 0.3, cost_known: true, output_tokens: 120, steps: 3 }),
            summary: ""
        }]
    );
}

#[test]
fn a_fire_that_joined_a_running_turn_is_partial_and_an_old_turn_is_not_its() {
    let mut a = armed(5_000);
    assert!(on_turn(&mut a, &turn(1_000, 1_000, Some(4_000), Some("completed"), 0.2)).is_empty(), "over before the fire");
    assert_eq!(on_turn(&mut a, &turn(4_500, 4_500, None, None, 0.0)), vec![Effect::Bind { turn_id: 4_500, partial: true }]);
}

#[test]
fn an_exited_process_interrupts_the_run_and_a_provider_without_cost_is_unknown() {
    let mut a = armed(1_000);
    on_turn(&mut a, &turn(1_100, 1_100, None, None, 0.0));
    let end = on_turn(&mut a, &turn(1_100, 1_100, Some(2_000), Some("exited"), 0.0));
    let Effect::Finish { infra: status, cost: Some(cost), .. } = &end[0] else { panic!("{end:?}") };
    assert_eq!(*status, infra::INTERRUPTED);
    assert!(!cost.cost_known, "zero cost: the provider reported none");
}

#[test]
fn a_newer_turn_closes_a_bound_run_whose_end_never_arrived() {
    let mut a = armed(1_000);
    on_turn(&mut a, &turn(1_100, 1_100, None, None, 0.5));
    assert!(on_turn(&mut a, &turn(900, 900, None, None, 0.0)).is_empty(), "an older turn's late publish");
    let end = on_turn(&mut a, &turn(3_000, 3_000, None, None, 0.0));
    let Effect::Finish { cost: Some(cost), .. } = &end[0] else { panic!("{end:?}") };
    assert_eq!(cost.cost_usd, 0.5, "the bound turn's last figures");
}

// ── helpers ─────────────────────────────────────────────────────────────

#[test]
fn missed_slots_counts_the_newest_and_the_earlier_ones() {
    let base = chrono::DateTime::parse_from_rfc3339("2026-10-23T06:00:30Z").unwrap().timestamp_millis();
    // Hourly: 07:00 … 11:00 missed by 11:30.
    let (newest, earlier) = missed_slots("0 * * * *", base, base + 5 * 60 * MIN, 1_000).unwrap();
    assert_eq!(newest, base - 30_000 + 5 * 60 * MIN);
    assert_eq!(earlier, Some((base - 30_000 + 60 * MIN, 4)));
    let (_, one) = missed_slots("0 * * * *", base, base + 61 * MIN, 1_000).unwrap();
    assert_eq!(one, None, "a single missed slot has no earlier ones");
    assert!(missed_slots("0 * * * *", base, base + MIN, 1_000).is_none());
    assert_eq!(next_slot_after("0 * * * *", base - 30_000), Some(base - 30_000 + 60 * MIN));
}

/// After long downtime the newest missed slot is still the last one before
/// now; only the count of earlier ones is capped.
#[test]
fn the_newest_missed_slot_survives_long_downtime() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-10-24T08:00:30Z").unwrap().timestamp_millis();
    let base = now - 70 * 24 * 60 * MIN;
    let (newest, earlier) = missed_slots("* * * * *", base, now, 100_000).unwrap();
    assert_eq!(newest, now - 30_000, "08:00, the last minute before now");
    let (first, count) = earlier.unwrap();
    assert_eq!(count, 100_000, "capped");
    assert!(first > base && first < base + MIN + 1, "counting starts at the first slot after base");
    assert_eq!(next_slot_after("* * * * *", newest), Some(newest + MIN), "a pending fire for it expires at 08:01");
}

#[test]
fn the_brief_line_names_the_run_and_a_late_one_says_so() {
    let line = brief_line("run-1", None, 0);
    assert!(line.contains("run-1") && line.contains("Schedule action report"), "{line}");
    assert!(!line.contains("late"));
    assert!(!line.contains("[AgentMux"), "never a marker srv would rewrite");
    let slot = chrono::DateTime::parse_from_rfc3339("2026-10-23T06:00:00Z").unwrap().timestamp_millis();
    let late = brief_line("run-1", Some(slot), slot + 3 * 60 * MIN);
    assert!(late.contains("2026-10-23 06:00 UTC") && late.contains("09:00 UTC") && late.contains("late"), "{late}");
}

#[test]
fn a_drone_runs_cost_is_the_sum_of_its_agent_blocks() {
    use crate::drone::types::BlockState;
    let block = |out: Option<serde_json::Value>| BlockState {
        status: "done".into(),
        output: out,
        error: None,
        started_at: None,
        completed_at: None,
    };
    let states: std::collections::HashMap<String, BlockState> = [
        ("a1".to_string(), block(Some(json!({"cost_usd": 0.25, "tokens": {"output": 100}})))),
        ("a2".to_string(), block(Some(json!({"cost_usd": 0.5, "tokens": {"output": 50}})))),
        ("t1".to_string(), block(Some(json!({"value": "text"})))),
        ("x".to_string(), block(None)),
    ]
    .into_iter()
    .collect();
    let cost = drone_cost(&states);
    assert_eq!((cost.cost_usd, cost.output_tokens, cost.cost_known), (0.75, 150, true));

    // No block reported a cost: not known, rather than a known $0.
    let none: std::collections::HashMap<String, BlockState> =
        [("t1".to_string(), block(Some(json!({"value": "text"})))), ("x".to_string(), block(None))].into_iter().collect();
    assert!(!drone_cost(&none).cost_known);
    assert!(!drone_cost(&Default::default()).cost_known);
}

#[test]
fn a_slash_command_gets_no_run_line() {
    let plain = crate::backend::cron::fire_message("check the build", Some("run-1"), None);
    assert!(plain.starts_with("check the build\n\n(Scheduled run run-1."), "{plain}");
    for cmd in ["/compact", "/model opus", "  /review 12"] {
        assert_eq!(crate::backend::cron::fire_message(cmd, Some("run-1"), None), cmd, "the command's arguments stay its own");
    }
    assert_eq!(crate::backend::cron::fire_message("check", None, None), "check", "no run, no line");
}

// ── the ledger with a scheduler ─────────────────────────────────────────

struct Rig {
    _dir: tempfile::TempDir,
    store: Arc<Store>,
    broker: Arc<Broker>,
    ledger: Arc<RunLedger>,
    sched: Arc<CronScheduler>,
    /// Requests the fake delivery received.
    seen: Arc<Mutex<Vec<crate::backend::reactive::types::InjectionRequest>>>,
    /// What the fake delivery answers.
    answer: Arc<Mutex<serde_json::Value>>,
    present: Arc<Mutex<bool>>,
    /// A turn ledger published while the fake delivery runs, as a real
    /// delivery that waits for the turn it starts may see.
    turn_during_delivery: Arc<Mutex<Option<serde_json::Value>>>,
}

const DELIVERED: &str = r#"{"success": true, "block_id": "b1"}"#;
const NOT_FOUND: &str = r#"{"success": false, "error": "agent not found: uid-y"}"#;

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap());
    rig_on(dir, store)
}

fn rig_on(dir: tempfile::TempDir, store: Arc<Store>) -> Rig {
    let broker = Arc::new(Broker::new());
    let ledger = RunLedger::new(store.clone(), None, broker.clone());
    let sched = CronScheduler::new(Some(store.clone()), reqwest::Client::new(), String::new(), String::new(), broker.clone());
    sched.install_runs(ledger.clone());
    let seen: Arc<Mutex<Vec<_>>> = Arc::default();
    let answer = Arc::new(Mutex::new(serde_json::from_str::<serde_json::Value>(DELIVERED).unwrap()));
    let (sink, reply) = (seen.clone(), answer.clone());
    let turn_during_delivery: Arc<Mutex<Option<serde_json::Value>>> = Arc::default();
    let (turn, publisher) = (turn_during_delivery.clone(), broker.clone());
    sched.install_delivery(Arc::new(move |req, _uid| {
        sink.lock().unwrap().push(req);
        if let Some(mut data) = turn.lock().unwrap().take() {
            // The turn starts and ends now, while the delivery runs.
            let started = agentmux_common::time::now_ms();
            data["turn_id"] = json!(started);
            data["started_at_ms"] = json!(started);
            data["ended_at_ms"] = json!(started + 1);
            publisher.publish(MuxEvent {
                event: EVENT_AGENT_TURN.to_string(),
                scopes: vec!["block:b1".to_string()],
                sender: String::new(),
                persist: 1,
                data: Some(data),
            });
        }
        let body = reply.lock().unwrap().clone();
        Box::pin(async move { body })
    }));
    let present = Arc::new(Mutex::new(false));
    let p = present.clone();
    sched.install_presence(Arc::new(move |_uid: &str| *p.lock().unwrap()));
    Rig { _dir: dir, store, broker, ledger, sched, seen, answer, present, turn_during_delivery }
}

impl Rig {
    fn job(&self, id: &str, catch_up: &str) -> crate::backend::storage::cron::CronJob {
        let mut job = test_job(id, "check the build", "AgentY", "uid-y", "uid-creator");
        // Hourly, so a test that reads the clock doesn't cross a slot.
        job.expression = "0 * * * *".to_string();
        job.catch_up = catch_up.to_string();
        self.store.cron_create(&job).unwrap();
        job
    }

    fn answer(&self, body: &str) {
        *self.answer.lock().unwrap() = serde_json::from_str(body).unwrap();
    }

    fn runs(&self, job: &str) -> Vec<JobRun> {
        self.store.job_runs_for_job(job, 100, None).unwrap()
    }

    fn fire_count(&self, job: &str) -> i64 {
        self.store.cron_get(job).unwrap().unwrap().fire_count
    }

    fn publish_turn(&self, data: serde_json::Value) {
        self.broker.publish(MuxEvent {
            event: EVENT_AGENT_TURN.to_string(),
            scopes: vec!["block:b1".to_string()],
            sender: String::new(),
            persist: 1,
            data: Some(data),
        });
    }

    async fn wait_for(&self, run_id: &str, pred: impl Fn(&JobRun) -> bool) -> JobRun {
        for _ in 0..100 {
            if let Some(r) = self.store.job_run_get(run_id).unwrap().filter(|r| pred(r)) {
                return r;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("run {run_id} never got there: {:?}", self.store.job_run_get(run_id).unwrap());
    }
}

/// Delivery waits for the turn it starts, so a fast turn can start and end
/// before the answer: it is still this run's whole turn, not a partial or
/// an earlier one.
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_that_starts_and_ends_during_delivery_is_the_runs() {
    let rig = rig();
    let job = rig.job("j-fast", CATCH_UP_SKIP);
    *rig.turn_during_delivery.lock().unwrap() = Some(json!({
        "block_id": "b1", "active": false, "end": "completed", "cost_usd": 0.05, "output_tokens": 10, "steps": 1,
    }));
    let run_id = rig.sched.fire(&job, Slot::Scheduled(1_800_000_000_000)).await.run_id.unwrap();
    // Answered only after the turn ended: the run still takes it, whole.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let done = rig.wait_for(&run_id, |r| r.state == state::FINISHED).await;
    assert!(done.turn_id.is_some(), "{done:?}");
    assert_eq!((done.cost_partial, done.cost_usd, done.infra.as_str()), (false, 0.05, infra::OK));
}

/// A pending fire's agent may be registered by another channel's AgentMux
/// on this computer: presence covers the shared registry the inject path
/// forwards through.
#[test]
fn an_agent_in_another_channel_counts_as_back() {
    let dir = tempfile::tempdir().unwrap();
    let write = |agent: &str, channel: &str, uid: &str, pid: u32| {
        let path = dir.path().join(agent);
        std::fs::create_dir_all(&path).unwrap();
        let entry = json!({
            "agent_id": agent, "local_url": "http://127.0.0.1:1", "block_id": "b", "pid": pid,
            "updated_at": 1, "channel": channel, "uid": uid,
        });
        std::fs::write(path.join(format!("{channel}.json")), entry.to_string()).unwrap();
    };
    write("agent-y", "channel-a", "uid-y", std::process::id());
    write("agent-z", "channel-a", "uid-z", u32::MAX);
    assert!(registered_in_another_channel(dir.path(), "uid-y", "channel-b"));
    assert!(!registered_in_another_channel(dir.path(), "uid-y", "channel-a"), "its own channel is the local check's");
    assert!(!registered_in_another_channel(dir.path(), "uid-z", "channel-b"), "a dead process isn't back");
    assert!(!registered_in_another_channel(dir.path(), "uid-q", "channel-b"));
    assert!(!registered_in_another_channel(dir.path(), "", "channel-b"));
}

/// A delivered fire: its run id is the request id and ends the prompt; the
/// row goes scheduled → running → finished with the turn's cost, and
/// `fire_count` rises.
#[tokio::test(flavor = "multi_thread")]
async fn a_cron_fire_runs_and_finishes_with_its_turns_cost() {
    let rig = rig();
    let job = rig.job("j-ok", CATCH_UP_SKIP);
    let slot = 1_800_000_000_000;
    let fired = rig.sched.fire(&job, Slot::Scheduled(slot)).await;
    let run_id = fired.run_id.clone().unwrap();
    {
        let seen = rig.seen.lock().unwrap();
        assert_eq!(seen[0].request_id.as_deref(), Some(run_id.as_str()));
        assert!(seen[0].message.starts_with("check the build\n\n(Scheduled run "), "{}", seen[0].message);
        assert!(seen[0].message.contains(&run_id));
        assert_eq!(seen[0].source_agent.as_deref(), Some("cron"));
    }
    let r = rig.store.job_run_get(&run_id).unwrap().unwrap();
    assert_eq!((r.state.as_str(), r.trigger.as_str(), r.scheduled_for_ms), (state::RUNNING, TRIGGER_SCHEDULE, Some(slot)));
    assert_eq!((r.block_id.as_str(), r.target_agent_uid.as_str()), ("b1", "uid-y"));
    assert_eq!(r.links["transcript"]["block_id"], "b1");
    assert_eq!(rig.fire_count("j-ok"), 1);

    let started = r.started_at_ms + 1;
    rig.publish_turn(json!({"block_id": "b1", "turn_id": started, "started_at_ms": started, "active": true, "cost_usd": 0.0}));
    let bound = rig.wait_for(&run_id, |r| r.turn_id.is_some()).await;
    assert_eq!((bound.turn_id, bound.cost_partial), (Some(started), false));
    rig.publish_turn(json!({
        "block_id": "b1", "turn_id": started, "started_at_ms": started, "active": false,
        "ended_at_ms": started + 5_000, "end": "completed", "cost_usd": 0.37, "output_tokens": 812, "steps": 9,
    }));
    let done = rig.wait_for(&run_id, |r| r.state == state::FINISHED).await;
    assert_eq!((done.infra.as_str(), done.cost_usd, done.cost_known), (infra::OK, 0.37, true));
    assert_eq!((done.output_tokens, done.steps, done.outcome.as_str()), (812, 9, "unknown"));
}

/// A fire to a stopped agent is never held: with catch-up `skip` it is
/// `missed` and `fire_count` stays; a missing agent is a collapsed skip;
/// another failure is `finished` with `infra: error`.
#[tokio::test(flavor = "multi_thread")]
async fn an_undelivered_fire_is_missed_and_not_counted() {
    let rig = rig();
    let job = rig.job("j-miss", CATCH_UP_SKIP);
    rig.answer(NOT_FOUND);
    let fired = rig.sched.fire(&job, Slot::Scheduled(1_800_000_000_000)).await;
    let r = rig.store.job_run_get(fired.run_id.as_deref().unwrap()).unwrap().unwrap();
    assert_eq!((r.state.as_str(), r.reason.as_str()), (state::MISSED, reason::TARGET_STOPPED));
    assert!(r.ended_at_ms.is_some());
    assert_eq!(rig.fire_count("j-miss"), 0, "an undelivered fire doesn't count");

    rig.answer(r#"{"success": false, "error": "rate limit exceeded"}"#);
    let fired = rig.sched.fire(&job, Slot::Scheduled(1_800_000_060_000)).await;
    let r = rig.store.job_run_get(fired.run_id.as_deref().unwrap()).unwrap().unwrap();
    assert_eq!((r.state.as_str(), r.infra.as_str(), r.summary.as_str()), (state::FINISHED, infra::ERROR, "rate limit exceeded"));
    assert_eq!(rig.fire_count("j-miss"), 0);

    // The ledger's mapping of a missing target (the cron fix decides it).
    let missing = crate::backend::cron::FireOutcome::Missed { reason: crate::backend::cron::MissReason::TargetMissing };
    for slot in [1_800_000_120_000, 1_800_000_180_000] {
        let Begin::Run(id) = rig.ledger.begin_cron(&job, TRIGGER_SCHEDULE, Some(slot), BY_SCHEDULER) else { panic!() };
        rig.ledger.record_delivery(&id, &job, &missing, true, 0);
    }
    let skips: Vec<_> = rig.runs("j-miss").into_iter().filter(|r| r.state == state::SKIPPED).collect();
    assert_eq!(skips.len(), 1, "consecutive skips collapse");
    assert_eq!((skips[0].reason.as_str(), skips[0].skip_count), (reason::TARGET_MISSING, 2));
}

/// A delivery whose answer names no pane (the relay's) has no turn to
/// follow: it closes as finished at once instead of waiting for a restart
/// to call it interrupted.
#[tokio::test(flavor = "multi_thread")]
async fn a_delivery_with_no_pane_closes_at_once() {
    let rig = rig();
    let job = rig.job("j-relay", CATCH_UP_SKIP);
    let Begin::Run(id) = rig.ledger.begin_cron(&job, TRIGGER_SCHEDULE, Some(1_800_000_000_000), BY_SCHEDULER) else { panic!() };
    let delivered = crate::backend::cron::FireOutcome::Delivered { block_id: None };
    rig.ledger.record_delivery(&id, &job, &delivered, true, 0);
    let r = rig.store.job_run_get(&id).unwrap().unwrap();
    assert_eq!((r.state.as_str(), r.infra.as_str(), r.cost_known), (state::FINISHED, infra::OK, false));
    assert!(r.ended_at_ms.is_some());
}

/// Two schedulers sharing one identity store fire a slot once.
#[tokio::test(flavor = "multi_thread")]
async fn two_schedulers_sharing_the_store_fire_a_slot_once() {
    let a = rig();
    let store = a.store.clone();
    let b = rig_on(tempfile::tempdir().unwrap(), store);
    let job = a.job("j-twice", CATCH_UP_SKIP);
    let slot = 1_800_000_000_000;
    let (fa, fb) = tokio::join!(a.sched.fire(&job, Slot::Scheduled(slot)), b.sched.fire(&job, Slot::Scheduled(slot)));
    let delivered = a.seen.lock().unwrap().len() + b.seen.lock().unwrap().len();
    assert_eq!(delivered, 1, "one delivery for the slot");
    assert_eq!([fa.outcome.is_some(), fb.outcome.is_some()].iter().filter(|x| **x).count(), 1);
    assert_eq!(a.runs("j-twice").len(), 1);
    assert_eq!(a.fire_count("j-twice"), 1);
}

/// Run now is a `manual` run with no slot; it doesn't count toward the
/// schedule's `fire_count`.
#[tokio::test(flavor = "multi_thread")]
async fn run_now_is_a_manual_run_through_the_same_path() {
    let rig = rig();
    let job = rig.job("j-now", CATCH_UP_SKIP);
    let fired = rig.sched.run_now(&job, "uid-creator").await;
    let r = rig.store.job_run_get(fired.run_id.as_deref().unwrap()).unwrap().unwrap();
    assert_eq!((r.trigger.as_str(), r.triggered_by.as_str(), r.scheduled_for_ms), (TRIGGER_MANUAL, "uid-creator", None));
    assert_eq!(r.state, state::RUNNING);
    assert_eq!(rig.fire_count("j-now"), 0);
    // A second Run now is its own run.
    rig.sched.run_now(&job, "operator").await;
    assert_eq!(rig.runs("j-now").len(), 2);
}

/// Catch-up `latest`: a missed fire is the job's one pending fire; a newer
/// missed fire supersedes it; it waits while the agent is away and is
/// delivered once when the agent is back, as the same run.
#[tokio::test(flavor = "multi_thread")]
async fn latest_keeps_one_pending_fire_and_delivers_it_once() {
    let rig = rig();
    let job = rig.job("j-latest", CATCH_UP_LATEST);
    rig.answer(NOT_FOUND);
    let now = agentmux_common::time::now_ms();
    let slot1 = now - now % HOUR;
    let first = rig.sched.fire(&job, Slot::Scheduled(slot1 - HOUR)).await.run_id.unwrap();
    let second = rig.sched.fire(&job, Slot::Scheduled(slot1)).await.run_id.unwrap();
    let r1 = rig.store.job_run_get(&first).unwrap().unwrap();
    assert_eq!((r1.state.as_str(), r1.reason.as_str()), (state::PENDING, reason::SUPERSEDED));
    assert!(r1.ended_at_ms.is_some());
    let r2 = rig.store.job_run_get(&second).unwrap().unwrap();
    assert_eq!((r2.state.as_str(), r2.reason.as_str(), r2.ended_at_ms), (state::PENDING, reason::TARGET_STOPPED, None));
    assert_eq!(rig.fire_count("j-latest"), 0);

    // Still away: nothing is attempted.
    let before = rig.seen.lock().unwrap().len();
    assert_eq!(rig.sched.deliver_pending().await, 0);
    assert_eq!(rig.seen.lock().unwrap().len(), before);

    // Back: delivered once, as the same run, marked late.
    *rig.present.lock().unwrap() = true;
    rig.answer(DELIVERED);
    assert_eq!(rig.sched.deliver_pending().await, 1);
    assert_eq!(rig.sched.deliver_pending().await, 0, "once");
    let seen = rig.seen.lock().unwrap().last().cloned().unwrap();
    assert_eq!(seen.request_id.as_deref(), Some(second.as_str()));
    assert!(seen.message.contains("(late)"), "{}", seen.message);
    let r2 = rig.store.job_run_get(&second).unwrap().unwrap();
    assert_eq!(r2.state, state::RUNNING);
    assert_eq!(rig.fire_count("j-latest"), 1, "a delivered pending fire counts");
}

/// A pending fire whose delivery answers with something the held-jekt replay
/// retries (the agent still starting, not signed in) stays pending for the
/// next wake; a refusal that isn't transient closes it.
#[tokio::test(flavor = "multi_thread")]
async fn a_pending_fire_survives_a_try_again_answer() {
    let rig = rig();
    let job = rig.job("j-retry", CATCH_UP_LATEST);
    rig.answer(NOT_FOUND);
    let now = agentmux_common::time::now_ms();
    let run = rig.sched.fire(&job, Slot::Scheduled(now - now % HOUR)).await.run_id.unwrap();
    *rig.present.lock().unwrap() = true;
    for answer in [
        r#"{"success": false, "error": "persistent process is still starting up — try again shortly"}"#,
        r#"{"success": false, "error": "identity spawn gate: no credentials for claude"}"#,
        r#"{"success": false, "error": "rate limit exceeded"}"#,
        r#"{"success": false, "error": "agent y's account is out of quota"}"#,
    ] {
        rig.answer(answer);
        assert_eq!(rig.sched.deliver_pending().await, 0);
        let r = rig.store.job_run_get(&run).unwrap().unwrap();
        assert_eq!((r.state.as_str(), r.ended_at_ms), (state::PENDING, None), "{answer}");
    }
    rig.answer(DELIVERED);
    assert_eq!(rig.sched.deliver_pending().await, 1, "delivered on the next pass");
    assert_eq!(rig.store.job_run_get(&run).unwrap().unwrap().state, state::RUNNING);

    // A refusal that won't change by waiting closes the pending fire.
    rig.answer(NOT_FOUND);
    let refused = rig.sched.fire(&job, Slot::Scheduled(now - now % HOUR + HOUR)).await.run_id.unwrap();
    rig.answer(r#"{"success": false, "error": "subprocess controller does not accept raw input"}"#);
    rig.sched.deliver_pending().await;
    let r = rig.store.job_run_get(&refused).unwrap().unwrap();
    assert_eq!((r.state.as_str(), r.infra.as_str()), (state::FINISHED, infra::ERROR));
}

/// A job one fire short of max_fires whose pending fire is delivered is
/// disabled and its loop stopped: it doesn't fire again.
#[tokio::test(flavor = "multi_thread")]
async fn a_delivered_pending_fire_counts_toward_max_fires() {
    let rig = rig();
    let mut job = test_job("j-max", "check the build", "AgentY", "uid-y", "uid-creator");
    job.expression = "0 * * * *".to_string();
    job.catch_up = CATCH_UP_LATEST.to_string();
    job.max_fires = Some(2);
    job.fire_count = 1;
    rig.store.cron_create(&job).unwrap();
    rig.sched.schedule_job(&job);
    rig.answer(NOT_FOUND);
    let now = agentmux_common::time::now_ms();
    rig.sched.fire(&job, Slot::Scheduled(now - now % HOUR)).await;
    *rig.present.lock().unwrap() = true;
    rig.answer(DELIVERED);
    assert_eq!(rig.sched.deliver_pending().await, 1);
    let stored = rig.store.cron_get("j-max").unwrap().unwrap();
    assert_eq!((stored.fire_count, stored.enabled), (2, false), "at max_fires: disabled");
    assert!(!rig.sched.is_scheduled("j-max"), "its loop is stopped, so it doesn't fire again");
}

/// A pending fire expires at the job's next fire time, and a delivered
/// scheduled fire ends an older pending one as expired.
#[tokio::test(flavor = "multi_thread")]
async fn a_pending_fire_expires_at_the_next_fire_time() {
    let rig = rig();
    let job = rig.job("j-exp", CATCH_UP_LATEST);
    rig.answer(NOT_FOUND);
    let now = agentmux_common::time::now_ms();
    let old_slot = now - now % HOUR - 5 * HOUR;
    let stale = rig.sched.fire(&job, Slot::Scheduled(old_slot)).await.run_id.unwrap();
    *rig.present.lock().unwrap() = true;
    rig.answer(DELIVERED);
    assert_eq!(rig.sched.deliver_pending().await, 0, "its next fire time has passed");
    let r = rig.store.job_run_get(&stale).unwrap().unwrap();
    assert_eq!((r.state.as_str(), r.reason.as_str()), (state::PENDING, reason::EXPIRED));

    rig.answer(NOT_FOUND);
    let waiting = rig.sched.fire(&job, Slot::Scheduled(now - now % HOUR)).await.run_id.unwrap();
    rig.answer(DELIVERED);
    rig.sched.fire(&job, Slot::Scheduled(now - now % HOUR + HOUR)).await;
    let r = rig.store.job_run_get(&waiting).unwrap().unwrap();
    assert_eq!((r.reason.as_str(), r.ended_at_ms.is_some()), (reason::EXPIRED, true));
}

/// Start a scheduler over one hourly job whose missed slots are counted from
/// `last_fired`, or from `created_at` when it never fired; returns its runs
/// and how many fires were delivered.
async fn startup_case(policy: &str, never_fired: bool, fires: usize) -> (Vec<JobRun>, usize) {
    let rig = rig();
    let mut job = test_job(&format!("j-start-{policy}-{never_fired}"), "p", "AgentY", "uid-y", "c");
    job.expression = "0 * * * *".into();
    job.catch_up = policy.into();
    // 3 h 10 min ago: three hourly slots missed since.
    let now = chrono::Utc::now().timestamp();
    let boundary = now - now % 3600 - 3 * 3600 + 60;
    if never_fired {
        job.created_at = boundary;
    } else {
        job.last_fired = Some(boundary);
        job.created_at = boundary - 86_400;
    }
    rig.store.cron_create(&job).unwrap();
    rig.sched.start().await;
    let mut runs = Vec::new();
    for _ in 0..100 {
        runs = rig.runs(&job.id);
        if runs.iter().any(|r| r.state == state::SKIPPED) && (fires == 0 || runs.len() >= 2) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    rig.sched.cancel_job(&job.id);
    let delivered = rig.seen.lock().unwrap().len();
    (runs, delivered)
}

/// Startup: a job that missed slots while AgentMux was off gets one
/// collapsed `app_not_running` skip; `latest` also fires once for the newest
/// slot, `skip` doesn't. A job that never fired counts from its creation, so
/// a slot after it was created is caught up too (§5.2).
#[tokio::test(flavor = "multi_thread")]
async fn startup_records_slots_missed_while_off_by_catch_up_policy() {
    for never_fired in [false, true] {
        for (policy, fires, skipped) in [(CATCH_UP_LATEST, 1, 2), (CATCH_UP_SKIP, 0, 3)] {
            let (runs, delivered) = startup_case(policy, never_fired, fires).await;
            let case = format!("{policy}, never_fired={never_fired}");
            let skips: Vec<_> = runs.iter().filter(|r| r.state == state::SKIPPED).collect();
            assert_eq!(skips.len(), 1, "{case}: one collapsed row: {runs:?}");
            assert_eq!((skips[0].reason.as_str(), skips[0].skip_count), (reason::APP_NOT_RUNNING, skipped), "{case}");
            let catch_ups = runs.iter().filter(|r| r.trigger == TRIGGER_CATCH_UP).count();
            assert_eq!(catch_ups, fires, "{case}: {runs:?}");
            assert_eq!(delivered, fires, "{case}");
        }
    }
}

/// Two schedulers sharing the identity store account for the same startup
/// interval: the downtime is counted once, whether its skip starts a row or
/// folds into an earlier one.
#[tokio::test(flavor = "multi_thread")]
async fn two_schedulers_count_the_same_downtime_once() {
    let a = rig();
    let b = RunLedger::new(a.store.clone(), None, Arc::new(Broker::new()));
    let job = a.job("j-down", CATCH_UP_SKIP);
    let first = 1_800_000_000_000;
    a.ledger.record_missed_while_off(&job, first, 5);
    b.record_missed_while_off(&job, first, 5);
    let skips = a.runs("j-down");
    assert_eq!(skips.iter().map(|r| r.skip_count).collect::<Vec<_>>(), vec![5]);

    // The next downtime folds into that row; the second scheduler still finds
    // its first slot leased and adds nothing.
    let later = first + 10 * HOUR;
    a.ledger.record_missed_while_off(&job, later, 3);
    b.record_missed_while_off(&job, later, 3);
    let skips = a.runs("j-down");
    assert_eq!(skips.len(), 1, "consecutive skips show as one row");
    assert_eq!(skips[0].skip_count, 8, "each downtime counted once");
}

/// After a restart, this channel's cron runs left `scheduled` or `running`
/// and its Drone runs left `running` are interrupted, in both tables; a
/// pending fire keeps waiting.
#[tokio::test(flavor = "multi_thread")]
async fn interrupted_runs_are_closed_after_a_restart() {
    use crate::drone::types::DroneRun;
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap());
    let mstore = Arc::new(Store::open_in_memory().unwrap());
    let ledger = RunLedger::new(store.clone(), Some(mstore.clone()), Arc::new(Broker::new()));
    let job = test_job("j-restart", "p", "AgentY", "uid-y", "c");
    let Begin::Run(running) = ledger.begin_cron(&job, TRIGGER_SCHEDULE, Some(1), BY_SCHEDULER) else { panic!() };
    store.job_run_mark_running(&running, "uid-y", "b1", "", "", 5).unwrap();
    let Begin::Run(scheduled) = ledger.begin_cron(&job, TRIGGER_SCHEDULE, Some(2), BY_SCHEDULER) else { panic!() };
    let Begin::Run(pending) = ledger.begin_cron(&job, TRIGGER_SCHEDULE, Some(3), BY_SCHEDULER) else { panic!() };
    store.job_run_set_undelivered(&pending, state::PENDING, reason::TARGET_STOPPED, 6).unwrap();
    // Another channel's running run is not this instance's to close.
    let mut theirs = JobRun::new("theirs", "j-restart", SOURCE_CRON, "n", TRIGGER_MANUAL, state::RUNNING, 7);
    theirs.recorded_by = "some-other-channel@1.0.0".into();
    store.job_run_insert(&theirs).unwrap();

    mstore
        .drone_run_insert(&DroneRun {
            id: "dr1".into(),
            drone_id: "d1".into(),
            status: "running".into(),
            started_at: 1,
            ended_at: 0,
            block_states: Default::default(),
            output: String::new(),
            error: String::new(),
        })
        .unwrap();
    let drone_row = ledger.drone_started("dr1", "d1", "My drone", 1).unwrap();

    assert_eq!(ledger.recover_after_restart(), 3);
    for id in [&running, &scheduled, &drone_row] {
        let r = store.job_run_get(id).unwrap().unwrap();
        assert_eq!((r.state.as_str(), r.infra.as_str()), (state::FINISHED, infra::INTERRUPTED), "{id}");
    }
    assert_eq!(store.job_run_get(&pending).unwrap().unwrap().ended_at_ms, None, "a pending fire keeps waiting");
    assert_eq!(store.job_run_get("theirs").unwrap().unwrap().state, state::RUNNING);
    let drone = mstore.drone_runs_for("d1", 10).unwrap();
    assert_eq!(drone[0].status, DRONE_STATUS_INTERRUPTED);
    assert!(drone[0].ended_at > 0);
}

/// A Drone run is a `manual` ledger row linked to the run, closed with the
/// summed cost of its Agent blocks.
#[tokio::test(flavor = "multi_thread")]
async fn a_drone_run_is_written_to_the_ledger_with_its_cost() {
    use crate::drone::types::BlockState;
    let rig = rig();
    let id = rig.ledger.drone_started("dr-9", "d-9", "Nightly graph", 100).unwrap();
    let r = rig.store.job_run_get(&id).unwrap().unwrap();
    assert_eq!((r.job_source.as_str(), r.trigger.as_str(), r.state.as_str()), (SOURCE_DRONE, TRIGGER_MANUAL, state::RUNNING));
    assert_eq!((r.job_name.as_str(), r.links["drone_run_id"].as_str()), ("Nightly graph", Some("dr-9")));
    let states: std::collections::HashMap<String, BlockState> = [(
        "a1".to_string(),
        BlockState {
            status: "done".into(),
            output: Some(json!({"cost_usd": 0.12})),
            error: None,
            started_at: None,
            completed_at: None,
        },
    )]
    .into_iter()
    .collect();
    rig.ledger.drone_finished(&id, true, &states, "");
    let r = rig.store.job_run_get(&id).unwrap().unwrap();
    assert_eq!((r.state.as_str(), r.infra.as_str(), r.cost_usd), (state::FINISHED, infra::OK, 0.12));

    let failed = rig.ledger.drone_started("dr-10", "d-9", "Nightly graph", 200).unwrap();
    rig.ledger.drone_finished(&failed, false, &Default::default(), "block t1: boom");
    let r = rig.store.job_run_get(&failed).unwrap().unwrap();
    assert_eq!((r.infra.as_str(), r.summary.as_str()), (infra::ERROR, "block t1: boom"));
    assert_eq!(rig.store.job_runs_for_job("d-9", 10, None).unwrap().len(), 2);
}
