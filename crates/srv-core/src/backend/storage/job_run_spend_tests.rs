// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::super::job_runs::{state, JobRun, RunCost};
use super::*;

const NOW: i64 = 1_800_000_000_000;
const MIN: i64 = 60_000;

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap();
    (dir, s)
}

/// A run of `job` on `account` started at `at`, ended with `cost`.
fn spent(s: &Store, id: &str, job: &str, account: &str, at: i64, cost: f64) {
    let mut r = JobRun::new(id, job, "cron", "nightly", "schedule", state::SCHEDULED, at);
    r.scheduled_for_ms = Some(at);
    r.recorded_by = "stable@0.0.0".into();
    s.job_run_insert(&r).unwrap();
    s.job_run_mark_running(id, "uid-a", "b1", account, "claude", at).unwrap();
    s.job_run_finish(id, "ok", Some(RunCost { cost_usd: cost, cost_known: true, output_tokens: 1, steps: 1 }), "", at + MIN).unwrap();
}

#[test]
fn an_accounts_spend_counts_only_its_runs_from_the_day_start() {
    let (_dir, s) = store();
    spent(&s, "r1", "j1", "acct-a", NOW - 2 * MIN, 1.25);
    spent(&s, "r2", "j2", "acct-a", NOW, 2.0);
    spent(&s, "r3", "j1", "acct-b", NOW, 9.0);
    spent(&s, "r4", "j1", "acct-a", NOW - 90 * MIN, 7.0);
    assert_eq!(s.account_runs_cost_since("acct-a", NOW - 10 * MIN).unwrap(), 3.25, "two jobs, one account, since then");
    assert_eq!(s.account_runs_cost_since("acct-b", NOW - 10 * MIN).unwrap(), 9.0);
    assert_eq!(s.account_runs_cost_since("", NOW - 10 * MIN).unwrap(), 0.0, "no account, nothing to add up");
}

#[test]
fn delivered_runs_count_and_waiting_or_skipped_ones_do_not() {
    let (_dir, s) = store();
    spent(&s, "r1", "j1", "a", NOW, 0.1);
    let mut waiting = JobRun::new("w1", "j1", "cron", "n", "schedule", state::SCHEDULED, NOW);
    waiting.reason = "overlap".into();
    s.job_run_insert(&waiting).unwrap();
    let mut skipped = JobRun::new("s1", "j1", "cron", "n", "schedule", state::SKIPPED, NOW);
    skipped.reason = "job_cap".into();
    s.job_run_insert(&skipped).unwrap();
    let delivering = JobRun::new("d1", "j1", "cron", "n", "schedule", state::SCHEDULED, NOW);
    s.job_run_insert(&delivering).unwrap();
    assert_eq!(s.job_runs_delivered_since("j1", NOW - MIN).unwrap(), 2, "the finished one and the one being delivered");
    assert_eq!(s.job_runs_delivered_since("j1", NOW + MIN).unwrap(), 0, "a later day starts again");
    assert_eq!(s.job_runs_delivered_since("other", NOW - MIN).unwrap(), 0);
}

#[test]
fn the_days_cap_skips_are_listed_once_each() {
    let (_dir, s) = store();
    for (i, why) in ["job_cap", "job_cap", "message_cap", "paused"].into_iter().enumerate() {
        let mut r = JobRun::new(&format!("k{i}"), "j1", "cron", "n", "schedule", state::SKIPPED, NOW + i as i64);
        r.reason = why.into();
        s.job_run_insert(&r).unwrap();
    }
    assert_eq!(s.job_cap_skips_since("j1", NOW - MIN).unwrap(), ["message_cap", "job_cap"]);
    assert!(s.job_cap_skips_since("j1", NOW + MIN).unwrap().is_empty());
}

#[test]
fn an_admitted_run_names_its_account_until_it_ends() {
    let (_dir, s) = store();
    let r = JobRun::new("r1", "j1", "cron", "n", "schedule", state::SCHEDULED, NOW);
    s.job_run_insert(&r).unwrap();
    assert!(s.job_run_set_account("r1", "acct-a").unwrap());
    assert_eq!(s.job_run_get("r1").unwrap().unwrap().account_id, "acct-a");
    s.job_run_finish("r1", "ok", None, "", NOW).unwrap();
    assert!(!s.job_run_set_account("r1", "acct-b").unwrap(), "an ended run is left alone");
}
