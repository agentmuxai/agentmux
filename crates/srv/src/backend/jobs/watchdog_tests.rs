// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use chrono::TimeZone;
use serde_json::json;

use super::*;
use crate::backend::mps::Broker;
use crate::backend::storage::job_cloud::{CloudStart, EventMark};
use crate::backend::storage::store::Store;

const JOB: &str = "acme.nightly";
const MIN: i64 = 60_000;
const HOUR: i64 = 60 * MIN;

fn at(day: u32, hour: u32, min: u32) -> i64 {
    chrono::Utc.with_ymd_and_hms(2026, 12, day, hour, min, 0).unwrap().timestamp_millis()
}

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

/// Expected at 02:00 UTC every day, a report due within `grace` minutes, approved at `since`.
fn nightly(since: i64, grace: i64) -> WatchedJob {
    let schedule = JobSchedule::cron("0 2 * * *", "UTC").unwrap();
    WatchedJob {
        id: JOB.into(),
        name: "Nightly".into(),
        since_ms: since,
        schedule: Some(WatchedSchedule { schedule, grace_ms: grace * MIN }),
        max_run_ms: 2 * HOUR,
    }
}

fn rows(rig: &Rig) -> Vec<JobRun> {
    let mut all = rig.store.job_runs_for_job(JOB, 500, None).unwrap();
    all.sort_by_key(|r| r.started_at_ms);
    all
}

/// A real run the reporter sent: `started` at `started_ms`, naming its slot when given.
fn reported(rig: &Rig, name: &str, state: &str, started_ms: i64, slot: Option<i64>) {
    let mut run = JobRun::new(&format!("cloud:{JOB}:{name}"), JOB, SOURCE_CLOUD, "Nightly", "schedule", state, started_ms);
    if state == "finished" {
        run.ended_at_ms = Some(started_ms + MIN);
    }
    if let Some(slot) = slot {
        run.links = json!({ "scheduled_for_ms": slot });
    }
    assert!(rig.store.job_run_insert(&run).unwrap());
}

#[test]
fn a_slot_with_no_report_after_its_grace_is_a_missed_row_once() {
    let rig = rig();
    let job = nightly(at(9, 3, 0), 30);

    // 02:10 on the 11th: the 10th's report is overdue, the 11th's is still within its grace.
    assert_eq!(rig.ledger.watch_cloud(std::slice::from_ref(&job), at(11, 2, 10)), Watched { missed: 1, lost: 0 });
    let r = &rows(&rig)[0];
    assert_eq!((r.state.as_str(), r.reason.as_str(), r.job_source.as_str(), r.trigger.as_str()), ("missed", "no_report", "cloud", "schedule"));
    assert_eq!((r.scheduled_for_ms, r.ended_at_ms, r.started_at_ms), (Some(at(10, 2, 0)), Some(at(11, 2, 10)), at(10, 2, 0)));
    assert_eq!((r.id.as_str(), r.triggered_by.as_str(), r.job_name.as_str()), (format!("watchdog:{JOB}:{}", at(10, 2, 0)).as_str(), "watchdog", "Nightly"));
    // The 9th's slot is before the job was approved: not a miss.
    assert_eq!(rows(&rig).len(), 1);

    // Again at the same time: nothing new. Then the 11th's grace passes.
    assert_eq!(rig.ledger.watch_cloud(std::slice::from_ref(&job), at(11, 2, 10)), Watched::default());
    assert_eq!(rig.ledger.watch_cloud(std::slice::from_ref(&job), at(11, 2, 40)), Watched { missed: 1, lost: 0 });
    assert_eq!(rig.ledger.watch_cloud(std::slice::from_ref(&job), at(11, 9, 0)), Watched::default(), "a slot is given up on once");
    assert_eq!(rows(&rig).len(), 2);
}

#[test]
fn a_slot_with_a_report_is_not_missed() {
    let rig = rig();
    // The 31st's run named its slot; the 11th's started at 02:00:20 and named none.
    reported(&rig, "r1", "finished", at(10, 2, 5), Some(at(10, 2, 0)));
    reported(&rig, "r2", "finished", at(11, 2, 0) + 20_000, None);
    assert_eq!(rig.ledger.watch_cloud(&[nightly(at(9, 3, 0), 30)], at(11, 9, 0)), Watched::default());
    assert_eq!(rows(&rig).len(), 2);
}

#[test]
fn a_job_is_not_blamed_for_slots_before_it_was_approved() {
    let rig = rig();
    assert_eq!(rig.ledger.watch_cloud(&[nightly(at(1, 3, 0), 30)], at(1, 9, 0)), Watched::default());
    assert!(rows(&rig).is_empty());
}

#[test]
fn a_job_that_events_start_has_no_slots_to_miss() {
    let rig = rig();
    let mut job = nightly(at(1, 0, 0), 30);
    job.schedule = None;
    assert_eq!(rig.ledger.watch_cloud(&[job], at(5, 9, 0)), Watched::default());
}

#[test]
fn a_long_silence_is_cut_to_the_newest_misses_and_not_filled_in_by_later_passes() {
    let rig = rig();
    let every_minute = JobSchedule::cron("* * * * *", "UTC").unwrap();
    let job = WatchedJob {
        id: JOB.into(),
        name: "Nightly".into(),
        since_ms: at(1, 0, 0),
        schedule: Some(WatchedSchedule { schedule: every_minute, grace_ms: 0 }),
        max_run_ms: HOUR,
    };
    let now = at(1, 3, 0);
    assert_eq!(rig.ledger.watch_cloud(std::slice::from_ref(&job), now), Watched { missed: MAX_MISSES_PER_PASS, lost: 0 });
    let oldest = rows(&rig)[0].scheduled_for_ms.unwrap();
    assert_eq!(oldest, now - (MAX_MISSES_PER_PASS as i64 - 1) * MIN, "the newest slots are the ones kept");
    assert_eq!(rig.ledger.watch_cloud(std::slice::from_ref(&job), now), Watched::default(), "the older ones are not written later");
    // A minute on, only the new slot is missed.
    assert_eq!(rig.ledger.watch_cloud(std::slice::from_ref(&job), now + MIN), Watched { missed: 1, lost: 0 });
}

#[test]
fn a_late_report_takes_over_the_watchdogs_row_and_the_slot_stays_one_row() {
    let rig = rig();
    let slot = at(10, 2, 0);
    rig.ledger.watch_cloud(&[nightly(at(9, 3, 0), 30)], at(10, 3, 0));
    let given_up = rows(&rig).pop().unwrap();
    assert_eq!((given_up.state.as_str(), given_up.scheduled_for_ms), ("missed", Some(slot)));

    let mut run = JobRun::new(&format!("cloud:{JOB}:r1"), JOB, SOURCE_CLOUD, "Nightly", "schedule", "running", at(10, 3, 30));
    run.links = json!({ "scheduled_for_ms": slot });
    let mark = EventMark { run_id: "r1", event_id: "e1", now_ms: at(10, 3, 30) };
    assert_eq!(rig.store.job_cloud_start(&run, Some(slot), &mark).unwrap(), Some(CloudStart::TookOverMissed));
    let r = rows(&rig);
    assert_eq!(r.len(), 1);
    assert_eq!((r[0].state.as_str(), r[0].reason.as_str()), ("running", "late"));
}

#[test]
fn a_run_that_stops_reporting_is_lost_and_a_heartbeat_keeps_it_alive() {
    let rig = rig();
    let mut job = nightly(at(1, 0, 0), 30);
    job.schedule = None;
    reported(&rig, "quiet", "running", at(1, 2, 0), None);
    reported(&rig, "beating", "running", at(1, 2, 0), None);
    rig.store
        .conn()
        .lock()
        .unwrap()
        .execute("UPDATE db_job_runs SET links = ?2 WHERE id = ?1", rusqlite::params![format!("cloud:{JOB}:beating"), json!({ "heartbeat_ms": at(1, 3, 30) }).to_string()])
        .unwrap();

    // Two hours is the limit: at 03:59 neither is overdue; at 04:10 only the one that stopped is.
    assert_eq!(rig.ledger.watch_cloud(std::slice::from_ref(&job), at(1, 3, 59)), Watched::default());
    assert_eq!(rig.ledger.watch_cloud(std::slice::from_ref(&job), at(1, 4, 10)), Watched { missed: 0, lost: 1 });
    let get = |name: &str| rig.store.job_run_get(&format!("cloud:{JOB}:{name}")).unwrap().unwrap();
    let quiet = get("quiet");
    assert_eq!((quiet.state.as_str(), quiet.reason.as_str(), quiet.ended_at_ms), ("lost", "no_report", Some(at(1, 4, 10))));
    assert_eq!(get("beating").state, "running");
    assert_eq!(rig.ledger.watch_cloud(std::slice::from_ref(&job), at(1, 4, 20)), Watched::default(), "a lost run is not lost again");
    assert_eq!(rig.ledger.watch_cloud(std::slice::from_ref(&job), at(1, 5, 31)), Watched { missed: 0, lost: 1 }, "the beating run stops too, in time");
}

#[test]
fn a_run_of_a_job_nobody_approves_any_more_is_lost_after_a_day() {
    let rig = rig();
    reported(&rig, "orphan", "running", at(1, 2, 0), None);
    assert_eq!(rig.ledger.watch_cloud(&[], at(2, 1, 59)), Watched::default());
    assert_eq!(rig.ledger.watch_cloud(&[], at(2, 2, 1)), Watched { missed: 0, lost: 1 });
}

#[test]
fn a_run_that_finished_is_never_lost() {
    let rig = rig();
    reported(&rig, "done", "finished", at(1, 2, 0), None);
    let mut job = nightly(at(1, 0, 0), 30);
    job.schedule = None;
    assert_eq!(rig.ledger.watch_cloud(&[job], at(9, 2, 0)), Watched::default());
    assert_eq!(rig.store.job_run_get(&format!("cloud:{JOB}:done")).unwrap().unwrap().state, "finished");
}
