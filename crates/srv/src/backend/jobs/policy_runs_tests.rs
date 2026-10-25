// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! A job's policy at work through the scheduler and the ledger: overlap,
//! concurrency groups, the global limit, the timeout, retries, and the
//! `task` and `spawn` actions (SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.1, §8).

use super::*;
use crate::backend::cron::ActionOutcome;
use crate::backend::jobs::definition::{CatchUp, Overlap};
use crate::backend::storage::cron::CronJob;
use crate::backend::storage::job_runs::reason;

const SLOT0: i64 = 1_800_000_000_000;

fn slot(n: i64) -> Slot {
    Slot::Scheduled(SLOT0 + n * 60_000)
}

/// A job stored with `edit` applied.
fn make(rig: &Rig, id: &str, edit: impl FnOnce(&mut CronJob)) -> CronJob {
    let mut job = test_job(id, "check the build", "AgentY", "uid-y", "uid-creator");
    job.expression = "0 * * * *".to_string();
    edit(&mut job);
    rig.store.cron_create(&job).unwrap();
    job
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

fn end(rig: &Rig, run_id: &str) {
    rig.ledger.turn_finished(run_id, infra::OK, None, "");
}

// ── overlap ─────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn overlap_skip_skips_the_slot_and_refuses_run_now() {
    let rig = rig();
    let job = make(&rig, "j-skip", |_| {});
    let first = rig.sched.fire(&job, slot(0)).await.run_id.unwrap();
    assert_eq!(rig.store.job_run_get(&first).unwrap().unwrap().state, state::RUNNING);

    let second = rig.sched.fire(&job, slot(1)).await;
    assert_eq!((second.run_id, second.outcome), (None, None));
    assert_eq!(states(&rig, "j-skip"), [("running".into(), "".into()), ("skipped".into(), "overlap".into())]);

    let now = rig.sched.run_now(&job, "operator").await;
    assert!(matches!(now.outcome, Some(crate::backend::cron::FireOutcome::Failed { ref error }) if error == OVERLAP_REFUSAL));
    assert_eq!(rig.runs("j-skip").len(), 2, "Run now leaves no row of its own");

    end(&rig, &first);
    let third = rig.sched.fire(&job, slot(2)).await;
    assert!(third.run_id.is_some(), "free again");
}

#[tokio::test(flavor = "multi_thread")]
async fn overlap_buffer_one_keeps_one_slot_and_starts_it_when_the_run_ends() {
    let rig = rig();
    let job = make(&rig, "j-buf", |j| j.policy.overlap = Overlap::BufferOne);
    let first = rig.sched.fire(&job, slot(0)).await.run_id.unwrap();

    let buffered = rig.sched.fire(&job, slot(1)).await;
    assert_eq!(buffered.waiting, Some(reason::OVERLAP));
    let skipped = rig.sched.fire(&job, slot(2)).await;
    assert_eq!((skipped.run_id, skipped.waiting), (None, None), "one place in the buffer");
    assert_eq!(
        states(&rig, "j-buf"),
        [("running".into(), "".into()), ("scheduled".into(), "overlap".into()), ("skipped".into(), "overlap".into())]
    );

    assert_eq!(rig.sched.run_policy_pass().await, 0, "the first is still running");
    end(&rig, &first);
    assert_eq!(rig.sched.run_policy_pass().await, 1);
    let second = rig.store.job_run_get(buffered.run_id.as_ref().unwrap()).unwrap().unwrap();
    assert_eq!((second.state.as_str(), second.reason.as_str()), (state::RUNNING, ""));
    assert_eq!(rig.seen.lock().unwrap().len(), 2, "delivered once each");
    assert!(
        rig.seen.lock().unwrap()[1].message.contains("(late)"),
        "a run that waited says when it was due"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn overlap_cancel_other_interrupts_the_old_run_and_starts_the_new_one() {
    let rig = rig();
    let stopped = interrupts(&rig);
    let job = make(&rig, "j-cancel", |j| j.policy.overlap = Overlap::CancelOther);
    let first = rig.sched.fire(&job, slot(0)).await.run_id.unwrap();
    let second = rig.sched.fire(&job, slot(1)).await.run_id.unwrap();

    let old = rig.store.job_run_get(&first).unwrap().unwrap();
    assert_eq!((old.state.as_str(), old.infra.as_str(), old.reason.as_str()), (state::FINISHED, infra::INTERRUPTED, reason::OVERLAP));
    assert_eq!(*stopped.lock().unwrap(), ["b1"], "the old run's turn was interrupted");
    assert_eq!(rig.store.job_run_get(&second).unwrap().unwrap().state, state::RUNNING);
}

/// A run stopped on purpose is closed already: its turn ending later files no
/// retry of it, however many retries the job has.
#[tokio::test(flavor = "multi_thread")]
async fn a_run_cancelled_on_purpose_is_not_retried_when_its_turn_ends() {
    let rig = rig();
    let _stopped = interrupts(&rig);
    let job = make(&rig, "j-cancel-retry", |j| {
        j.policy.overlap = Overlap::CancelOther;
        j.policy.retries = 2;
    });
    let first = rig.sched.fire(&job, slot(0)).await.run_id.unwrap();
    rig.sched.fire(&job, slot(1)).await.run_id.unwrap();

    rig.ledger.turn_finished(&first, infra::INTERRUPTED, None, "interrupted");
    rig.ledger.turn_finished(&first, infra::INTERRUPTED, None, "interrupted");
    let runs = rig.runs("j-cancel-retry");
    assert_eq!(runs.len(), 2, "no retry row: {:?}", states(&rig, "j-cancel-retry"));
    assert!(runs.iter().all(|r| r.links["retried_by"].is_null()));
}

/// The same retry is filed once, however often the end of its turn is seen.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_run_is_retried_once() {
    let rig = rig();
    let job = make(&rig, "j-once-retry", |j| j.policy.retries = 3);
    let first = rig.sched.fire(&job, slot(0)).await.run_id.unwrap();
    rig.ledger.turn_finished(&first, infra::INTERRUPTED, None, "process exited");
    rig.ledger.maybe_retry(&first);
    assert_eq!(rig.runs("j-once-retry").len(), 2, "{:?}", states(&rig, "j-once-retry"));
}

/// A job whose schedule is done (a one-time job after its time) is disabled,
/// not paused: its last run, if it was waiting for a place, still starts.
#[tokio::test(flavor = "multi_thread")]
async fn a_waiting_run_of_a_finished_one_time_job_still_starts() {
    let rig = rig();
    let a = make(&rig, "j-once-w1", |j| j.policy.group = Some("g".into()));
    let b = make(&rig, "j-once-w2", |j| {
        j.policy.group = Some("g".into());
        j.trigger_kind = "once".into();
    });
    let busy = rig.sched.fire(&a, slot(0)).await.run_id.unwrap();
    let waits = rig.sched.fire(&b, slot(0)).await.run_id.unwrap();
    rig.store.cron_set_enabled("j-once-w2", false).unwrap();
    end(&rig, &busy);
    rig.sched.run_policy_pass().await;
    let run = rig.store.job_run_get(&waits).unwrap().unwrap();
    assert_eq!(run.state, state::RUNNING, "it started: {:?}", states(&rig, "j-once-w2"));
}

/// A job whose run waits for its group doesn't queue a second one for the next
/// slot: skip skips it, buffer_one keeps the one it has.
#[tokio::test(flavor = "multi_thread")]
async fn a_further_slot_of_a_job_already_waiting_for_a_place_is_skipped() {
    let rig = rig();
    let blocker = make(&rig, "j-block-a", |j| j.policy.group = Some("g".into()));
    let a = make(&rig, "j-block-b", |j| j.policy.group = Some("g".into()));
    let buf = make(&rig, "j-block-c", |j| {
        j.policy.group = Some("g".into());
        j.policy.overlap = Overlap::BufferOne;
    });
    rig.sched.fire(&blocker, slot(0)).await;
    for job in [&a, &buf] {
        rig.sched.fire(job, slot(0)).await.run_id.unwrap();
        let again = rig.sched.fire(job, slot(1)).await;
        assert_eq!((again.run_id, again.waiting), (None, None), "{}: the second slot is skipped", job.id);
        assert_eq!(rig.runs(&job.id).len(), 2, "{}: one waiting run and one skipped row", job.id);
    }
}

/// Run now on a paused job still gets its place when it has to wait.
#[tokio::test(flavor = "multi_thread")]
async fn a_run_now_waiting_for_a_place_on_a_paused_job_still_starts() {
    let rig = rig();
    let busy = make(&rig, "j-rn-a", |j| j.policy.group = Some("g".into()));
    let paused = make(&rig, "j-rn-b", |j| {
        j.policy.group = Some("g".into());
        j.enabled = false;
    });
    let first = rig.sched.fire(&busy, slot(0)).await.run_id.unwrap();
    let waits = rig.sched.run_now(&paused, "operator").await.run_id.unwrap();
    end(&rig, &first);
    rig.sched.run_policy_pass().await;
    assert_eq!(rig.store.job_run_get(&waits).unwrap().unwrap().state, state::RUNNING, "{:?}", states(&rig, "j-rn-b"));
}

/// A restart ends the runs that were running, not the ones still waiting for
/// a place or out their retry's backoff: those start afterwards.
#[tokio::test(flavor = "multi_thread")]
async fn a_restart_leaves_waiting_runs_waiting() {
    let rig = rig();
    let busy = make(&rig, "j-rs-a", |j| j.policy.group = Some("g".into()));
    let waiter = make(&rig, "j-rs-b", |j| j.policy.group = Some("g".into()));
    let running = rig.sched.fire(&busy, slot(0)).await.run_id.unwrap();
    let waits = rig.sched.fire(&waiter, slot(0)).await.run_id.unwrap();

    rig.ledger.recover_after_restart();
    let ended = rig.store.job_run_get(&running).unwrap().unwrap();
    assert_eq!((ended.state.as_str(), ended.infra.as_str()), (state::FINISHED, infra::INTERRUPTED));
    let still = rig.store.job_run_get(&waits).unwrap().unwrap();
    assert_eq!((still.state.as_str(), still.reason.as_str()), (state::SCHEDULED, reason::GROUP_BUSY));

    rig.sched.run_policy_pass().await;
    assert_eq!(rig.store.job_run_get(&waits).unwrap().unwrap().state, state::RUNNING, "it starts once the group is free");
}

/// A run the job cancels doesn't hold the job's group against its replacement.
#[tokio::test(flavor = "multi_thread")]
async fn cancel_other_works_inside_a_concurrency_group() {
    let rig = rig();
    let stopped = interrupts(&rig);
    let job = make(&rig, "j-cancel-g", |j| {
        j.policy.overlap = Overlap::CancelOther;
        j.policy.group = Some("g".into());
    });
    let first = rig.sched.fire(&job, slot(0)).await.run_id.unwrap();
    let second = rig.sched.fire(&job, slot(1)).await.run_id.unwrap();
    let old = rig.store.job_run_get(&first).unwrap().unwrap();
    assert_eq!((old.state.as_str(), old.infra.as_str()), (state::FINISHED, infra::INTERRUPTED));
    assert_eq!(stopped.lock().unwrap().len(), 1);
    assert_eq!(rig.store.job_run_get(&second).unwrap().unwrap().state, state::RUNNING);
}

/// Pending catch-up deliveries wait for their place like any other run: two
/// jobs of one group don't both start when their agents come back.
#[tokio::test(flavor = "multi_thread")]
async fn pending_deliveries_respect_the_concurrency_group() {
    let rig = rig();
    let a = make(&rig, "j-pend-a", |j| {
        j.policy.group = Some("g".into());
        j.catch_up = "latest".into();
    });
    let b = make(&rig, "j-pend-b", |j| {
        j.policy.group = Some("g".into());
        j.catch_up = "latest".into();
    });
    rig.answer(NOT_FOUND);
    rig.sched.fire(&a, slot(0)).await;
    rig.sched.fire(&b, slot(0)).await;

    *rig.present.lock().unwrap() = true;
    rig.answer(DELIVERED);
    assert_eq!(rig.sched.deliver_pending().await, 1, "one of the two starts");
    assert_eq!(rig.sched.deliver_pending().await, 0, "the other waits for the group");
}

/// The ledger is shared by every instance, the jobs store is not: a waiting
/// run another instance recorded is left alone, not closed as a deleted job.
#[tokio::test(flavor = "multi_thread")]
async fn a_waiting_run_another_instance_recorded_is_left_alone() {
    let rig = rig();
    let a = make(&rig, "j-foreign-a", |j| j.policy.group = Some("g".into()));
    let b = make(&rig, "j-foreign-b", |j| j.policy.group = Some("g".into()));
    rig.sched.fire(&a, slot(0)).await;
    let waits = rig.sched.fire(&b, slot(0)).await.run_id.unwrap();
    rig.store.conn().lock().unwrap().execute("UPDATE db_job_runs SET recorded_by = 'other@elsewhere' WHERE id = ?1", [&waits]).unwrap();
    rig.store.cron_delete("j-foreign-b").unwrap();
    rig.sched.run_policy_pass().await;
    let run = rig.store.job_run_get(&waits).unwrap().unwrap();
    assert_eq!((run.state.as_str(), run.reason.as_str()), (state::SCHEDULED, reason::GROUP_BUSY));
}

/// A run another channel's instance follows can't be interrupted from here, so
/// a timeout or a replacement doesn't close its row and free its place.
#[tokio::test(flavor = "multi_thread")]
async fn a_run_followed_by_another_channel_is_not_stopped_from_here() {
    let rig = rig();
    let stopped = interrupts(&rig);
    let job = make(&rig, "j-remote", |j| {
        j.policy.overlap = Overlap::CancelOther;
        j.policy.timeout_minutes = Some(1);
    });
    let first = rig.sched.fire(&job, slot(0)).await.run_id.unwrap();
    rig.store.job_run_add_links(&first, &json!({ "followed_by_channel": "elsewhere", "followed_by_pid": 1 })).unwrap();
    rig.store.conn().lock().unwrap().execute("UPDATE db_job_runs SET started_at_ms = 1 WHERE id = ?1", [&first]).unwrap();

    assert_eq!(rig.ledger.enforce_timeouts(), 0, "past its timeout, but not ours to stop");
    let second = rig.sched.fire(&job, slot(1)).await;
    assert_eq!(second.run_id, None, "cancel_other can't replace it: the slot is skipped");
    assert!(stopped.lock().unwrap().is_empty());
    assert_eq!(rig.store.job_run_get(&first).unwrap().unwrap().state, state::RUNNING);
}

/// A pending fire told to start holds its place while it is delivered, so a
/// fire that lands in between waits instead of taking the same place.
#[tokio::test(flavor = "multi_thread")]
async fn a_pending_fire_being_delivered_holds_its_place() {
    let rig = rig();
    let a = make(&rig, "j-claim-a", |j| {
        j.policy.group = Some("g".into());
        j.catch_up = "latest".into();
    });
    let b = make(&rig, "j-claim-b", |j| j.policy.group = Some("g".into()));
    rig.answer(NOT_FOUND);
    let pending = rig.sched.fire(&a, slot(0)).await.run_id.unwrap();

    assert!(rig.ledger.pending_may_start(&a, &pending), "the group is free");
    let fired = rig.sched.fire(&b, slot(0)).await;
    assert_eq!(fired.waiting, Some(reason::GROUP_BUSY), "its place is taken while it is delivered");
    rig.ledger.pending_delivery_done(&pending);
    assert!(rig.ledger.pending_may_start(&a, &pending), "released once the delivery is over");
}

/// A retry re-delivers a fire that was already counted: it doesn't count again,
/// so a job with `max_fires` and retries isn't stopped early.
#[tokio::test(flavor = "multi_thread")]
async fn a_retry_does_not_count_as_another_fire() {
    let rig = rig();
    let job = make(&rig, "j-retry-count", |j| j.policy.retries = 1);
    let first = rig.sched.fire(&job, slot(0)).await.run_id.unwrap();
    assert_eq!(rig.fire_count("j-retry-count"), 1);
    rig.ledger.turn_finished(&first, infra::INTERRUPTED, None, "process exited");
    let retry = rig.runs("j-retry-count").into_iter().find(|r| r.trigger == TRIGGER_RETRY).expect("a retry was filed");
    make_due(&rig, &retry.id);
    assert_eq!(rig.sched.run_policy_pass().await, 1);
    assert_eq!(rig.fire_count("j-retry-count"), 1, "the retry is not a second fire");
}

// ── groups and the global limit ─────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn a_concurrency_group_runs_one_at_a_time() {
    let rig = rig();
    let a = make(&rig, "j-g1", |j| j.policy.group = Some("builds".into()));
    let b = make(&rig, "j-g2", |j| j.policy.group = Some("builds".into()));
    let other = make(&rig, "j-g3", |_| {});
    let first = rig.sched.fire(&a, slot(0)).await.run_id.unwrap();

    let waits = rig.sched.fire(&b, slot(0)).await;
    assert_eq!(waits.waiting, Some(reason::GROUP_BUSY));
    assert!(rig.sched.fire(&other, slot(0)).await.waiting.is_none(), "another group isn't held");
    assert_eq!(rig.sched.run_policy_pass().await, 0);

    end(&rig, &first);
    assert_eq!(rig.sched.run_policy_pass().await, 1);
    let b_run = rig.store.job_run_get(waits.run_id.as_ref().unwrap()).unwrap().unwrap();
    assert_eq!(b_run.state, state::RUNNING);
}

/// Deploys are one built-in group: a job that declares a deploy output is in
/// it without saying so.
#[tokio::test(flavor = "multi_thread")]
async fn deploys_run_one_at_a_time() {
    let rig = rig();
    let a = make(&rig, "j-d1", |j| j.outputs = vec!["deploy:staging".into()]);
    let b = make(&rig, "j-d2", |j| j.policy.group = Some("deploy".into()));
    let first = rig.sched.fire(&a, slot(0)).await.run_id.unwrap();
    let second = rig.sched.fire(&b, slot(0)).await;
    assert_eq!(second.waiting, Some(reason::GROUP_BUSY));
    end(&rig, &first);
    assert_eq!(rig.sched.run_policy_pass().await, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn at_most_three_unattended_agent_runs_at_once() {
    let rig = rig();
    let jobs: Vec<CronJob> = (0..4).map(|i| make(&rig, &format!("j-lim{i}"), |_| {})).collect();
    let task = make(&rig, "j-task", |j| j.action_kind = "task".into());
    let mut running = Vec::new();
    for job in &jobs[..3] {
        running.push(rig.sched.fire(job, slot(0)).await.run_id.unwrap());
    }
    let fourth = rig.sched.fire(&jobs[3], slot(0)).await;
    assert_eq!(fourth.waiting, Some(reason::GLOBAL_LIMIT));
    assert_eq!(rig.sched.run_policy_pass().await, 0);

    // A task starts no agent: the limit doesn't hold it.
    rig.sched.install_actions(Arc::new(|_, _| {
        Box::pin(async { ActionOutcome::Filed { work_id: "w".into(), num: 1 } })
    }));
    assert!(rig.sched.fire(&task, slot(0)).await.waiting.is_none());

    end(&rig, &running[0]);
    assert_eq!(rig.sched.run_policy_pass().await, 1);
    let started = rig.store.job_run_get(fourth.run_id.as_ref().unwrap()).unwrap().unwrap();
    assert_eq!(started.state, state::RUNNING);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_waiting_run_of_a_paused_job_ends_as_paused() {
    let rig = rig();
    let a = make(&rig, "j-w1", |j| j.policy.group = Some("g".into()));
    let b = make(&rig, "j-w2", |j| j.policy.group = Some("g".into()));
    rig.sched.fire(&a, slot(0)).await;
    let waits = rig.sched.fire(&b, slot(0)).await.run_id.unwrap();
    rig.store.cron_set_enabled("j-w2", false).unwrap();
    rig.sched.run_policy_pass().await;
    let run = rig.store.job_run_get(&waits).unwrap().unwrap();
    assert_eq!((run.state.as_str(), run.reason.as_str()), (state::SKIPPED, reason::PAUSED));
}

// ── timeout ─────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn a_run_past_its_timeout_is_interrupted_and_ends_with_infra_timeout() {
    let rig = rig();
    let stopped = interrupts(&rig);
    let timed = make(&rig, "j-to", |j| j.policy.timeout_minutes = Some(30));
    let unlimited = make(&rig, "j-nolimit", |_| {});
    let young = make(&rig, "j-young", |j| j.policy.timeout_minutes = Some(30));
    let run = rig.sched.fire(&timed, slot(0)).await.run_id.unwrap();
    let no_limit = rig.sched.fire(&unlimited, slot(0)).await.run_id.unwrap();
    let recent = rig.sched.fire(&young, slot(0)).await.run_id.unwrap();
    let long_ago = agentmux_common::time::now_ms() - 31 * 60_000;
    for id in [&run, &no_limit] {
        rig.store
            .conn()
            .lock()
            .unwrap()
            .execute("UPDATE db_job_runs SET started_at_ms = ?2 WHERE id = ?1", rusqlite::params![id, long_ago])
            .unwrap();
    }

    assert_eq!(rig.ledger.enforce_timeouts(), 1);
    let done = rig.store.job_run_get(&run).unwrap().unwrap();
    assert_eq!((done.state.as_str(), done.infra.as_str(), done.reason.as_str()), (state::FINISHED, infra::TIMEOUT, reason::TIMEOUT));
    assert!(done.summary.contains("30 minutes"), "{}", done.summary);
    assert_eq!(*stopped.lock().unwrap(), ["b1"]);
    assert_eq!(rig.store.job_run_get(&no_limit).unwrap().unwrap().state, state::RUNNING);
    assert_eq!(rig.store.job_run_get(&recent).unwrap().unwrap().state, state::RUNNING);
    assert_eq!(rig.ledger.enforce_timeouts(), 0, "a stopped run isn't stopped twice");
    assert!(!super::policy::infra_retryable(&done), "its own timeout isn't retried");
}

// ── retries ─────────────────────────────────────────────────────────────

const RATE_LIMITED: &str = r#"{"success": false, "error": "Rate limited by the API (429)"}"#;

fn make_due(rig: &Rig, run_id: &str) {
    rig.store.job_run_add_links(run_id, &json!({ "retry_at_ms": 0 })).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_infrastructure_failure_is_retried_with_backoff_up_to_the_limit() {
    let rig = rig();
    let job = make(&rig, "j-retry", |j| j.policy.retries = 2);
    rig.answer(RATE_LIMITED);
    let first = rig.sched.fire(&job, slot(0)).await.run_id.unwrap();

    let runs = rig.runs("j-retry");
    assert_eq!(runs.len(), 2);
    let retry = runs.iter().find(|r| r.trigger == TRIGGER_RETRY).expect("a retry was filed");
    assert_eq!((retry.attempt, retry.state.as_str(), retry.reason.as_str()), (2, state::SCHEDULED, reason::RETRY_WAIT));
    let wait = retry.links["retry_at_ms"].as_i64().unwrap() - agentmux_common::time::now_ms();
    assert!((50_000..=60_000).contains(&wait), "the first retry waits a minute: {wait}");
    let failed = rig.store.job_run_get(&first).unwrap().unwrap();
    assert_eq!(failed.links["retried_by"], retry.id.as_str());
    assert!(rig.store.job_run_last_decisive("j-retry").unwrap().is_none(), "a failure with a retry coming is no incident yet");

    assert_eq!(rig.sched.run_policy_pass().await, 0, "still waiting out the backoff");
    make_due(&rig, &retry.id);
    assert_eq!(rig.sched.run_policy_pass().await, 1);
    let again = rig.store.job_run_get(&retry.id).unwrap().unwrap();
    assert_eq!((again.state.as_str(), again.infra.as_str()), (state::FINISHED, infra::ERROR), "failed again");
    let third = rig.runs("j-retry").into_iter().find(|r| r.attempt == 3).expect("the second retry");
    let wait = third.links["retry_at_ms"].as_i64().unwrap() - agentmux_common::time::now_ms();
    assert!((4 * 60_000..=5 * 60_000).contains(&wait), "then five minutes: {wait}");

    make_due(&rig, &third.id);
    rig.sched.run_policy_pass().await;
    assert_eq!(rig.runs("j-retry").len(), 3, "two retries at most: the third attempt's failure is final");
    assert!(rig.store.job_run_last_decisive("j-retry").unwrap().is_some_and(|r| r.id == third.id));
}

#[tokio::test(flavor = "multi_thread")]
async fn retries_are_for_infrastructure_failures_only() {
    let rig = rig();
    let job = make(&rig, "j-noretry", |j| j.policy.retries = 3);
    // Not transient: nothing to retry.
    rig.answer(r#"{"success": false, "error": "invalid argument in the message"}"#);
    rig.sched.fire(&job, slot(0)).await;
    assert_eq!(rig.runs("j-noretry").len(), 1);

    // The agent concluded the run failed: it alerts, it isn't retried.
    rig.answer(DELIVERED);
    let run = rig.sched.fire(&job, slot(1)).await.run_id.unwrap();
    rig.store.job_run_report(&run, "failed", "the build is red", &json!(null)).unwrap();
    rig.ledger.turn_finished(&run, infra::INTERRUPTED, None, "");
    assert_eq!(rig.runs("j-noretry").len(), 2, "outcome failed is never retried");

    // A turn that ended and a process that exited: only the exit is.
    let done = rig.sched.fire(&job, slot(2)).await.run_id.unwrap();
    end(&rig, &done);
    assert_eq!(rig.runs("j-noretry").len(), 3);
    let exited = rig.sched.fire(&job, slot(3)).await.run_id.unwrap();
    rig.ledger.turn_finished(&exited, infra::INTERRUPTED, None, "");
    assert_eq!(rig.runs("j-noretry").len(), 5, "the exit and its retry");

    // A job with no retries files none.
    let plain = make(&rig, "j-plain", |_| {});
    rig.answer(RATE_LIMITED);
    rig.sched.fire(&plain, slot(0)).await;
    assert_eq!(rig.runs("j-plain").len(), 1);
}

// ── actions ─────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn a_task_job_files_its_task_and_the_run_is_ok_at_once() {
    let rig = rig();
    let job = make(&rig, "j-task", |j| {
        j.action_kind = "task".into();
        j.action = json!({ "kind": "task", "title": "Triage the board" });
    });
    let seen: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let sink = seen.clone();
    rig.sched.install_actions(Arc::new(move |job, run_id| {
        sink.lock().unwrap().push((job.id.clone(), run_id));
        Box::pin(async { ActionOutcome::Filed { work_id: "work-1".into(), num: 17 } })
    }));
    let fired = rig.sched.fire(&job, slot(0)).await;
    let run = rig.store.job_run_get(fired.run_id.as_ref().unwrap()).unwrap().unwrap();
    assert_eq!((run.state.as_str(), run.infra.as_str(), run.outcome.as_str()), (state::FINISHED, infra::OK, "ok"));
    assert_eq!((run.links["kanban_task"].as_i64(), run.links["work_id"].as_str()), (Some(17), Some("work-1")));
    assert_eq!(*seen.lock().unwrap(), [("j-task".to_string(), run.id.clone())]);
    assert!(rig.seen.lock().unwrap().is_empty(), "nothing is sent to an agent");
    assert_eq!(rig.fire_count("j-task"), 1, "a filed task is a delivered fire");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_task_that_could_not_be_filed_fails_the_run() {
    let rig = rig();
    let job = make(&rig, "j-badtask", |j| j.action_kind = "task".into());
    rig.sched.install_actions(Arc::new(|_, _| Box::pin(async { ActionOutcome::Failed("the queue is full".into()) })));
    let fired = rig.sched.fire(&job, slot(0)).await;
    let run = rig.store.job_run_get(fired.run_id.as_ref().unwrap()).unwrap().unwrap();
    assert_eq!((run.state.as_str(), run.infra.as_str()), (state::FINISHED, infra::ERROR));
    assert!(run.summary.contains("queue is full"));
}

fn spawned(keep: bool) -> ActionOutcome {
    ActionOutcome::Spawned {
        block_id: "b-worker".into(),
        def_id: "uid-worker".into(),
        slug: "worker-1".into(),
        brief: "do the thing".into(),
        keep,
    }
}

fn worker_stops(rig: &Rig) -> Arc<Mutex<Vec<String>>> {
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = seen.clone();
    rig.ledger.install_worker_stop(Arc::new(move |block: &str| sink.lock().unwrap().push(block.to_string())));
    seen
}

/// The worker gets the brief, the run follows the worker's pane, and when its
/// first turn ends the run does and the worker is stopped (unless kept).
#[tokio::test(flavor = "multi_thread")]
async fn a_spawn_job_briefs_the_worker_and_ends_with_its_first_turn() {
    let rig = rig();
    let stops = worker_stops(&rig);
    let job = make(&rig, "j-spawn", |j| j.action_kind = "spawn".into());
    rig.sched.install_actions(Arc::new(|_, _| Box::pin(async { spawned(false) })));
    rig.answer(r#"{"success": true, "block_id": "b-worker"}"#);
    let run_id = rig.sched.fire(&job, slot(0)).await.run_id.unwrap();

    let req = rig.seen.lock().unwrap()[0].clone();
    assert_eq!((req.target_agent.as_str(), req.request_id.as_deref()), ("uid-worker", Some(run_id.as_str())));
    assert!(req.message.starts_with("do the thing"), "{}", req.message);
    let run = rig.store.job_run_get(&run_id).unwrap().unwrap();
    assert_eq!((run.state.as_str(), run.block_id.as_str(), run.target_agent_uid.as_str()), (state::RUNNING, "b-worker", "uid-worker"));
    assert_eq!(run.links["worker"]["block_id"], "b-worker");
    assert!(stops.lock().unwrap().is_empty());

    rig.ledger.turn_finished(&run_id, infra::OK, None, "");
    assert_eq!(*stops.lock().unwrap(), ["b-worker"]);
    assert_eq!(rig.store.job_run_get(&run_id).unwrap().unwrap().state, state::FINISHED);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_kept_worker_is_not_stopped() {
    let rig = rig();
    let stops = worker_stops(&rig);
    let job = make(&rig, "j-keep", |j| j.action_kind = "spawn".into());
    rig.sched.install_actions(Arc::new(|_, _| Box::pin(async { spawned(true) })));
    rig.answer(r#"{"success": true, "block_id": "b-worker"}"#);
    let kept = rig.sched.fire(&job, slot(0)).await.run_id.unwrap();
    rig.ledger.turn_finished(&kept, infra::OK, None, "");
    assert!(stops.lock().unwrap().is_empty(), "keep_worker");
}

/// The worker opened but nobody answers to its name: the run fails, with no
/// worker left running.
#[tokio::test(flavor = "multi_thread")]
async fn a_worker_that_took_no_brief_fails_the_run_and_is_stopped() {
    let rig = rig();
    let stops = worker_stops(&rig);
    let job = make(&rig, "j-nobrief", |j| j.action_kind = "spawn".into());
    rig.sched.install_actions(Arc::new(|_, _| Box::pin(async { spawned(false) })));
    rig.answer(NOT_FOUND);
    let fired = rig.sched.fire(&job, slot(0)).await;
    let run = rig.store.job_run_get(fired.run_id.as_ref().unwrap()).unwrap().unwrap();
    assert_eq!((run.state.as_str(), run.infra.as_str()), (state::FINISHED, infra::ERROR));
    assert_eq!(*stops.lock().unwrap(), ["b-worker"]);
}

/// A spawn that can't open (the leader isn't running, too many workers)
/// fails the run before anything is delivered.
#[tokio::test(flavor = "multi_thread")]
async fn a_spawn_that_cannot_open_fails_the_run() {
    let rig = rig();
    let job = make(&rig, "j-nospawn", |j| j.action_kind = "spawn".into());
    rig.sched.install_actions(Arc::new(|_, _| Box::pin(async { ActionOutcome::Failed("TOO_MANY_WORKERS".into()) })));
    let fired = rig.sched.fire(&job, slot(0)).await;
    let run = rig.store.job_run_get(fired.run_id.as_ref().unwrap()).unwrap().unwrap();
    assert_eq!((run.state.as_str(), run.infra.as_str()), (state::FINISHED, infra::ERROR));
    assert!(rig.seen.lock().unwrap().is_empty());
}

/// A new job skips an overlapping slot and a missed one, and retries nothing.
#[test]
fn the_policy_defaults_are_skip_everywhere() {
    let job = test_job("j", "p", "t", "", "");
    assert_eq!((job.policy.overlap, job.policy.catch_up, job.policy.retries), (Overlap::Skip, CatchUp::Skip, 0));
}

/// A one-time job fires at its time and is then done: disabled, not fired again.
#[tokio::test(flavor = "multi_thread")]
async fn a_one_time_job_fires_once_and_is_then_disabled() {
    let rig = rig();
    let at = agentmux_common::time::now_ms() + 300;
    let job = make(&rig, "j-once", |j| {
        j.expression = String::new();
        j.trigger_kind = "once".into();
        j.once_at_ms = Some(at);
    });
    rig.sched.schedule_job(&job);
    for _ in 0..200 {
        if !rig.store.cron_get("j-once").unwrap().unwrap().enabled {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let stored = rig.store.cron_get("j-once").unwrap().unwrap();
    assert!(!stored.enabled, "done");
    assert_eq!(rig.runs("j-once").len(), 1, "fired once");
    assert_eq!(rig.runs("j-once")[0].scheduled_for_ms, Some(at));
    assert_eq!(rig.fire_count("j-once"), 1);
}
