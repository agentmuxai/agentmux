// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use chrono::TimeZone;
use chrono_tz::Tz;

use super::*;

fn caps(run: Option<f64>, day: Option<f64>, messages: Option<u32>) -> Caps {
    Caps { run_usd: run, day_usd: day, messages_per_day: messages }
}

fn eff(run: f64, day: f64) -> Effective {
    Effective { run_usd: run, day_usd: day, messages_per_day: 24 }
}

fn spend(job: f64, account: Option<f64>) -> Spend {
    Spend { job_usd: job, account_usd: account, messages: 0 }
}

// ── the defaults ────────────────────────────────────────────────────────

#[test]
fn caps_are_on_by_default_and_a_bad_setting_is_the_default() {
    assert_eq!(Limits::from_settings(&SettingsType::default()), Limits { run_usd: 5.0, job_day_usd: 20.0, account_day_usd: 50.0, messages_per_day: 24 });
    let mut s = SettingsType::default();
    s.jobs_cap_run_usd = Some(-1.0);
    s.jobs_cap_job_day_usd = Some(f64::NAN);
    s.jobs_cap_account_day_usd = Some(75.0);
    s.jobs_cap_messages_day = Some(3);
    let l = Limits::from_settings(&s);
    assert_eq!((l.run_usd, l.job_day_usd, l.account_day_usd, l.messages_per_day), (5.0, 20.0, 75.0, 3));
}

#[test]
fn a_jobs_own_figures_win_over_the_defaults() {
    let l = Limits::default();
    assert_eq!(effective(None, &l), Effective { run_usd: 5.0, day_usd: 20.0, messages_per_day: 24 });
    let own = caps(Some(1.0), None, Some(2));
    assert_eq!(effective(Some(&own), &l), Effective { run_usd: 1.0, day_usd: 20.0, messages_per_day: 2 });
}

// ── an agent's higher figures ───────────────────────────────────────────

#[test]
fn an_agents_higher_caps_are_clamped_and_the_reply_says_so() {
    let l = Limits::default();
    let (kept, notes) = clamp_to_defaults(Some(caps(Some(50.0), Some(400.0), Some(1000))), &l, None);
    assert_eq!(kept, Some(caps(Some(5.0), Some(20.0), Some(24))));
    assert_eq!(notes.len(), 3);
    assert!(notes[0].contains("run_usd 50") && notes[0].contains("only the operator raises a cap"), "{notes:?}");
    assert!(notes[1].contains("day_usd") && notes[2].contains("messages_per_day"), "{notes:?}");
}

#[test]
fn lower_or_equal_caps_are_kept_untouched() {
    let l = Limits::default();
    let own = caps(Some(5.0), Some(1.0), Some(24));
    assert_eq!(clamp_to_defaults(Some(own.clone()), &l, None), (Some(own.clone()), vec![]));
    assert_eq!(clamp_to_defaults(None, &l, None), (None, vec![]));
}

/// An edit that leaves a figure as the operator raised it keeps it; one that
/// raises it further, or sets another, is held to the defaults.
#[test]
fn an_edit_keeps_the_figures_the_job_already_had() {
    let l = Limits::default();
    let raised = caps(Some(50.0), Some(400.0), None);
    let (kept, notes) = clamp_to_defaults(Some(raised.clone()), &l, Some(&raised));
    assert_eq!((kept, notes), (Some(raised.clone()), vec![]));
    let (kept, notes) = clamp_to_defaults(Some(caps(Some(60.0), Some(400.0), Some(99))), &l, Some(&raised));
    assert_eq!(kept, Some(caps(Some(5.0), Some(400.0), Some(24))), "only what changed is clamped");
    assert_eq!(notes.len(), 2);
}

// ── the reservation ─────────────────────────────────────────────────────

#[test]
fn a_run_starts_only_if_its_whole_cap_fits_the_days_room() {
    let l = Limits::default();
    let e = eff(5.0, 20.0);
    assert_eq!(check(&e, &l, &spend(15.0, None), false), None, "15 + 5 fits 20 exactly");
    assert_eq!(check(&e, &l, &spend(15.01, None), false), Some(Refusal::JobCap));
    assert_eq!(check(&e, &l, &spend(0.0, Some(45.0)), false), None);
    assert_eq!(check(&e, &l, &spend(0.0, Some(45.5)), false), Some(Refusal::AccountCap));
    assert_eq!(check(&e, &l, &spend(19.0, Some(49.0)), false), Some(Refusal::JobCap), "the job's cap is checked first");
    assert_eq!(check(&e, &l, &spend(0.0, None), false), None, "an account that isn't known has no cap to weigh");
}

/// However the runs spend, as much as their caps allow every time, the day's
/// total never passes the daily cap.
#[test]
fn the_daily_cap_holds_even_if_every_run_spends_its_whole_per_run_cap() {
    let l = Limits::default();
    for (run, day) in [(5.0, 20.0), (3.0, 20.0), (0.7, 20.0), (5.0, 4.0), (2.5, 7.0), (0.0, 1.0)] {
        let e = eff(run, day);
        let (mut spent, mut started) = (0.0_f64, 0);
        for _ in 0..1000 {
            if check(&e, &l, &spend(spent, None), false).is_some() {
                break;
            }
            spent += run;
            started += 1;
            assert!(spent <= day + 1e-6, "run cap {run}, day cap {day}: spent {spent} after {started} runs");
        }
        if run > 0.0 {
            assert!(spent + run > day - 1e-6, "it stops only when the next run's cap no longer fits ({run}, {day}, spent {spent})");
        }
    }
}

#[test]
fn a_job_sending_to_another_agent_is_held_to_its_messages_a_day() {
    let l = Limits::default();
    let e = Effective { run_usd: 1.0, day_usd: 20.0, messages_per_day: 3 };
    let sent = |n| Spend { job_usd: 0.0, account_usd: None, messages: n };
    assert_eq!(check(&e, &l, &sent(2), true), None);
    assert_eq!(check(&e, &l, &sent(3), true), Some(Refusal::MessageCap));
    assert_eq!(check(&e, &l, &sent(99), false), None, "a job that messages its owner has no message cap");
    assert_eq!(Refusal::MessageCap.reason(), "message_cap");
}

#[test]
fn a_run_stops_when_its_spend_reaches_its_cap() {
    assert!(!reached(0.0, 0.0), "nothing spent is never over");
    assert!(!reached(4.99, 5.0));
    assert!(reached(5.0, 5.0));
    assert!(reached(7.2, 5.0));
    assert!(reached(0.01, 0.0));
}

// ── whose message it is ─────────────────────────────────────────────────

fn message_job(owner_uid: &str, created_by: &str, target: &str, target_uid: &str) -> CronJob {
    CronJob {
        id: "j".into(),
        owner: owner_uid.into(),
        created_by: created_by.into(),
        created_by_uid: owner_uid.into(),
        target: target.into(),
        target_uid: target_uid.into(),
        ..Default::default()
    }
}

#[test]
fn only_a_message_to_an_agent_other_than_the_owner_is_limited() {
    assert!(sends_to_another(&message_job("uid-a", "Alpha", "Beta", "uid-b")));
    assert!(!sends_to_another(&message_job("uid-a", "Alpha", "Alpha", "uid-a")), "to itself");
    assert!(sends_to_another(&message_job("", "Alpha", "Beta", "")), "by name");
    assert!(!sends_to_another(&message_job("", "alpha", "ALPHA", "")), "by name, same agent");
    assert!(!sends_to_another(&message_job("", "operator", "Beta", "")), "the operator's own job");
    let mut task = message_job("uid-a", "Alpha", "Beta", "uid-b");
    task.action_kind = "task".into();
    assert!(!sends_to_another(&task), "a task is no message");
}

// ── the operator's day ──────────────────────────────────────────────────

fn ny() -> Tz {
    "America/New_York".parse().unwrap()
}

fn ms(tz: &Tz, y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
    tz.with_ymd_and_hms(y, mo, d, h, mi, 0).earliest().unwrap().timestamp_millis()
}

#[test]
fn the_day_starts_at_local_midnight_and_resets_there() {
    let tz = ny();
    let midnight = ms(&tz, 2026, 10, 24, 0, 0);
    assert_eq!(day_start_ms(ms(&tz, 2026, 10, 24, 0, 0), &tz), midnight, "midnight itself starts the day");
    assert_eq!(day_start_ms(ms(&tz, 2026, 10, 24, 23, 59), &tz), midnight);
    assert_eq!(day_start_ms(ms(&tz, 2026, 10, 24, 0, 0) - 1, &tz), ms(&tz, 2026, 10, 23, 0, 0), "a millisecond before is yesterday");
    // A run at 23:30 and one at 00:30 are in different days.
    assert_ne!(day_start_ms(ms(&tz, 2026, 10, 24, 23, 30), &tz), day_start_ms(ms(&tz, 2026, 10, 25, 0, 30), &tz));
}

#[test]
fn a_daylight_saving_day_is_23_or_25_hours_long() {
    let tz = ny();
    // Spring forward 2026-03-08, fall back 2026-11-01.
    let (spring, fall) = (ms(&tz, 2026, 3, 8, 0, 0), ms(&tz, 2026, 11, 1, 0, 0));
    assert_eq!(ms(&tz, 2026, 3, 9, 0, 0) - spring, 23 * 3_600_000);
    assert_eq!(ms(&tz, 2026, 11, 2, 0, 0) - fall, 25 * 3_600_000);
    assert_eq!(day_start_ms(ms(&tz, 2026, 3, 8, 12, 0), &tz), spring);
    assert_eq!(day_start_ms(ms(&tz, 2026, 11, 1, 23, 0), &tz), fall);
}

#[test]
fn a_zone_with_no_midnight_that_day_starts_at_its_first_hour() {
    // Sao Paulo moved its clocks at midnight until 2018: 00:00 didn't exist.
    let tz: Tz = "America/Sao_Paulo".parse().unwrap();
    let noon = tz.with_ymd_and_hms(2018, 11, 4, 12, 0, 0).earliest().unwrap();
    let start = day_start_ms(noon.timestamp_millis(), &tz);
    let at = tz.timestamp_millis_opt(start).single().unwrap();
    assert_eq!(at.format("%Y-%m-%d %H:%M").to_string(), "2018-11-04 01:00");
}
