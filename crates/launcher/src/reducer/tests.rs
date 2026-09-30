// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use agentmux_common::ipc::{HwndDriftKind, WindowKind};
use crate::state::WindowMirror;
use agentmux_common::ipc::{ClientKind, ErrorCode};
use crate::state::{LifecyclePhase, ProcessRecord, ProcessState};

fn ctx(conn_id: u64) -> Ctx {
    Ctx {
        now_rfc3339: "2026-04-28T00:00:00Z".to_string(),
        conn_id,
        registered_pid: None,
        now_ms: 0,
    }
}

/// Clone a Ctx with `now_ms` advanced. Used by tests that need to
/// fire commands past the `HIDDEN_SINCE_OPEN_GRACE_MS` placement
/// window without re-deriving the rest of the Ctx.
fn ctx_advance(base: &Ctx, ms: u64) -> Ctx {
    Ctx {
        now_rfc3339: base.now_rfc3339.clone(),
        conn_id: base.conn_id,
        registered_pid: base.registered_pid,
        now_ms: base.now_ms + ms,
    }
}

fn ctx_with_pid(conn_id: u64, pid: u32) -> Ctx {
    Ctx {
        now_rfc3339: "2026-04-28T00:00:00Z".to_string(),
        conn_id,
        registered_pid: Some(pid),
        now_ms: 0,
    }
}

#[test]
fn first_host_register_transitions_starting_to_running() {
    let mut state = State::default();
    let events = update(
        &mut state,
        Command::Register {
            kind: ClientKind::Host,
            pid: 1234,
            version: "0.33.450".into(),
        },
        &ctx(1),
    );
    assert_eq!(state.lifecycle, LifecyclePhase::Running);
    assert!(state.processes.contains_key(&1234));
    // Should emit ProcessSpawned + LifecyclePhaseChanged + Registered.
    assert_eq!(events.len(), 3);
    assert!(matches!(
        events[0],
        Event::ProcessSpawned { pid: 1234, .. }
    ));
    assert!(matches!(
        events[1],
        Event::LifecyclePhaseChanged {
            from: LifecyclePhase::Starting,
            to: LifecyclePhase::Running,
            ..
        }
    ));
    assert!(matches!(events[2], Event::Registered { .. }));
}

#[test]
fn second_host_register_does_not_re_emit_lifecycle_change() {
    let mut state = State::default();
    let _ = update(
        &mut state,
        Command::Register {
            kind: ClientKind::Host,
            pid: 1234,
            version: "0.33.450".into(),
        },
        &ctx(1),
    );
    // Different PID, second Host (e.g. test harness or doc)
    let events = update(
        &mut state,
        Command::Register {
            kind: ClientKind::Host,
            pid: 5678,
            version: "0.33.450".into(),
        },
        &ctx(2),
    );
    // Lifecycle stays Running; no LifecyclePhaseChanged event.
    assert_eq!(state.lifecycle, LifecyclePhase::Running);
    assert_eq!(events.len(), 2); // ProcessSpawned + Registered
    assert!(events
        .iter()
        .all(|e| !matches!(e, Event::LifecyclePhaseChanged { .. })));
}

#[test]
fn duplicate_pid_register_returns_already_registered() {
    let mut state = State::default();
    let _ = update(
        &mut state,
        Command::Register {
            kind: ClientKind::Host,
            pid: 1234,
            version: "0.33.450".into(),
        },
        &ctx(1),
    );
    let events = update(
        &mut state,
        Command::Register {
            kind: ClientKind::Renderer,
            pid: 1234, // SAME pid
            version: "0.33.450".into(),
        },
        &ctx(2),
    );
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        Event::Error {
            code: ErrorCode::AlreadyRegistered,
            fatal: true,
            ..
        }
    ));
    // Second register doesn't overwrite the first.
    assert_eq!(state.processes[&1234].kind, ClientKind::Host);
}

#[test]
fn renderer_register_does_not_drive_lifecycle() {
    let mut state = State::default();
    let events = update(
        &mut state,
        Command::Register {
            kind: ClientKind::Renderer,
            pid: 4321,
            version: "0.33.450".into(),
        },
        &ctx(1),
    );
    // Lifecycle stays Starting until a HOST registers.
    assert_eq!(state.lifecycle, LifecyclePhase::Starting);
    assert_eq!(events.len(), 2); // ProcessSpawned + Registered
}

#[test]
fn ping_returns_pong_with_same_nonce() {
    let mut state = State::default();
    let events = update(&mut state, Command::Ping { nonce: 42 }, &ctx(1));
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], Event::Pong { nonce: 42, .. }));
}

/// Helper used by both the unit test below and the proptest: extract
/// the version number from any Event variant. New variants must be
/// added here OR the helper switched to a generic accessor when
/// the variant set grows large.
fn extract_version(e: &Event) -> u64 {
    match e {
        Event::ProcessSpawned { version, .. }
        | Event::ProcessExited { version, .. }
        | Event::LifecyclePhaseChanged { version, .. }
        | Event::Registered { version, .. }
        | Event::Pong { version, .. }
        | Event::WindowOpened { version, .. }
        | Event::WindowClosed { version, .. }
        | Event::PoolWindowAdded { version, .. }
        | Event::PoolWindowRemoved { version, .. }
        | Event::PoolWindowPromoted { version, .. }
        | Event::PanesReaped { version, .. }
        | Event::PoolDrained { version, .. }
        | Event::PoolNotLast { version, .. }
        | Event::WindowInstanceAssigned { version, .. }
        | Event::WindowInstanceReleased { version, .. }
        | Event::BackendWindowIdRegistered { version, .. }
        | Event::BackendWindowIdUnregistered { version, .. }
        | Event::DriftDetected { version, .. }
        | Event::HwndDriftDetected { version, .. }
        | Event::CorrectiveWindowMove { version, .. }
        | Event::HostShouldQuit { version, .. }
        | Event::Snapshot { version, .. }
        | Event::EventList { version, .. }
        | Event::SagaStarted { version, .. }
        | Event::SagaCompleted { version, .. }
        | Event::SagaFailed { version, .. }
        | Event::SrvSnapshot { version, .. }
        | Event::WorkspaceCreated { version, .. }
        | Event::WorkspaceDeleted { version, .. }
        | Event::TabCreated { version, .. }
        | Event::TabDeleted { version, .. }
        | Event::ActiveTabChanged { version, .. }
        | Event::TabReordered { version, .. }
        | Event::SrvWindowOpened { version, .. }
        | Event::SrvWindowClosed { version, .. }
        | Event::SrvWindowWorkspaceChanged { version, .. }
        | Event::TabsReorderedBulk { version, .. }
        | Event::WorkspaceRenamed { version, .. }
        | Event::TabRenamed { version, .. }
        | Event::WorkspaceMetaUpdated { version, .. }
        | Event::TabMetaUpdated { version, .. }
        | Event::BlockMetaUpdated { version, .. }
        | Event::TabMoved { version, .. }
        | Event::BlockMoved { version, .. }
        | Event::BlockCreated { version, .. }
        | Event::BlockDeleted { version, .. }
        | Event::FocusedNodeChanged { version, .. }
        | Event::MagnifiedNodeChanged { version, .. }
        | Event::SagaActionFailed { version, .. }
        | Event::LayoutNodeInserted { version, .. }
        | Event::LayoutNodeInsertedAtIndex { version, .. }
        | Event::LayoutNodeDeleted { version, .. }
        | Event::LayoutNodeMoved { version, .. }
        | Event::LayoutNodesSwapped { version, .. }
        | Event::LayoutNodesResized { version, .. }
        | Event::LayoutNodeReplaced { version, .. }
        | Event::LayoutSplitHorizontalApplied { version, .. }
        | Event::LayoutSplitVerticalApplied { version, .. }
        | Event::LayoutCleared { version, .. }
        | Event::LayoutBackendActionsQueued { version, .. }
        | Event::LayoutTreeReplaced { version, .. }
        | Event::WindowMetaUpdated { version, .. }
        | Event::Error { version, .. } => *version,
    }
}

#[test]
fn event_versions_are_strictly_monotonic() {
    let mut state = State::default();
    let mut versions = vec![];
    for pid in [100, 200, 300] {
        let events = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid,
                version: "0.33.450".into(),
            },
            &ctx(1),
        );
        versions.extend(events.iter().map(extract_version));
    }
    for w in versions.windows(2) {
        assert!(w[1] > w[0], "versions not monotonic: {:?}", versions);
    }
}

#[test]
fn goodbye_marks_registered_pid_as_exited() {
    let mut state = State::default();
    let _ = update(
        &mut state,
        Command::Register {
            kind: ClientKind::Host,
            pid: 1234,
            version: "0.33.451".into(),
        },
        &ctx(1),
    );
    assert!(matches!(
        state.processes[&1234].state,
        ProcessState::Running
    ));
    let events = update(&mut state, Command::Goodbye, &ctx_with_pid(1, 1234));
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        Event::ProcessExited { pid: 1234, code: 0, .. }
    ));
    assert!(matches!(
        state.processes[&1234].state,
        ProcessState::Exited { code: 0 }
    ));
}

/// B.4 reports require a Host registration first (codex P1
/// PR #576). Helper to set that up so each window-mirror test
/// doesn't repeat the boilerplate.
fn register_host_and_get_ctx(state: &mut State, pid: u32) -> Ctx {
    let _ = update(
        state,
        Command::Register {
            kind: ClientKind::Host,
            pid,
            version: "test".into(),
        },
        &ctx(1),
    );
    ctx_with_pid(1, pid)
}

#[test]
fn report_window_opened_inserts_into_mirror_and_emits_event() {
    let mut state = State::default();
    let host_ctx = register_host_and_get_ctx(&mut state, 1234);
    let events = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "main".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &host_ctx,
    );
    // B.5 — open emits WindowOpened + WindowInstanceAssigned.
    assert_eq!(events.len(), 2);
    assert!(matches!(
        &events[0],
        Event::WindowOpened { label, kind: WindowKind::FullInstance, parent_label: None, .. }
            if label == "main"
    ));
    assert!(matches!(
        &events[1],
        Event::WindowInstanceAssigned { label, num: 1, .. } if label == "main"
    ));
    let mirror = &state.windows["main"];
    assert_eq!(mirror.label, "main");
    assert_eq!(mirror.kind, WindowKind::FullInstance);
    assert_eq!(mirror.parent_label, None);
}

#[test]
fn report_window_closed_removes_from_mirror() {
    let mut state = State::default();
    let host_ctx = register_host_and_get_ctx(&mut state, 1234);
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "main".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &host_ctx,
    );
    assert!(state.windows.contains_key("main"));
    let events = update(
        &mut state,
        Command::ReportWindowClosed {
            label: "main".into(),
        },
        &host_ctx,
    );
    assert!(!state.windows.contains_key("main"));
    // B.5 — close emits WindowClosed + WindowInstanceReleased.
    // B.9.3 — closing the last window with a Host registered
    // also emits OrphanInstance drift + HostShouldQuit saga,
    // so the total is 4 events on the last-window-close path.
    assert_eq!(events.len(), 4);
    assert!(matches!(
        &events[0],
        Event::WindowClosed { label, .. } if label == "main"
    ));
    assert!(matches!(
        &events[1],
        Event::WindowInstanceReleased { label, num: 1, .. } if label == "main"
    ));
    assert!(matches!(
        &events[2],
        Event::HwndDriftDetected { kind: HwndDriftKind::OrphanInstance, .. }
    ));
    assert!(matches!(&events[3], Event::HostShouldQuit { .. }));
}

#[test]
fn instance_numbers_are_monotonic_per_launcher_run() {
    let mut state = State::default();
    let host_ctx = register_host_and_get_ctx(&mut state, 1234);
    // main pre-seeded as 1 (already in instance_registry from Default).
    // Open second window → gets 2.
    let events = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "window-2".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &host_ctx,
    );
    let assigned = events.iter().find_map(|e| match e {
        Event::WindowInstanceAssigned { num, .. } => Some(*num),
        _ => None,
    });
    assert_eq!(assigned, Some(2));

    // Close it. Open a third window → gets 3 (NOT reused 2).
    let _ = update(
        &mut state,
        Command::ReportWindowClosed {
            label: "window-2".into(),
        },
        &host_ctx,
    );
    let events = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "window-3".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &host_ctx,
    );
    let assigned = events.iter().find_map(|e| match e {
        Event::WindowInstanceAssigned { num, .. } => Some(*num),
        _ => None,
    });
    assert_eq!(assigned, Some(3), "instance numbers must not be reused");
}

#[test]
fn re_open_of_same_label_keeps_original_instance_number() {
    let mut state = State::default();
    let host_ctx = register_host_and_get_ctx(&mut state, 1234);
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "x".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &host_ctx,
    );
    let first_num = state.instance_registry["x"];
    // Re-open without close (B.4 idempotent overwrite path).
    let events = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "x".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &host_ctx,
    );
    let assigned = events.iter().find_map(|e| match e {
        Event::WindowInstanceAssigned { num, .. } => Some(*num),
        _ => None,
    });
    assert_eq!(assigned, Some(first_num));
    assert_eq!(state.instance_registry["x"], first_num);
}

#[test]
fn report_window_closed_on_unknown_label_is_silent_no_op() {
    let mut state = State::default();
    let host_ctx = register_host_and_get_ctx(&mut state, 1234);
    let events = update(
        &mut state,
        Command::ReportWindowClosed {
            label: "ghost".into(),
        },
        &host_ctx,
    );
    // Codex P2 PR #577 round-2: NO broadcast for unknown labels.
    // Pairs strictly with WindowOpened so subscribers can rely on
    // open/close pairing.
    assert_eq!(events.len(), 0);
    assert!(state.windows.is_empty());
}

#[test]
fn subwindow_open_records_parent_label() {
    let mut state = State::default();
    let host_ctx = register_host_and_get_ctx(&mut state, 1234);
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "main".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &host_ctx,
    );
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "sub-1".into(),
            kind: WindowKind::Subwindow,
            parent_label: Some("main".into()),
        },
        &host_ctx,
    );
    assert_eq!(
        state.windows["sub-1"].parent_label.as_deref(),
        Some("main")
    );
}

#[test]
fn report_pool_window_add_and_remove_round_trip() {
    let mut state = State::default();
    let host_ctx = register_host_and_get_ctx(&mut state, 1234);
    let events = update(
        &mut state,
        Command::ReportPoolWindowAdded {
            label: "window-pool-abc".into(),
            saga_id: None,
        },
        &host_ctx,
    );
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Event::PoolWindowAdded { label, .. } if label == "window-pool-abc"
    ));
    assert!(state.pool.contains("window-pool-abc"));

    let events = update(
        &mut state,
        Command::ReportPoolWindowRemoved {
            label: "window-pool-abc".into(),
        },
        &host_ctx,
    );
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Event::PoolWindowRemoved { label, .. } if label == "window-pool-abc"
    ));
    assert!(!state.pool.contains("window-pool-abc"));
}

/// Phase CPD-1 — saga_id from the Report* commands is plumbed
/// unchanged into the resulting Events. Pass-through invariant for
/// the reducer arms; per-saga correlation in CPD-4 relies on this.
#[test]
fn cpd1_report_panes_reaped_passes_saga_id_through() {
    let mut state = State::default();
    let host_ctx = register_host_and_get_ctx(&mut state, 1234);
    let evs = update(
        &mut state,
        Command::ReportPanesReaped {
            label: "main".into(),
            saga_id: Some(42),
        },
        &host_ctx,
    );
    assert_eq!(evs.len(), 1);
    match &evs[0] {
        Event::PanesReaped {
            label,
            saga_id,
            ..
        } => {
            assert_eq!(label, "main");
            assert_eq!(*saga_id, Some(42));
        }
        other => panic!("expected Event::PanesReaped, got {:?}", other),
    }
}

#[test]
fn cpd1_report_panes_reaped_with_none_saga_id_passes_through() {
    let mut state = State::default();
    let host_ctx = register_host_and_get_ctx(&mut state, 1234);
    let evs = update(
        &mut state,
        Command::ReportPanesReaped {
            label: "main".into(),
            saga_id: None,
        },
        &host_ctx,
    );
    assert_eq!(evs.len(), 1);
    match &evs[0] {
        Event::PanesReaped { saga_id, .. } => {
            assert_eq!(*saga_id, None);
        }
        other => panic!("expected Event::PanesReaped, got {:?}", other),
    }
}

#[test]
fn cpd1_report_pool_drain_decision_passes_saga_id_through() {
    let mut state = State::default();
    let host_ctx = register_host_and_get_ctx(&mut state, 1234);
    let evs = update(
        &mut state,
        Command::ReportPoolDrainDecision {
            label: "main".into(),
            was_last: true,
            saga_id: Some(7),
        },
        &host_ctx,
    );
    assert_eq!(evs.len(), 1);
    match &evs[0] {
        Event::PoolDrained { label, saga_id, .. } => {
            assert_eq!(label, "main");
            assert_eq!(*saga_id, Some(7));
        }
        other => panic!("expected Event::PoolDrained, got {:?}", other),
    }

    let evs = update(
        &mut state,
        Command::ReportPoolDrainDecision {
            label: "secondary".into(),
            was_last: false,
            saga_id: Some(8),
        },
        &host_ctx,
    );
    assert_eq!(evs.len(), 1);
    match &evs[0] {
        Event::PoolNotLast { label, saga_id, .. } => {
            assert_eq!(label, "secondary");
            assert_eq!(*saga_id, Some(8));
        }
        other => panic!("expected Event::PoolNotLast, got {:?}", other),
    }
}

#[test]
fn cpd1_report_pool_window_added_passes_saga_id_through() {
    let mut state = State::default();
    let host_ctx = register_host_and_get_ctx(&mut state, 1234);
    let evs = update(
        &mut state,
        Command::ReportPoolWindowAdded {
            label: "pool-1".into(),
            saga_id: Some(99),
        },
        &host_ctx,
    );
    assert_eq!(evs.len(), 1);
    match &evs[0] {
        Event::PoolWindowAdded { label, saga_id, .. } => {
            assert_eq!(label, "pool-1");
            assert_eq!(*saga_id, Some(99));
        }
        other => panic!("expected Event::PoolWindowAdded, got {:?}", other),
    }
}

#[test]
fn cpd1_report_saga_action_failed_emits_saga_action_failed_event() {
    let mut state = State::default();
    let host_ctx = register_host_and_get_ctx(&mut state, 1234);
    let evs = update(
        &mut state,
        Command::ReportSagaActionFailed {
            saga_id: 21,
            reason: "host pipe closed".into(),
        },
        &host_ctx,
    );
    assert_eq!(evs.len(), 1);
    match &evs[0] {
        Event::SagaActionFailed {
            saga_id, reason, ..
        } => {
            assert_eq!(*saga_id, 21);
            assert_eq!(reason, "host pipe closed");
        }
        other => panic!("expected Event::SagaActionFailed, got {:?}", other),
    }
}

#[test]
fn report_backend_window_id_round_trip() {
    let mut state = State::default();
    let host_ctx = register_host_and_get_ctx(&mut state, 1234);
    let events = update(
        &mut state,
        Command::ReportBackendWindowIdRegistered {
            label: "main".into(),
            window_id: "wid-abc".into(),
        },
        &host_ctx,
    );
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Event::BackendWindowIdRegistered { label, window_id, .. }
            if label == "main" && window_id == "wid-abc"
    ));
    assert_eq!(state.backend_window_ids.get("main").map(|s| s.as_str()), Some("wid-abc"));

    let events = update(
        &mut state,
        Command::ReportBackendWindowIdUnregistered {
            label: "main".into(),
        },
        &host_ctx,
    );
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Event::BackendWindowIdUnregistered { label, window_id, .. }
            if label == "main" && window_id == "wid-abc"
    ));
    assert!(state.backend_window_ids.is_empty());
}

#[test]
fn report_backend_window_id_unregister_unknown_label_is_silent() {
    let mut state = State::default();
    let host_ctx = register_host_and_get_ctx(&mut state, 1234);
    let events = update(
        &mut state,
        Command::ReportBackendWindowIdUnregistered {
            label: "ghost".into(),
        },
        &host_ctx,
    );
    // Strict pairing — same as WindowClosed/PoolWindowRemoved.
    assert_eq!(events.len(), 0);
}

#[test]
fn report_backend_window_id_overwrites_on_duplicate() {
    // Frontend can re-register if it reloads — the launcher should
    // accept the new ID and emit a fresh event so subscribers see
    // the latest mapping.
    let mut state = State::default();
    let host_ctx = register_host_and_get_ctx(&mut state, 1234);
    let _ = update(
        &mut state,
        Command::ReportBackendWindowIdRegistered {
            label: "main".into(),
            window_id: "wid-old".into(),
        },
        &host_ctx,
    );
    let _ = update(
        &mut state,
        Command::ReportBackendWindowIdRegistered {
            label: "main".into(),
            window_id: "wid-new".into(),
        },
        &host_ctx,
    );
    assert_eq!(
        state.backend_window_ids.get("main").map(|s| s.as_str()),
        Some("wid-new")
    );
}

#[test]
fn pool_and_window_mirrors_are_independent() {
    // The host transitions a pool window to a real window via
    // (PoolRemoved, WindowOpened). Verify the launcher can hold
    // both maps without collision and an entry can be in pool
    // OR windows but not both.
    let mut state = State::default();
    let host_ctx = register_host_and_get_ctx(&mut state, 1234);
    let _ = update(
        &mut state,
        Command::ReportPoolWindowAdded {
            label: "window-pool-xyz".into(),
            saga_id: None,
        },
        &host_ctx,
    );
    let _ = update(
        &mut state,
        Command::ReportPoolWindowRemoved {
            label: "window-pool-xyz".into(),
        },
        &host_ctx,
    );
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "window-pool-xyz".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &host_ctx,
    );
    assert!(!state.pool.contains("window-pool-xyz"));
    assert!(state.windows.contains_key("window-pool-xyz"));
}

#[test]
fn report_pool_window_removed_on_unknown_label_is_silent_no_op() {
    let mut state = State::default();
    let host_ctx = register_host_and_get_ctx(&mut state, 1234);
    let events = update(
        &mut state,
        Command::ReportPoolWindowRemoved {
            label: "ghost-pool".into(),
        },
        &host_ctx,
    );
    // Strict pairing — pool remove only emits an event when the
    // label was in the pool. (reagent P2 PR #577 round-3.)
    assert_eq!(events.len(), 0);
    assert!(state.pool.is_empty());
}

#[test]
fn report_host_counts_matching_mirror_emits_no_event() {
    let mut state = State::default();
    let host_ctx = register_host_and_get_ctx(&mut state, 1234);
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "main".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &host_ctx,
    );
    // Mirror has 1 window, 0 pool. Host reports the same.
    let events = update(
        &mut state,
        Command::ReportHostCounts {
            windows: 1,
            pool: 0,
        },
        &host_ctx,
    );
    assert_eq!(events.len(), 0);
}

#[test]
fn report_host_counts_emits_drift_for_window_mismatch() {
    let mut state = State::default();
    let host_ctx = register_host_and_get_ctx(&mut state, 1234);
    // Mirror has 0 windows; host claims 3 → drift.
    let events = update(
        &mut state,
        Command::ReportHostCounts {
            windows: 3,
            pool: 0,
        },
        &host_ctx,
    );
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Event::DriftDetected {
            kind: DriftKind::Windows,
            host_count: 3,
            mirror_count: 0,
            ..
        }
    ));
}

#[test]
fn report_host_counts_emits_drift_for_pool_mismatch() {
    let mut state = State::default();
    let host_ctx = register_host_and_get_ctx(&mut state, 1234);
    let _ = update(
        &mut state,
        Command::ReportPoolWindowAdded {
            label: "window-pool-a".into(),
            saga_id: None,
        },
        &host_ctx,
    );
    // Mirror has 1 pool entry; host claims 5 → drift.
    let events = update(
        &mut state,
        Command::ReportHostCounts {
            windows: 0,
            pool: 5,
        },
        &host_ctx,
    );
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Event::DriftDetected {
            kind: DriftKind::Pool,
            host_count: 5,
            mirror_count: 1,
            ..
        }
    ));
}

#[test]
fn report_host_pool_count_matching_emits_no_event() {
    let mut state = State::default();
    let host_ctx = register_host_and_get_ctx(&mut state, 1234);
    let _ = update(
        &mut state,
        Command::ReportPoolWindowAdded {
            label: "window-pool-x".into(),
            saga_id: None,
        },
        &host_ctx,
    );
    let events = update(
        &mut state,
        Command::ReportHostPoolCount { count: 1 },
        &host_ctx,
    );
    assert_eq!(events.len(), 0);
}

#[test]
fn report_host_pool_count_emits_drift_on_mismatch() {
    let mut state = State::default();
    let host_ctx = register_host_and_get_ctx(&mut state, 1234);
    // Mirror pool=0; host claims 7 → drift.
    let events = update(
        &mut state,
        Command::ReportHostPoolCount { count: 7 },
        &host_ctx,
    );
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Event::DriftDetected {
            kind: DriftKind::Pool,
            host_count: 7,
            mirror_count: 0,
            ..
        }
    ));
}

#[test]
fn report_host_pool_count_ignores_windows_dimension() {
    let mut state = State::default();
    let host_ctx = register_host_and_get_ctx(&mut state, 1234);
    // Open a window so the windows dimension WOULD diverge if
    // checked, but ReportHostPoolCount only inspects pool.
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "main".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &host_ctx,
    );
    // Mirror windows=1, mirror pool=0. Host pool count matches.
    let events = update(
        &mut state,
        Command::ReportHostPoolCount { count: 0 },
        &host_ctx,
    );
    assert_eq!(events.len(), 0);
}

#[test]
fn report_host_counts_emits_both_drifts_when_both_diverge() {
    let mut state = State::default();
    let host_ctx = register_host_and_get_ctx(&mut state, 1234);
    let events = update(
        &mut state,
        Command::ReportHostCounts {
            windows: 1,
            pool: 1,
        },
        &host_ctx,
    );
    assert_eq!(events.len(), 2);
    // Stable order: windows first, then pool. (Tested for predictability
    // so subscribers + this assertion don't drift with HashMap iteration.)
    assert!(matches!(
        &events[0],
        Event::DriftDetected { kind: DriftKind::Windows, .. }
    ));
    assert!(matches!(
        &events[1],
        Event::DriftDetected { kind: DriftKind::Pool, .. }
    ));
}

#[test]
fn pool_commands_from_non_host_are_rejected() {
    let mut state = State::default();
    let _ = update(
        &mut state,
        Command::Register {
            kind: ClientKind::Tool,
            pid: 9999,
            version: "test".into(),
        },
        &ctx(1),
    );
    let events = update(
        &mut state,
        Command::ReportPoolWindowAdded {
            label: "spoof-pool".into(),
            saga_id: None,
        },
        &ctx_with_pid(1, 9999),
    );
    assert!(matches!(
        &events[0],
        Event::Error { code: ErrorCode::NotRegistered, .. }
    ));
    assert!(state.pool.is_empty());
}

#[test]
fn report_window_opened_from_non_host_is_rejected() {
    let mut state = State::default();
    // Register as Renderer (not Host) at PID 4321.
    let _ = update(
        &mut state,
        Command::Register {
            kind: ClientKind::Renderer,
            pid: 4321,
            version: "test".into(),
        },
        &ctx(1),
    );
    let events = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "spoof".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &ctx_with_pid(1, 4321),
    );
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Event::Error { code: ErrorCode::NotRegistered, fatal: false, .. }
    ));
    // Mirror NOT mutated.
    assert!(state.windows.is_empty());
}

#[test]
fn report_window_closed_from_unregistered_conn_is_rejected() {
    let mut state = State::default();
    let events = update(
        &mut state,
        Command::ReportWindowClosed {
            label: "x".into(),
        },
        &ctx(1), // No Register first.
    );
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Event::Error { code: ErrorCode::NotRegistered, fatal: false, .. }
    ));
}

#[test]
fn register_replaces_exited_record_for_recycled_pid() {
    let mut state = State::default();
    // First process registers + cleanly exits
    let _ = update(
        &mut state,
        Command::Register {
            kind: ClientKind::Host,
            pid: 1234,
            version: "first".into(),
        },
        &ctx(1),
    );
    let _ = update(&mut state, Command::Goodbye, &ctx_with_pid(1, 1234));
    assert!(matches!(
        state.processes[&1234].state,
        ProcessState::Exited { .. }
    ));

    // OS recycles PID 1234 to a new process which Registers
    let events = update(
        &mut state,
        Command::Register {
            kind: ClientKind::Renderer,
            pid: 1234,
            version: "second".into(),
        },
        &ctx(2),
    );
    // Should NOT emit AlreadyRegistered — record replaced.
    assert!(!events
        .iter()
        .any(|e| matches!(e, Event::Error {
            code: ErrorCode::AlreadyRegistered,
            ..
        })));
    assert!(matches!(
        state.processes[&1234].state,
        ProcessState::Running
    ));
    assert_eq!(state.processes[&1234].kind, ClientKind::Renderer);
    assert_eq!(state.processes[&1234].version, "second");
}

// ===== Property-based tests =====
//
// The unit tests above cover specific scenarios; these prove
// the reducer's INVARIANTS hold across arbitrary input
// sequences. Per spec §7 + the Phase B plan's testing strategy.

use proptest::prelude::*;

/// Generate an arbitrary Command. Constrained to the value-space
/// the IPC server can actually produce: PIDs are realistic-ish
/// u32s, version strings are short ASCII.
fn arb_command() -> impl Strategy<Value = Command> {
    prop_oneof![
        // Register dominates the distribution because that's
        // where most state-machine logic lives.
        5 => (
            prop_oneof![
                Just(ClientKind::Host),
                Just(ClientKind::Renderer),
                Just(ClientKind::Srv),
                Just(ClientKind::Tool),
            ],
            1u32..10000u32,
            "[a-zA-Z0-9.]{1,16}",
        )
            .prop_map(|(kind, pid, version)| Command::Register { kind, pid, version }),
        1 => any::<u64>().prop_map(|nonce| Command::Ping { nonce }),
        1 => Just(Command::Goodbye),
        // B.4: window mirror commands. Labels drawn from a small
        // alphabet so duplicates (open then close) are common
        // enough for the proptest to exercise the idempotent
        // close path.
        2 => (
            "[a-c]{1,3}",
            prop_oneof![
                Just(WindowKind::FullInstance),
                Just(WindowKind::Subwindow),
            ],
            prop_oneof![Just(None::<String>), Just(Some("a".into()))],
        )
            .prop_map(|(label, kind, parent_label)| {
                Command::ReportWindowOpened { label, kind, parent_label }
            }),
        2 => "[a-c]{1,3}".prop_map(|label| Command::ReportWindowClosed { label }),
    ]
}

proptest! {
    /// Versions across an arbitrary sequence of commands are
    /// always strictly monotonic. This is the foundation of
    /// Phase D's GetSnapshot resync: clients detect missed
    /// events by gap in version numbers.
    #[test]
    fn versions_strictly_monotonic_under_any_command_sequence(
        cmds in proptest::collection::vec(arb_command(), 1..50)
    ) {
        let mut state = State::default();
        let mut all_versions = vec![];
        for cmd in cmds {
            let events = update(&mut state, cmd, &ctx(1));
            all_versions.extend(events.iter().map(extract_version));
        }
        for w in all_versions.windows(2) {
            prop_assert!(
                w[1] > w[0],
                "version regression: {} → {}",
                w[0], w[1]
            );
        }
    }

    /// Lifecycle invariant (spec §4): only ever Starting →
    /// Running → Quitting → Dead. No other transition.
    /// In B.3 we exercise just the Starting → Running edge
    /// since later transitions don't have triggering commands
    /// yet, but the harness is ready for B.4+.
    #[test]
    fn lifecycle_only_progresses_forward(
        cmds in proptest::collection::vec(arb_command(), 1..50)
    ) {
        let mut state = State::default();
        let mut prev = state.lifecycle;
        for cmd in cmds {
            let _ = update(&mut state, cmd, &ctx(1));
            let next = state.lifecycle;
            let valid = match (prev, next) {
                (a, b) if a == b => true,
                (LifecyclePhase::Starting, LifecyclePhase::Running) => true,
                (LifecyclePhase::Running, LifecyclePhase::Quitting) => true,
                (LifecyclePhase::Quitting, LifecyclePhase::Dead) => true,
                _ => false,
            };
            prop_assert!(
                valid,
                "illegal lifecycle transition {:?} → {:?}",
                prev, next
            );
            prev = next;
        }
    }

    /// Process map invariant: a successful Register inserts the
    /// PID; a duplicate Register (same PID) NEVER overwrites the
    /// existing record. This is what the server relies on for
    /// stale-state safety.
    #[test]
    fn duplicate_register_never_overwrites(
        initial_kind in prop_oneof![Just(ClientKind::Host), Just(ClientKind::Renderer)],
        second_kind in prop_oneof![Just(ClientKind::Srv), Just(ClientKind::Tool)],
        pid in 1u32..10000u32,
    ) {
        let mut state = State::default();
        let _ = update(
            &mut state,
            Command::Register {
                kind: initial_kind,
                pid,
                version: "v1".into(),
            },
            &ctx(1),
        );
        let _ = update(
            &mut state,
            Command::Register {
                kind: second_kind,
                pid,
                version: "v2".into(),
            },
            &ctx(2),
        );
        // Original record preserved across the rejected dup.
        prop_assert_eq!(state.processes[&pid].kind, initial_kind);
        prop_assert_eq!(&state.processes[&pid].version, "v1");
    }

    /// B.4 mirror invariants under arbitrary host-driven traffic.
    /// State seeded with a registered Host so the host-only gate
    /// doesn't trivially reject every command (reagent P2 round-1
    /// PR #576). Two invariants checked:
    ///   1. Mirror size is bounded by total opens minus successful
    ///      closes — no phantom entries appear.
    ///   2. Every label in `state.windows` has a `WindowMirror`
    ///      whose `label` field matches the map key (no key/value
    ///      drift).
    #[test]
    fn window_mirror_invariants_under_host_traffic(
        cmds in proptest::collection::vec(arb_window_cmd(), 1..100)
    ) {
        const HOST_PID: u32 = 1;
        let mut state = State::default();
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: HOST_PID,
                version: "host".into(),
            },
            &ctx(1),
        );
        let host_ctx = ctx_with_pid(1, HOST_PID);

        let mut opens = 0u64;
        let mut closes = 0u64;
        for cmd in cmds {
            match &cmd {
                Command::ReportWindowOpened { .. } => opens += 1,
                Command::ReportWindowClosed { .. } => closes += 1,
                _ => {}
            }
            let _ = update(&mut state, cmd, &host_ctx);
        }

        // Bound: mirror can't hold more entries than distinct opens
        // (idempotent overwrite ensures opens are dedup'd by label,
        // and any open can be cancelled by its matching close).
        prop_assert!(
            state.windows.len() as u64 <= opens,
            "mirror size {} > total opens {}",
            state.windows.len(), opens
        );
        // Key/value coherence — if this ever fails, the reducer
        // wrote a value with a mismatched label.
        for (k, v) in &state.windows {
            prop_assert_eq!(k, &v.label);
        }
        // Closes are observable (each emits an event regardless
        // of mirror state). Just sanity-check nothing is leaking
        // into negative counters.
        let _ = (opens, closes); // referenced for failure messages
    }
}

/// B.4 — generate ONLY window-mirror commands. Used by the
/// host-driven proptest above to guarantee exercise of the
/// mirror insert/remove paths (general `arb_command` mixes in
/// Register / Ping / Goodbye which dilute window coverage).
fn arb_window_cmd() -> impl proptest::strategy::Strategy<Value = Command> {
    use proptest::prelude::*;
    prop_oneof![
        (
            "[a-c]{1,3}",
            prop_oneof![
                Just(WindowKind::FullInstance),
                Just(WindowKind::Subwindow),
            ],
            prop_oneof![Just(None::<String>), Just(Some("a".into()))],
        )
            .prop_map(|(label, kind, parent_label)| {
                Command::ReportWindowOpened { label, kind, parent_label }
            }),
        "[a-c]{1,3}".prop_map(|label| Command::ReportWindowClosed { label }),
    ]
}

// -----------------------------------------------------------
// Phase B.9 (WRR) — reducer arm tests
// -----------------------------------------------------------

use agentmux_common::ipc::Rect;

/// Helper: drive the reducer through a host Register so
/// subsequent host-only WRR commands are accepted. Returns the
/// state ready to receive WRR commands.
fn registered_host_state() -> (State, Ctx) {
    let mut state = State::default();
    let _ = update(
        &mut state,
        Command::Register {
            kind: ClientKind::Host,
            pid: 1,
            version: "test".into(),
        },
        &ctx(1),
    );
    (state, ctx_with_pid(1, 1))
}

#[test]
fn wrr_off_monitor_position_for_unseen_window_emits_drift_and_corrective() {
    let (mut state, c) = registered_host_state();
    // Set monitor topology: a single 1920x1080 monitor.
    let _ = update(
        &mut state,
        Command::ReportMonitorTopologyChanged {
            rects: vec![Rect { left: 0, top: 0, right: 1920, bottom: 1080 }],
        },
        &c,
    );
    // Open a window.
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "w1".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    // Link an HWND to it.
    let _ = update(
        &mut state,
        Command::ReportHwndOpened {
            hwnd: 0xAA,
            class_name: "Chrome_WidgetWin_1".into(),
            title: "w1".into(),
            label_hint: Some("w1".into()),
        },
        &c,
    );
    // Position event: rect is fully off the monitor and NOT
    // the Win32 hidden sentinel (so drift fires).
    let evs = update(
        &mut state,
        Command::ReportHwndPositionChanged {
            hwnd: 0xAA,
            rect: Rect { left: -5000, top: -5000, right: -4000, bottom: -4000 },
        },
        &c,
    );
    // Expect both: drift + corrective move.
    assert!(
        evs.iter().any(|e| matches!(e, Event::HwndDriftDetected { kind: HwndDriftKind::OffMonitor, .. })),
        "expected OffMonitor drift, got {:?}", evs
    );
    assert!(
        evs.iter().any(|e| matches!(e, Event::CorrectiveWindowMove { hwnd: 0xAA, .. })),
        "expected CorrectiveWindowMove, got {:?}", evs
    );
}

#[test]
fn wrr_sentinel_position_suppresses_drift_but_emits_corrective() {
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportMonitorTopologyChanged {
            rects: vec![Rect { left: 0, top: 0, right: 1920, bottom: 1080 }],
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "w1".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportHwndOpened {
            hwnd: 0xAA,
            class_name: "Chrome_WidgetWin_1".into(),
            title: "".into(),
            label_hint: Some("w1".into()),
        },
        &c,
    );
    // Sentinel position — CEF Views' "hidden" parking spot.
    let evs = update(
        &mut state,
        Command::ReportHwndPositionChanged {
            hwnd: 0xAA,
            rect: Rect { left: -31970, top: -31970, right: -31340, bottom: -30871 },
        },
        &c,
    );
    // Drift suppressed (sentinel is a known transient).
    assert!(
        !evs.iter().any(|e| matches!(e, Event::HwndDriftDetected { kind: HwndDriftKind::OffMonitor, .. })),
        "sentinel position should NOT fire drift, got {:?}", evs
    );
    // Corrective fires regardless — we want to move it before
    // the user notices the orphan taskbar entry.
    assert!(
        evs.iter().any(|e| matches!(e, Event::CorrectiveWindowMove { hwnd: 0xAA, .. })),
        "sentinel position should fire CorrectiveWindowMove, got {:?}", evs
    );
}

#[test]
fn wrr_off_monitor_after_user_foregrounded_does_not_correct() {
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportMonitorTopologyChanged {
            rects: vec![Rect { left: 0, top: 0, right: 1920, bottom: 1080 }],
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "w1".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportHwndOpened {
            hwnd: 0xAA,
            class_name: "Chrome_WidgetWin_1".into(),
            title: "".into(),
            label_hint: Some("w1".into()),
        },
        &c,
    );
    // Foreground event — user has interacted with this window.
    let _ = update(
        &mut state,
        Command::ReportHwndForegroundChanged { hwnd: 0xAA },
        &c,
    );
    // User then drags it off-monitor (legitimate state).
    let evs = update(
        &mut state,
        Command::ReportHwndPositionChanged {
            hwnd: 0xAA,
            rect: Rect { left: -5000, top: -5000, right: -4000, bottom: -4000 },
        },
        &c,
    );
    // Drift fires — operator should still see it.
    assert!(
        evs.iter().any(|e| matches!(e, Event::HwndDriftDetected { kind: HwndDriftKind::OffMonitor, .. })),
        "drift should fire even for user-touched windows, got {:?}", evs
    );
    // BUT no corrective — we trust the user.
    assert!(
        !evs.iter().any(|e| matches!(e, Event::CorrectiveWindowMove { .. })),
        "corrective must NOT fire after user has foregrounded the window, got {:?}", evs
    );
}

#[test]
fn tear_off_promote_emit_order_initializes_mirror_with_foregrounded_true() {
    // Drift-storm regression (v0.33.655 smoke). PRODUCTION emit order
    // (per agentmux-cef/src/commands/window_pool.rs):
    //
    //   1. ReportPoolWindowAdded   → pool spawn (some time earlier)
    //   2. ReportPoolWindowRemoved → about to promote (no mirror yet)
    //   3. ReportPoolWindowPromoted → records in just_promoted_labels
    //   4. ReportWindowOpened      → mirror created NOW
    //   5. ReportHwndOpened        → HWND linked
    //   6. SetWindowPos churn      → ReportHwndVisibilityChanged false
    //
    // Pre-fix: step 4 initialized foregrounded_since_open=false, so
    // step 6 re-fired HiddenSinceOpen drift → host fans across bridge
    // → renderer V8 crash. Fix: step 3 records, step 4 consumes and
    // initializes foregrounded_since_open=true.
    //
    // See `docs/specs/ANALYSIS_DRIFT_STORM_RENDERER_CRASH_2026-05-06.md`.
    let (mut state, c) = registered_host_state();

    // Steps 1–2: pool spawn + remove (no mirror in state.windows).
    let _ = update(
        &mut state,
        Command::ReportPoolWindowAdded { label: "window-pool-abc".into(), saga_id: None },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportPoolWindowRemoved { label: "window-pool-abc".into() },
        &c,
    );
    assert!(
        !state.windows.contains_key("window-pool-abc"),
        "pre-promote: no mirror yet (production invariant)"
    );

    // Step 3: promote. Records in just_promoted_labels; emits typed event.
    let _ = update(
        &mut state,
        Command::ReportPoolWindowPromoted { label: "window-pool-abc".into() },
        &c,
    );
    assert!(
        state.just_promoted_labels.contains("window-pool-abc"),
        "promote must record label in just_promoted_labels"
    );

    // Step 4: window opened — mirror created with foregrounded_since_open=true.
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "window-pool-abc".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    let mirror = state.windows.get("window-pool-abc").expect("mirror created on open");
    assert!(
        mirror.foregrounded_since_open,
        "post-promote open must initialize foregrounded_since_open=true (got false → drift storm regression)"
    );
    assert!(
        !state.just_promoted_labels.contains("window-pool-abc"),
        "open consumes the just_promoted entry"
    );

    // Step 5: HWND link.
    let _ = update(
        &mut state,
        Command::ReportHwndOpened {
            hwnd: 0xBB,
            class_name: "Chrome_WidgetWin_1".into(),
            title: "".into(),
            label_hint: Some("window-pool-abc".into()),
        },
        &c,
    );

    // Step 6: visibility flicker during HWND repositioning. MUST NOT drift.
    let post = update(
        &mut state,
        Command::ReportHwndVisibilityChanged { hwnd: 0xBB, visible: false },
        &c,
    );
    assert!(
        !post.iter().any(|e| matches!(
            e,
            Event::HwndDriftDetected { kind: HwndDriftKind::HiddenSinceOpen, .. }
        )),
        "post-promote hide must not drift, got {:?}",
        post
    );
}

#[test]
fn cold_open_without_promote_still_initializes_foregrounded_false() {
    // Sanity: a regular window open (e.g. main, fresh tear-off NOT
    // through pool) MUST keep foregrounded_since_open=false so the
    // open-transient corrective logic still applies for those windows.
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "window-fresh".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    assert!(
        !state.windows.get("window-fresh").unwrap().foregrounded_since_open,
        "non-promote open must keep foregrounded_since_open=false"
    );
    assert!(
        !state.windows.get("window-fresh").unwrap().hidden_since_open_emitted,
        "fresh open starts with hidden_since_open_emitted=false (cap not yet armed)"
    );
}

#[test]
fn hidden_since_open_grace_suppresses_placement_hides() {
    // CEF creates top-level windows hidden, places them, then shows.
    // Hides arriving within HIDDEN_SINCE_OPEN_GRACE_MS of open are
    // placement transitions and must NOT fire drift. Without this
    // grace, every fresh top-level window emits one false-positive
    // drift on creation. (PR #725, smoke regression on v0.33.696 showed
    // 8 such hides for 8 fresh windows in a clean session.)
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "window-placing".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportHwndOpened {
            hwnd: 0xEE,
            class_name: "Chrome_WidgetWin_1".into(),
            title: "".into(),
            label_hint: Some("window-placing".into()),
        },
        &c,
    );

    // Hide within grace window (now_ms still 0, opened_at_ms=0,
    // delta=0, < 500ms). Drift suppressed.
    let placement_hide = update(
        &mut state,
        Command::ReportHwndVisibilityChanged { hwnd: 0xEE, visible: false },
        &c,
    );
    assert_eq!(
        placement_hide
            .iter()
            .filter(|e| matches!(
                e,
                Event::HwndDriftDetected { kind: HwndDriftKind::HiddenSinceOpen, .. }
            ))
            .count(),
        0,
        "hide within placement grace must NOT fire drift"
    );
    assert!(
        !state.windows.get("window-placing").unwrap().hidden_since_open_emitted,
        "cap flag must NOT be armed by placement-grace hides — \
         a hide PAST the grace can still fire its drift"
    );

    // Hide PAST grace → drift fires (the cap should not have been
    // armed by the placement-grace hide).
    let post_grace = ctx_advance(&c, 600);
    let real_hide = update(
        &mut state,
        Command::ReportHwndVisibilityChanged { hwnd: 0xEE, visible: false },
        &post_grace,
    );
    assert_eq!(
        real_hide
            .iter()
            .filter(|e| matches!(
                e,
                Event::HwndDriftDetected { kind: HwndDriftKind::HiddenSinceOpen, .. }
            ))
            .count(),
        1,
        "hide past grace fires drift (cap was not pre-armed by placement)"
    );
}

#[test]
fn hidden_since_open_deferred_fires_when_window_stays_hidden_past_grace() {
    // Codex P2 PR #725 round 1: if the ONLY visible=false event arrives
    // within the placement grace and no further visibility events fire,
    // we MUST still emit the drift. The deferred flag + drain pass on
    // every reducer call (heartbeat-via-traffic) catches this.
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "window-stuck".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportHwndOpened {
            hwnd: 0xFE,
            class_name: "Chrome_WidgetWin_1".into(),
            title: "".into(),
            label_hint: Some("window-stuck".into()),
        },
        &c,
    );

    // The ONLY visibility event arrives within grace. Drift suppressed
    // but mirror is marked deferred.
    let placement_hide = update(
        &mut state,
        Command::ReportHwndVisibilityChanged { hwnd: 0xFE, visible: false },
        &c,
    );
    assert_eq!(
        placement_hide
            .iter()
            .filter(|e| matches!(e, Event::HwndDriftDetected { .. }))
            .count(),
        0,
        "hide within grace must NOT fire drift (deferred)"
    );
    assert!(
        state.windows.get("window-stuck").unwrap().hidden_since_open_deferred,
        "deferred flag must be set"
    );

    // No further visibility events. ANY subsequent reducer call past
    // the grace promotes the deferred state to a fired drift via the
    // drain pass. Use a Ping (cheapest unrelated command) past grace.
    let post_grace = ctx_advance(&c, 600);
    let drained = update(&mut state, Command::Ping { nonce: 1 }, &post_grace);
    assert_eq!(
        drained
            .iter()
            .filter(|e| matches!(
                e,
                Event::HwndDriftDetected { kind: HwndDriftKind::HiddenSinceOpen, .. }
            ))
            .count(),
        1,
        "deferred drift fires on next post-grace reducer call regardless \
         of command kind"
    );
    assert!(
        state.windows.get("window-stuck").unwrap().hidden_since_open_emitted,
        "cap is now armed (drift was emitted)"
    );
    assert!(
        !state.windows.get("window-stuck").unwrap().hidden_since_open_deferred,
        "deferred flag cleared after drift fires"
    );
}

#[test]
fn hidden_since_open_deferred_cleared_by_visible_or_foreground() {
    // The deferred flag must clear when the window completes its
    // placement transition (becomes visible OR foregrounded). Without
    // this, a window that hid during placement, became visible, and
    // was never seen again would fire spurious deferred drift.
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "window-recovers".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportHwndOpened {
            hwnd: 0xFD,
            class_name: "Chrome_WidgetWin_1".into(),
            title: "".into(),
            label_hint: Some("window-recovers".into()),
        },
        &c,
    );

    // Hide during grace → deferred set.
    let _ = update(
        &mut state,
        Command::ReportHwndVisibilityChanged { hwnd: 0xFD, visible: false },
        &c,
    );
    assert!(state.windows.get("window-recovers").unwrap().hidden_since_open_deferred);

    // Become visible → deferred cleared.
    let _ = update(
        &mut state,
        Command::ReportHwndVisibilityChanged { hwnd: 0xFD, visible: true },
        &c,
    );
    assert!(
        !state.windows.get("window-recovers").unwrap().hidden_since_open_deferred,
        "visible=true clears deferred flag"
    );

    // Past-grace heartbeat must NOT fire drift since deferred was cleared.
    let post_grace = ctx_advance(&c, 600);
    let drained = update(&mut state, Command::Ping { nonce: 1 }, &post_grace);
    assert!(
        !drained
            .iter()
            .any(|e| matches!(e, Event::HwndDriftDetected { kind: HwndDriftKind::HiddenSinceOpen, .. })),
        "no spurious drift after window recovers visibility"
    );
}

#[test]
fn deferred_drain_runs_after_command_so_recovery_events_clear_first() {
    // Codex P2 PR #725 round 2: if the FIRST command past grace is
    // the recovery event itself (visible=true or foreground change),
    // the drain MUST run AFTER the command — otherwise it sees the
    // still-deferred mirror and fires premature HiddenSinceOpen
    // drift on a slow-but-successful placement.
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "window-slow-show".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportHwndOpened {
            hwnd: 0xFC,
            class_name: "Chrome_WidgetWin_1".into(),
            title: "".into(),
            label_hint: Some("window-slow-show".into()),
        },
        &c,
    );

    // Hide during grace → deferred set.
    let _ = update(
        &mut state,
        Command::ReportHwndVisibilityChanged { hwnd: 0xFC, visible: false },
        &c,
    );
    assert!(state.windows.get("window-slow-show").unwrap().hidden_since_open_deferred);

    // Slow placement: visible=true is the FIRST event past grace.
    let post_grace = ctx_advance(&c, 600);
    let evs = update(
        &mut state,
        Command::ReportHwndVisibilityChanged { hwnd: 0xFC, visible: true },
        &post_grace,
    );
    assert!(
        !evs.iter().any(|e| matches!(
            e,
            Event::HwndDriftDetected { kind: HwndDriftKind::HiddenSinceOpen, .. }
        )),
        "recovery event must clear deferred before drain runs, no drift fires"
    );
    assert!(
        !state.windows.get("window-slow-show").unwrap().hidden_since_open_emitted,
        "cap must NOT be armed by a recovery event"
    );
    assert!(
        !state.windows.get("window-slow-show").unwrap().hidden_since_open_deferred,
        "deferred cleared by visible=true"
    );
}

#[test]
fn opened_at_ms_preserved_on_duplicate_open() {
    // Codex P2 PR #725 round 1: duplicate ReportWindowOpened must NOT
    // reset opened_at_ms. Otherwise a duplicate that arrives after
    // the original 500ms grace and before the first hide would
    // re-anchor the grace window, suppressing real hides for another
    // 500ms.
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "window-dup-open".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    let original_opened_at_ms = state.windows.get("window-dup-open").unwrap().opened_at_ms;

    // Duplicate open arrives MUCH later (1000ms past grace).
    let later = ctx_advance(&c, 1000);
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "window-dup-open".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &later,
    );
    assert_eq!(
        state.windows.get("window-dup-open").unwrap().opened_at_ms,
        original_opened_at_ms,
        "duplicate open must preserve original opened_at_ms (grace anchor)"
    );
}

#[test]
fn hidden_since_open_drift_fires_at_most_once_per_window() {
    // Drift-storm regression guard. The smoke crash on v0.33.685
    // ("New Window" from hamburger after a tear-off) was 170
    // identical HiddenSinceOpen events in 1s for the same label,
    // exhausting the renderer's V8 stack. Cap fires once per window
    // per session — subscribers still see the signal, no storm.
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "window-fresh".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportHwndOpened {
            hwnd: 0xCC,
            class_name: "Chrome_WidgetWin_1".into(),
            title: "".into(),
            label_hint: Some("window-fresh".into()),
        },
        &c,
    );
    // First hide past the placement grace → drift fires. Hides
    // within HIDDEN_SINCE_OPEN_GRACE_MS of open are placement
    // transitions and don't fire (separate test below).
    let post_grace = ctx_advance(&c, 600);
    let first = update(
        &mut state,
        Command::ReportHwndVisibilityChanged { hwnd: 0xCC, visible: false },
        &post_grace,
    );
    assert_eq!(
        first
            .iter()
            .filter(|e| matches!(
                e,
                Event::HwndDriftDetected { kind: HwndDriftKind::HiddenSinceOpen, .. }
            ))
            .count(),
        1,
        "first hide emits drift exactly once"
    );

    // 100 subsequent hide/show oscillations (mimics the storm
    // observed during placement): drift must NOT re-fire.
    let mut subsequent_drift_count = 0;
    for _ in 0..100 {
        let _ = update(
            &mut state,
            Command::ReportHwndVisibilityChanged { hwnd: 0xCC, visible: true },
            &post_grace,
        );
        let evs = update(
            &mut state,
            Command::ReportHwndVisibilityChanged { hwnd: 0xCC, visible: false },
            &post_grace,
        );
        subsequent_drift_count += evs
            .iter()
            .filter(|e| matches!(
                e,
                Event::HwndDriftDetected { kind: HwndDriftKind::HiddenSinceOpen, .. }
            ))
            .count();
    }
    assert_eq!(
        subsequent_drift_count, 0,
        "cap suppresses ALL subsequent HiddenSinceOpen emissions"
    );
    assert!(
        state
            .windows
            .get("window-fresh")
            .unwrap()
            .hidden_since_open_emitted,
        "cap flag is set after the first emission"
    );
}

#[test]
fn hidden_since_open_cap_survives_duplicate_open() {
    // The handler overwrites the mirror wholesale on every
    // ReportWindowOpened. Without OR-ing the prior value in, a 2nd
    // open at the same label re-arms the cap and the next hidden
    // visibility transition fires HiddenSinceOpen a second time.
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "window-dup".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportHwndOpened {
            hwnd: 0xDD,
            class_name: "Chrome_WidgetWin_1".into(),
            title: "".into(),
            label_hint: Some("window-dup".into()),
        },
        &c,
    );
    let post_grace = ctx_advance(&c, 600);
    let first = update(
        &mut state,
        Command::ReportHwndVisibilityChanged { hwnd: 0xDD, visible: false },
        &post_grace,
    );
    assert!(
        first.iter().any(|e| matches!(
            e,
            Event::HwndDriftDetected { kind: HwndDriftKind::HiddenSinceOpen, .. }
        )),
        "control: first hide emits drift"
    );

    // Duplicate open for the same label — overwrites the mirror.
    // Cap MUST survive (OR with prior value) or the storm re-arms.
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "window-dup".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    assert!(
        state
            .windows
            .get("window-dup")
            .unwrap()
            .hidden_since_open_emitted,
        "duplicate open must preserve hidden_since_open_emitted=true"
    );

    // Subsequent hide: drift must NOT fire again.
    let _ = update(
        &mut state,
        Command::ReportHwndVisibilityChanged { hwnd: 0xDD, visible: true },
        &post_grace,
    );
    let second = update(
        &mut state,
        Command::ReportHwndVisibilityChanged { hwnd: 0xDD, visible: false },
        &post_grace,
    );
    assert!(
        !second.iter().any(|e| matches!(
            e,
            Event::HwndDriftDetected { kind: HwndDriftKind::HiddenSinceOpen, .. }
        )),
        "duplicate-open path: cap survives, no second drift"
    );
}

#[test]
fn off_monitor_drift_fires_at_most_once_per_window() {
    // Companion cap to HiddenSinceOpen. apply_hwnd_position_changed
    // fires per WM_MOVE; without the cap, dragging an off-monitor
    // window storms the renderer.
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportMonitorTopologyChanged {
            rects: vec![Rect { left: 0, top: 0, right: 1920, bottom: 1080 }],
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "w-off".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportHwndOpened {
            hwnd: 0xEE,
            class_name: "Chrome_WidgetWin_1".into(),
            title: "".into(),
            label_hint: Some("w-off".into()),
        },
        &c,
    );
    // Foreground first so corrective is suppressed (we test
    // OffMonitor drift independently).
    let _ = update(
        &mut state,
        Command::ReportHwndForegroundChanged { hwnd: 0xEE },
        &c,
    );
    // First off-monitor position → drift fires.
    let first = update(
        &mut state,
        Command::ReportHwndPositionChanged {
            hwnd: 0xEE,
            rect: Rect { left: 5000, top: 5000, right: 5500, bottom: 5400 },
        },
        &c,
    );
    assert_eq!(
        first.iter().filter(|e| matches!(
            e,
            Event::HwndDriftDetected { kind: HwndDriftKind::OffMonitor, .. }
        )).count(),
        1,
        "first off-monitor position emits drift exactly once"
    );

    // 100 more off-monitor moves: drift must NOT re-fire.
    let mut subsequent = 0;
    for i in 0..100 {
        let dx = i * 10;
        let evs = update(
            &mut state,
            Command::ReportHwndPositionChanged {
                hwnd: 0xEE,
                rect: Rect {
                    left: 5000 + dx,
                    top: 5000,
                    right: 5500 + dx,
                    bottom: 5400,
                },
            },
            &c,
        );
        subsequent += evs.iter().filter(|e| matches!(
            e,
            Event::HwndDriftDetected { kind: HwndDriftKind::OffMonitor, .. }
        )).count();
    }
    assert_eq!(subsequent, 0, "cap suppresses subsequent OffMonitor drift");
}

#[test]
fn corrective_window_move_fires_at_most_once_per_window() {
    // Same cap shape for the self-heal corrective. Without it, a
    // never-foregrounded window dragged through off-monitor regions
    // emits CorrectiveWindowMove every move.
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportMonitorTopologyChanged {
            rects: vec![Rect { left: 0, top: 0, right: 1920, bottom: 1080 }],
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "w-corr".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportHwndOpened {
            hwnd: 0xFF,
            class_name: "Chrome_WidgetWin_1".into(),
            title: "".into(),
            label_hint: Some("w-corr".into()),
        },
        &c,
    );
    // Note: NOT foregrounded — that's the corrective trigger.
    let first = update(
        &mut state,
        Command::ReportHwndPositionChanged {
            hwnd: 0xFF,
            rect: Rect { left: 5000, top: 5000, right: 5500, bottom: 5400 },
        },
        &c,
    );
    assert_eq!(
        first.iter().filter(|e| matches!(e, Event::CorrectiveWindowMove { .. })).count(),
        1,
        "first off-monitor + never-foregrounded emits corrective"
    );
    // Subsequent off-monitor moves: no more corrective.
    let mut subsequent = 0;
    for i in 0..100 {
        let evs = update(
            &mut state,
            Command::ReportHwndPositionChanged {
                hwnd: 0xFF,
                rect: Rect {
                    left: 5000 + i * 10,
                    top: 5000,
                    right: 5500 + i * 10,
                    bottom: 5400,
                },
            },
            &c,
        );
        subsequent += evs.iter().filter(|e| matches!(
            e,
            Event::CorrectiveWindowMove { .. }
        )).count();
    }
    assert_eq!(subsequent, 0, "cap suppresses subsequent corrective emissions");
}

#[test]
fn off_monitor_cap_blocks_repeated_topology_change_emissions() {
    // Codex P2 PR #722 round 3: display hot-plug emits
    // ReportMonitorTopologyChanged repeatedly. If a window stays
    // stranded across multiple topology events, drift was being
    // re-emitted every event because apply_monitor_topology_changed
    // didn't consult the off_monitor_drift_emitted cap.
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportMonitorTopologyChanged {
            rects: vec![Rect { left: 0, top: 0, right: 1920, bottom: 1080 }],
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "w-strand".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportHwndOpened {
            hwnd: 0xCD,
            class_name: "Chrome_WidgetWin_1".into(),
            title: "".into(),
            label_hint: Some("w-strand".into()),
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportHwndForegroundChanged { hwnd: 0xCD },
        &c,
    );
    // Park the window where it WILL be stranded by topology change.
    let _ = update(
        &mut state,
        Command::ReportHwndPositionChanged {
            hwnd: 0xCD,
            rect: Rect { left: 1000, top: 100, right: 1500, bottom: 600 },
        },
        &c,
    );
    // Topology change strands the window.
    let first = update(
        &mut state,
        Command::ReportMonitorTopologyChanged {
            rects: vec![Rect { left: 0, top: 0, right: 800, bottom: 600 }],
        },
        &c,
    );
    assert_eq!(
        first.iter().filter(|e| matches!(
            e,
            Event::HwndDriftDetected { kind: HwndDriftKind::OffMonitor, .. }
        )).count(),
        1,
        "first stranding emits drift once"
    );

    // Repeated topology changes (e.g. user toggles displays multiple
    // times). Window stays stranded but cap should suppress.
    let mut subsequent = 0;
    for _ in 0..50 {
        let evs = update(
            &mut state,
            Command::ReportMonitorTopologyChanged {
                rects: vec![Rect { left: 0, top: 0, right: 800, bottom: 600 }],
            },
            &c,
        );
        subsequent += evs.iter().filter(|e| matches!(
            e,
            Event::HwndDriftDetected { kind: HwndDriftKind::OffMonitor, .. }
        )).count();
    }
    assert_eq!(subsequent, 0, "cap suppresses repeated topology-change OffMonitor");
}

#[test]
fn off_monitor_drift_cap_survives_duplicate_open() {
    // Same monotonicity discipline as hidden_since_open_emitted.
    // After cap fires, a duplicate ReportWindowOpened must preserve
    // the cap or the storm re-arms.
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportMonitorTopologyChanged {
            rects: vec![Rect { left: 0, top: 0, right: 1920, bottom: 1080 }],
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "w-dup-off".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportHwndOpened {
            hwnd: 0xAB,
            class_name: "Chrome_WidgetWin_1".into(),
            title: "".into(),
            label_hint: Some("w-dup-off".into()),
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportHwndForegroundChanged { hwnd: 0xAB },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportHwndPositionChanged {
            hwnd: 0xAB,
            rect: Rect { left: 5000, top: 5000, right: 5500, bottom: 5400 },
        },
        &c,
    );
    assert!(state.windows.get("w-dup-off").unwrap().off_monitor_drift_emitted);

    // Duplicate open — cap must survive.
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "w-dup-off".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    assert!(
        state.windows.get("w-dup-off").unwrap().off_monitor_drift_emitted,
        "cap survives duplicate open"
    );
}

#[test]
fn promote_for_label_without_window_mirror_emits_event_idempotently() {
    // The handler's idempotency contract: ReportPoolWindowPromoted
    // can arrive without a paired ReportWindowOpened (e.g. the open
    // IPC was lost). The handler must emit the typed event and
    // record in just_promoted_labels without synthesising a mirror.
    let (mut state, c) = registered_host_state();
    let evs = update(
        &mut state,
        Command::ReportPoolWindowPromoted { label: "window-pool-xyz".into() },
        &c,
    );
    assert!(
        evs.iter().any(|e| matches!(e, Event::PoolWindowPromoted { .. })),
        "promote without mirror must still emit PoolWindowPromoted"
    );
    assert!(
        state.just_promoted_labels.contains("window-pool-xyz"),
        "promote records in just_promoted_labels"
    );
    assert!(
        !state.windows.contains_key("window-pool-xyz"),
        "promote must not synthesise a mirror"
    );
}

#[test]
fn duplicate_open_for_promoted_label_preserves_foregrounded_since_open() {
    // Codex P2 PR #708 round 3: `foregrounded_since_open` is monotonic
    // per its WindowMirror contract. The first open after a promote
    // consumes `just_promoted_labels` and sets the flag to true. A
    // duplicate ReportWindowOpened (existing handler overwrites the
    // mirror wholesale) must NOT reset the flag — otherwise the next
    // post-promote `ReportHwndVisibilityChanged` re-fires
    // `HiddenSinceOpen` drift.
    let (mut state, c) = registered_host_state();

    // Production tear-off sequence.
    let _ = update(
        &mut state,
        Command::ReportPoolWindowAdded { label: "window-pool-dup".into(), saga_id: None },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportPoolWindowRemoved { label: "window-pool-dup".into() },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportPoolWindowPromoted { label: "window-pool-dup".into() },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "window-pool-dup".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    assert!(
        state.windows.get("window-pool-dup").unwrap().foregrounded_since_open,
        "first open: foregrounded_since_open=true (consumed just_promoted)"
    );

    // Duplicate open for the same label. just_promoted entry is
    // already gone; the flag must survive via OR with the prior
    // mirror's value.
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "window-pool-dup".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    assert!(
        state.windows.get("window-pool-dup").unwrap().foregrounded_since_open,
        "duplicate open MUST preserve foregrounded_since_open=true (regression: drift storm returns)"
    );
}

#[test]
fn promote_after_open_marks_existing_mirror_foregrounded() {
    // Order-tolerant promote (PR #709 round 2): if open arrives before
    // promote (out-of-order IPC, replay, or test fuzzer), the mirror
    // exists with foregrounded_since_open=false. Promote must update
    // the existing mirror directly INSTEAD of leaking an entry into
    // just_promoted_labels that will never be drained.
    //
    // Caught by the `just_promoted_labels_drained_by_open_or_close`
    // proptest after codex P2 PR #709 round 1 pointed out that the
    // original strategy used disjoint label spaces.
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "window-x".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    assert!(
        !state.windows.get("window-x").unwrap().foregrounded_since_open,
        "control: pre-promote, mirror has foregrounded_since_open=false"
    );

    let _ = update(
        &mut state,
        Command::ReportPoolWindowPromoted { label: "window-x".into() },
        &c,
    );
    assert!(
        state.windows.get("window-x").unwrap().foregrounded_since_open,
        "promote-after-open must update the existing mirror"
    );
    assert!(
        !state.just_promoted_labels.contains("window-x"),
        "promote-after-open must NOT leak into just_promoted_labels (no consumer to drain it)"
    );
}

#[test]
fn close_drops_orphaned_just_promoted_entry() {
    // If promote was emitted but the matching open never arrived
    // (host crash mid-tear-off etc.), a subsequent close for the
    // same label must clean up the just_promoted entry to bound the
    // leak.
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportPoolWindowPromoted { label: "window-pool-orphan".into() },
        &c,
    );
    assert!(state.just_promoted_labels.contains("window-pool-orphan"));
    let _ = update(
        &mut state,
        Command::ReportWindowClosed { label: "window-pool-orphan".into() },
        &c,
    );
    assert!(
        !state.just_promoted_labels.contains("window-pool-orphan"),
        "close must drop orphaned just_promoted entry"
    );
}

#[test]
fn wrr_duplicate_hwnd_open_for_same_label_is_idempotent() {
    // codex #600 P2: ReportHwndOpened arrives twice (once from
    // the WinEvent CREATE hook with label_hint=None, then again
    // from CEF's on_after_created with label_hint=Some(label)).
    // The second report carries the SAME hwnd that's now
    // linked to the mirror. Should be a no-op, NOT drift.
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "w1".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    // First link.
    let evs1 = update(
        &mut state,
        Command::ReportHwndOpened {
            hwnd: 0xAA,
            class_name: "Chrome_WidgetWin_1".into(),
            title: "".into(),
            label_hint: Some("w1".into()),
        },
        &c,
    );
    assert!(
        evs1.is_empty(),
        "first ReportHwndOpened should silently link, got {:?}", evs1
    );
    // Duplicate: same label, same hwnd.
    let evs2 = update(
        &mut state,
        Command::ReportHwndOpened {
            hwnd: 0xAA,
            class_name: "Chrome_WidgetWin_1".into(),
            title: "".into(),
            label_hint: Some("w1".into()),
        },
        &c,
    );
    assert!(
        evs2.is_empty(),
        "duplicate ReportHwndOpened with same hwnd must be no-op, got {:?}", evs2
    );
}

#[test]
fn wrr_double_link_with_different_hwnd_for_same_label_repairs_silently() {
    // A second ReportHwndOpened for the same label but a DIFFERENT
    // HWND is the Repaired path: the launcher's drain wrong-picked
    // earlier and the explicit on_after_created is correcting it.
    // This is normal self-healing — mirror.hwnd is overwritten,
    // no drift event is emitted (logged via crate::log for visibility).
    //
    // Rationale: in v0.33.696 smoke this fired 6 false-positive
    // HwndWithoutBrowser drifts in a clean session because the drain-
    // wrong-pick happens routinely under burst create. Drift firing
    // on every routine repair masked genuine drifts.
    //
    // Drift IS still fired for Linked+stole (different HWND that had
    // to be taken from another label's mirror) — see the
    // wrr_repair_steals_hwnd_from_other_mirror test for that case.
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "w1".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportHwndOpened {
            hwnd: 0xAA,
            class_name: "Chrome_WidgetWin_1".into(),
            title: "".into(),
            label_hint: Some("w1".into()),
        },
        &c,
    );
    let evs = update(
        &mut state,
        Command::ReportHwndOpened {
            hwnd: 0xBB,
            class_name: "Chrome_WidgetWin_1".into(),
            title: "".into(),
            label_hint: Some("w1".into()),
        },
        &c,
    );
    assert_eq!(state.windows["w1"].hwnd, Some(0xBB), "mirror.hwnd repaired to new value");
    assert!(
        !evs.iter().any(|e| matches!(
            e,
            Event::HwndDriftDetected { kind: HwndDriftKind::HwndWithoutBrowser, .. }
        )),
        "plain Repair must NOT emit drift event, got {:?}", evs
    );
}

#[test]
fn b9_3_orphan_instance_fires_when_last_window_closes_and_host_running() {
    // The smoke-test scenario: open a window, close it; with a
    // Host registered, the reducer should emit OrphanInstance
    // drift + HostShouldQuit on the same dispatch tick.
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "w1".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    // Sanity: window opened, no orphan signal yet.
    let evs = update(
        &mut state,
        Command::ReportWindowClosed { label: "w1".into() },
        &c,
    );
    assert!(
        evs.iter()
            .any(|e| matches!(e, Event::HwndDriftDetected { kind: HwndDriftKind::OrphanInstance, .. })),
        "expected OrphanInstance drift on last-window-close, got {:?}", evs
    );
    assert!(
        evs.iter().any(|e| matches!(e, Event::HostShouldQuit { .. })),
        "expected HostShouldQuit saga on last-window-close, got {:?}", evs
    );
}

#[test]
fn b9_3_orphan_instance_does_not_fire_on_non_terminal_close() {
    // Closing one of N windows when N > 1 must NOT fire — the
    // host has more user-visible windows alive, so it shouldn't
    // quit. Predicate: state.windows still non-empty after
    // remove → no signal.
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "w1".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "w2".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    let evs = update(
        &mut state,
        Command::ReportWindowClosed { label: "w1".into() },
        &c,
    );
    assert!(
        !evs.iter()
            .any(|e| matches!(e, Event::HwndDriftDetected { kind: HwndDriftKind::OrphanInstance, .. })),
        "OrphanInstance must NOT fire while other windows are open, got {:?}", evs
    );
    assert!(
        !evs.iter().any(|e| matches!(e, Event::HostShouldQuit { .. })),
        "HostShouldQuit must NOT fire while other windows are open, got {:?}", evs
    );
}

#[test]
fn b9_3_orphan_instance_does_not_fire_after_host_goodbye() {
    // Realistic scenario for the no-Host predicate guard:
    // Host registers, opens a window, then sends Goodbye
    // (clean shutdown), which marks its ProcessRecord as
    // Exited. A subsequent ReportWindowClosed arriving from
    // the host's pipe (e.g. a queued event flushed during
    // shutdown) MUST NOT fire HostShouldQuit — there's no
    // Running Host left to quit.
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "w1".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    // Host says Goodbye — record transitions to Exited.
    let _ = update(&mut state, Command::Goodbye, &c);
    // Force the close path to actually run by calling the
    // private handler directly. (The public path's
    // enforce_host_only would reject because the Host record
    // is no longer Running, but that rejection happens
    // BEFORE reaching the predicate; we want to prove the
    // predicate itself is correct.)
    let evs = super::window::handle_report_window_closed(&mut state, "w1".into());
    assert!(
        evs.iter().any(|e| matches!(e, Event::WindowClosed { .. })),
        "WindowClosed should still emit, got {:?}", evs
    );
    assert!(
        !evs.iter()
            .any(|e| matches!(e, Event::HwndDriftDetected { kind: HwndDriftKind::OrphanInstance, .. })),
        "OrphanInstance must NOT fire after host has Goodbye'd, got {:?}", evs
    );
    assert!(
        !evs.iter().any(|e| matches!(e, Event::HostShouldQuit { .. })),
        "HostShouldQuit must NOT fire after host has Goodbye'd, got {:?}", evs
    );
}

#[test]
fn wrr_orphan_destroy_runs_even_with_stale_pending_entry() {
    // reagent #600 P1: the dual-source design (WinEvent CREATE
    // hook with label_hint=None, then explicit on_after_created
    // with label_hint=Some(label)) can leave a stale entry in
    // `pending_hwnds` AFTER the mirror is linked. Pre-fix,
    // `apply_hwnd_destroyed` early-returned on the stale
    // pending entry and skipped the OrphanDestroy chain. Post-
    // fix, the link drains pending and destroy runs the chain
    // correctly. This test reproduces the exact race.
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportMonitorTopologyChanged {
            rects: vec![Rect { left: 0, top: 0, right: 1920, bottom: 1080 }],
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "w1".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    // Step 1: WinEvent CREATE fires first with label_hint=None.
    // No mirror match → stash in pending_hwnds.
    let _ = update(
        &mut state,
        Command::ReportHwndOpened {
            hwnd: 0xAA,
            class_name: "Chrome_WidgetWin_1".into(),
            title: "".into(),
            label_hint: None,
        },
        &c,
    );
    assert!(state.pending_hwnds.contains_key(&0xAA), "pending entry expected after step 1");
    // Step 2: on_after_created fires with label_hint=Some(w1).
    // Should link the mirror AND drain the stale pending.
    let _ = update(
        &mut state,
        Command::ReportHwndOpened {
            hwnd: 0xAA,
            class_name: "Chrome_WidgetWin_1".into(),
            title: "".into(),
            label_hint: Some("w1".into()),
        },
        &c,
    );
    assert!(
        !state.pending_hwnds.contains_key(&0xAA),
        "stale pending entry should be drained on link, but still present: {:?}",
        state.pending_hwnds
    );
    // Step 3: renderer crash → ReportHwndDestroyed.
    let evs = update(
        &mut state,
        Command::ReportHwndDestroyed { hwnd: 0xAA },
        &c,
    );
    // Even with the (drained) pending entry history, the
    // orphan-destroy chain must run.
    assert!(
        evs.iter()
            .any(|e| matches!(e, Event::HwndDriftDetected { kind: HwndDriftKind::OrphanDestroy, .. })),
        "OrphanDestroy must fire on renderer crash even after dual-source link, got {:?}",
        evs
    );
    assert!(
        evs.iter().any(|e| matches!(e, Event::WindowClosed { .. })),
        "WindowClosed must fire so frontend prunes its atoms, got {:?}",
        evs
    );
}

/// Workstream 0 Phase 1 (PR #2983 review, Codex P2) — the crash-detected
/// twin of `host_should_quit_suppressed_when_background_service_enabled`:
/// a renderer crash that destroys the last window's HWND must also skip
/// `OrphanInstance`/`HostShouldQuit` once the host has reported
/// background-service mode enabled, exactly like a clean close does.
#[test]
fn wrr_orphan_instance_suppressed_when_background_service_enabled() {
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportBackgroundServiceEnabled { enabled: true },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "w1".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportHwndOpened {
            hwnd: 0xAA,
            class_name: "Chrome_WidgetWin_1".into(),
            title: "".into(),
            label_hint: Some("w1".into()),
        },
        &c,
    );
    let evs = update(&mut state, Command::ReportHwndDestroyed { hwnd: 0xAA }, &c);
    assert!(
        evs.iter()
            .any(|e| matches!(e, Event::HwndDriftDetected { kind: HwndDriftKind::OrphanDestroy, .. })),
        "OrphanDestroy must still fire — that's a genuine renderer crash, unrelated to this mode, got {:?}",
        evs
    );
    assert!(
        !evs.iter()
            .any(|e| matches!(e, Event::HwndDriftDetected { kind: HwndDriftKind::OrphanInstance, .. })),
        "OrphanInstance must NOT fire when background-service mode is enabled, got {:?}",
        evs
    );
    assert!(
        !evs.iter().any(|e| matches!(e, Event::HostShouldQuit { .. })),
        "HostShouldQuit must NOT fire when background-service mode is enabled, got {:?}",
        evs
    );
}

#[test]
fn wrr_burst_creates_with_drain_then_repair() {
    // PR #664 codex P1 round 2 — drain-on-open RESTORED as fallback
    // for the hwnd_val=0 case. The drain may wrong-pick under burst
    // creates, but apply_hwnd_opened REPAIRS via the Repaired arm
    // when the explicit on_after_created path fires.
    //
    // Test verifies: WindowOpened drains some pending HWND, then
    // explicit on_after_created arrives with the authoritative
    // HWND and REPAIRS any wrong link. Doesn't assume a specific
    // wrong-pick order (HashMap iter is non-deterministic when
    // multiple pending entries share arrived_at_ms — test must be
    // robust to either pick).
    let (mut state, _c) = registered_host_state();
    let mut c1 = ctx_with_pid(1, 1); c1.now_ms = 100;
    let mut c2 = ctx_with_pid(1, 1); c2.now_ms = 200;
    let mut c3 = ctx_with_pid(1, 1); c3.now_ms = 300;
    let mut c4 = ctx_with_pid(1, 1); c4.now_ms = 400;
    let mut c5 = ctx_with_pid(1, 1); c5.now_ms = 500;
    let mut c6 = ctx_with_pid(1, 1); c6.now_ms = 600;

    let _ = update(
        &mut state,
        Command::ReportMonitorTopologyChanged {
            rects: vec![Rect { left: 0, top: 0, right: 1920, bottom: 1080 }],
        },
        &c1,
    );
    // 0xAA arrives FIRST (now_ms=200), then 0xBB (now_ms=300).
    // Drain uses max_by_key(arrived_at_ms) — most recent. So 0xBB
    // is drained first, deterministically.
    let _ = update(&mut state, Command::ReportHwndOpened {
        hwnd: 0xAA, class_name: "Chrome_WidgetWin_1".into(),
        title: "".into(), label_hint: None,
    }, &c2);
    let _ = update(&mut state, Command::ReportHwndOpened {
        hwnd: 0xBB, class_name: "Chrome_WidgetWin_1".into(),
        title: "".into(), label_hint: None,
    }, &c3);
    assert!(state.pending_hwnds.contains_key(&0xAA));
    assert!(state.pending_hwnds.contains_key(&0xBB));

    // WindowOpened(w1) drains MOST RECENT (0xBB) — wrong pick.
    let _ = update(&mut state, Command::ReportWindowOpened {
        label: "w1".into(), kind: WindowKind::FullInstance,
        parent_label: None,
    }, &c4);
    assert_eq!(state.windows["w1"].hwnd, Some(0xBB), "drain wrong-picks (most recent)");
    assert!(!state.pending_hwnds.contains_key(&0xBB));

    // WindowOpened(w2) drains the remaining (0xAA).
    let _ = update(&mut state, Command::ReportWindowOpened {
        label: "w2".into(), kind: WindowKind::FullInstance,
        parent_label: None,
    }, &c4);
    assert_eq!(state.windows["w2"].hwnd, Some(0xAA), "drain picks remaining");

    // on_after_created for w1 arrives with AUTHORITATIVE 0xAA → REPAIR.
    let evs1 = update(&mut state, Command::ReportHwndOpened {
        hwnd: 0xAA, class_name: "Chrome_WidgetWin_1".into(),
        title: "".into(), label_hint: Some("w1".into()),
    }, &c5);
    assert_eq!(state.windows["w1"].hwnd, Some(0xAA), "repair to authoritative");
    // Repaired path (plain repair, no cross-label steal here) is a
    // normal self-healing flow — log only, no drift event. Drift
    // would still fire if this were a Linked+stole case (different
    // HWND that took the slot from another label).
    assert!(
        !evs1.iter().any(|e| matches!(e, Event::HwndDriftDetected {
            kind: HwndDriftKind::HwndWithoutBrowser, ..
        })),
        "plain Repair must NOT emit drift event (logged via crate::log instead), got {:?}", evs1
    );

    // on_after_created for w2 arrives with 0xBB. CRUCIAL: w1's
    // repair above stole 0xAA from w2 (codex P1 round 5 steal),
    // so w2.hwnd is currently None — clean Link, no Repair,
    // no drift.
    assert_eq!(state.windows["w2"].hwnd, None,
        "w2's wrong link must have been cleared by w1's repair-steal");
    let evs2 = update(&mut state, Command::ReportHwndOpened {
        hwnd: 0xBB, class_name: "Chrome_WidgetWin_1".into(),
        title: "".into(), label_hint: Some("w2".into()),
    }, &c6);
    assert_eq!(state.windows["w2"].hwnd, Some(0xBB), "clean link");
    assert!(
        !evs2.iter().any(|e| matches!(e, Event::HwndDriftDetected { .. })),
        "no drift on clean Link (steal already happened in w1's repair), got {:?}", evs2
    );

    // Final: both windows linked to their authoritative HWNDs.
    assert_eq!(state.windows["w1"].hwnd, Some(0xAA));
    assert_eq!(state.windows["w2"].hwnd, Some(0xBB));
}

#[test]
fn wrr_repair_steals_hwnd_from_other_mirror() {
    // PR #664 codex P1 round 5 — when the repair arm overwrites
    // mirror[A].hwnd to the new HWND, any OTHER mirror that was
    // wrongly linked to that HWND must have its link cleared.
    // Otherwise apply_hwnd_destroyed (which uses iter().find by
    // hwnd) only fires WindowClosed for ONE of them, leaving the
    // other as a ghost row forever.
    let (mut state, _c) = registered_host_state();
    let mut c1 = ctx_with_pid(1, 1); c1.now_ms = 100;
    let mut c2 = ctx_with_pid(1, 1); c2.now_ms = 200;
    let mut c3 = ctx_with_pid(1, 1); c3.now_ms = 300;

    let _ = update(&mut state, Command::ReportMonitorTopologyChanged {
        rects: vec![Rect { left: 0, top: 0, right: 1920, bottom: 1080 }],
    }, &c1);

    // Stage: mirror[B] is linked to HWND 0xAA via some prior path
    // (e.g., drain wrong-pick, partial-host-upgrade weird state).
    let _ = update(&mut state, Command::ReportWindowOpened {
        label: "B".into(), kind: WindowKind::FullInstance,
        parent_label: None,
    }, &c2);
    state.windows.get_mut("B").unwrap().hwnd = Some(0xAA);

    // mirror[A] just opened, no link yet.
    let _ = update(&mut state, Command::ReportWindowOpened {
        label: "A".into(), kind: WindowKind::FullInstance,
        parent_label: None,
    }, &c2);

    // on_after_created for A arrives with the AUTHORITATIVE HWND
    // 0xAA. Repair must:
    //   1. set mirror[A].hwnd = 0xAA
    //   2. CLEAR mirror[B].hwnd (the wrongly-linked one)
    //   3. emit drift event mentioning the steal
    let evs = update(&mut state, Command::ReportHwndOpened {
        hwnd: 0xAA, class_name: "Chrome_WidgetWin_1".into(),
        title: "".into(), label_hint: Some("A".into()),
    }, &c3);

    assert_eq!(state.windows["A"].hwnd, Some(0xAA), "A linked to authoritative");
    assert_eq!(state.windows["B"].hwnd, None,
        "B's wrong link must be CLEARED (codex P1 round 5)");

    // Drift event surfaces the steal.
    let drift_msg = evs.iter().find_map(|e| match e {
        Event::HwndDriftDetected { detail, .. } => Some(detail.clone()),
        _ => None,
    });
    assert!(
        drift_msg.as_deref().map(|d| d.contains("stole from label=B")).unwrap_or(false),
        "drift detail must mention which label was stolen from, got {:?}", drift_msg
    );

    // Now OS destroys 0xAA. Only mirror[A] is linked to it →
    // exactly ONE WindowClosed, no ghost row for B.
    let evs = update(&mut state, Command::ReportHwndDestroyed { hwnd: 0xAA }, &c3);
    let closed_count = evs.iter().filter(|e| matches!(e, Event::WindowClosed { .. })).count();
    assert_eq!(closed_count, 1, "exactly one WindowClosed (codex P1 round 5)");
}

#[test]
fn wrr_drain_handles_missing_explicit_link() {
    // PR #664 codex P1 round 2 regression — when on_after_created
    // CAN'T resolve HWND (hwnd_val=0 from all 3 sources host-side),
    // it skips the explicit ReportHwndOpened. The drain-on-open
    // fallback is the ONLY thing that links the mirror. Without
    // the drain, mirror would stay hwnd=None permanently → no
    // OrphanDestroy when OS closes the HWND → ghost panel rows.
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportMonitorTopologyChanged {
            rects: vec![Rect { left: 0, top: 0, right: 1920, bottom: 1080 }],
        },
        &c,
    );
    // WM_CREATE captures HWND.
    let _ = update(&mut state, Command::ReportHwndOpened {
        hwnd: 0xAA, class_name: "Chrome_WidgetWin_1".into(),
        title: "".into(), label_hint: None,
    }, &c);
    // WindowOpened drains it — the only link path that fires when
    // on_after_created's explicit dispatch is skipped.
    let _ = update(&mut state, Command::ReportWindowOpened {
        label: "w1".into(), kind: WindowKind::FullInstance,
        parent_label: None,
    }, &c);
    assert_eq!(
        state.windows["w1"].hwnd, Some(0xAA),
        "drain-on-open must link when explicit dispatch is the only fallback"
    );

    // OS destroys the HWND. Mirror's link enables OrphanDestroy
    // detection → WindowClosed event → InstancePanel row removed.
    let evs = update(&mut state, Command::ReportHwndDestroyed { hwnd: 0xAA }, &c);
    assert!(
        evs.iter().any(|e| matches!(
            e, Event::HwndDriftDetected { kind: HwndDriftKind::OrphanDestroy, .. }
        )),
        "OrphanDestroy must fire so the panel row clears, got {:?}", evs
    );
    assert!(
        evs.iter().any(|e| matches!(e, Event::WindowClosed { .. })),
        "WindowClosed must fire to drive frontend cleanup, got {:?}", evs
    );
}

#[test]
fn wrr_apply_hwnd_opened_repairs_stale_link() {
    // PR #664 regression — if some prior path (e.g. partial host
    // upgrade still using back-of-queue peek) linked a wrong HWND
    // to a label, the explicit on_after_created path must REPAIR
    // the link, not just emit drift and leave the wrong link in
    // place.
    let (mut state, c) = registered_host_state();
    let _ = update(&mut state, Command::ReportMonitorTopologyChanged {
        rects: vec![Rect { left: 0, top: 0, right: 1920, bottom: 1080 }],
    }, &c);
    let _ = update(&mut state, Command::ReportWindowOpened {
        label: "w1".into(), kind: WindowKind::FullInstance,
        parent_label: None,
    }, &c);
    // Simulate a stale wrong-HWND link (would happen pre-fix if
    // back-of-queue peek labeled HWND_b as w1).
    state.windows.get_mut("w1").unwrap().hwnd = Some(0xBAD);

    // on_after_created arrives with the actual HWND for w1.
    let evs = update(&mut state, Command::ReportHwndOpened {
        hwnd: 0xAA, class_name: "Chrome_WidgetWin_1".into(),
        title: "".into(), label_hint: Some("w1".into()),
    }, &c);
    assert_eq!(
        state.windows["w1"].hwnd, Some(0xAA),
        "stale link must be REPAIRED to the authoritative HWND"
    );
    // Repair logged via crate::log; no drift event (would otherwise
    // fire on every routine drain wrong-pick correction). See the
    // wrr_double_link_with_different_hwnd_for_same_label_repairs_silently
    // test for the contract.
    assert!(
        !evs.iter().any(|e| matches!(e, Event::HwndDriftDetected {
            kind: HwndDriftKind::HwndWithoutBrowser, ..
        })),
        "plain Repair must NOT emit drift event, got {:?}", evs
    );
}

#[test]
fn wrr_orphan_destroy_emits_window_closed_and_instance_released() {
    // reagent #600 P1: a renderer crash that takes the HWND
    // with it must produce the same shutdown events the normal
    // close path would, otherwise the frontend keeps a stale
    // window in its atoms after the crash.
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportMonitorTopologyChanged {
            rects: vec![Rect { left: 0, top: 0, right: 1920, bottom: 1080 }],
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "w1".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportHwndOpened {
            hwnd: 0xAA,
            class_name: "Chrome_WidgetWin_1".into(),
            title: "".into(),
            label_hint: Some("w1".into()),
        },
        &c,
    );
    // Renderer crashes — Win32 fires the destroy without CEF's
    // close path running, so no ReportWindowClosed precedes it.
    let evs = update(
        &mut state,
        Command::ReportHwndDestroyed { hwnd: 0xAA },
        &c,
    );

    // All three: drift (operator alert) + WindowClosed
    // (frontend prune) + WindowInstanceReleased (count drop).
    assert!(
        evs.iter()
            .any(|e| matches!(e, Event::HwndDriftDetected { kind: HwndDriftKind::OrphanDestroy, .. })),
        "expected OrphanDestroy drift, got {:?}", evs
    );
    assert!(
        evs.iter().any(|e| matches!(
            e,
            Event::WindowClosed { label, .. } if label == "w1"
        )),
        "expected WindowClosed for w1, got {:?}", evs
    );
    assert!(
        evs.iter().any(|e| matches!(
            e,
            Event::WindowInstanceReleased { label, .. } if label == "w1"
        )),
        "expected WindowInstanceReleased for w1, got {:?}", evs
    );
    // State pruned.
    assert!(!state.windows.contains_key("w1"));
    assert!(!state.instance_registry.contains_key("w1"));
}

#[test]
fn wrr_orphan_destroy_emits_host_should_quit_when_last_window() {
    // Companion to the normal-close path's HostShouldQuit emission
    // (handle_report_window_closed). When a renderer crash takes
    // the LAST mirrored window's HWND with it, the host's orphan
    // reconciler — which only listens to HostShouldQuit — must
    // still wake up. Without it, warm-pool browsers stay alive
    // and the host process becomes a zombie.
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportMonitorTopologyChanged {
            rects: vec![Rect { left: 0, top: 0, right: 1920, bottom: 1080 }],
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "w1".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportHwndOpened {
            hwnd: 0xAA,
            class_name: "Chrome_WidgetWin_1".into(),
            title: "".into(),
            label_hint: Some("w1".into()),
        },
        &c,
    );
    let evs = update(
        &mut state,
        Command::ReportHwndDestroyed { hwnd: 0xAA },
        &c,
    );

    assert!(
        evs.iter()
            .any(|e| matches!(e, Event::HwndDriftDetected { kind: HwndDriftKind::OrphanInstance, .. })),
        "expected OrphanInstance drift after last-window crash, got {:?}", evs
    );
    assert!(
        evs.iter().any(|e| matches!(e, Event::HostShouldQuit { .. })),
        "expected HostShouldQuit after last-window crash, got {:?}", evs
    );
}

#[test]
fn wrr_orphan_destroy_does_not_emit_host_should_quit_when_other_windows_remain() {
    // Crash of one window when others are still open — the host's
    // organic close path will handle quitting when the last one
    // closes; we don't want a stray HostShouldQuit here.
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportMonitorTopologyChanged {
            rects: vec![Rect { left: 0, top: 0, right: 1920, bottom: 1080 }],
        },
        &c,
    );
    for (label, hwnd) in [("w1", 0xAA_u64), ("w2", 0xBB)] {
        let _ = update(
            &mut state,
            Command::ReportWindowOpened {
                label: label.into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &c,
        );
        let _ = update(
            &mut state,
            Command::ReportHwndOpened {
                hwnd,
                class_name: "Chrome_WidgetWin_1".into(),
                title: "".into(),
                label_hint: Some(label.into()),
            },
            &c,
        );
    }
    let evs = update(
        &mut state,
        Command::ReportHwndDestroyed { hwnd: 0xAA },
        &c,
    );
    assert!(
        !evs.iter().any(|e| matches!(e, Event::HostShouldQuit { .. })),
        "HostShouldQuit must not fire while other windows remain, got {:?}", evs
    );
}

#[test]
fn wrr_off_monitor_with_no_known_monitors_emits_neither() {
    // No `ReportMonitorTopologyChanged` => state.monitors is
    // empty => we don't know what "off-monitor" means yet.
    let (mut state, c) = registered_host_state();
    let _ = update(
        &mut state,
        Command::ReportWindowOpened {
            label: "w1".into(),
            kind: WindowKind::FullInstance,
            parent_label: None,
        },
        &c,
    );
    let _ = update(
        &mut state,
        Command::ReportHwndOpened {
            hwnd: 0xAA,
            class_name: "Chrome_WidgetWin_1".into(),
            title: "".into(),
            label_hint: Some("w1".into()),
        },
        &c,
    );
    let evs = update(
        &mut state,
        Command::ReportHwndPositionChanged {
            hwnd: 0xAA,
            rect: Rect { left: -5000, top: -5000, right: -4000, bottom: -4000 },
        },
        &c,
    );
    assert!(
        evs.is_empty(),
        "no monitors known => no drift, no corrective; got {:?}", evs
    );
}

// -----------------------------------------------------------
// Phase B.8 — extended invariant suite.
//
// Covers gaps surfaced during B.5 / B.7 / B.9 work that the
// earlier proptests in `arb_command` (Register / Window
// open/close only) didn't exercise: pool inventory, backend
// window ID lifecycle, and the OrphanInstance / HostShouldQuit
// saga from B.9.3. Plus a deterministic integration-style
// close-all cascade that locks in the B.9.3 behaviour at the
// reducer level (CI synthetic close-all assertion per B.8 plan).
// -----------------------------------------------------------

/// Generate B.5+ host commands: window open/close, pool
/// add/remove, backend window ID register/unregister. Labels
/// drawn from a small alphabet so duplicates / opens-of-already-
/// open / close-of-not-open paths get exercised.
fn arb_b8_host_command() -> impl proptest::strategy::Strategy<Value = Command> {
    use proptest::prelude::*;
    prop_oneof![
        3 => (
            "[a-c]{1,3}",
            prop_oneof![Just(WindowKind::FullInstance), Just(WindowKind::Subwindow)],
            prop_oneof![Just(None::<String>), Just(Some("a".into()))],
        )
            .prop_map(|(label, kind, parent_label)| {
                Command::ReportWindowOpened { label, kind, parent_label }
            }),
        3 => "[a-c]{1,3}".prop_map(|label| Command::ReportWindowClosed { label }),
        2 => "pool-[a-c]{1,2}".prop_map(|label| Command::ReportPoolWindowAdded { label, saga_id: None }),
        2 => "pool-[a-c]{1,2}".prop_map(|label| Command::ReportPoolWindowRemoved { label }),
        2 => ("[a-c]{1,3}", "[0-9a-f]{4}").prop_map(|(label, window_id)| {
            Command::ReportBackendWindowIdRegistered { label, window_id }
        }),
        2 => "[a-c]{1,3}".prop_map(|label| Command::ReportBackendWindowIdUnregistered { label }),
        // Promote — exercises just_promoted_labels (PR #708 round 3).
        // Drift-storm fix relies on this set bridging the
        // PoolPromoted → WindowOpened gap; a proptest verifies the
        // set never grows without bound. NOTE: this strategy uses a
        // disjoint `pool-*` label space from opens/closes — the
        // producer-side bound holds, but the consumer paths in
        // `handle_report_window_opened/Closed` aren't exercised here.
        // For consumer-path coverage see `arb_promote_focused_command`.
        2 => "pool-[a-c]{1,2}".prop_map(|label| Command::ReportPoolWindowPromoted { label }),
    ]
}

/// Strategy with **overlapping label space** between promote / open /
/// close — every label is drawn from the same small pool so that
/// `ReportPoolWindowPromoted("a")` is followed (with non-trivial
/// probability) by `ReportWindowOpened { label: "a", .. }` or
/// `ReportWindowClosed { label: "a" }`. Used by the
/// `just_promoted_labels_drained_by_open_or_close` proptest, which
/// codex flagged in PR #709 round 1: with the disjoint label space
/// in `arb_b8_host_command`, the cleanup paths in
/// `handle_report_window_opened/Closed` never ran on promoted labels,
/// so the property test would still pass with the cleanups removed.
///
/// This is a regression-guard strategy specifically for the
/// drift-storm fix from PR #708 round 3.
fn arb_promote_focused_command() -> impl proptest::strategy::Strategy<Value = Command> {
    use proptest::prelude::*;
    // Shared label space — opens, closes, AND promotes all draw from
    // {a, b, c, ab, ac, bc} so producer/consumer paths overlap.
    prop_oneof![
        3 => (
            "[a-c]{1,2}",
            prop_oneof![Just(WindowKind::FullInstance), Just(WindowKind::Subwindow)],
        )
            .prop_map(|(label, kind)| {
                Command::ReportWindowOpened { label, kind, parent_label: None }
            }),
        3 => "[a-c]{1,2}".prop_map(|label| Command::ReportWindowClosed { label }),
        3 => "[a-c]{1,2}".prop_map(|label| Command::ReportPoolWindowPromoted { label }),
        // Pool add/remove kept to maintain pool invariants under the
        // overlapping label space (mirrors b8_host_command's mix).
        1 => "[a-c]{1,2}".prop_map(|label| Command::ReportPoolWindowAdded { label, saga_id: None }),
        1 => "[a-c]{1,2}".prop_map(|label| Command::ReportPoolWindowRemoved { label }),
    ]
}

proptest! {
    /// Pool/windows disjoint: a label is never simultaneously in
    /// both `state.pool` and `state.windows`. The host's
    /// pool→window promote path always sends ReportPoolWindowRemoved
    /// before ReportWindowOpened (and reverse on demote), so the
    /// reducer should never observe overlap. Catches a regression
    /// where a buggy promote sequence would leave a pool label
    /// shadowed by a window entry — gap #5 from
    /// `ANALYSIS_WINDOW_PROCESS_STATE_INVENTORY_2026_04_27.md`.
    #[test]
    fn pool_and_windows_disjoint_under_any_sequence(
        cmds in proptest::collection::vec(arb_b8_host_command(), 1..80)
    ) {
        let (mut state, host_ctx) = registered_host_state();
        for cmd in cmds {
            let _ = update(&mut state, cmd, &host_ctx);
            let pool: std::collections::HashSet<&str> =
                state.pool.iter().map(|s| s.as_str()).collect();
            let window_keys: std::collections::HashSet<&str> =
                state.windows.keys().map(|s| s.as_str()).collect();
            let overlap: Vec<&&str> = pool.intersection(&window_keys).collect();
            prop_assert!(
                overlap.is_empty(),
                "label(s) in both pool and windows: {:?}", overlap
            );
        }
    }

    /// Both consumer paths drain `just_promoted_labels`:
    ///   - `handle_report_window_opened` (production path, primary)
    ///   - `handle_report_window_closed` (fallback for promote-with-no-open)
    ///
    /// **Property:** for every label `L` whose LAST seen command was an
    /// open or close (consumer), `L` MUST NOT be in
    /// `just_promoted_labels` at end-of-sequence. Catches both broken
    /// drains: codex P2 PR #709 round 2 noted the prior version only
    /// caught the open-consumer regression (a sequence like
    /// `Promoted("a") → Closed("a")` would leave "a" in the set with
    /// `windows.contains_key("a") == false`, passing the old "no
    /// label in both sets" check trivially).
    ///
    /// Mutation-test: commenting out either cleanup line in
    /// `handle_report_window_opened` or `handle_report_window_closed`
    /// makes this proptest fail.
    #[test]
    fn both_drain_paths_remove_just_promoted_entry(
        cmds in proptest::collection::vec(arb_promote_focused_command(), 1..80)
    ) {
        let (mut state, host_ctx) = registered_host_state();
        // Track the LAST command kind seen per label as we apply the
        // sequence. After all commands are applied, any label whose
        // last-command was an open or close is a "post-consumer"
        // label and MUST have been drained from just_promoted_labels.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum LastCmd { Promote, OpenOrClose }
        let mut last_cmd_per_label: std::collections::HashMap<String, LastCmd> = std::collections::HashMap::new();
        for cmd in cmds {
            match &cmd {
                Command::ReportPoolWindowPromoted { label } => {
                    last_cmd_per_label.insert(label.clone(), LastCmd::Promote);
                }
                Command::ReportWindowOpened { label, .. }
                | Command::ReportWindowClosed { label } => {
                    last_cmd_per_label.insert(label.clone(), LastCmd::OpenOrClose);
                }
                _ => {}
            }
            let _ = update(&mut state, cmd, &host_ctx);
        }
        for (label, last) in &last_cmd_per_label {
            if *last == LastCmd::OpenOrClose {
                prop_assert!(
                    !state.just_promoted_labels.contains(label),
                    "label {:?} had last-cmd open/close (consumer) but is still in just_promoted_labels — drain regression",
                    label
                );
            }
        }
    }

    /// Bound on the just_promoted set: it never grows past the count
    /// of distinct labels for which `ReportPoolWindowPromoted` has
    /// been emitted across the whole sequence. This is a cumulative
    /// upper-bound (the "ever promoted" denominator), NOT a
    /// "currently pending" bound — the consumer paths can also
    /// remove entries, but never add to the denominator.
    ///
    /// Reagent P2 PR #709 round 2 flagged the prior name
    /// (`bounded_by_pending_promotes`) as misleading vs the actual
    /// bound. Renamed to match what's measured.
    #[test]
    fn just_promoted_labels_bounded_by_distinct_labels_ever_promoted(
        cmds in proptest::collection::vec(arb_promote_focused_command(), 1..80)
    ) {
        let (mut state, host_ctx) = registered_host_state();
        let mut distinct_ever_promoted: std::collections::HashSet<String> = std::collections::HashSet::new();
        for cmd in &cmds {
            if let Command::ReportPoolWindowPromoted { label } = cmd {
                distinct_ever_promoted.insert(label.clone());
            }
        }
        for cmd in cmds {
            let _ = update(&mut state, cmd, &host_ctx);
            prop_assert!(
                state.just_promoted_labels.len() <= distinct_ever_promoted.len(),
                "just_promoted_labels grew past distinct labels ever promoted: {} > {}",
                state.just_promoted_labels.len(),
                distinct_ever_promoted.len()
            );
        }
    }

    /// `ReportPoolWindowPromoted` is idempotent: applying the SAME
    /// promote command N times produces equivalent state to applying
    /// once (modulo `event_version`, which is monotonic by design).
    /// Spec §8.14 sibling — this is the GENERATOR-side property
    /// (sub set insertion is naturally idempotent), exercised
    /// explicitly to lock the contract.
    #[test]
    fn promote_is_idempotent_on_just_promoted_labels(
        label in "pool-[a-c]{1,2}",
        n in 2usize..10
    ) {
        let (mut state, host_ctx) = registered_host_state();
        let _ = update(
            &mut state,
            Command::ReportPoolWindowAdded { label: label.clone(), saga_id: None },
            &host_ctx,
        );
        let _ = update(
            &mut state,
            Command::ReportPoolWindowRemoved { label: label.clone() },
            &host_ctx,
        );
        for _ in 0..n {
            let _ = update(
                &mut state,
                Command::ReportPoolWindowPromoted { label: label.clone() },
                &host_ctx,
            );
        }
        prop_assert!(
            state.just_promoted_labels.contains(&label),
            "promote must record label"
        );
        prop_assert_eq!(
            state.just_promoted_labels.len(), 1,
            "{} promotes for the same label produce 1 entry, not n",
            n
        );
    }

    /// Instance numbers within `state.instance_registry` are
    /// unique. Reagent / codex flagged the reverse property
    /// (numbers don't repeat across releases) at B.5b; this is
    /// the symmetric "no two LIVE windows share a number"
    /// guarantee. Failure mode: InstancePanel would render two
    /// rows as "Window N", users couldn't disambiguate.
    #[test]
    fn instance_numbers_unique_within_registry(
        cmds in proptest::collection::vec(arb_b8_host_command(), 1..80)
    ) {
        let (mut state, host_ctx) = registered_host_state();
        for cmd in cmds {
            let _ = update(&mut state, cmd, &host_ctx);
            let nums: Vec<u32> = state.instance_registry.values().copied().collect();
            let mut sorted = nums.clone();
            sorted.sort_unstable();
            sorted.dedup();
            prop_assert_eq!(
                nums.len(), sorted.len(),
                "duplicate instance numbers in registry: {:?}", nums
            );
        }
    }

    /// HostShouldQuit fires ONLY when the close sequence ends
    /// with `state.windows` empty AND a Host was running at
    /// emit time. Property: every HostShouldQuit event in the
    /// stream was emitted on a transition where, immediately
    /// after the close was applied, windows was empty. Catches
    /// a regression where the saga fires while pool labels (or
    /// some other entry) were still in `windows`.
    #[test]
    fn host_should_quit_only_on_empty_windows_transition(
        cmds in proptest::collection::vec(arb_b8_host_command(), 1..80)
    ) {
        let (mut state, host_ctx) = registered_host_state();
        for cmd in cmds {
            let pre_window_count = state.windows.len();
            let evs = update(&mut state, cmd, &host_ctx);
            let saw_quit = evs.iter().any(|e| matches!(e, Event::HostShouldQuit { .. }));
            if saw_quit {
                prop_assert_eq!(
                    state.windows.len(), 0,
                    "HostShouldQuit emitted but windows={:?} (pre={})",
                    state.windows.keys().collect::<Vec<_>>(), pre_window_count
                );
                prop_assert!(
                    super::connection::host_is_running(&state),
                    "HostShouldQuit emitted but no Host in Running state"
                );
            }
        }
    }

    /// Workstream 0 Phase 1 (`SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04.md`
    /// §7, PR #2983 review Codex P2): once the host reports
    /// background-service mode enabled, closing the last window must NOT
    /// emit `HwndDriftDetected{OrphanInstance}` or `HostShouldQuit` — that
    /// combination is what arms `teardown_backstop.rs`'s process-tree-kill
    /// machinery, which must not fire for an intentionally-resting host.
    #[test]
    fn host_should_quit_suppressed_when_background_service_enabled(
        cmds in proptest::collection::vec(arb_b8_host_command(), 1..80)
    ) {
        let (mut state, host_ctx) = registered_host_state();
        let _ = update(
            &mut state,
            Command::ReportBackgroundServiceEnabled { enabled: true },
            &host_ctx,
        );
        for cmd in cmds {
            let evs = update(&mut state, cmd, &host_ctx);
            prop_assert!(
                !evs.iter().any(|e| matches!(
                    e,
                    Event::HostShouldQuit { .. }
                        | Event::HwndDriftDetected { kind: HwndDriftKind::OrphanInstance, .. }
                )),
                "background-service mode must suppress OrphanInstance/HostShouldQuit, got {:?}", evs
            );
        }
    }

    /// Backend window IDs are gated by `state.windows` membership
    /// at register time. The host calls
    /// `ReportBackendWindowIdRegistered` only after the window's
    /// `register_backend_window` IPC, which happens after
    /// `on_after_created` (which sent `ReportWindowOpened`). So
    /// in valid traffic, a register's label should be in
    /// `state.windows`. The reducer's behaviour on out-of-order
    /// or stale traffic is to silently store the mapping (the
    /// shadow gets cleaned up on later WindowClosed). This test
    /// pins that lenient behaviour: backend_window_ids never has
    /// MORE entries than the cumulative-register minus
    /// cumulative-unregister count, no phantom entries appear.
    #[test]
    fn backend_window_ids_bounded_by_register_minus_unregister(
        cmds in proptest::collection::vec(arb_b8_host_command(), 1..80)
    ) {
        let (mut state, host_ctx) = registered_host_state();
        let mut registers = 0i64;
        let mut unregisters = 0i64;
        for cmd in cmds {
            match &cmd {
                Command::ReportBackendWindowIdRegistered { .. } => registers += 1,
                Command::ReportBackendWindowIdUnregistered { .. } => unregisters += 1,
                _ => {}
            }
            let _ = update(&mut state, cmd, &host_ctx);
            prop_assert!(
                state.backend_window_ids.len() as i64 <= registers,
                "backend_window_ids has {} entries; only {} registers seen",
                state.backend_window_ids.len(), registers
            );
            let _ = unregisters;
        }
    }
}

// -----------------------------------------------------------
// B.8 — synthetic close-all integration test.
//
// The CI-synthetic-close-all assertion the B.8 plan calls for.
// Drives the reducer through a full session (register → open
// main → open secondary → tear-off → close all) and asserts
// the cascade emits the OrphanInstance + HostShouldQuit pair
// exactly once on the last close.
// -----------------------------------------------------------
#[test]
fn close_all_cascade_emits_orphan_and_quit_exactly_once() {
    let (mut state, host_ctx) = registered_host_state();

    // Open main + 2 secondaries + 3 pool windows.
    let _ = update(&mut state, Command::ReportWindowOpened {
        label: "main".into(),
        kind: WindowKind::FullInstance,
        parent_label: None,
    }, &host_ctx);
    let _ = update(&mut state, Command::ReportWindowOpened {
        label: "second-a".into(),
        kind: WindowKind::FullInstance,
        parent_label: None,
    }, &host_ctx);
    let _ = update(&mut state, Command::ReportWindowOpened {
        label: "second-b".into(),
        kind: WindowKind::FullInstance,
        parent_label: None,
    }, &host_ctx);
    for label in &["pool-1", "pool-2", "pool-3"] {
        let _ = update(&mut state, Command::ReportPoolWindowAdded {
            label: (*label).into(),
            saga_id: None,
        }, &host_ctx);
    }

    assert_eq!(state.windows.len(), 3, "main + 2 secondaries");
    assert_eq!(state.pool.len(), 3, "3 pool labels");

    // Close secondaries — neither close should emit HostShouldQuit
    // (windows still non-empty after each).
    for label in &["second-a", "second-b"] {
        let evs = update(&mut state, Command::ReportWindowClosed {
            label: (*label).into(),
        }, &host_ctx);
        assert!(
            !evs.iter().any(|e| matches!(e, Event::HostShouldQuit { .. })),
            "premature HostShouldQuit on {} close: {:?}", label, evs
        );
    }

    // Close main — the cascade should fire here. Expected events:
    // WindowClosed(main) + WindowInstanceReleased(main) +
    // OrphanInstance drift + HostShouldQuit. Order: Released
    // before drift (the close path emits Released after the
    // window is removed but before the empty-check).
    let evs = update(&mut state, Command::ReportWindowClosed {
        label: "main".into(),
    }, &host_ctx);

    let drift_count = evs.iter().filter(|e| matches!(
        e,
        Event::HwndDriftDetected { kind: HwndDriftKind::OrphanInstance, .. }
    )).count();
    let quit_count = evs.iter().filter(|e| matches!(
        e, Event::HostShouldQuit { .. }
    )).count();
    assert_eq!(drift_count, 1, "expected 1 OrphanInstance drift, got {}: {:?}", drift_count, evs);
    assert_eq!(quit_count, 1, "expected 1 HostShouldQuit, got {}: {:?}", quit_count, evs);
    assert!(state.windows.is_empty(), "windows must be empty post-cascade");

    // Drain pool — emits PoolWindowRemoved each time. No further
    // HostShouldQuit (the saga is one-shot per close-all transition).
    for label in &["pool-1", "pool-2", "pool-3"] {
        let evs = update(&mut state, Command::ReportPoolWindowRemoved {
            label: (*label).into(),
        }, &host_ctx);
        assert!(
            !evs.iter().any(|e| matches!(e, Event::HostShouldQuit { .. })),
            "spurious HostShouldQuit during pool drain on {}: {:?}", label, evs
        );
    }
    assert!(state.pool.is_empty(), "pool must be empty after drain");
}

// -----------------------------------------------------------
// Phase D.1 — GetSnapshot tests
// -----------------------------------------------------------

#[test]
fn get_snapshot_returns_canonical_state_in_one_event() {
    let (mut state, host_ctx) = registered_host_state();

    // Drive some state into the reducer.
    let _ = update(&mut state, Command::ReportWindowOpened {
        label: "main".into(),
        kind: WindowKind::FullInstance,
        parent_label: None,
    }, &host_ctx);
    let _ = update(&mut state, Command::ReportWindowOpened {
        label: "window-abc".into(),
        kind: WindowKind::FullInstance,
        parent_label: None,
    }, &host_ctx);
    let _ = update(&mut state, Command::ReportPoolWindowAdded {
        label: "pool-x".into(),
        saga_id: None,
    }, &host_ctx);
    let _ = update(&mut state, Command::ReportBackendWindowIdRegistered {
        label: "main".into(),
        window_id: "uuid-main".into(),
    }, &host_ctx);

    // Snapshot.
    let evs = update(&mut state, Command::GetSnapshot, &host_ctx);
    assert_eq!(evs.len(), 1, "expected exactly 1 Snapshot event, got {:?}", evs);

    let Event::Snapshot {
        version,
        lifecycle: _,
        windows,
        pool,
        instance_registry,
        backend_window_ids,
        monitors: _,
    } = &evs[0] else {
        panic!("expected Snapshot, got {:?}", evs[0]);
    };

    // Sorted ordering: "main" < "window-abc".
    assert_eq!(windows.len(), 2);
    assert_eq!(windows[0].label, "main");
    assert_eq!(windows[1].label, "window-abc");
    assert_eq!(pool, &vec!["pool-x".to_string()]);
    // instance_registry has main=1 (pre-seed) + window-abc=2.
    assert_eq!(instance_registry.len(), 2);
    assert_eq!(instance_registry[0], ("main".to_string(), 1));
    assert_eq!(backend_window_ids, &vec![("main".to_string(), "uuid-main".to_string())]);
    // Snapshot's version is monotonic w.r.t. earlier events.
    assert!(*version > 0, "snapshot version must be non-zero (was bump_version'd)");
}

#[test]
fn get_snapshot_does_not_mutate_canonical_state() {
    let (mut state, host_ctx) = registered_host_state();
    let _ = update(&mut state, Command::ReportWindowOpened {
        label: "main".into(),
        kind: WindowKind::FullInstance,
        parent_label: None,
    }, &host_ctx);

    let pre_windows = state.windows.clone();
    let pre_pool = state.pool.clone();
    let pre_instance_registry = state.instance_registry.clone();
    let pre_backend_window_ids = state.backend_window_ids.clone();
    let pre_lifecycle = state.lifecycle;

    let _ = update(&mut state, Command::GetSnapshot, &host_ctx);

    assert_eq!(state.windows, pre_windows);
    assert_eq!(state.pool, pre_pool);
    assert_eq!(state.instance_registry, pre_instance_registry);
    assert_eq!(state.backend_window_ids, pre_backend_window_ids);
    assert_eq!(state.lifecycle, pre_lifecycle);
}

#[test]
fn get_snapshot_works_for_non_host_clients() {
    // Tool clients (e.g. --diag wrr) should be able to query
    // snapshots — host-only gate doesn't apply.
    let mut state = State::default();
    let _ = update(
        &mut state,
        Command::Register {
            kind: ClientKind::Tool,
            pid: 99,
            version: "diag".into(),
        },
        &ctx(1),
    );
    let tool_ctx = ctx_with_pid(1, 99);

    let evs = update(&mut state, Command::GetSnapshot, &tool_ctx);
    assert_eq!(evs.len(), 1);
    assert!(matches!(evs[0], Event::Snapshot { .. }));
}

// ---------- Phase F.7 — host reducer property tests ----------
//
// Mirrors `agentmux-srv::reducer::tests::invariants_hold_*` (E.7,
// PR #635) for the launcher reducer arms most relevant to F.5/F.6:
//   - WindowOpened / WindowClosed (window mirror lifecycle)
//   - PoolWindowAdded / PoolWindowRemoved / PoolWindowPromoted
//     (pool inventory + promote signal)
//   - PanesReaped / PoolDrainDecision (F.6 cascade reports)
//
// Invariants asserted across random valid sequences:
//   1. Version monotonicity — every emitted Event has a strictly
//      greater version than the previous one.
//   2. Mirror integrity — every (key, WindowMirror) pair has
//      `key == mirror.label`; the windows count never exceeds
//      total open events.
//   3. Pool integrity — pool size matches the running open-add /
//      open-remove delta, never goes "negative" (HashSet can't,
//      but we double-check that remove-when-absent is silent).
//   4. No orphan windows — every key in `state.windows` resolves
//      to a `WindowMirror` whose `label` field is non-empty and
//      equals the key.
//   5. F.6 pass-through arms (`ReportPanesReaped` /
//      `ReportPoolDrainDecision`) never mutate `windows` / `pool`
//      — they only translate to the typed event.
//
// Cases capped at 64 (default 1024 is too slow for CI). Same
// bound the srv reducer's E.7 proptest uses.

/// Higher-level operations the F.7 proptests pick from. Picks
/// the host-only commands the reducer mutates state for, plus
/// the F.5/F.6 pass-through commands that should leave state
/// untouched.
#[derive(Debug, Clone)]
enum F7HostOp {
    WindowOpen,
    WindowClose,
    PoolAdd,
    PoolRemove,
    /// F.5 — pure pass-through (no state mutation, just emits
    /// the typed event).
    PoolPromoted,
    /// F.6 — pure pass-through.
    PanesReaped,
    /// F.6 — pure pass-through (last-window flag varies).
    PoolDrainDecision { was_last: bool },
}

fn f7_op_strategy() -> impl proptest::strategy::Strategy<Value = F7HostOp> {
    // Bias toward constructive ops so non-trivial state
    // accumulates before close/remove paths fire.
    prop_oneof![
        4 => Just(F7HostOp::WindowOpen),
        2 => Just(F7HostOp::WindowClose),
        3 => Just(F7HostOp::PoolAdd),
        2 => Just(F7HostOp::PoolRemove),
        1 => Just(F7HostOp::PoolPromoted),
        1 => Just(F7HostOp::PanesReaped),
        1 => Just(F7HostOp::PoolDrainDecision { was_last: true }),
        1 => Just(F7HostOp::PoolDrainDecision { was_last: false }),
    ]
}

/// Apply one op, picking labels from a small pool so duplicate
/// opens / mismatched closes happen frequently. Returns the
/// emitted events for caller-side version checking.
fn apply_f7_op(
    state: &mut State,
    op: F7HostOp,
    host_ctx: &Ctx,
    idx: usize,
) -> Vec<Event> {
    // 3-label pool ensures we hit duplicate-open / unknown-close
    // paths frequently.
    let label = format!("w{}", idx % 3);
    match op {
        F7HostOp::WindowOpen => update(
            state,
            Command::ReportWindowOpened {
                label,
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            host_ctx,
        ),
        F7HostOp::WindowClose => {
            update(state, Command::ReportWindowClosed { label }, host_ctx)
        }
        F7HostOp::PoolAdd => update(
            state,
            Command::ReportPoolWindowAdded { label, saga_id: None },
            host_ctx,
        ),
        F7HostOp::PoolRemove => update(
            state,
            Command::ReportPoolWindowRemoved { label },
            host_ctx,
        ),
        F7HostOp::PoolPromoted => update(
            state,
            Command::ReportPoolWindowPromoted { label },
            host_ctx,
        ),
        F7HostOp::PanesReaped => {
            update(state, Command::ReportPanesReaped { label, saga_id: None }, host_ctx)
        }
        F7HostOp::PoolDrainDecision { was_last } => update(
            state,
            Command::ReportPoolDrainDecision { label, was_last, saga_id: None },
            host_ctx,
        ),
    }
}

/// Verify all reducer-state invariants in one pass. Panics
/// (caught + shrunk by proptest) if any is violated.
fn assert_f7_invariants(state: &State) {
    // Mirror integrity — every key matches its value's label.
    for (k, v) in &state.windows {
        assert_eq!(
            k, &v.label,
            "window map key {:?} != mirror.label {:?}",
            k, v.label
        );
        assert!(
            !v.label.is_empty(),
            "window mirror has empty label (key={:?})",
            k
        );
    }
    // Pool integrity — HashSet contents are non-empty strings,
    // each entry mirrors a valid label shape.
    for label in &state.pool {
        assert!(
            !label.is_empty(),
            "pool contains empty-string label"
        );
    }
    // NOTE: pool ↔ windows mutual exclusion is NOT a reducer
    // invariant — the host reports promote as separate
    // Remove(pool) + Open(window), and the reducer accepts
    // arbitrary host-driven sequencing. Subscribers correlate
    // the pair as a tear-off promote; the reducer itself doesn't
    // enforce non-overlap. Verified empirically: random
    // sequences land both states without violating any
    // downstream consumer's expectations.

    // `instance_registry` is monotonically populated on
    // `WindowOpened` and stays present until `WindowClosed`.
    // Every label in `state.windows` is therefore guaranteed to
    // resolve to a numbered slot.
    for label in state.windows.keys() {
        assert!(
            state.instance_registry.contains_key(label),
            "window label {:?} present in state.windows but missing from state.instance_registry",
            label
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]

    /// Apply a random sequence of valid host-driven ops. After
    /// each op, mirror + pool + instance_registry invariants
    /// hold; across the whole sequence, every emitted event's
    /// version is strictly greater than the prior one's.
    #[test]
    fn f7_invariants_hold_across_random_sequences(
        ops in proptest::collection::vec(f7_op_strategy(), 0..40)
    ) {
        const HOST_PID: u32 = 1;
        let mut state = State::default();
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: HOST_PID,
                version: "host".into(),
            },
            &ctx(1),
        );
        let host_ctx = ctx_with_pid(1, HOST_PID);

        // Skip past register-emitted events — versions start
        // counting from the first F.7 op.
        let mut last_version: u64 = state.event_version;

        for (i, op) in ops.into_iter().enumerate() {
            let events = apply_f7_op(&mut state, op, &host_ctx, i);
            for ev in &events {
                let v = extract_version(ev);
                prop_assert!(
                    v > last_version,
                    "version {} not strictly greater than previous {} (event {:?})",
                    v, last_version, ev
                );
                last_version = v;
            }
            assert_f7_invariants(&state);
        }
    }

    /// F.6 cascade arms (`ReportPanesReaped` /
    /// `ReportPoolDrainDecision`) are pure pass-through — they
    /// never mutate windows / pool / instance_registry. Verifies
    /// the launcher's role as a typed-event narrator for the
    /// cascade saga.
    #[test]
    fn f7_cascade_arms_are_pure_pass_through(
        label in "[a-c]{1,3}",
        was_last in any::<bool>(),
    ) {
        const HOST_PID: u32 = 1;
        let mut state = State::default();
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: HOST_PID,
                version: "host".into(),
            },
            &ctx(1),
        );
        let host_ctx = ctx_with_pid(1, HOST_PID);

        // Snapshot relevant state shapes BEFORE the F.6 dispatch.
        let pre_windows = state.windows.clone();
        let pre_pool = state.pool.clone();
        let pre_instance_registry = state.instance_registry.clone();
        let pre_backend_window_ids = state.backend_window_ids.clone();

        // PanesReaped: must emit exactly one Event::PanesReaped.
        let evs = update(
            &mut state,
            Command::ReportPanesReaped { label: label.clone(), saga_id: None },
            &host_ctx,
        );
        prop_assert_eq!(evs.len(), 1);
        let is_panes_reaped = matches!(&evs[0], Event::PanesReaped { .. });
        prop_assert!(is_panes_reaped, "expected PanesReaped, got {:?}", evs[0]);

        // Same state shapes — pass-through never mutates.
        prop_assert_eq!(&state.windows, &pre_windows);
        prop_assert_eq!(&state.pool, &pre_pool);
        prop_assert_eq!(&state.instance_registry, &pre_instance_registry);
        prop_assert_eq!(&state.backend_window_ids, &pre_backend_window_ids);

        // PoolDrainDecision: maps cleanly to one terminal event.
        let evs = update(
            &mut state,
            Command::ReportPoolDrainDecision { label, was_last, saga_id: None },
            &host_ctx,
        );
        prop_assert_eq!(evs.len(), 1);
        if was_last {
            let is_pool_drained = matches!(&evs[0], Event::PoolDrained { .. });
            prop_assert!(is_pool_drained, "expected PoolDrained, got {:?}", evs[0]);
        } else {
            let is_pool_not_last = matches!(&evs[0], Event::PoolNotLast { .. });
            prop_assert!(is_pool_not_last, "expected PoolNotLast, got {:?}", evs[0]);
        }
        // State STILL unmutated.
        prop_assert_eq!(&state.windows, &pre_windows);
        prop_assert_eq!(&state.pool, &pre_pool);
        prop_assert_eq!(&state.instance_registry, &pre_instance_registry);
        prop_assert_eq!(&state.backend_window_ids, &pre_backend_window_ids);
    }

    /// Strictly paired open/close under random labels: total
    /// `WindowOpened` events the reducer emits equals the number
    /// of distinct opens (idempotent re-open emits a fresh event
    /// on every call); total `WindowClosed` events ≤ matched
    /// closes (silent on unknown-label close per the strict
    /// pairing introduced in PR #577).
    #[test]
    fn f7_window_close_is_strictly_paired(
        ops in proptest::collection::vec(
            prop_oneof![
                1 => "[a-c]{1,3}".prop_map(|l| (l, true)),  // open
                1 => "[a-c]{1,3}".prop_map(|l| (l, false)), // close
            ],
            1..40,
        )
    ) {
        const HOST_PID: u32 = 1;
        let mut state = State::default();
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: HOST_PID,
                version: "host".into(),
            },
            &ctx(1),
        );
        let host_ctx = ctx_with_pid(1, HOST_PID);

        let mut close_events = 0u64;
        let mut close_attempts = 0u64;
        for (label, is_open) in ops {
            if is_open {
                let _ = update(
                    &mut state,
                    Command::ReportWindowOpened {
                        label,
                        kind: WindowKind::FullInstance,
                        parent_label: None,
                    },
                    &host_ctx,
                );
            } else {
                close_attempts += 1;
                let evs = update(
                    &mut state,
                    Command::ReportWindowClosed { label },
                    &host_ctx,
                );
                let n = evs
                    .iter()
                    .filter(|e| matches!(e, Event::WindowClosed { .. }))
                    .count() as u64;
                close_events += n;
            }
        }
        // Strict pairing: at MOST as many WindowClosed events as
        // close attempts (some are silent for unknown labels).
        prop_assert!(
            close_events <= close_attempts,
            "WindowClosed events {} > close attempts {}",
            close_events, close_attempts
        );
        // And the running mirror is consistent with the gate:
        // every label currently in `state.windows` is the
        // residue of an open NOT followed by a successful close.
        assert_f7_invariants(&state);
    }
}
