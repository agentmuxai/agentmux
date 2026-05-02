// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// LSD-1 — unit tests for `LauncherSagaLog`.
//
// Exercises the in-isolation API surface defined by LSD spec §3.3.
// Mirrors the test inventory pinned in the LSD spec (PR1 acceptance):
//   1. round-trip:  start_saga + start_step + finish_step +
//                   terminate_saga → log rows correct
//   2. idempotent compensation: mark_failed_compensation called twice
//                   → no duplicate updates, no errors
//   3. unresolved_sagas filter: only non-terminal sagas surface
//   4. vacuum_older_than: removes terminal rows older than cutoff;
//                   never touches running/compensating
//   5. schema migration: open() applies DDL on a fresh DB; reopen()
//                   doesn't error (CREATE TABLE IF NOT EXISTS)
//
// Tests use `open_in_memory()` for speed except `schema_migration_*`
// which uses a real `NamedTempFile` so the WAL pragmas + reopen path
// are exercised against on-disk SQLite.

use super::*;

use agentmux_common::ipc::{Command, Event};
use chrono::Duration as ChronoDuration;
use tempfile::NamedTempFile;

fn ping(nonce: u64) -> Command {
    Command::Ping { nonce }
}

fn pong(nonce: u64, version: u64) -> Event {
    Event::Pong { nonce, version }
}

#[test]
fn schema_migration_clean_db_then_reopen_is_idempotent() {
    // Fresh tempfile — first open creates schema.
    let f = NamedTempFile::new().expect("tempfile");
    {
        let _log = LauncherSagaLog::open(f.path()).expect("first open creates schema");
    }
    // Second open is a no-op via `CREATE TABLE IF NOT EXISTS` —
    // verify it doesn't error and that prior schema is still
    // queryable (e.g. max_saga_id on an empty DB returns 0).
    let log = LauncherSagaLog::open(f.path()).expect("reopen idempotent");
    assert_eq!(log.max_saga_id().unwrap(), 0);
}

#[test]
fn open_in_memory_works_for_tests() {
    let log = LauncherSagaLog::open_in_memory().expect("in-memory log opens");
    // Trivial round-trip — start a saga, query max_saga_id.
    log.start_saga(1, "saga_a", &serde_json::json!({})).unwrap();
    assert_eq!(log.max_saga_id().unwrap(), 1);
}

#[test]
fn round_trip_completed_saga_writes_expected_rows() {
    let log = LauncherSagaLog::open_in_memory().unwrap();
    let input = serde_json::json!({"label": "win-3"});
    log.start_saga(1, "window_cleanup_cascade", &input).unwrap();
    log.start_step(1, 0, "issue_cmd_host_reap_panes", PipeTarget::Host, &ping(7))
        .unwrap();
    log.finish_step(1, 0, &pong(7, 1)).unwrap();
    log.start_step(1, 1, "issue_cmd_host_drain_pool", PipeTarget::Host, &ping(8))
        .unwrap();
    log.finish_step(1, 1, &pong(8, 2)).unwrap();
    log.terminate_saga(1, SagaOutcome::Completed).unwrap();

    let snap = log.snapshot_recent(10).unwrap();
    assert_eq!(snap.len(), 1);
    assert_eq!(snap[0].saga_id, 1);
    assert_eq!(snap[0].name, "window_cleanup_cascade");
    assert_eq!(snap[0].state, "completed");
    assert!(snap[0].ended_at.is_some());
    assert!(snap[0].failure_reason.is_none());
    assert_eq!(snap[0].step_count, 2);

    // Input JSON round-trips.
    let parsed: serde_json::Value = serde_json::from_str(&snap[0].input_json).unwrap();
    assert_eq!(parsed, input);
}

#[test]
fn start_saga_rejects_duplicate_id() {
    // Same surfacing rule as srv's SagaLog: plain INSERT (not OR
    // REPLACE) so a coordinator-side allocator bug becomes visible
    // instead of silently overwriting an in-progress saga's lifecycle.
    let log = LauncherSagaLog::open_in_memory().unwrap();
    log.start_saga(42, "saga_x", &serde_json::json!({})).unwrap();
    let err = log
        .start_saga(42, "saga_x", &serde_json::json!({}))
        .unwrap_err();
    let msg = format!("{}", err).to_lowercase();
    assert!(
        msg.contains("unique") || msg.contains("constraint"),
        "expected unique-constraint error, got: {msg}",
    );
}

#[test]
fn foreign_keys_enforced_on_step_with_unknown_saga() {
    // `PRAGMA foreign_keys=ON` (configure_and_migrate) catches
    // orphan step rows. Trying to insert a step for a saga that
    // was never started is a configuration bug worth surfacing.
    let log = LauncherSagaLog::open_in_memory().unwrap();
    let err = log
        .start_step(999, 0, "step", PipeTarget::Host, &ping(0))
        .unwrap_err();
    let msg = format!("{}", err).to_lowercase();
    assert!(
        msg.contains("foreign key") || msg.contains("constraint"),
        "expected foreign-key error, got: {msg}",
    );
}

#[test]
fn fail_step_records_reason_in_failure_reason_column() {
    let log = LauncherSagaLog::open_in_memory().unwrap();
    log.start_saga(5, "saga_fail", &serde_json::json!({})).unwrap();
    log.start_step(5, 0, "step_a", PipeTarget::Host, &ping(0)).unwrap();
    log.fail_step(5, 0, "host pipe disconnected").unwrap();

    // Inspect via unresolved_sagas — saga is still 'running' since
    // terminate wasn't called.
    let unresolved = log.unresolved_sagas().unwrap();
    assert_eq!(unresolved.len(), 1);
    let step = &unresolved[0].steps[0];
    assert_eq!(step.state, "failed");
    assert_eq!(step.failure_reason.as_deref(), Some("host pipe disconnected"));
    // Step's target column survives the round-trip.
    assert_eq!(step.target.as_deref(), Some("host"));
}

#[test]
fn unresolved_sagas_returns_only_in_flight_or_failed() {
    let log = LauncherSagaLog::open_in_memory().unwrap();

    // Saga 1: completed (terminal — excluded).
    log.start_saga(1, "s1", &serde_json::json!({})).unwrap();
    log.terminate_saga(1, SagaOutcome::Completed).unwrap();

    // Saga 2: still running (included).
    log.start_saga(2, "s2", &serde_json::json!({"x": 1})).unwrap();
    log.start_step(2, 0, "step_a", PipeTarget::Host, &ping(0)).unwrap();
    log.finish_step(2, 0, &pong(0, 1)).unwrap();

    // Saga 3: failed (terminal-but-included — recovery walker
    // upgrades these to failed_compensation as well; LSD spec §3.5
    // mirrors srv's codex P1 PR #631 round 2 reasoning).
    log.start_saga(3, "s3", &serde_json::json!({})).unwrap();
    log.terminate_saga(
        3,
        SagaOutcome::Failed {
            reason: "boom".to_string(),
        },
    )
    .unwrap();

    // Saga 4: compensating (non-terminal — included).
    log.start_saga(4, "s4", &serde_json::json!({})).unwrap();
    // Manually flip to compensating to mirror what a future
    // class-D/E saga's compensate path would do.
    log.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE launcher_saga SET state = 'compensating' WHERE saga_id = 4",
            [],
        )
        .unwrap();

    // Saga 5: failed_compensation (terminal — excluded; the recovery
    // walker has already done its job for this row).
    log.start_saga(5, "s5", &serde_json::json!({})).unwrap();
    log.mark_failed_compensation(5, "launcher restart").unwrap();

    let unresolved = log.unresolved_sagas().unwrap();
    let mut ids: Vec<u64> = unresolved.iter().map(|u| u.saga_id).collect();
    ids.sort();
    assert_eq!(ids, vec![2, 3, 4]);

    // Saga 2 carries its succeeded step.
    let saga2 = unresolved.iter().find(|u| u.saga_id == 2).unwrap();
    assert_eq!(saga2.state, "running");
    assert_eq!(saga2.steps.len(), 1);
    assert_eq!(saga2.steps[0].state, "succeeded");
    assert_eq!(saga2.steps[0].name, "step_a");
    assert_eq!(saga2.steps[0].target.as_deref(), Some("host"));

    // Saga 4 has no steps yet.
    let saga4 = unresolved.iter().find(|u| u.saga_id == 4).unwrap();
    assert_eq!(saga4.state, "compensating");
    assert!(saga4.steps.is_empty());
}

#[test]
fn mark_failed_compensation_preserves_original_failure_reason() {
    // PR LSD-3 round 3 (codex P2): when a saga is already in `failed`
    // state at restart with a populated failure_reason (e.g. from
    // terminate_saga(SagaOutcome::Failed{reason})), recovery marking
    // must PRESERVE the original reason and APPEND the restart
    // context — overwriting would discard the precise pre-crash
    // cause that operators need for post-mortem.
    //
    // Idempotence is also exercised: calling mark twice doesn't
    // duplicate rows.
    let log = LauncherSagaLog::open_in_memory().unwrap();
    log.start_saga(7, "saga_recov", &serde_json::json!({})).unwrap();
    // First call: failure_reason starts NULL, gets the new reason.
    log.mark_failed_compensation(7, "first attempt").unwrap();
    let snap1 = log.snapshot_recent(10).unwrap();
    assert_eq!(snap1[0].failure_reason.as_deref(), Some("first attempt"));
    // Second call: failure_reason is populated; new reason gets
    // APPENDED (not overwritten).
    log.mark_failed_compensation(7, "second attempt").unwrap();
    let snap2 = log.snapshot_recent(10).unwrap();
    assert_eq!(snap2.len(), 1, "no duplicate rows after repeated marks");
    assert_eq!(snap2[0].state, "failed_compensation");
    assert_eq!(
        snap2[0].failure_reason.as_deref(),
        Some("first attempt | recovered: second attempt")
    );
    // And the saga is no longer "unresolved" once marked.
    let unresolved = log.unresolved_sagas().unwrap();
    assert!(unresolved.iter().all(|s| s.saga_id != 7));
}

#[test]
fn vacuum_removes_only_terminal_rows_older_than_cutoff() {
    // Setup: insert 4 sagas with different states / ages by directly
    // overwriting `started_at` and `ended_at` in SQL (avoids real-
    // wall-clock dependence).
    let log = LauncherSagaLog::open_in_memory().unwrap();
    let old_ts = (Utc::now() - ChronoDuration::days(30)).to_rfc3339();
    let new_ts = Utc::now().to_rfc3339();

    // Saga 1: completed + old → SHOULD be vacuumed.
    log.start_saga(1, "old_completed", &serde_json::json!({})).unwrap();
    log.terminate_saga(1, SagaOutcome::Completed).unwrap();
    log.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE launcher_saga SET ended_at = ?1 WHERE saga_id = 1",
            params![old_ts.as_str()],
        )
        .unwrap();

    // Saga 2: completed + recent → SHOULD survive.
    log.start_saga(2, "recent_completed", &serde_json::json!({})).unwrap();
    log.terminate_saga(2, SagaOutcome::Completed).unwrap();
    log.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE launcher_saga SET ended_at = ?1 WHERE saga_id = 2",
            params![new_ts.as_str()],
        )
        .unwrap();

    // Saga 3: running + has an "old" started_at but ended_at is NULL
    // → MUST survive (in-flight sagas are never vacuumed even if
    // their started_at is ancient — LSD spec §3.6).
    log.start_saga(3, "old_running", &serde_json::json!({})).unwrap();
    log.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE launcher_saga SET started_at = ?1 WHERE saga_id = 3",
            params![old_ts.as_str()],
        )
        .unwrap();

    // Saga 4: failed_compensation + old → SHOULD be vacuumed.
    log.start_saga(4, "old_failed_comp", &serde_json::json!({})).unwrap();
    log.mark_failed_compensation(4, "stale").unwrap();
    log.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE launcher_saga SET ended_at = ?1 WHERE saga_id = 4",
            params![old_ts.as_str()],
        )
        .unwrap();

    // Cutoff = 7 days ago — sagas 1 and 4 should go.
    let cutoff = Utc::now() - ChronoDuration::days(7);
    let removed = log.vacuum_older_than(cutoff).unwrap();
    assert_eq!(removed, 2, "saga 1 + saga 4 vacuumed");

    let snap = log.snapshot_recent(10).unwrap();
    let surviving: Vec<u64> = snap.iter().map(|s| s.saga_id).collect();
    // Saga 2 (recent completed) + saga 3 (still running, never
    // vacuumed) should remain.
    assert!(surviving.contains(&2), "recent completed survives");
    assert!(surviving.contains(&3), "in-flight sagas never vacuumed");
    assert!(!surviving.contains(&1), "old completed removed");
    assert!(!surviving.contains(&4), "old failed_compensation removed");
}

#[test]
fn vacuum_cascades_to_step_rows() {
    // ON DELETE CASCADE on launcher_saga_step.saga_id means vacuuming
    // a saga removes its steps in the same transaction. Verify by
    // checking the step table directly.
    let log = LauncherSagaLog::open_in_memory().unwrap();
    let old_ts = (Utc::now() - ChronoDuration::days(30)).to_rfc3339();
    log.start_saga(1, "to_vacuum", &serde_json::json!({})).unwrap();
    log.start_step(1, 0, "step_a", PipeTarget::Host, &ping(0)).unwrap();
    log.finish_step(1, 0, &pong(0, 1)).unwrap();
    log.terminate_saga(1, SagaOutcome::Completed).unwrap();
    log.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE launcher_saga SET ended_at = ?1 WHERE saga_id = 1",
            params![old_ts.as_str()],
        )
        .unwrap();

    let cutoff = Utc::now() - ChronoDuration::days(7);
    let removed = log.vacuum_older_than(cutoff).unwrap();
    assert_eq!(removed, 1);

    let step_count: i64 = log
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM launcher_saga_step WHERE saga_id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        step_count, 0,
        "ON DELETE CASCADE removed the orphaned step rows"
    );
}

#[test]
fn max_saga_id_returns_highest_persisted() {
    let log = LauncherSagaLog::open_in_memory().unwrap();
    assert_eq!(log.max_saga_id().unwrap(), 0);
    log.start_saga(7, "a", &serde_json::Value::Null).unwrap();
    log.start_saga(42, "b", &serde_json::Value::Null).unwrap();
    log.start_saga(13, "c", &serde_json::Value::Null).unwrap();
    assert_eq!(log.max_saga_id().unwrap(), 42);
}

#[test]
fn pipe_target_serializes_to_canonical_strings() {
    // Pin the canonical strings so future PRs don't accidentally
    // change them — `--diag sagas` and any external tooling parsing
    // launcher-sagas.db rely on these exact tokens.
    assert_eq!(pipe_target_str(PipeTarget::LauncherSelf), "launcher_self");
    assert_eq!(pipe_target_str(PipeTarget::Host), "host");
    assert_eq!(pipe_target_str(PipeTarget::Srv), "srv");
}

#[test]
fn snapshot_recent_orders_most_recent_first_and_respects_limit() {
    let log = LauncherSagaLog::open_in_memory().unwrap();
    for i in 1..=5u64 {
        log.start_saga(i, &format!("saga_{i}"), &serde_json::json!({})).unwrap();
        log.terminate_saga(i, SagaOutcome::Completed).unwrap();
        // Tiny pause so ended_at differs across iterations on
        // platforms with millisecond-resolution clocks.
        std::thread::sleep(std::time::Duration::from_millis(2));
    }

    let snap = log.snapshot_recent(3).unwrap();
    assert_eq!(snap.len(), 3);
    // Most recent first → saga_ids 5, 4, 3.
    assert_eq!(snap[0].saga_id, 5);
    assert_eq!(snap[1].saga_id, 4);
    assert_eq!(snap[2].saga_id, 3);
}

#[test]
fn terminate_saga_records_failure_reason() {
    let log = LauncherSagaLog::open_in_memory().unwrap();
    log.start_saga(1, "saga_fail", &serde_json::json!({})).unwrap();
    log.terminate_saga(
        1,
        SagaOutcome::Failed {
            reason: "evicted by same-kind retrigger".to_string(),
        },
    )
    .unwrap();
    let snap = log.snapshot_recent(1).unwrap();
    assert_eq!(snap[0].state, "failed");
    assert_eq!(
        snap[0].failure_reason.as_deref(),
        Some("evicted by same-kind retrigger")
    );
}
