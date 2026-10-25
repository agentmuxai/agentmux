// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! A job's slots: time zones across daylight-saving changes, the jitter,
//! and the other trigger kinds (SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.1).

use chrono::TimeZone;

use super::*;

/// ms epoch of a UTC time.
fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
    Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap().timestamp_millis()
}

fn slots(s: &JobSchedule, from: i64, n: usize) -> Vec<i64> {
    s.after(from).take(n).collect()
}

const NY: &str = "America/New_York";

/// 2026-03-08, New York springs forward at 02:00 EST to 03:00 EDT: a 02:30
/// job fires once that night, at 03:00 EDT, and at 02:30 again the next.
#[test]
fn a_slot_in_a_spring_forward_gap_fires_once_at_the_next_valid_time() {
    let s = JobSchedule::cron("30 2 * * *", NY).unwrap();
    let got = slots(&s, utc(2026, 3, 7, 0, 0), 3);
    // 02:30 EST is 07:30Z; 03:00 EDT is 07:00Z; 02:30 EDT is 06:30Z.
    assert_eq!(got, vec![utc(2026, 3, 7, 7, 30), utc(2026, 3, 8, 7, 0), utc(2026, 3, 9, 6, 30)]);
}

/// Every slot in a gap is the same one fire: a one-minute job goes from
/// 01:59 EST straight to 03:00 EDT, then 03:01, with nothing twice.
#[test]
fn several_slots_in_one_gap_are_one_fire() {
    let s = JobSchedule::cron("* * * * *", NY).unwrap();
    let got = slots(&s, utc(2026, 3, 8, 6, 58), 3);
    assert_eq!(got, vec![utc(2026, 3, 8, 6, 59), utc(2026, 3, 8, 7, 0), utc(2026, 3, 8, 7, 1)]);
}

/// 2026-11-01, New York falls back at 02:00 EDT to 01:00 EST: 01:30 happens
/// twice, and the job fires once, at the first.
#[test]
fn a_slot_in_a_fall_back_overlap_fires_once() {
    let s = JobSchedule::cron("30 1 * * *", NY).unwrap();
    let got = slots(&s, utc(2026, 10, 31, 0, 0), 3);
    // 01:30 EDT is 05:30Z; 01:30 EST (06:30Z) is skipped; the next day's is EST.
    assert_eq!(got, vec![utc(2026, 10, 31, 5, 30), utc(2026, 11, 1, 5, 30), utc(2026, 11, 2, 6, 30)]);
    // Asked from inside the repeated hour, it doesn't fire it again.
    assert_eq!(s.next_after(utc(2026, 11, 1, 6, 0)), Some(utc(2026, 11, 2, 6, 30)));
}

/// A 15-minute job runs through the first 01:xx hour and not the repeated one.
#[test]
fn a_repeated_hour_runs_once() {
    let s = JobSchedule::cron("*/15 * * * *", NY).unwrap();
    let got = slots(&s, utc(2026, 11, 1, 4, 50), 6);
    let want = [(5, 0), (5, 15), (5, 30), (5, 45), (7, 0), (7, 15)].map(|(h, m)| utc(2026, 11, 1, h, m));
    assert_eq!(got, want);
}

/// The slot before a time, inside the repeated hour, is the last one that
/// fired (01:59 EDT), not a second-pass time that never fires.
#[test]
fn the_previous_slot_inside_an_overlap_is_the_last_that_fired() {
    let s = JobSchedule::cron("* * * * *", NY).unwrap();
    assert_eq!(s.prev_before(utc(2026, 11, 1, 6, 10)), Some(utc(2026, 11, 1, 5, 59)));
    assert_eq!(s.prev_before(utc(2026, 3, 8, 7, 0)), Some(utc(2026, 3, 8, 6, 59)));
    let daily = JobSchedule::cron("0 6 * * *", "UTC").unwrap();
    assert_eq!(daily.prev_before(utc(2026, 1, 2, 6, 0)), Some(utc(2026, 1, 1, 6, 0)));
}

/// 09:00 in Berlin is 08:00Z in winter and 07:00Z in summer.
#[test]
fn a_zone_keeps_the_local_time_across_the_year() {
    let s = JobSchedule::cron("0 9 * * *", "Europe/Berlin").unwrap();
    assert_eq!(s.next_after(utc(2026, 1, 15, 0, 0)), Some(utc(2026, 1, 15, 8, 0)));
    assert_eq!(s.next_after(utc(2026, 7, 15, 0, 0)), Some(utc(2026, 7, 15, 7, 0)));
    // UTC (and an empty zone) is the expression as it was before zones.
    let plain = JobSchedule::cron("0 9 * * *", "").unwrap();
    assert_eq!(plain.next_after(utc(2026, 7, 15, 0, 0)), Some(utc(2026, 7, 15, 9, 0)));
}

#[test]
fn an_unknown_zone_or_expression_is_refused() {
    assert!(JobSchedule::cron("0 9 * * *", "Mars/Olympus").unwrap_err().contains("unknown time zone"));
    assert!(JobSchedule::cron("0 9 * *", "UTC").is_err());
    assert!(parse_tz("utc").is_err(), "zone names are case-sensitive, as IANA's are");
}

#[test]
fn the_jitter_is_fixed_per_job_and_at_most_two_minutes() {
    assert_eq!(jitter_ms("job-a"), jitter_ms("job-a"));
    let all: Vec<i64> = (0..200).map(|i| jitter_ms(&format!("job-{i}"))).collect();
    assert!(all.iter().all(|&j| (0..=JITTER_MAX_MS).contains(&j)));
    let distinct: std::collections::BTreeSet<i64> = all.iter().copied().collect();
    assert!(distinct.len() > 150, "spread out: {} distinct of 200", distinct.len());
}

#[test]
fn every_once_and_manual_triggers() {
    let every = JobSchedule::Every { anchor_ms: 1_000_000, period_ms: 3_600_000 };
    assert_eq!(slots(&every, 0, 2), vec![4_600_000, 8_200_000], "the first slot is one period after creation");
    assert_eq!(every.next_after(4_600_000), Some(8_200_000));
    assert_eq!(every.prev_before(8_200_000), Some(4_600_000));
    assert_eq!(every.prev_before(4_600_000), None);

    let once = JobSchedule::Once { at_ms: 5_000 };
    assert_eq!(slots(&once, 0, 3), vec![5_000]);
    assert_eq!(once.next_after(5_000), None);
    assert_eq!(once.prev_before(5_001), Some(5_000));

    assert_eq!(JobSchedule::Manual.next_after(0), None);
    assert_eq!(JobSchedule::Manual.prev_before(i64::MAX), None);
}

#[test]
fn the_shortest_gap_between_slots() {
    let from = utc(2026, 1, 1, 0, 0);
    assert_eq!(JobSchedule::cron("*/5 * * * *", "UTC").unwrap().min_gap_secs(from, 50), Some(300));
    assert_eq!(JobSchedule::cron("0,10 6 * * *", "UTC").unwrap().min_gap_secs(from, 50), Some(600));
    assert_eq!(JobSchedule::cron("0 6 * * *", "UTC").unwrap().min_gap_secs(from, 50), Some(86_400));
    assert_eq!(JobSchedule::Once { at_ms: from + 1 }.min_gap_secs(from, 50), None);
}

/// A stored job's trigger, by kind.
#[test]
fn a_stored_job_reads_its_trigger_by_kind() {
    let mut job = crate::backend::cron::test_job("j", "p", "t", "", "");
    job.created_at = 1_000;
    assert!(matches!(JobSchedule::of(&job).unwrap(), JobSchedule::Cron { .. }), "a job from before kinds is cron");

    (job.trigger_kind, job.every_secs) = ("every".into(), Some(3600));
    let every = JobSchedule::of(&job).unwrap();
    assert_eq!(every.next_after(1_000_000), Some(1_000_000 + 3_600_000), "from its creation, one period at a time");
    job.every_secs = None;
    assert!(JobSchedule::of(&job).is_err(), "an interval needs its seconds");

    (job.trigger_kind, job.once_at_ms) = ("once".into(), Some(9_000));
    assert_eq!(JobSchedule::of(&job).unwrap().next_after(0), Some(9_000));
    job.trigger_kind = "manual".into();
    assert_eq!(JobSchedule::of(&job).unwrap().next_after(0), None);
    job.trigger_kind = "event".into();
    assert!(JobSchedule::of(&job).unwrap_err().contains("can't be scheduled"));
}
