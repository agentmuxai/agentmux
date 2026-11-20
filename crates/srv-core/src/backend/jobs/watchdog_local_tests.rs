// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The watchdog's local half (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §4.4): a local slot that passed with
//! no row of any kind is a `missed` row.

use super::*;
use crate::backend::cron::test_job;
use crate::backend::mps::Broker;

const MIN: i64 = 60_000;

struct Rig {
    _dir: tempfile::TempDir,
    store: Arc<Store>,
    ledger: Arc<RunLedger>,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap());
    let ledger = RunLedger::new(store.clone(), None, Arc::new(Broker::new()));
    Rig { _dir: dir, store, ledger }
}

/// A job whose slots are every minute (`test_job`'s own expression).
fn every_minute(id: &str) -> CronJob {
    let job = test_job(id, "p", "t", "uid-t", "uid-c");
    assert_eq!(JobSchedule::of(&job).unwrap().next_after(0), Some(MIN), "the test job is due every minute");
    job
}

fn rows(rig: &Rig, id: &str) -> Vec<JobRun> {
    let mut all = rig.store.job_runs_for_job(id, 500, None).unwrap();
    all.sort_by_key(|r| r.scheduled_for_ms);
    all
}

/// A row the scheduler wrote for `slot`, in `state`.
fn handled(rig: &Rig, id: &str, slot: i64, state: &str) {
    let mut run = JobRun::new(&format!("sched:{id}:{slot}"), id, SOURCE_CRON, "Job", TRIGGER_SCHEDULE, state, slot);
    run.scheduled_for_ms = Some(slot);
    assert!(rig.store.job_run_insert(&run).unwrap());
}

#[test]
fn a_slot_with_no_row_after_its_grace_is_a_missed_row_once() {
    let rig = rig();
    let job = every_minute("j");
    let mut watch = LocalWatch::new(0);
    handled(&rig, "j", 3 * MIN, "finished");
    handled(&rig, "j", 4 * MIN, "skipped");

    // At 15 minutes the slots up to 10 minutes are past their grace (5 minutes): ten slots, two of them handled.
    assert_eq!(rig.ledger.watch_local(&mut watch, std::slice::from_ref(&job), 15 * MIN), 8);
    let all = rows(&rig, "j");
    assert_eq!(all.len(), 10);
    let missed: Vec<_> = all.iter().filter(|r| r.state == "missed").collect();
    assert_eq!(missed.len(), 8);
    let r = missed[0];
    assert_eq!((r.reason.as_str(), r.job_source.as_str(), r.trigger.as_str(), r.triggered_by.as_str()), ("not_started", "cron", "schedule", "watchdog"));
    assert_eq!((r.scheduled_for_ms, r.started_at_ms, r.ended_at_ms), (Some(MIN), MIN, Some(15 * MIN)));
    assert_eq!(r.id, format!("watchdog:j:{MIN}"));
    assert_eq!(r.target_agent_uid, "uid-t");
    for handled_slot in [3 * MIN, 4 * MIN] {
        assert_ne!(all.iter().find(|r| r.scheduled_for_ms == Some(handled_slot)).unwrap().state, "missed");
    }

    // The same pass again changes nothing, and a later one judges only what is new.
    assert_eq!(rig.ledger.watch_local(&mut watch, std::slice::from_ref(&job), 15 * MIN), 0);
    assert_eq!(rig.ledger.watch_local(&mut watch, std::slice::from_ref(&job), 16 * MIN), 1);
    assert_eq!(rows(&rig, "j").len(), 11);
}

#[test]
fn a_slot_inside_its_grace_is_not_judged() {
    let rig = rig();
    let job = every_minute("j");
    let mut watch = LocalWatch::new(0);
    // 7 minutes: the grace is 5, so only the slots at 1 and 2 minutes are past it.
    assert_eq!(rig.ledger.watch_local(&mut watch, std::slice::from_ref(&job), 7 * MIN), 2);
}

#[test]
fn a_row_the_scheduler_writes_late_wins_the_slot() {
    let rig = rig();
    let job = every_minute("j");
    let mut watch = LocalWatch::new(0);
    assert_eq!(rig.ledger.watch_local(&mut watch, std::slice::from_ref(&job), 7 * MIN), 2);
    // A scheduler that wakes late finds the slot taken by the watchdog's row: the slot's unique index decides.
    let mut late = JobRun::new("sched:j:late", "j", SOURCE_CRON, "Job", TRIGGER_SCHEDULE, "scheduled", 2 * MIN);
    late.scheduled_for_ms = Some(2 * MIN);
    assert!(!rig.store.job_run_insert(&late).unwrap());
    // And the other way round: a slot with its row never gets a miss.
    let rig = self::rig();
    let mut watch = LocalWatch::new(0);
    handled(&rig, "j", MIN, "running");
    handled(&rig, "j", 2 * MIN, "scheduled");
    assert_eq!(rig.ledger.watch_local(&mut watch, std::slice::from_ref(&job), 7 * MIN), 0);
}

#[test]
fn slots_from_before_the_watchdog_started_are_not_its_to_judge() {
    let rig = rig();
    let job = every_minute("j");
    // Started at 20 minutes: the first pass at 30 judges the slots from 21 to 25 minutes only.
    let mut watch = LocalWatch::new(20 * MIN);
    assert_eq!(rig.ledger.watch_local(&mut watch, std::slice::from_ref(&job), 30 * MIN), 5);
    assert_eq!(rows(&rig, "j")[0].scheduled_for_ms, Some(21 * MIN));
}

#[test]
fn a_job_that_appears_after_the_first_pass_counts_from_then() {
    let rig = rig();
    let early = every_minute("early");
    let late = every_minute("late");
    let mut watch = LocalWatch::new(0);
    assert_eq!(rig.ledger.watch_local(&mut watch, std::slice::from_ref(&early), 10 * MIN), 5);
    // `late` is first seen at 15 minutes: its slots before that were nobody's to watch. `early` goes on from 5 to 10.
    assert_eq!(rig.ledger.watch_local(&mut watch, &[early.clone(), late.clone()], 15 * MIN), 5);
    assert!(rows(&rig, "late").is_empty());
    // At 21 minutes: `early`'s slots from 11 to 16, and `late`'s one at 16.
    assert_eq!(rig.ledger.watch_local(&mut watch, &[early, late], 21 * MIN), 7);
    assert_eq!(rows(&rig, "late").len(), 1);
}

#[test]
fn an_edited_schedule_is_judged_from_the_edit() {
    let rig = rig();
    let mut job = every_minute("j");
    let mut watch = LocalWatch::new(0);
    assert_eq!(rig.ledger.watch_local(&mut watch, std::slice::from_ref(&job), 10 * MIN), 5);
    // Edited to every second minute at 15: the slots of either schedule before that were not the new one's to start.
    job.expression = "*/2 * * * *".to_string();
    assert_eq!(rig.ledger.watch_local(&mut watch, std::slice::from_ref(&job), 15 * MIN), 0);
    // 21 minutes: the new schedule's slot at 16 is past its grace.
    assert_eq!(rig.ledger.watch_local(&mut watch, std::slice::from_ref(&job), 21 * MIN), 1);
    // 26 minutes: and the ones at 18 and 20.
    assert_eq!(rig.ledger.watch_local(&mut watch, std::slice::from_ref(&job), 26 * MIN), 2);
}

#[test]
fn a_pass_after_the_computer_slept_judges_nothing_before_it() {
    let rig = rig();
    let job = every_minute("j");
    let mut watch = LocalWatch::new(0);
    assert_eq!(rig.ledger.watch_local(&mut watch, std::slice::from_ref(&job), 7 * MIN), 2);
    // An hour with no pass: the scheduler tasks woke late and fire what they can.
    assert_eq!(rig.ledger.watch_local(&mut watch, std::slice::from_ref(&job), 67 * MIN), 0);
    assert_eq!(rows(&rig, "j").len(), 2);
    // Passes go on as before from there: the slots after the wake that are past their grace.
    assert_eq!(rig.ledger.watch_local(&mut watch, std::slice::from_ref(&job), 72 * MIN), 0);
    assert_eq!(rig.ledger.watch_local(&mut watch, std::slice::from_ref(&job), 77 * MIN), 5);
}

#[test]
fn a_job_with_no_slots_is_not_watched() {
    let rig = rig();
    let mut manual = every_minute("manual");
    manual.trigger_kind = "manual".to_string();
    let mut once = every_minute("once");
    once.trigger_kind = "once".to_string();
    once.once_at_ms = Some(MIN);
    let mut event = every_minute("event");
    event.trigger_kind = "event".to_string();
    let mut watch = LocalWatch::new(0);
    assert_eq!(rig.ledger.watch_local(&mut watch, &[manual, once, event], 60 * MIN), 0);
}

#[test]
fn a_job_that_was_paused_and_comes_back_is_judged_from_then() {
    let rig = rig();
    let job = every_minute("j");
    let mut watch = LocalWatch::new(0);
    rig.ledger.watch_local(&mut watch, std::slice::from_ref(&job), 7 * MIN);
    // Paused (gone from the enabled list) for 30 minutes: its slots were counted as skips elsewhere.
    rig.ledger.watch_local(&mut watch, &[], 12 * MIN);
    rig.ledger.watch_local(&mut watch, &[], 17 * MIN);
    let before = rows(&rig, "j").len();
    assert_eq!(rig.ledger.watch_local(&mut watch, std::slice::from_ref(&job), 20 * MIN), 0);
    assert_eq!(rows(&rig, "j").len(), before);
}

#[test]
fn a_miss_reads_as_never_started() {
    let rig = rig();
    let job = every_minute("j");
    let mut watch = LocalWatch::new(0);
    assert_eq!(rig.ledger.watch_local(&mut watch, std::slice::from_ref(&job), 7 * MIN), 2);
    let r = &rows(&rig, "j")[0];
    assert_eq!(super::super::incidents::failure_words(r), "missed: the scheduler never started it");
}
