// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
// STATUS_DONE isn't used by mod.rs's own code after the module split (only
// by host_spawn/container_spawn, which import it separately), so it isn't
// glob-visible via `use super::*` above — import it directly here.
use crate::backend::blockcontroller::STATUS_DONE;

#[test]
fn test_subprocess_controller_new() {
    let ctrl = SubprocessController::new(
        "tab-1".to_string(),
        "block-1".to_string(),
        None,
        None,
        None,
        None,
    );
    assert_eq!(ctrl.controller_type(), BLOCK_CONTROLLER_SUBPROCESS);
    assert_eq!(ctrl.block_id(), "block-1");

    let status = ctrl.get_runtime_status();
    assert_eq!(status.shellprocstatus, STATUS_INIT);
    assert_eq!(status.blockid, "block-1");
}

#[test]
fn test_subprocess_controller_rejects_raw_input() {
    let ctrl = SubprocessController::new(
        "tab-1".to_string(),
        "block-1".to_string(),
        None,
        None,
        None,
        None,
    );
    let result = ctrl.send_input(BlockInputUnion::data(b"hello".to_vec()), None);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("AgentInputCommand"));
}

#[test]
fn test_subprocess_controller_start_is_noop() {
    let ctrl = SubprocessController::new(
        "tab-1".to_string(),
        "block-1".to_string(),
        None,
        None,
        None,
        None,
    );
    let result = ctrl.start(HashMap::new(), None, false);
    assert!(result.is_ok());

    // Still in init state — no auto-start
    let status = ctrl.get_runtime_status();
    assert_eq!(status.shellprocstatus, STATUS_INIT);
}

#[test]
fn test_subprocess_controller_stop_when_idle() {
    let ctrl = SubprocessController::new(
        "tab-1".to_string(),
        "block-1".to_string(),
        None,
        None,
        None,
        None,
    );
    let result = ctrl.stop(true, STATUS_DONE);
    assert!(result.is_ok());

    let status = ctrl.get_runtime_status();
    assert_eq!(status.shellprocstatus, STATUS_DONE);
}

#[test]
fn test_subprocess_controller_session_id_initially_none() {
    let ctrl = SubprocessController::new(
        "tab-1".to_string(),
        "block-1".to_string(),
        None,
        None,
        None,
        None,
    );
    assert!(ctrl.session_id().is_none());
}

#[test]
fn test_subprocess_controller_concurrent_spawn_blocked() {
    let ctrl = SubprocessController::new(
        "tab-1".to_string(),
        "block-1".to_string(),
        None,
        None,
        None,
        None,
    );

    // Manually acquire run lock
    ctrl.run_lock.store(true, Ordering::SeqCst);

    let config = SubprocessSpawnConfig {
        cli_command: "echo".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        message: "test".to_string(),
        resume_flag: String::new(),
        session_id_field: "session_id".to_string(),
        message_id: None,
        session_id: None,
    };

    let result = ctrl.spawn_turn(config);
    // spawn_turn now queues instead of rejecting when busy
    assert!(result.is_ok());

    // Verify the message was queued
    let inner = ctrl.inner.lock().unwrap();
    assert_eq!(inner.pending_messages.len(), 1);
    assert_eq!(inner.pending_messages[0].message, "test");
    drop(inner);

    // Release lock
    ctrl.run_lock.store(false, Ordering::SeqCst);
}

#[test]
fn hydrate_session_id_populates_inner_when_none() {
    // Regression test for the 2026-05-24 "clicking My Agents
    // re-inserts the startup context" report. A fresh
    // SubprocessController is created for the reattached block;
    // its inner.session_id starts as None. The picker reattach
    // flow persists the prior block's session id into
    // `agent:sessionid` meta, the caller plumbs it into
    // `SubprocessSpawnConfig::session_id`, and spawn_turn calls
    // `hydrate_session_id_from_config` before building args.
    // After hydration, the existing args-builder appends
    // `--resume <sid>` on this very first turn — no
    // re-injected startup context.
    let ctrl = SubprocessController::new(
        "tab-1".to_string(),
        "block-reattach".to_string(),
        None,
        None,
        None,
        None,
    );
    assert!(ctrl.inner.lock().unwrap().session_id.is_none());

    ctrl.hydrate_session_id_from_config(Some("prior-sid-from-meta"));
    assert_eq!(
        ctrl.inner.lock().unwrap().session_id.as_deref(),
        Some("prior-sid-from-meta")
    );
}

#[test]
fn hydrate_session_id_is_noop_when_value_already_present() {
    // Hydration is best-effort, not authoritative — it only
    // sets `inner.session_id` when None. The reason isn't
    // captured-id-wins (that's enforced at CAPTURE time below);
    // it's just to avoid re-hydrating on every spawn_turn call
    // within a controller lifetime. A stale value here is fine
    // because the next CLI emit at `record_captured_session_id_inner`
    // will overwrite.
    let ctrl = SubprocessController::new(
        "tab-1".to_string(),
        "block-resume".to_string(),
        None,
        None,
        None,
        None,
    );
    ctrl.inner.lock().unwrap().session_id = Some("captured-sid".to_string());

    ctrl.hydrate_session_id_from_config(Some("different-config-sid"));
    assert_eq!(
        ctrl.inner.lock().unwrap().session_id.as_deref(),
        Some("captured-sid"),
        "hydration must not overwrite an existing value"
    );
}

#[test]
fn record_captured_overwrites_hydrated_value() {
    // The CLI is authoritative for session id once it speaks.
    // Codex P1 on PR #1018 first cut: my original
    // `if !already_captured` guard in the stdout reader meant
    // that a hydrated (possibly stale) session id would lock
    // out every subsequent CLI-emitted value, so a wrong
    // `--resume <stale>` would be passed forever. The fix
    // (`record_captured_session_id_inner`) always overwrites
    // and returns whether the value changed.
    let ctrl = SubprocessController::new(
        "tab-1".to_string(),
        "block-overwrite".to_string(),
        None,
        None,
        None,
        None,
    );
    ctrl.hydrate_session_id_from_config(Some("stale-hydrated-sid"));
    assert_eq!(
        ctrl.session_id().as_deref(),
        Some("stale-hydrated-sid")
    );

    let changed = ctrl.record_captured_session_id("authoritative-sid");
    assert!(changed, "value differs from hydrated; must report changed");
    assert_eq!(
        ctrl.session_id().as_deref(),
        Some("authoritative-sid"),
        "CLI-emitted id must overwrite hydrated value"
    );
}

#[test]
fn record_captured_dedups_same_value() {
    // Real CLI streams emit `session_id` on every NDJSON frame,
    // not just the first. The dedup is a perf knob (skips the
    // meta-update broadcast on repeats), not a correctness
    // gate — captured-id is still authoritative on first emit.
    let ctrl = SubprocessController::new(
        "tab-1".to_string(),
        "block-dedup".to_string(),
        None,
        None,
        None,
        None,
    );
    assert!(ctrl.record_captured_session_id("sid-1"));
    assert!(!ctrl.record_captured_session_id("sid-1"),
        "second call with same value must return false (no broadcast)");
    assert_eq!(ctrl.session_id().as_deref(), Some("sid-1"));
}

#[test]
fn record_captured_ignores_empty() {
    // Defensive: empty string from a malformed CLI emit must
    // not clear a valid prior value.
    let ctrl = SubprocessController::new(
        "tab-1".to_string(),
        "block-empty".to_string(),
        None,
        None,
        None,
        None,
    );
    ctrl.record_captured_session_id("real-sid");
    assert!(!ctrl.record_captured_session_id(""),
        "empty CLI emit must be ignored");
    assert_eq!(ctrl.session_id().as_deref(), Some("real-sid"));
}

#[test]
fn hydrate_session_id_ignores_empty_and_none() {
    // Greenfield launches pass `None` (or `Some("")` if the
    // caller didn't filter) — hydration must be a no-op in
    // either case so inner.session_id stays None until the CLI
    // captures its own.
    let ctrl = SubprocessController::new(
        "tab-1".to_string(),
        "block-greenfield".to_string(),
        None,
        None,
        None,
        None,
    );
    ctrl.hydrate_session_id_from_config(None);
    assert!(ctrl.inner.lock().unwrap().session_id.is_none());

    ctrl.hydrate_session_id_from_config(Some(""));
    assert!(ctrl.inner.lock().unwrap().session_id.is_none());
}

#[test]
fn spawn_turn_preserves_session_id_in_queued_config() {
    // When the controller is busy, spawn_turn queues the config
    // for the drain-from-queue path. The hydration ONLY runs on
    // the direct-spawn path (after try_lock_run), so the queued
    // config must carry session_id through unchanged for the
    // drain path's recursive call to see it.
    let ctrl = SubprocessController::new(
        "tab-1".to_string(),
        "block-queued".to_string(),
        None,
        None,
        None,
        None,
    );
    ctrl.run_lock.store(true, Ordering::SeqCst);

    let config = SubprocessSpawnConfig {
        cli_command: "claude".to_string(),
        cli_args: vec!["-p".to_string()],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        message: "hi".to_string(),
        resume_flag: "--resume".to_string(),
        session_id_field: "session_id".to_string(),
        message_id: None,
        session_id: Some("prior-sid".to_string()),
    };
    let _ = ctrl.spawn_turn(config);

    let inner = ctrl.inner.lock().unwrap();
    assert_eq!(inner.pending_messages.len(), 1);
    assert_eq!(
        inner.pending_messages[0].session_id.as_deref(),
        Some("prior-sid"),
    );
    // Hydration didn't run yet — direct-spawn path was bypassed
    // by the busy lock; the drain will hydrate when it dequeues.
    assert!(inner.session_id.is_none());
}
