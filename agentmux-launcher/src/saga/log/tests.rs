// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Contract tests for the in-memory saga registry (Pillar 1 Step 6).
// Ported from the durable-SQLite era: the behavioral contract
// (lifecycle states, unresolved semantics, failure-reason preservation,
// snapshot ordering) is unchanged; the SQLite-specific tests (schema
// migration idempotence, PRAGMA foreign_keys, vacuum) were replaced by
// the retention-cap test, since those concerns no longer exist.

use super::*;
use agentmux_common::ipc::{Command, Event};

fn ping(nonce: u64) -> Command {
    Command::Ping { nonce }
}

fn pong(nonce: u64, version: u64) -> Event {
    Event::Pong { nonce, version }
}

#[test]
fn new_registry_is_empty() {
    let log = LauncherSagaLog::new();
    assert_eq!(log.max_saga_id().unwrap(), 0);
    assert!(log.snapshot_recent(10).unwrap().is_empty());
    assert!(log.unresolved_sagas().unwrap().is_empty());
}

#[test]
fn round_trip_completed_saga_writes_expected_rows() {
    let log = LauncherSagaLog::new();
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
    // Same surfacing rule as the durable log's plain INSERT (not OR
    // REPLACE): a coordinator-side allocator bug becomes visible
    // instead of silently overwriting an in-progress saga's lifecycle.
    let log = LauncherSagaLog::new();
    log.start_saga(42, "saga_x", &serde_json::json!({})).unwrap();
    let err = log
        .start_saga(42, "saga_x", &serde_json::json!({}))
        .unwrap_err();
    assert!(
        matches!(err, LogError::DuplicateSagaId(42)),
        "expected DuplicateSagaId(42), got: {err}",
    );
}

#[test]
fn step_for_unknown_saga_is_a_noop() {
    // The durable log surfaced this via a FOREIGN KEY constraint; the
    // registry treats it like its UPDATE-on-missing-row siblings — a
    // silent no-op. (The coordinator only ever writes steps for sagas
    // it just started, so this is defense-in-depth, not a live path.)
    let log = LauncherSagaLog::new();
    log.start_step(999, 0, "step", PipeTarget::Host, &ping(0))
        .unwrap();
    assert!(log.get_saga_steps(999).unwrap().is_empty());
    assert!(log.snapshot_recent(10).unwrap().is_empty());
}

#[test]
fn fail_step_records_reason() {
    let log = LauncherSagaLog::new();
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
    // Step's target survives the round-trip.
    assert_eq!(step.target.as_deref(), Some("host"));
}

#[test]
fn unresolved_sagas_returns_only_in_flight_or_failed() {
    let log = LauncherSagaLog::new();

    // Saga 1: completed (terminal — excluded).
    log.start_saga(1, "s1", &serde_json::json!({})).unwrap();
    log.terminate_saga(1, SagaOutcome::Completed).unwrap();

    // Saga 2: still running (included).
    log.start_saga(2, "s2", &serde_json::json!({"x": 1})).unwrap();
    log.start_step(2, 0, "step_a", PipeTarget::Host, &ping(0)).unwrap();
    log.finish_step(2, 0, &pong(0, 1)).unwrap();

    // Saga 3: failed (terminal-but-included — mirrors the durable-era
    // contract, srv codex P1 PR #631 round 2 reasoning).
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
    log.set_state_for_test(4, "compensating");

    // Saga 5: failed_compensation (terminal — excluded).
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
    // Failure-reason preservation contract from the durable era
    // (codex P2 PR #647 round 1): a populated failure_reason is
    // APPENDED to, never overwritten — the precise original cause
    // stays visible for post-mortem. Idempotence across repeated
    // marks is exercised too.
    let log = LauncherSagaLog::new();
    log.start_saga(7, "saga_recov", &serde_json::json!({})).unwrap();
    // First call: failure_reason starts None, gets the new reason.
    log.mark_failed_compensation(7, "first attempt").unwrap();
    let snap1 = log.snapshot_recent(10).unwrap();
    assert_eq!(snap1[0].failure_reason.as_deref(), Some("first attempt"));
    // Second call: failure_reason is populated; new reason APPENDED.
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
fn max_saga_id_returns_highest() {
    let log = LauncherSagaLog::new();
    log.start_saga(3, "a", &serde_json::json!({})).unwrap();
    log.start_saga(9, "b", &serde_json::json!({})).unwrap();
    log.start_saga(5, "c", &serde_json::json!({})).unwrap();
    assert_eq!(log.max_saga_id().unwrap(), 9);
}

#[test]
fn pipe_target_serializes_to_canonical_strings() {
    assert_eq!(pipe_target_str(PipeTarget::LauncherSelf), "launcher_self");
    assert_eq!(pipe_target_str(PipeTarget::Host), "host");
    assert_eq!(pipe_target_str(PipeTarget::Srv), "srv");
}

#[test]
fn snapshot_recent_orders_most_recent_first_and_respects_limit() {
    let log = LauncherSagaLog::new();
    // Terminated sagas get ended_at stamps in call order; the later
    // termination must sort first. Same-timestamp ties (plausible at
    // RFC3339 precision on a fast machine) break by saga_id DESC,
    // which matches call order here too.
    log.start_saga(1, "older", &serde_json::json!({})).unwrap();
    log.terminate_saga(1, SagaOutcome::Completed).unwrap();
    log.start_saga(2, "newer", &serde_json::json!({})).unwrap();
    log.terminate_saga(2, SagaOutcome::Completed).unwrap();

    let snap = log.snapshot_recent(1).unwrap();
    assert_eq!(snap.len(), 1, "limit respected");
    assert_eq!(snap[0].saga_id, 2, "most recent first");
}

#[test]
fn terminate_saga_records_failure_reason() {
    let log = LauncherSagaLog::new();
    log.start_saga(1, "s", &serde_json::json!({})).unwrap();
    log.terminate_saga(
        1,
        SagaOutcome::Failed {
            reason: "saga timeout".to_string(),
        },
    )
    .unwrap();
    let snap = log.snapshot_recent(10).unwrap();
    assert_eq!(snap[0].state, "failed");
    assert_eq!(snap[0].failure_reason.as_deref(), Some("saga timeout"));
}

#[test]
fn retention_cap_evicts_oldest_terminal_only() {
    // Replaces the durable log's vacuum tests: terminal sagas beyond
    // the cap are evicted oldest-first; in-flight sagas are NEVER
    // evicted regardless of age.
    let log = LauncherSagaLog::new();

    // Saga 1: in-flight, oldest of all — must survive.
    log.start_saga(1, "inflight", &serde_json::json!({})).unwrap();

    // Fill with terminal sagas to one past the cap.
    let first_terminal = 2u64;
    let last_terminal = first_terminal + TERMINAL_RETENTION_CAP as u64; // cap+1 terminals
    for id in first_terminal..=last_terminal {
        log.start_saga(id, "t", &serde_json::json!({})).unwrap();
        log.terminate_saga(id, SagaOutcome::Completed).unwrap();
    }
    // One more start triggers retention enforcement.
    log.start_saga(last_terminal + 1, "trigger", &serde_json::json!({}))
        .unwrap();

    // Oldest terminal (saga 2) evicted; in-flight saga 1 survives.
    let snap = log.snapshot_recent(usize::MAX).unwrap();
    let ids: Vec<u64> = snap.iter().map(|s| s.saga_id).collect();
    assert!(!ids.contains(&first_terminal), "oldest terminal evicted");
    assert!(ids.contains(&1), "in-flight saga never evicted");
    assert!(ids.contains(&last_terminal), "newest terminal kept");
    let terminal_count = snap
        .iter()
        .filter(|s| s.state != "running")
        .count();
    assert_eq!(terminal_count, TERMINAL_RETENTION_CAP);
}
