// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Spending caps at work through the scheduler and the ledger: the
//! reservation before a run starts, the stop at a run's own cap, the
//! once-a-day alert, messages a day, and providers that report no cost
//! (SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.3, §8).

use super::*;
use crate::backend::cron::{ActionOutcome, FireOutcome};
use crate::backend::jobs::caps::Limits;
use crate::backend::jobs::definition::{Caps, Overlap};
use crate::backend::jobs::reservation::Target;
use crate::backend::storage::cron::CronJob;
use crate::backend::storage::job_runs::reason;

const SLOT0: i64 = 1_800_000_000_000;

fn slot(n: i64) -> Slot {
    Slot::Scheduled(SLOT0 + n * 60_000)
}

fn make(rig: &Rig, id: &str, run_usd: f64, day_usd: f64, edit: impl FnOnce(&mut CronJob)) -> CronJob {
    let mut job = test_job(id, "check the build", "AgentY", "uid-y", "uid-creator");
    job.expression = "0 * * * *".to_string();
    job.policy.caps = Some(Caps { run_usd: Some(run_usd), day_usd: Some(day_usd), messages_per_day: None });
    edit(&mut job);
    rig.store.cron_create(&job).unwrap();
    job
}

/// Settings' caps, and where every run would go: `account` and `provider`.
fn weigh(rig: &Rig, limits: Limits, account: &'static str, provider: &'static str) {
    rig.ledger.install_caps(
        Arc::new(move || limits),
        Arc::new(move |_| Target { account: account.to_string(), provider: provider.to_string() }),
    );
}

/// The run ends having spent `usd`.
fn spend(rig: &Rig, run_id: &str, usd: f64) {
    let cost = RunCost { cost_usd: usd, cost_known: true, output_tokens: 10, steps: 1 };
    rig.ledger.turn_finished(run_id, infra::OK, Some(cost), "");
}

fn states(rig: &Rig, job: &str) -> Vec<(String, String)> {
    let mut runs = rig.runs(job);
    runs.reverse();
    runs.into_iter().map(|r| (r.state, r.reason)).collect()
}

fn interrupts(rig: &Rig) -> Arc<Mutex<Vec<String>>> {
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = seen.clone();
    rig.ledger.install_interrupt(Arc::new(move |block: &str| sink.lock().unwrap().push(block.to_string())));
    seen
}

fn total_spent(rig: &Rig, job: &str) -> f64 {
    rig.runs(job).iter().map(|r| r.cost_usd).sum()
}

// ── the reservation ─────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn a_run_the_days_room_cannot_hold_is_a_collapsed_job_cap_skip_with_one_alert() {
    let rig = rig();
    let job = make(&rig, "j-day", 5.0, 8.0, |_| {});
    let first = rig.sched.fire(&job, slot(0)).await.run_id.unwrap();
    spend(&rig, &first, 1.0);
    let second = rig.sched.fire(&job, slot(1)).await.run_id.expect("1 + 5 fits 8");
    spend(&rig, &second, 4.0);

    let skipped = rig.sched.fire(&job, slot(2)).await;
    assert_eq!((skipped.run_id, skipped.outcome), (None, None), "5 spent + 5 reserved passes 8");
    rig.sched.fire(&job, slot(3)).await;
    rig.sched.fire(&job, slot(4)).await;

    assert_eq!(rig.runs("j-day").len(), 3, "the later skips collapse into one row");
    let row = &rig.runs("j-day")[0];
    assert_eq!((row.state.as_str(), row.reason.as_str(), row.skip_count), (state::SKIPPED, reason::JOB_CAP, 3));
    assert_eq!(rig.ledger.cap_alerts(), [("j-day".to_string(), "job_cap".to_string())], "one alert for the day");
    assert_eq!(rig.ledger.over_cap_today(&job), Some("job_cap"), "the tab says so");
    assert!(rig.store.cron_get("j-day").unwrap().unwrap().enabled, "the job isn't paused");
    assert_eq!(rig.seen.lock().unwrap().len(), 2, "only two runs were delivered");
}

/// However the runs spend, every one all of its per-run cap, the day's total
/// stays inside the daily cap.
#[tokio::test(flavor = "multi_thread")]
async fn the_daily_cap_holds_even_if_every_run_spends_its_whole_per_run_cap() {
    let rig = rig();
    let job = make(&rig, "j-hold", 3.0, 10.0, |_| {});
    for n in 0..8 {
        if let Some(run_id) = rig.sched.fire(&job, slot(n)).await.run_id {
            spend(&rig, &run_id, 3.0);
        }
    }
    assert_eq!(total_spent(&rig, "j-hold"), 9.0, "three runs of $3: a fourth could reach $12");
    assert!(total_spent(&rig, "j-hold") <= 10.0);
    assert_eq!(rig.runs("j-hold").iter().filter(|r| r.state == state::FINISHED).count(), 3);
    assert_eq!(rig.ledger.cap_alerts().len(), 1);
}

/// A run still going holds its whole per-run cap, so the next can't start on
/// room the first may yet spend; and a start that is refused leaves the first alone.
#[tokio::test(flavor = "multi_thread")]
async fn a_run_still_going_holds_its_whole_cap_in_the_reservation() {
    let rig = rig();
    let stopped = interrupts(&rig);
    let job = make(&rig, "j-hold-open", 5.0, 8.0, |j| j.policy.overlap = Overlap::CancelOther);
    let running = rig.sched.fire(&job, slot(0)).await.run_id.unwrap();
    let next = rig.sched.fire(&job, slot(1)).await;
    assert_eq!(next.run_id, None, "$5 held for the first + $5 passes $8");
    assert_eq!(rig.store.job_run_get(&running).unwrap().unwrap().state, state::RUNNING, "not cancelled for a run that can't start");
    assert!(stopped.lock().unwrap().is_empty());
    assert_eq!(states(&rig, "j-hold-open"), [("running".into(), "".into()), ("skipped".into(), "job_cap".into())]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_waiting_run_that_no_longer_fits_is_skipped_when_its_turn_comes() {
    let rig = rig();
    let job = make(&rig, "j-wait", 5.0, 8.0, |j| j.policy.overlap = Overlap::BufferOne);
    let running = rig.sched.fire(&job, slot(0)).await.run_id.unwrap();
    let waiting = rig.sched.fire(&job, slot(1)).await.run_id.unwrap();
    spend(&rig, &running, 4.5);
    assert_eq!(rig.sched.run_policy_pass().await, 0);
    let run = rig.store.job_run_get(&waiting).unwrap().unwrap();
    assert_eq!((run.state.as_str(), run.reason.as_str()), (state::SKIPPED, reason::JOB_CAP));
    assert_eq!(rig.ledger.cap_alerts().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_accounts_daily_cap_is_shared_by_every_job_on_it() {
    let rig = rig();
    weigh(&rig, Limits { run_usd: 5.0, job_day_usd: 20.0, account_day_usd: 6.0, messages_per_day: 24 }, "acct-a", "claude");
    let (a, b) = (make(&rig, "j-acct-a", 3.0, 20.0, |_| {}), make(&rig, "j-acct-b", 3.0, 20.0, |_| {}));
    let ra = rig.sched.fire(&a, slot(0)).await.run_id.unwrap();
    spend(&rig, &ra, 3.0);
    let rb = rig.sched.fire(&b, slot(0)).await.run_id.expect("3 + 3 fits 6");
    spend(&rig, &rb, 3.0);

    let again = rig.sched.fire(&a, slot(1)).await;
    assert_eq!((again.run_id, again.outcome), (None, None));
    assert_eq!(states(&rig, "j-acct-a"), [("finished".into(), "".into()), ("skipped".into(), "account_cap".into())]);
    assert_eq!(rig.ledger.cap_alerts(), [("j-acct-a".to_string(), "account_cap".to_string())]);
    assert_eq!(rig.ledger.over_cap_today(&a), Some("account_cap"));
    assert_eq!(rig.store.account_runs_cost_since("acct-a", 0).unwrap(), 6.0);
    assert_eq!(rig.store.account_runs_cost_since("acct-b", 0).unwrap(), 0.0, "another account is its own");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_in_flight_counts_against_the_account_at_its_whole_cap() {
    let rig = rig();
    weigh(&rig, Limits { run_usd: 5.0, job_day_usd: 20.0, account_day_usd: 5.0, messages_per_day: 24 }, "acct-a", "claude");
    let (a, b) = (make(&rig, "j-fl-a", 3.0, 20.0, |_| {}), make(&rig, "j-fl-b", 3.0, 20.0, |_| {}));
    rig.sched.fire(&a, slot(0)).await.run_id.expect("nothing spent yet");
    let held = rig.store.job_run_get(&rig.runs("j-fl-a")[0].id).unwrap().unwrap();
    assert_eq!(held.account_id, "acct-a", "the account is noted at admission");
    let blocked = rig.sched.fire(&b, slot(0)).await;
    assert_eq!(blocked.run_id, None, "$3 held for the first + $3 for this one passes $5");
    assert_eq!(states(&rig, "j-fl-b"), [("skipped".into(), "account_cap".into())]);
}

// ── the day ─────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn spend_resets_at_midnight_and_the_alert_comes_again() {
    let rig = rig();
    let day1 = agentmux_common::time::now_ms() - 3_600_000;
    rig.ledger.set_day_start(Arc::new(move |_| day1));
    let job = make(&rig, "j-midnight", 5.0, 5.0, |_| {});
    let run = rig.sched.fire(&job, slot(0)).await.run_id.unwrap();
    spend(&rig, &run, 5.0);
    assert_eq!(rig.sched.fire(&job, slot(1)).await.run_id, None, "the day is spent");
    assert_eq!(rig.sched.fire(&job, slot(2)).await.run_id, None);
    assert_eq!(rig.ledger.cap_alerts().len(), 1);

    // Midnight passes: the day's start moves past everything spent so far.
    let day2 = agentmux_common::time::now_ms() + 1;
    rig.ledger.set_day_start(Arc::new(move |_| day2));
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    assert_eq!(rig.ledger.over_cap_today(&job), None, "a new day");
    let run = rig.sched.fire(&job, slot(3)).await.run_id.expect("runs again");
    spend(&rig, &run, 5.0);
    assert_eq!(rig.sched.fire(&job, slot(4)).await.run_id, None);
    assert_eq!(rig.ledger.cap_alerts().len(), 2, "the first skip of the new day raises its own alert");
}

// ── Run now ─────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn run_now_is_refused_while_a_cap_leaves_no_room() {
    let rig = rig();
    let job = make(&rig, "j-now", 5.0, 5.0, |_| {});
    let run = rig.sched.fire(&job, slot(0)).await.run_id.unwrap();
    spend(&rig, &run, 5.0);
    let fired = rig.sched.run_now(&job, "operator").await;
    assert!(
        matches!(fired.outcome, Some(FireOutcome::Failed { ref error }) if error.contains("Run now is refused") && error.contains("daily cap")),
        "{fired:?}"
    );
    assert_eq!(rig.runs("j-now").len(), 1, "Run now leaves no row of its own");
}

// ── stopping a run at its cap ───────────────────────────────────────────

fn turn_event(block: &str, turn: i64, started: i64, cost: f64, end: Option<&str>) -> serde_json::Value {
    let mut v = json!({"block_id": block, "turn_id": turn, "started_at_ms": started, "active": end.is_none(), "cost_usd": cost});
    if let Some(end) = end {
        v["end"] = json!(end);
        v["ended_at_ms"] = json!(started + 5_000);
    }
    v
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_that_reaches_its_cap_is_interrupted_and_ends_over_budget() {
    let rig = rig();
    let stopped = interrupts(&rig);
    let job = make(&rig, "j-over", 1.0, 20.0, |_| {});
    let run_id = rig.sched.fire(&job, slot(0)).await.run_id.unwrap();
    let started = rig.store.job_run_get(&run_id).unwrap().unwrap().started_at_ms + 1;

    rig.publish_turn(turn_event("b1", started, started, 0.4, None));
    rig.wait_for(&run_id, |r| r.turn_id.is_some()).await;
    assert!(stopped.lock().unwrap().is_empty(), "$0.40 of $1.00");
    rig.publish_turn(turn_event("b1", started, started, 1.2, None));
    let done = rig.wait_for(&run_id, |r| r.state == state::FINISHED).await;

    assert_eq!((done.infra.as_str(), done.reason.as_str()), (infra::OVER_BUDGET, reason::RUN_CAP));
    assert_eq!((done.cost_usd, done.cost_known), (1.2, true), "what it had spent is kept");
    assert!(done.summary.contains("spending cap"), "{}", done.summary);
    assert_eq!(*stopped.lock().unwrap(), ["b1"], "its turn was interrupted");
    assert!(crate::backend::jobs::verdict::run_failed(&done), "a failure for incidents");

    // The turn's own ending afterwards changes nothing.
    rig.publish_turn(turn_event("b1", started, started, 1.2, Some("completed")));
    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
    assert_eq!(rig.store.job_run_get(&run_id).unwrap().unwrap().infra, infra::OVER_BUDGET);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turn_that_ends_under_its_cap_finishes_normally() {
    let rig = rig();
    let stopped = interrupts(&rig);
    let job = make(&rig, "j-under", 1.0, 20.0, |_| {});
    let run_id = rig.sched.fire(&job, slot(0)).await.run_id.unwrap();
    let started = rig.store.job_run_get(&run_id).unwrap().unwrap().started_at_ms + 1;
    rig.publish_turn(turn_event("b1", started, started, 0.9, Some("completed")));
    let done = rig.wait_for(&run_id, |r| r.state == state::FINISHED).await;
    assert_eq!((done.infra.as_str(), done.cost_usd), (infra::OK, 0.9));
    assert!(stopped.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_spawned_worker_that_reaches_its_cap_is_stopped_even_if_the_job_keeps_workers() {
    let rig = rig();
    let stopped = interrupts(&rig);
    let stops: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = stops.clone();
    rig.ledger.install_worker_stop(Arc::new(move |block: &str| sink.lock().unwrap().push(block.to_string())));
    let job = make(&rig, "j-worker", 1.0, 20.0, |j| j.action_kind = "spawn".into());
    rig.sched.install_actions(Arc::new(|_, _| {
        Box::pin(async {
            ActionOutcome::Spawned {
                block_id: "b-worker".into(),
                def_id: "uid-worker".into(),
                slug: "worker-1".into(),
                brief: "do the thing".into(),
                keep: true,
            }
        })
    }));
    rig.answer(r#"{"success": true, "block_id": "b-worker"}"#);
    let run_id = rig.sched.fire(&job, slot(0)).await.run_id.unwrap();
    let started = rig.store.job_run_get(&run_id).unwrap().unwrap().started_at_ms + 1;

    rig.publish_turn(turn_event("b-worker", started, started, 1.5, None));
    let done = rig.wait_for(&run_id, |r| r.state == state::FINISHED).await;
    assert_eq!(done.infra, infra::OVER_BUDGET);
    assert_eq!(*stopped.lock().unwrap(), ["b-worker"]);
    assert_eq!(*stops.lock().unwrap(), ["b-worker"], "stopped, though keep_worker is set");
}

// ── messages to another agent ───────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn a_job_messaging_another_agent_is_limited_to_its_messages_a_day() {
    let rig = rig();
    let job = make(&rig, "j-msg", 1.0, 20.0, |j| j.policy.caps.as_mut().unwrap().messages_per_day = Some(2));
    for n in 0..2 {
        let run = rig.sched.fire(&job, slot(n)).await.run_id.expect("within the limit");
        spend(&rig, &run, 0.1);
    }
    assert_eq!(rig.sched.fire(&job, slot(2)).await.run_id, None);
    assert_eq!(rig.sched.fire(&job, slot(3)).await.run_id, None);
    let row = &rig.runs("j-msg")[0];
    assert_eq!((row.state.as_str(), row.reason.as_str(), row.skip_count), (state::SKIPPED, reason::MESSAGE_CAP, 2));
    assert_eq!(rig.ledger.cap_alerts(), [("j-msg".to_string(), "message_cap".to_string())]);
    assert_eq!(rig.ledger.over_cap_today(&job), Some("message_cap"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_job_messaging_its_owner_has_no_message_limit() {
    let rig = rig();
    let job = make(&rig, "j-self", 1.0, 20.0, |j| {
        j.created_by_uid = "uid-y".into();
        j.policy.caps.as_mut().unwrap().messages_per_day = Some(1);
    });
    for n in 0..3 {
        let run = rig.sched.fire(&job, slot(n)).await.run_id.expect("to its owner");
        spend(&rig, &run, 0.1);
    }
    assert!(rig.ledger.cap_alerts().is_empty());
}

// ── providers that report no cost ───────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn a_provider_with_no_cost_counts_zero_toward_the_dollar_caps() {
    let rig = rig();
    weigh(&rig, Limits::default(), "acct-x", "codex");
    // A $1 daily cap and a $5 run cap: a claude job couldn't start; this one weighs $0.
    let job = make(&rig, "j-nocost", 5.0, 1.0, |_| {});
    let run_id = rig.sched.fire(&job, slot(0)).await.run_id.expect("counts $0");
    assert_eq!(rig.store.job_run_get(&run_id).unwrap().unwrap().state, state::RUNNING);
    assert_eq!(rig.ledger.stop_at_usd(&run_id, "codex"), None, "there is no cost to stop it at: only the timeout limits it");
    assert_eq!(rig.ledger.stop_at_usd(&run_id, "claude"), Some(5.0));
    assert!(!crate::backend::jobs::caps::reports_cost("codex"));
    assert!(crate::backend::jobs::caps::reports_cost("claude"));
    assert!(crate::backend::jobs::caps::reports_cost("anthropic"), "an alias of claude");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_same_job_on_a_provider_that_reports_cost_is_held_back() {
    let rig = rig();
    weigh(&rig, Limits::default(), "acct-x", "claude");
    let job = make(&rig, "j-cost", 5.0, 1.0, |_| {});
    assert_eq!(rig.sched.fire(&job, slot(0)).await.run_id, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_task_spends_nothing_so_no_cap_holds_it() {
    let rig = rig();
    let job = make(&rig, "j-task", 0.0, 0.0, |j| j.action_kind = "task".into());
    rig.sched.install_actions(Arc::new(|_, _| Box::pin(async { ActionOutcome::Filed { work_id: "w".into(), num: 1 } })));
    let fired = rig.sched.fire(&job, slot(0)).await;
    assert!(matches!(fired.outcome, Some(FireOutcome::Delivered { .. })), "{fired:?}");
}

// ── a pending fire ──────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn a_pending_fire_a_cap_refuses_is_skipped_not_left_to_expire_as_a_miss() {
    let rig = rig();
    let job = make(&rig, "j-pend", 5.0, 5.0, |j| j.catch_up = "latest".into());
    let run = rig.sched.fire(&job, slot(0)).await.run_id.unwrap();
    spend(&rig, &run, 5.0);
    let mut pending = JobRun::new("p1", "j-pend", SOURCE_CRON, "n", TRIGGER_SCHEDULE, state::PENDING, agentmux_common::time::now_ms());
    pending.reason = reason::TARGET_STOPPED.to_string();
    pending.scheduled_for_ms = Some(SLOT0 + 600_000);
    pending.recorded_by = rig.ledger.recorded_by.clone();
    rig.store.job_run_insert(&pending).unwrap();
    assert!(rig.ledger.pending_refused_by_cap(&job, "p1"));
    let row = rig.store.job_run_get("p1").unwrap().unwrap();
    assert_eq!((row.state.as_str(), row.reason.as_str()), (state::SKIPPED, reason::JOB_CAP));
    assert!(!crate::backend::jobs::verdict::run_failed(&row), "a cap skip is not a failure");
}
