// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

const NOW: i64 = 1_800_000_000_000;
const MIN: i64 = 60_000;

fn store_at(dir: &tempfile::TempDir) -> Store {
    Store::open_identity_store(&dir.path().join("identity-store.db")).unwrap()
}

fn slot_run(id: &str, job: &str, slot: Option<i64>) -> JobRun {
    let mut r = JobRun::new(id, job, "cron", "nightly", "schedule", state::SCHEDULED, NOW);
    r.scheduled_for_ms = slot;
    r.recorded_by = "stable@0.0.0".into();
    r
}

/// Two schedulers sharing the identity store (two handles on one file, as
/// two instances have) insert the same slot: the second finds it taken, so
/// it doesn't fire. Manual runs have no slot and never collide.
#[test]
fn a_slot_is_leased_once_across_two_handles_on_one_store() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (store_at(&dir), store_at(&dir));
    assert!(a.job_run_insert(&slot_run("r1", "j1", Some(NOW))).unwrap());
    assert!(!b.job_run_insert(&slot_run("r2", "j1", Some(NOW))).unwrap(), "the slot is taken");
    assert!(b.job_run_insert(&slot_run("r3", "j1", Some(NOW + MIN))).unwrap(), "the next slot is free");
    assert!(b.job_run_insert(&slot_run("r4", "j2", Some(NOW))).unwrap(), "another job's slot is its own");
    assert!(a.job_run_insert(&slot_run("m1", "j1", None)).unwrap());
    assert!(b.job_run_insert(&slot_run("m2", "j1", None)).unwrap(), "manual runs have no slot");
    assert_eq!(a.job_runs_for_job("j1", 10, None).unwrap().len(), 4);
}

/// Consecutive skips for one reason are one row with a count; another
/// reason, or a run between, starts a new row.
#[test]
fn consecutive_skips_collapse_with_a_count() {
    let dir = tempfile::tempdir().unwrap();
    let s = store_at(&dir);
    let skip = |id: &str, slot: i64, why: &str| {
        let mut r = slot_run(id, "j1", Some(slot));
        r.started_at_ms = slot;
        s.job_run_insert(&r).unwrap();
        s.job_run_skip(id, why, 1, slot).unwrap()
    };
    assert_eq!(skip("r1", NOW, reason::TARGET_MISSING), "r1");
    assert_eq!(skip("r2", NOW + MIN, reason::TARGET_MISSING), "r1", "folded into r1");
    assert_eq!(skip("r3", NOW + 2 * MIN, reason::TARGET_MISSING), "r1");
    let runs = s.job_runs_for_job("j1", 10, None).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!((runs[0].skip_count, runs[0].last_skip_at_ms), (3, Some(NOW + 2 * MIN)));
    assert_eq!(runs[0].state, state::SKIPPED);

    assert_eq!(skip("r4", NOW + 3 * MIN, reason::APP_NOT_RUNNING), "r4", "another reason");
    assert_eq!(s.job_runs_for_job("j1", 10, None).unwrap().len(), 2);

    // A folded skip keeps its row, hidden from history, so its slot stays
    // leased: inserting it again is refused.
    let folded = s.job_run_get("r2").unwrap().unwrap();
    assert_eq!((folded.state.as_str(), folded.skip_count), (state::SKIPPED, 0));
    assert!(!s.job_run_insert(&slot_run("r2b", "j1", Some(NOW + MIN))).unwrap(), "the slot is still taken");
}

/// A run goes scheduled → running → finished, with the turn's figures; a
/// closed run isn't reopened or closed twice.
#[test]
fn a_run_moves_through_its_states_and_keeps_its_cost() {
    let dir = tempfile::tempdir().unwrap();
    let s = store_at(&dir);
    s.job_run_insert(&slot_run("r1", "j1", Some(NOW))).unwrap();
    assert!(s.job_run_mark_running("r1", "uid-y", "block-y", "acct-1", "claude", NOW + 5).unwrap());
    assert!(s.job_run_bind_turn("r1", 77, true).unwrap());
    let cost = RunCost { cost_usd: 0.42, cost_known: true, output_tokens: 900, steps: 6 };
    assert!(s.job_run_finish("r1", infra::OK, Some(cost), "", NOW + 60_000).unwrap());
    assert!(!s.job_run_finish("r1", infra::ERROR, None, "late", NOW + 70_000).unwrap(), "already closed");
    assert!(!s.job_run_mark_running("r1", "", "b", "", "", NOW).unwrap(), "not reopened");
    let r = s.job_run_get("r1").unwrap().unwrap();
    assert_eq!((r.state.as_str(), r.infra.as_str()), (state::FINISHED, infra::OK));
    assert_eq!((r.block_id.as_str(), r.account_id.as_str(), r.provider.as_str()), ("block-y", "acct-1", "claude"));
    assert_eq!((r.turn_id, r.cost_partial, r.cost_known), (Some(77), true, true));
    assert_eq!((r.cost_usd, r.output_tokens, r.steps), (0.42, 900, 6));
    assert_eq!((r.started_at_ms, r.ended_at_ms), (NOW + 5, Some(NOW + 60_000)));
    assert_eq!(r.outcome, "unknown", "no report: the outcome stays unknown");
}

/// A run reports its own verdict; one that never ran can't.
#[test]
fn a_report_sets_the_outcome_only_on_a_run_that_ran() {
    let dir = tempfile::tempdir().unwrap();
    let s = store_at(&dir);
    s.job_run_insert(&slot_run("r1", "j1", Some(NOW))).unwrap();
    s.job_run_mark_running("r1", "uid-y", "", "", "", NOW).unwrap();
    let long = "x".repeat(400);
    assert!(s.job_run_report("r1", "findings", &long, &serde_json::json!(["https://example.com/pr/1"])).unwrap());
    let r = s.job_run_get("r1").unwrap().unwrap();
    assert_eq!(r.outcome, "findings");
    assert_eq!(r.summary.chars().count(), SUMMARY_MAX_CHARS);
    assert_eq!(r.links["reported"][0], "https://example.com/pr/1");

    s.job_run_insert(&slot_run("r2", "j1", Some(NOW + MIN))).unwrap();
    s.job_run_set_undelivered("r2", state::MISSED, reason::TARGET_STOPPED, NOW).unwrap();
    assert!(!s.job_run_report("r2", "ok", "", &serde_json::Value::Null).unwrap(), "a missed run never ran");
    assert!(!s.job_run_report("nope", "ok", "", &serde_json::Value::Null).unwrap());
}

/// Catch-up `latest`: a pending fire waits open; a newer one supersedes it,
/// or it expires; an ended pending fire is never delivered.
#[test]
fn a_pending_fire_waits_until_superseded_or_expired() {
    let dir = tempfile::tempdir().unwrap();
    let s = store_at(&dir);
    s.job_run_insert(&slot_run("p1", "j1", Some(NOW))).unwrap();
    s.job_run_set_undelivered("p1", state::PENDING, reason::TARGET_STOPPED, NOW).unwrap();
    let open = s.job_runs_open_recorded_by("stable", "cron", &[state::PENDING]).unwrap();
    assert_eq!(open.iter().map(|r| (r.id.as_str(), r.ended_at_ms)).collect::<Vec<_>>(), vec![("p1", None)]);

    s.job_run_insert(&slot_run("p2", "j1", Some(NOW + MIN))).unwrap();
    s.job_run_set_undelivered("p2", state::PENDING, reason::TARGET_STOPPED, NOW + MIN).unwrap();
    assert_eq!(s.job_runs_end_pending("j1", "p2", reason::SUPERSEDED, NOW + MIN).unwrap(), vec!["p1".to_string()]);
    let p1 = s.job_run_get("p1").unwrap().unwrap();
    assert_eq!((p1.state.as_str(), p1.reason.as_str(), p1.ended_at_ms), (state::PENDING, reason::SUPERSEDED, Some(NOW + MIN)));
    assert!(!s.job_run_mark_running("p1", "", "b", "", "", NOW).unwrap(), "an ended pending fire isn't delivered");

    assert!(s.job_run_end_one_pending("p2", reason::EXPIRED, NOW + 2 * MIN).unwrap());
    assert!(!s.job_run_end_one_pending("p2", reason::EXPIRED, NOW + 3 * MIN).unwrap(), "once");
}

/// Startup cleanup reads only the rows its own channel wrote.
#[test]
fn open_runs_are_read_per_channel() {
    let dir = tempfile::tempdir().unwrap();
    let s = store_at(&dir);
    for (id, by) in [("a", "stable@1.0.0"), ("b", "stable@0.9.0"), ("c", "stable-dev@1.0.0"), ("d", "stable")] {
        let mut r = slot_run(id, "j1", None);
        r.recorded_by = by.into();
        r.state = state::RUNNING.into();
        s.job_run_insert(&r).unwrap();
    }
    let mut ids: Vec<String> =
        s.job_runs_open_recorded_by("stable", "cron", &[state::RUNNING]).unwrap().into_iter().map(|r| r.id).collect();
    ids.sort();
    assert_eq!(ids, vec!["a", "b", "d"]);
}

/// History pages newest first, and nothing in the ledger removes a run: no
/// retention sweep (§3.1).
#[test]
fn history_pages_newest_first_and_keeps_every_run() {
    let dir = tempfile::tempdir().unwrap();
    let s = store_at(&dir);
    for i in 0..30 {
        let mut r = slot_run(&format!("r{i}"), "j1", Some(NOW + i * MIN));
        r.started_at_ms = NOW + i * MIN;
        s.job_run_insert(&r).unwrap();
        s.job_run_finish(&format!("r{i}"), infra::OK, None, "", NOW + i * MIN + 1).unwrap();
    }
    let first = s.job_runs_for_job("j1", 10, None).unwrap();
    assert_eq!(first.first().map(|r| r.id.as_str()), Some("r29"));
    let next = s.job_runs_for_job("j1", 10, Some(first.last().unwrap().started_at_ms)).unwrap();
    assert_eq!(next.first().map(|r| r.id.as_str()), Some("r19"));
    assert_eq!(s.job_runs_for_job("j1", 1000, None).unwrap().len(), 30);
    // Reopening the store (a restart) keeps them all too.
    drop(s);
    assert_eq!(store_at(&dir).job_runs_for_job("j1", 1000, None).unwrap().len(), 30);
}

/// A `db_job_runs` an earlier build created, without `collapsed_into`,
/// gains it when the store opens, and a fold works on it.
#[test]
fn an_older_table_gains_the_collapsed_column_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity-store.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        let old = SCHEMA.replace(",
        collapsed_into   TEXT NOT NULL DEFAULT ''", "");
        assert!(!old.contains("collapsed_into"), "the old shape has no column");
        conn.execute_batch(&old).unwrap();
    }
    let s = Store::open_identity_store(&path).unwrap();
    let skip = |id: &str, slot: i64| {
        let mut r = slot_run(id, "j1", Some(slot));
        r.started_at_ms = slot;
        assert!(s.job_run_insert(&r).unwrap());
        s.job_run_skip(id, reason::TARGET_MISSING, 1, slot).unwrap()
    };
    assert_eq!(skip("r1", NOW), "r1");
    assert_eq!(skip("r2", NOW + MIN), "r1", "folded");
    assert_eq!(s.job_runs_for_job("j1", 10, None).unwrap().len(), 1);
    drop(s);
    // Opening again finds the column there and changes nothing.
    assert_eq!(store_at(&dir).job_runs_for_job("j1", 10, None).unwrap()[0].skip_count, 2);
}

/// `OPEN_NOT_ENDED`'s SQL list and `state::OPEN` are the same set, so
/// `JobRun::is_open` agrees with every conditional update here.
#[test]
fn the_open_states_are_one_list() {
    let list = OPEN_NOT_ENDED.split(['(', ')']).nth(1).unwrap();
    let mut sql: Vec<&str> = list.split(',').map(|s| s.trim().trim_matches('\'')).collect();
    let mut open = state::OPEN.to_vec();
    sql.sort_unstable();
    open.sort_unstable();
    assert_eq!(sql, open);
    let mut run = slot_run("r", "j", None);
    for (st, ended, open) in [(state::PENDING, None, true), (state::RUNNING, Some(1), false), (state::FINISHED, None, false)] {
        run.state = st.to_string();
        run.ended_at_ms = ended;
        assert_eq!(run.is_open(), open, "{st} {ended:?}");
    }
}
