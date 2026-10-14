// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::state::{BrowserPaneLifecycle, WindowKind};

/// A fixed instant for commands that carry the caller's clock reading.
/// Arms only store it, so any value works; one value keeps tests deterministic.
fn t0() -> std::time::Instant {
    use std::sync::OnceLock;
    static T0: OnceLock<std::time::Instant> = OnceLock::new();
    *T0.get_or_init(std::time::Instant::now)
}

fn entry(label: &str) -> PendingWindowCreation {
    PendingWindowCreation {
        label: label.to_string(),
        kind: WindowKind::FullInstance,
        parent_instance_id: None,
    }
}

#[test]
fn enqueue_then_dequeue_round_trips() {
    let mut state = HostState::default();
    let out1 = update(
        &mut state,
        HostCommand::EnqueuePendingWindowCreation { entry: entry("w1") },
    );
    assert!(matches!(
        out1.events.as_slice(),
        [HostEvent::PendingWindowEnqueued { queue_len_after: 1, .. }]
    ));
    assert!(out1.dequeued.is_none());

    let out2 = update(&mut state, HostCommand::DequeuePendingWindowCreation);
    assert!(matches!(
        out2.events.as_slice(),
        [HostEvent::PendingWindowDequeued { queue_len_after: 0, .. }]
    ));
    assert_eq!(out2.dequeued.as_ref().unwrap().label, "w1");
}

#[test]
fn dequeue_on_empty_returns_queue_empty_event() {
    let mut state = HostState::default();
    let out = update(&mut state, HostCommand::DequeuePendingWindowCreation);
    assert!(matches!(
        out.events.as_slice(),
        [HostEvent::PendingWindowQueueEmpty { .. }]
    ));
    assert!(out.dequeued.is_none());
}

#[test]
fn fifo_order_preserved() {
    let mut state = HostState::default();
    for label in ["w1", "w2", "w3"] {
        update(
            &mut state,
            HostCommand::EnqueuePendingWindowCreation { entry: entry(label) },
        );
    }
    for expected in ["w1", "w2", "w3"] {
        let out = update(&mut state, HostCommand::DequeuePendingWindowCreation);
        assert_eq!(out.dequeued.as_ref().unwrap().label, expected);
    }
}

/// Two creations queued together can finish in either order. Each browser
/// takes its own entry by label, and the other stays queued for its owner.
/// (2026-10-05: a pane-pool and a window-pool window refilled together got
/// each other's labels through the FIFO pop.)
#[test]
fn take_by_label_removes_that_entry_wherever_it_is() {
    let mut state = HostState::default();
    for label in ["floating-pool-a", "window-pool-b", "window-c"] {
        update(&mut state, HostCommand::EnqueuePendingWindowCreation { entry: entry(label) });
    }
    // The second-queued window's browser arrives first.
    let out = update(
        &mut state,
        HostCommand::TakePendingWindowCreation { label: "window-pool-b".into() },
    );
    assert_eq!(out.dequeued.as_ref().unwrap().label, "window-pool-b");
    assert!(matches!(
        out.events.as_slice(),
        [HostEvent::PendingWindowDequeued { queue_len_after: 2, .. }]
    ));
    // The first-queued one is still there for its own browser.
    let out = update(
        &mut state,
        HostCommand::TakePendingWindowCreation { label: "floating-pool-a".into() },
    );
    assert_eq!(out.dequeued.as_ref().unwrap().label, "floating-pool-a");
    // And the FIFO fallback still sees what's left, in order.
    let out = update(&mut state, HostCommand::DequeuePendingWindowCreation);
    assert_eq!(out.dequeued.as_ref().unwrap().label, "window-c");
}

#[test]
fn take_by_label_misses_without_touching_the_queue() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::EnqueuePendingWindowCreation { entry: entry("w1") });
    let out = update(
        &mut state,
        HostCommand::TakePendingWindowCreation { label: "w2".into() },
    );
    assert!(out.dequeued.is_none());
    assert!(matches!(
        out.events.as_slice(),
        [HostEvent::PendingWindowNotFound { label, .. }] if label == "w2"
    ));
    assert_eq!(state.pending_window_creations.len(), 1);
}

#[test]
fn version_increments_monotonically() {
    let mut state = HostState::default();
    let out1 = update(
        &mut state,
        HostCommand::EnqueuePendingWindowCreation { entry: entry("w1") },
    );
    let out2 = update(&mut state, HostCommand::DequeuePendingWindowCreation);
    let out3 = update(&mut state, HostCommand::DequeuePendingWindowCreation);

    // Helper that pulls the `version` field out of any HostEvent.
    // Kept exhaustive so adding a new event variant forces an
    // explicit decision here (vs. silently defaulting to 0).
    let extract_version = |events: &[HostEvent]| match &events[0] {
        HostEvent::PendingWindowEnqueued { version, .. } => *version,
        HostEvent::PendingWindowDequeued { version, .. } => *version,
        HostEvent::PendingWindowQueueEmpty { version } => *version,
        HostEvent::PendingWindowNotFound { version, .. } => *version,
        HostEvent::BrowserPaneCreateRequested { version, .. } => *version,
        HostEvent::BrowserPaneClosing { version, .. } => *version,
        HostEvent::BrowserPaneClosed { version, .. } => *version,
        HostEvent::BrowserRegistered { version, .. } => *version,
        HostEvent::BrowserUnregistered { version, .. } => *version,
        HostEvent::DragStarted { version, .. } => *version,
        HostEvent::DragEnded { version, .. } => *version,
        HostEvent::PoolWindowEntered { version, .. } => *version,
        HostEvent::PoolWindowLeft { version, .. } => *version,
        HostEvent::QuitDraining { version, .. } => *version,
        HostEvent::WindowOpacityApplied { version, .. } => *version,
        HostEvent::WindowOpacityCleared { version, .. } => *version,
        HostEvent::PaneWindowStateChanged { version, .. } => *version,
        HostEvent::FloatingAlwaysOnTopChanged { version, .. } => *version,
        HostEvent::Error { version, .. } => *version,
    };
    let v1 = extract_version(&out1.events);
    let v2 = extract_version(&out2.events);
    let v3 = extract_version(&out3.events);
    assert!(v1 < v2);
    assert!(v2 < v3);
}

// ── Phase H foundations tests ───────────────────────────────────────

// ── H.1 panes ────────────────────────────────────────────────────────

#[test]
fn pane_close_lifecycle() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into(), pending: None });
    update(&mut state, HostCommand::EnqueueBrowserPaneClose { block_id: "b1".into(), now: t0() });
    assert!(matches!(
        state.browser_panes["b1"].lifecycle,
        BrowserPaneLifecycle::Closing { .. }
    ));
    update(&mut state, HostCommand::CompleteBrowserPaneClose { block_id: "b1".into() });
    assert!(!state.browser_panes.contains_key("b1"));
}

#[test]
fn pane_close_idempotent_for_missing() {
    let mut state = HostState::default();
    let out = update(&mut state, HostCommand::EnqueueBrowserPaneClose { block_id: "missing".into(), now: t0() });
    assert!(out.events.is_empty()); // idempotent no-op
    // The load-bearing property for issue #2218 B.4:
    // BrowserPaneManager::close() (browser_panes/close.rs) checks exactly this
    // field to decide whether to do any HWND/UI-thread work at all. B.4
    // calls close() unconditionally for every block_id cascaded out of a
    // deleted tab/workspace (most of which are never browser panes), so
    // this field staying None for an unknown block_id is what makes that
    // safe — pin it explicitly, not just via the events-are-empty proxy
    // above.
    assert!(out.closed_browser_pane_label.is_none());
}

// ── H.1.d (PR #5) — TryRegisterBrowserPaneLive / EnqueueBrowserPaneClose return-values
//   / DrainBrowserPaneByLabel ────────────────────────────────────────────────

#[test]
fn try_register_browser_pane_live_fresh_returns_label_and_inserts_live() {
    let mut state = HostState::default();
    let out = update(
        &mut state,
        HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into(), pending: None },
    );
    let label = match out.browser_pane_register_result {
        Some(RegisterResult::Fresh(l)) => l,
        other => panic!("expected Fresh(_), got {:?}", other),
    };
    // The sequence lives in HostState and starts at 1, so a fresh state's
    // first label is fixed.
    assert_eq!(label, "browser-pane-b1-1");
    assert_eq!(state.next_browser_pane_label_seq, 2);
    assert_eq!(state.browser_panes["b1"].label, label);
    assert!(matches!(state.browser_panes["b1"].lifecycle, BrowserPaneLifecycle::Live));
    assert!(matches!(out.events[0], HostEvent::BrowserPaneCreateRequested { .. }));
}

#[test]
fn try_register_browser_pane_live_already_live_returns_existing_label() {
    let mut state = HostState::default();
    let first = match update(
        &mut state,
        HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into(), pending: None },
    ).browser_pane_register_result
    {
        Some(RegisterResult::Fresh(l)) => l,
        _ => unreachable!(),
    };
    let out = update(
        &mut state,
        HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into(), pending: None },
    );
    match out.browser_pane_register_result {
        Some(RegisterResult::AlreadyLive(l)) => assert_eq!(l, first),
        other => panic!("expected AlreadyLive, got {:?}", other),
    }
    assert!(out.events.is_empty(), "no event for AlreadyLive — caller just navigates");
}

fn pending_in(window: &str) -> crate::state::PendingBrowserPaneCreate {
    crate::state::PendingBrowserPaneCreate {
        url: "https://agentmux.ai".into(),
        x: 0,
        y: 0,
        width: 800,
        height: 600,
        window_label: window.into(),
    }
}

#[test]
fn try_register_same_window_relive_returns_already_live() {
    // A re-create targeting the SAME window the pane already lives in is a
    // genuine re-navigation — keep the AlreadyLive path, no pending stash.
    let mut state = HostState::default();
    let first = match update(
        &mut state,
        HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into(), pending: Some(pending_in("main")) },
    ).browser_pane_register_result
    {
        Some(RegisterResult::Fresh(l)) => l,
        _ => unreachable!(),
    };
    assert_eq!(state.browser_panes["b1"].window_label, "main");
    let out = update(
        &mut state,
        HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into(), pending: Some(pending_in("main")) },
    );
    match out.browser_pane_register_result {
        Some(RegisterResult::AlreadyLive(l)) => assert_eq!(l, first),
        other => panic!("expected AlreadyLive, got {:?}", other),
    }
    assert!(!state.pending_browser_pane_creates.contains_key("b1"), "same-window re-nav must not stash a pending create");
}

#[test]
fn try_register_cross_window_returns_already_live_elsewhere_and_stashes() {
    // A create targeting a DIFFERENT window (tear-off / redock) must NOT
    // re-navigate in place — it returns AlreadyLiveElsewhere and stashes the
    // pending create so the caller's close-completion replays it in the new
    // window. This is the black-screen fix.
    let mut state = HostState::default();
    let first = match update(
        &mut state,
        HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into(), pending: Some(pending_in("main")) },
    ).browser_pane_register_result
    {
        Some(RegisterResult::Fresh(l)) => l,
        _ => unreachable!(),
    };
    let out = update(
        &mut state,
        HostCommand::TryRegisterBrowserPaneLive {
            block_id: "b1".into(),
            pending: Some(pending_in("floating-abc")),
        },
    );
    match out.browser_pane_register_result {
        Some(RegisterResult::AlreadyLiveElsewhere(l)) => assert_eq!(l, first),
        other => panic!("expected AlreadyLiveElsewhere, got {:?}", other),
    }
    // Pending create was stashed, targeting the new window, for replay on close.
    let stashed = state.pending_browser_pane_creates.get("b1").expect("pending create must be stashed");
    assert_eq!(stashed.window_label, "floating-abc");
    // The old entry is still Live (caller will close it next).
    assert!(matches!(state.browser_panes["b1"].lifecycle, BrowserPaneLifecycle::Live));
}

#[test]
fn try_register_browser_pane_live_closing_returns_closing() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into(), pending: None });
    update(&mut state, HostCommand::EnqueueBrowserPaneClose { block_id: "b1".into(), now: t0() });
    let out = update(
        &mut state,
        HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into(), pending: None },
    );
    assert!(matches!(out.browser_pane_register_result, Some(RegisterResult::Closing)));
}

#[test]
fn enqueue_browser_pane_close_returns_label_for_live_entry() {
    let mut state = HostState::default();
    let label = match update(
        &mut state,
        HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into(), pending: None },
    ).browser_pane_register_result
    {
        Some(RegisterResult::Fresh(l)) => l,
        _ => unreachable!(),
    };
    let out = update(
        &mut state,
        HostCommand::EnqueueBrowserPaneClose { block_id: "b1".into(), now: t0() },
    );
    assert_eq!(out.closed_browser_pane_label, Some(label));
}

#[test]
fn enqueue_browser_pane_close_returns_none_for_already_closing() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into(), pending: None });
    update(&mut state, HostCommand::EnqueueBrowserPaneClose { block_id: "b1".into(), now: t0() });
    let out = update(
        &mut state,
        HostCommand::EnqueueBrowserPaneClose { block_id: "b1".into(), now: t0() },
    );
    assert!(out.closed_browser_pane_label.is_none());
}

#[test]
fn drain_browser_pane_by_label_removes_entry_and_returns_block_id() {
    let mut state = HostState::default();
    let label = match update(
        &mut state,
        HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into(), pending: None },
    ).browser_pane_register_result
    {
        Some(RegisterResult::Fresh(l)) => l,
        _ => unreachable!(),
    };
    let out = update(&mut state, HostCommand::DrainBrowserPaneByLabel { label });
    assert_eq!(out.drained_browser_pane_block_id, Some("b1".to_string()));
    assert!(!state.browser_panes.contains_key("b1"));
    assert!(matches!(out.events[0], HostEvent::BrowserPaneClosed { .. }));
}

#[test]
fn drain_browser_pane_by_label_idempotent_on_miss() {
    let mut state = HostState::default();
    let out = update(
        &mut state,
        HostCommand::DrainBrowserPaneByLabel { label: "no-such-label".into() },
    );
    assert!(out.drained_browser_pane_block_id.is_none());
    assert!(out.events.is_empty());
}

#[test]
fn pane_lifecycle_supports_h7_invariant_check() {
    // PR #6 H.7 — `AppState::any_browser_pane_closing()` reads
    // `state.browser_panes.values()` and matches `BrowserPaneLifecycle::Closing`.
    // This test verifies the reducer's pane state transitions in
    // the way that helper relies on: Live entries don't trip the
    // gate; Closing entries do; drained (removed) entries don't.
    let mut state = HostState::default();

    // baseline: no panes → no closing
    assert!(!state.browser_panes.values().any(|e| matches!(e.lifecycle, BrowserPaneLifecycle::Closing { .. })));

    update(&mut state, HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into(), pending: None });
    // Live pane present → gate is OPEN (not closing)
    assert!(!state.browser_panes.values().any(|e| matches!(e.lifecycle, BrowserPaneLifecycle::Closing { .. })));

    update(&mut state, HostCommand::EnqueueBrowserPaneClose { block_id: "b1".into(), now: t0() });
    // Closing pane present → gate is CLOSED
    assert!(state.browser_panes.values().any(|e| matches!(e.lifecycle, BrowserPaneLifecycle::Closing { .. })));

    update(&mut state, HostCommand::CompleteBrowserPaneClose { block_id: "b1".into() });
    // Drained → gate is OPEN again
    assert!(!state.browser_panes.values().any(|e| matches!(e.lifecycle, BrowserPaneLifecycle::Closing { .. })));
}

#[test]
fn drain_after_close_recreate_does_not_evict_new_entry() {
    // The exact bug the pane-label sequence defends against: register → close →
    // drain by OLD label → register again → drain by OLD label must
    // NOT evict the new entry.
    let mut state = HostState::default();
    let first = match update(
        &mut state,
        HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into(), pending: None },
    ).browser_pane_register_result
    {
        Some(RegisterResult::Fresh(l)) => l,
        _ => unreachable!(),
    };
    update(&mut state, HostCommand::EnqueueBrowserPaneClose { block_id: "b1".into(), now: t0() });
    update(&mut state, HostCommand::DrainBrowserPaneByLabel { label: first.clone() });

    // re-register — gets a different label
    let second = match update(
        &mut state,
        HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into(), pending: None },
    ).browser_pane_register_result
    {
        Some(RegisterResult::Fresh(l)) => l,
        _ => unreachable!(),
    };
    assert_ne!(first, second);

    // late on_before_close for the OLD browser tries to drain by old label
    let stale = update(&mut state, HostCommand::DrainBrowserPaneByLabel { label: first });
    assert!(stale.drained_browser_pane_block_id.is_none(), "stale drain must not evict the new entry");
    assert!(state.browser_panes.contains_key("b1"), "new entry must survive stale drain");
    assert_eq!(state.browser_panes["b1"].label, second);
}

// Deferred-create stash/replay (redock load race, #1168). A create that hits
// `Closing` stashes its params IN THE REDUCER, atomically with the Closing
// observation; the close-completion arm removes and hands them back to replay.
#[test]
fn closing_register_stashes_pending_and_complete_close_returns_it() {
    use crate::state::PendingBrowserPaneCreate;
    let mut state = HostState::default();

    // b1 is Live, then close-requested → Closing.
    update(&mut state, HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into(), pending: None });
    update(&mut state, HostCommand::EnqueueBrowserPaneClose { block_id: "b1".into(), now: t0() });

    // Re-register while Closing, WITH pending params (the redock re-create).
    let pend = PendingBrowserPaneCreate {
        url: "https://x".into(), x: 1, y: 2, width: 3, height: 4, window_label: "main".into(),
    };
    let out = update(
        &mut state,
        HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into(), pending: Some(pend.clone()) },
    );
    assert!(matches!(out.browser_pane_register_result, Some(RegisterResult::Closing)));
    assert!(out.pending_browser_pane_create_to_replay.is_none(), "stash isn't returned until close completes");
    assert!(state.pending_browser_pane_creates.contains_key("b1"), "pending create stashed in the reducer");

    // Close completes → the stash is removed AND handed back to replay.
    let done = update(&mut state, HostCommand::CompleteBrowserPaneClose { block_id: "b1".into() });
    let (bid, returned) = done.pending_browser_pane_create_to_replay.expect("deferred create returned to replay");
    assert_eq!(bid, "b1");
    assert_eq!(returned.url, "https://x");
    assert_eq!((returned.x, returned.y, returned.width, returned.height), (1, 2, 3, 4));
    assert!(!state.pending_browser_pane_creates.contains_key("b1"), "stash removed on close (no leak)");
}

// ── H.3 drag (singleton invariant) ───────────────────────────────────

fn drag_session(id: &str) -> DragSession {
    DragSession {
        drag_id: id.to_string(),
        drag_type: crate::state::DragType::Tab,
        source_window: "main".to_string(),
        source_workspace_id: "ws1".to_string(),
        source_tab_id: "tab1".to_string(),
        payload: crate::state::DragPayload {
            block_id: None,
            tab_id: Some("tab1".to_string()),
        },
        started_at: 0,
    }
}

#[test]
fn drag_singleton_invariant() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::StartDrag { session: drag_session("d1") });
    assert!(state.active_drag.is_some());
    let out = update(&mut state, HostCommand::StartDrag { session: drag_session("d2") });
    assert!(matches!(out.events[0], HostEvent::Error { .. }));
    assert_eq!(state.active_drag.as_ref().unwrap().drag_id, "d1");
}

#[test]
fn drag_end_clears_session() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::StartDrag { session: drag_session("d1") });
    update(
        &mut state,
        HostCommand::EndDrag {
            drag_id: "d1".into(),
            outcome: DragOutcome::Cancelled,
        },
    );
    assert!(state.active_drag.is_none());
}

#[test]
fn drag_end_with_wrong_id_is_noop() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::StartDrag { session: drag_session("d1") });
    let out = update(
        &mut state,
        HostCommand::EndDrag {
            drag_id: "wrong".into(),
            outcome: DragOutcome::Cancelled,
        },
    );
    assert!(out.events.is_empty());
    assert!(state.active_drag.is_some());
}

// ── H.5 quit (monotonic transitions) ─────────────────────────────────

#[test]
fn quit_state_monotonic() {
    let mut state = HostState::default();
    assert_eq!(state.quit_state, QuitState::Running);
    update(&mut state, HostCommand::BeginDrain { reason: QuitReason::LastWindowClosed });
    assert!(matches!(state.quit_state, QuitState::Draining { .. }));
    // Subsequent BeginDrain is a no-op (monotonic): the first reason sticks.
    let out = update(&mut state, HostCommand::BeginDrain { reason: QuitReason::External });
    assert!(out.events.is_empty());
    assert_eq!(
        state.quit_state,
        QuitState::Draining { reason: QuitReason::LastWindowClosed }
    );
}

// ── H.4 pool ─────────────────────────────────────────────────────────

#[test]
fn pool_spawn_then_ready_enters_queue() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::PoolWindowSpawnStart { label: "p1".into() });
    assert!(state.pool.unpromoted.contains("p1"));
    assert!(state.pool.respawn_in_flight);
    update(&mut state, HostCommand::PoolWindowReady { label: "p1".into() });
    assert!(!state.pool.unpromoted.contains("p1"));
    assert_eq!(state.pool.queue.len(), 1);
    assert!(!state.pool.respawn_in_flight);
}

#[test]
fn pool_spawn_during_quit_suppressed() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::BeginDrain { reason: QuitReason::LastWindowClosed });
    let out = update(&mut state, HostCommand::PoolWindowSpawnStart { label: "p1".into() });
    assert!(out.events.is_empty()); // suppressed
    assert!(state.pool.unpromoted.is_empty());
}

/// Regression test for codex P1 on PR #654 round 2.
///
/// `PoolWindowReady` moves a label from `unpromoted` to `queue`.
/// If the window is then destroyed externally before promotion,
/// the destroy handler must scrub the label from BOTH sets — not
/// just `unpromoted`. Otherwise dead inventory remains in `queue`
/// and a later `PromotePoolWindow` operates on a stale label.
#[test]
fn pool_destroy_after_ready_clears_queue() {
    let mut state = HostState::default();
    // Step 1: spawn + ready → label lands in queue.
    update(&mut state, HostCommand::PoolWindowSpawnStart { label: "p1".into() });
    update(&mut state, HostCommand::PoolWindowReady { label: "p1".into() });
    assert_eq!(state.pool.queue.len(), 1);
    assert!(!state.pool.unpromoted.contains("p1"));
    // Step 2: external destroy after ready, before promote.
    let out = update(
        &mut state,
        HostCommand::PoolWindowDestroyedBeforePromote { label: "p1".into() },
    );
    // CRITICAL: queue must be drained.
    assert!(state.pool.queue.is_empty(), "queue must not retain destroyed label");
    assert!(state.pool.unpromoted.is_empty());
    // Event must fire (we did real cleanup).
    assert!(
        out.events.iter().any(|e| matches!(e, HostEvent::PoolWindowLeft { reason: PoolLeaveReason::DestroyedBeforePromote, .. })),
        "PoolWindowLeft event must fire for queue-state destroy"
    );
}

// ── Top-level window-pool eviction under memory pressure (issue #1936/#2218
// follow-up; both closed this as explicitly deferred, "track separately if
// it comes up again" — docs/incident/INCIDENT_2026_09_20_APP_CLOSED.md is
// that recurrence) ──────────────────────────────────────────────────────
//
// `evict_idle_pool_window` (commands/window_pool.rs) claims the front
// window-pool label atomically via `PopFrontPoolWindowForEviction`, mirroring
// `PopFrontPanePoolWindowForEviction`'s own reasoning exactly (mutual
// exclusion with a concurrent real promote via `PopAndPromoteFrontPoolWindow`
// for the same front label — same mutex-guarded-dispatch argument, same
// pool-command shape).

/// Mirrors `pop_front_pane_pool_for_eviction_pops_and_clears_state` for the
/// top-level window pool: pops the front label, clears it from
/// `unpromoted`, and resets `respawn_in_flight` in one dispatch — same as
/// `PopAndPromoteFrontPoolWindow` does for the promote path, which is what
/// makes the two commands mutually exclusive for a given front label.
#[test]
fn pop_front_pool_for_eviction_pops_and_clears_state() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::PoolWindowSpawnStart { label: "p1".into() });
    update(&mut state, HostCommand::PoolWindowReady { label: "p1".into() });
    assert_eq!(state.pool.queue.len(), 1);

    let out = update(&mut state, HostCommand::PopFrontPoolWindowForEviction);
    assert!(state.pool.queue.is_empty(), "queue must not retain the evicted label");
    assert!(state.pool.unpromoted.is_empty());
    assert!(!state.pool.respawn_in_flight);
    assert_eq!(out.evicted_pool_label, Some("p1".to_string()));
    assert_eq!(out.pool_size_after, Some(0));
}

/// Empty queue must be a genuine no-op — `evict_idle_pool_window` relies on
/// `evicted_pool_label: None` to bail out cleanly when pressure fires with
/// nothing left to evict (the memory heartbeat loops on this return value
/// until it hits this case).
#[test]
fn pop_front_pool_for_eviction_on_empty_queue_is_noop() {
    let mut state = HostState::default();
    let out = update(&mut state, HostCommand::PopFrontPoolWindowForEviction);
    assert_eq!(out.evicted_pool_label, None);
    assert!(state.pool.queue.is_empty());
    assert!(state.pool.unpromoted.is_empty());
}

/// A label still in `unpromoted` (mid-flight spawn, no HWND cached yet) must
/// never be popped for eviction — there is nothing yet to destroy, and
/// `evict_idle_pool_window`'s HWND-cache lookup would find nothing and log
/// a "pool short by one" warning if it were. Only `queue` (fully spawned,
/// ready) is eviction-eligible; same restriction as the pane-pool version,
/// for the identical reason.
#[test]
fn pop_front_pool_for_eviction_does_not_touch_unpromoted_only_label() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::PoolWindowSpawnStart { label: "p1".into() });
    assert!(state.pool.unpromoted.contains("p1"));
    assert!(state.pool.queue.is_empty());

    let out = update(&mut state, HostCommand::PopFrontPoolWindowForEviction);
    assert_eq!(out.evicted_pool_label, None, "an unpromoted-only label has no queue entry to pop");
    assert!(
        state.pool.unpromoted.contains("p1"),
        "must not remove a mid-flight spawn with no HWND yet to destroy"
    );
}

/// Eviction and a real user promote must never be able to race for the same
/// front label: `PopFrontPoolWindowForEviction` and
/// `PopAndPromoteFrontPoolWindow` both pop under the same `host_state`
/// mutex, so once one dispatch has claimed a label, the queue no longer
/// has it for the other to claim — whichever command runs second on an
/// otherwise-empty queue gets a clean no-op, not a double-claim.
#[test]
fn eviction_and_promote_cannot_double_claim_the_same_front_label() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::PoolWindowSpawnStart { label: "p1".into() });
    update(&mut state, HostCommand::PoolWindowReady { label: "p1".into() });

    let evicted = update(&mut state, HostCommand::PopFrontPoolWindowForEviction);
    assert_eq!(evicted.evicted_pool_label, Some("p1".to_string()));

    let promoted = update(&mut state, HostCommand::PopAndPromoteFrontPoolWindow);
    assert_eq!(
        promoted.promoted_pool_label, None,
        "the label was already claimed by eviction — promote must see an empty queue, not double-claim it"
    );
}

// ── B.5 Part 1 pane-pool eviction (issue #2218) ─────────────────────────
//
// `evict_idle_pane_pool_window` (commands/window_pool.rs) claims the front
// pane-pool label atomically via `PopFrontPanePoolWindowForEviction` (round
// 2 — replaced an initial peek-then-separately-dispatch design after reagent
// flagged a race with a concurrent real tear-off promotion, P2). The older
// `PanePoolWindowDestroyedBeforePromote` command below is still exercised —
// `cleanup_failed_pane_pool_creation` still dispatches it for a
// creation-failure cleanup — just no longer by the eviction path.

/// Mirrors `pool_destroy_after_ready_clears_queue` for the pane pool: a
/// label that reached `queue` (spawn + ready) is fully scrubbed from BOTH
/// `queue` and `unpromoted` by `PanePoolWindowDestroyedBeforePromote`, and
/// `respawn_in_flight` resets — this is `evict_idle_pane_pool_window`'s
/// only cleanup step after the actual Win32 destroy is posted.
#[test]
fn pane_pool_destroyed_before_promote_pops_and_clears_state() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::PanePoolWindowSpawnStart { label: "pp1".into() });
    update(&mut state, HostCommand::PanePoolWindowReady { label: "pp1".into() });
    assert_eq!(state.pane_pool.queue.len(), 1);
    assert!(!state.pane_pool.unpromoted.contains("pp1"));

    let out = update(
        &mut state,
        HostCommand::PanePoolWindowDestroyedBeforePromote { label: "pp1".into() },
    );
    assert!(state.pane_pool.queue.is_empty(), "queue must not retain the evicted label");
    assert!(state.pane_pool.unpromoted.is_empty());
    assert!(!state.pane_pool.respawn_in_flight);
    assert_eq!(out.pane_pool_size_after, Some(0));
}

/// `evict_idle_pane_pool_window` peeks `queue.front()` before dispatching
/// this command, so in normal operation it never fires with an unknown
/// label — but the command itself stays idempotent for one anyway
/// (matching `PoolWindowDestroyedBeforePromote`'s pattern), so a stale/
/// racing call can never corrupt pool bookkeeping.
#[test]
fn pane_pool_destroyed_before_promote_on_unknown_label_is_noop() {
    let mut state = HostState::default();
    let out = update(
        &mut state,
        HostCommand::PanePoolWindowDestroyedBeforePromote { label: "ghost".into() },
    );
    assert_eq!(out.pane_pool_size_after, Some(0));
    assert!(state.pane_pool.queue.is_empty());
    assert!(state.pane_pool.unpromoted.is_empty());
}

/// `PopFrontPanePoolWindowForEviction` — the atomic claim
/// `evict_idle_pane_pool_window` dispatches (issue #2218, B.5 Part 1, round
/// 2 fix). Pops the front label, clears it from `unpromoted`, and resets
/// `respawn_in_flight` in one mutex-guarded dispatch, same as
/// `PopAndPromoteFrontPanePoolWindow` does for the promote path — this is
/// what makes the two commands mutually exclusive for a given front label
/// instead of racing via a non-atomic peek-then-separate-mutate.
#[test]
fn pop_front_pane_pool_for_eviction_pops_and_clears_state() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::PanePoolWindowSpawnStart { label: "pp1".into() });
    update(&mut state, HostCommand::PanePoolWindowReady { label: "pp1".into() });
    assert_eq!(state.pane_pool.queue.len(), 1);

    let out = update(&mut state, HostCommand::PopFrontPanePoolWindowForEviction);
    assert!(state.pane_pool.queue.is_empty(), "queue must not retain the evicted label");
    assert!(state.pane_pool.unpromoted.is_empty());
    assert!(!state.pane_pool.respawn_in_flight);
    assert_eq!(out.evicted_pane_pool_label, Some("pp1".to_string()));
    assert_eq!(out.pane_pool_size_after, Some(0));
}

/// Empty queue must be a genuine no-op — `evict_idle_pane_pool_window`
/// relies on `evicted_pane_pool_label: None` to bail out cleanly when
/// pressure fires with nothing left to evict.
#[test]
fn pop_front_pane_pool_for_eviction_on_empty_queue_is_noop() {
    let mut state = HostState::default();
    let out = update(&mut state, HostCommand::PopFrontPanePoolWindowForEviction);
    assert_eq!(out.evicted_pane_pool_label, None);
    assert!(state.pane_pool.queue.is_empty());
    assert!(state.pane_pool.unpromoted.is_empty());
}

// ── Round 6 — pool demote ────────────────────────────────────────────

/// The demote round-trip: promoted label re-enters `unpromoted`, and the
/// normal `PoolWindowReady` handshake (fired after the demote path reloads
/// the window to its pool boot URL) moves it into the queue — a full
/// promote → demote → ready → re-promote cycle on one label.
#[test]
fn pool_demote_reenters_unpromoted_then_ready_requeues() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::PoolWindowSpawnStart { label: "p1".into() });
    update(&mut state, HostCommand::PoolWindowReady { label: "p1".into() });
    let out = update(&mut state, HostCommand::PopAndPromoteFrontPoolWindow);
    assert_eq!(out.promoted_pool_label.as_deref(), Some("p1"));
    assert!(state.pool.queue.is_empty());

    // Demote: back into unpromoted, accepted.
    let out = update(&mut state, HostCommand::DemotePoolWindow { label: "p1".into() });
    assert!(out.pool_demote_accepted, "demote of a promoted label must be accepted");
    assert!(state.pool.unpromoted.contains("p1"));
    assert!(state.pool.queue.is_empty(), "queue entry waits for renderer-ready");

    // The reloaded frontend re-sends pool_window_ready → queue re-entry.
    update(&mut state, HostCommand::PoolWindowReady { label: "p1".into() });
    assert!(!state.pool.unpromoted.contains("p1"));
    assert_eq!(state.pool.queue.len(), 1);

    // And it can be promoted again.
    let out = update(&mut state, HostCommand::PopAndPromoteFrontPoolWindow);
    assert_eq!(out.promoted_pool_label.as_deref(), Some("p1"));
}

/// Idempotency: demoting a label already pool-side (double demote, or a
/// race with a fresh spawn) is rejected — the caller takes the destroy
/// fallback rather than double-inserting.
#[test]
fn pool_demote_already_pool_side_is_rejected() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::PoolWindowSpawnStart { label: "p1".into() });
    // Still unpromoted → demote must be a rejected no-op.
    let out = update(&mut state, HostCommand::DemotePoolWindow { label: "p1".into() });
    assert!(!out.pool_demote_accepted);
    // Queued → also rejected.
    update(&mut state, HostCommand::PoolWindowReady { label: "p1".into() });
    let out = update(&mut state, HostCommand::DemotePoolWindow { label: "p1".into() });
    assert!(!out.pool_demote_accepted);
    assert_eq!(state.pool.queue.len(), 1, "no duplicate insertion");
}

/// Residual 1 (SPEC_POOL_ADOPTION_AND_WINDOW_ROW_CRUMB_2026_07_11) — pool
/// ADOPTION: a foreign `window-{uuid}` label that never entered the pool via
/// a spawn (cold-path or tear-off window, minted when the pool was empty)
/// runs the exact same demote → ready → promote cycle as a `window-pool-*`
/// label. Pool membership is the `unpromoted`/`queue` sets + the is_pool
/// flag, never the label string — this test is the reducer-level contract
/// for that.
#[test]
fn pool_adoption_foreign_label_full_cycle() {
    let mut state = HostState::default();

    // A label the pool has never seen — no spawn, no prior promote.
    let out = update(&mut state, HostCommand::DemotePoolWindow { label: "window-abc123".into() });
    assert!(out.pool_demote_accepted, "adoption of a foreign window-* label must be accepted");
    assert!(state.pool.unpromoted.contains("window-abc123"));

    // The demote reload boots the renderer in pool mode; its ready signal
    // moves the adopted label into the serving queue like any other.
    update(&mut state, HostCommand::PoolWindowReady { label: "window-abc123".into() });
    assert!(!state.pool.unpromoted.contains("window-abc123"));
    assert_eq!(state.pool.queue.len(), 1);

    // And it serves the next promote.
    let out = update(&mut state, HostCommand::PopAndPromoteFrontPoolWindow);
    assert_eq!(out.promoted_pool_label.as_deref(), Some("window-abc123"));

    // Double-adopt of the same label while pool-side stays rejected
    // (idempotency contract unchanged by adoption).
    update(&mut state, HostCommand::DemotePoolWindow { label: "window-abc123".into() });
    let out = update(&mut state, HostCommand::DemotePoolWindow { label: "window-abc123".into() });
    assert!(!out.pool_demote_accepted);
}

/// Demote does NOT touch the respawn semaphore — a demote mid-spawn must
/// not let a second spawn start.
#[test]
fn pool_demote_leaves_respawn_semaphore_alone() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::PoolWindowSpawnStart { label: "fresh".into() });
    assert!(state.pool.respawn_in_flight);
    let out = update(&mut state, HostCommand::DemotePoolWindow { label: "promoted-1".into() });
    assert!(out.pool_demote_accepted);
    assert!(state.pool.respawn_in_flight, "demote must not clear the spawn semaphore");
}

/// Sister test: destroy with the label in NEITHER set is still a no-op.
#[test]
fn pool_destroy_with_unknown_label_is_noop() {
    let mut state = HostState::default();
    let out = update(
        &mut state,
        HostCommand::PoolWindowDestroyedBeforePromote { label: "ghost".into() },
    );
    assert!(out.events.is_empty());
}

// ── Level-triggered quit reconciliation (spec §5.1/§10) ───────────────────
//
// Safety net for the reported regression (closing the last window orphaned the
// whole process tree). The decision was previously edge-triggered inside
// `client::on_before_close` with no test coverage; these pin the level-triggered
// `reconcile_quit` decision so a gate regression can't ship silently again.

use super::quit::{
    is_live_user_window, reconcile_quit, should_begin_drain, user_creation_in_flight,
};

/// The last-window quit gate is decided PURELY BY TYPE (`is_live_user_window`),
/// never by label-prefix string (SPEC_REDUCER_SSOT_CONSOLIDATION L4). Floaters
/// (both `floating-<uuid>` and `floating-pool-<uuid>`) are `BrowserKind::Floater`
/// and never keep the instance alive (invariant FP-LIFE) — this replaced a
/// `!starts_with("floating-pool-")` string check that wrongly counted direct
/// `floating-<uuid>` floaters (and the reagent P0 #1676 where warm pane-pool
/// windows pinned the gate above 0 on macOS/Linux).
#[test]
fn is_live_user_window_counts_only_top_level_by_type() {
    use crate::state::BrowserKind;
    // Real user windows: main + a promoted window-pool window (keeps its
    // `window-pool-` label, is_pool flipped false on promote) → both count.
    assert!(is_live_user_window(&BrowserKind::TopLevel { is_pool: false }));
    // Warm window pool → not a user window.
    assert!(!is_live_user_window(&BrowserKind::TopLevel { is_pool: true }));
    // Floaters NEVER count — warm pane-pool AND promoted/direct floaters alike,
    // regardless of is_pool. Excluded by type, not by label.
    assert!(!is_live_user_window(&BrowserKind::Floater { is_pool: true }));
    assert!(!is_live_user_window(&BrowserKind::Floater { is_pool: false }));
    // Browser-pane children never count.
    assert!(!is_live_user_window(&BrowserKind::Pane {
        block_id: "b1".into()
    }));
}

/// Registered browsers are classified by the authoritative `is_pool` flag, NOT
/// by label — a PROMOTED pool window keeps its `window-pool-*` label but is
/// `is_pool:false`, i.e. the user's real live window, and MUST count (reagent P1
/// #1676). Unpromoted pool windows (`is_pool:true`) and panes never count.
#[test]
fn is_live_user_window_classifies_by_is_pool_not_label() {
    use crate::state::BrowserKind;
    assert!(
        is_live_user_window(&BrowserKind::TopLevel { is_pool: false }),
        "promoted pool window / main / new window keeps the instance alive"
    );
    assert!(
        !is_live_user_window(&BrowserKind::TopLevel { is_pool: true }),
        "unpromoted warm-pool window does not"
    );
    assert!(
        !is_live_user_window(&BrowserKind::Pane { block_id: "b1".into() }),
        "browser-pane child never does"
    );
}

/// The full decision truth table (CEF-free pure core).
#[test]
fn should_begin_drain_truth_table() {
    // Armed + Running + zero user windows + no creation in flight → begin draining.
    assert_eq!(
        should_begin_drain(true, 0, false, &QuitState::Running, false),
        Some(QuitReason::LastWindowClosed)
    );
    // A live user window blocks drain.
    assert_eq!(should_begin_drain(true, 1, false, &QuitState::Running, false), None);
    // §10.2 corner: a user creation in flight blocks drain even at zero
    // registered windows — never quit while the user's "New Window" is loading.
    assert_eq!(should_begin_drain(true, 0, true, &QuitState::Running, false), None);
    // Already draining / quit → never re-drains (monotonic with handle_begin_drain).
    assert_eq!(
        should_begin_drain(
            true,
            0,
            false,
            &QuitState::Draining { reason: QuitReason::LastWindowClosed },
            false,
        ),
        None
    );
    // UNARMED (sanitize-then-decide §1.E): the startup gap — no live user
    // window has ever registered — must never drain, even though every other
    // input reads "drainable" (main's creation path enqueues no pending entry,
    // so this exact input combination is live between process start and main's
    // RegisterBrowser).
    assert_eq!(should_begin_drain(false, 0, false, &QuitState::Running, false), None);
    assert_eq!(should_begin_drain(false, 0, true, &QuitState::Running, false), None);
}

/// Workstream 0 Phase 1 (`SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04.md`
/// §7): background-service mode suppresses ONLY the `LastWindowClosed`
/// verdict — every other input in the truth table above is unaffected by it.
#[test]
fn should_begin_drain_background_service_suppresses_last_window_closed_only() {
    // The one case that would otherwise drain now doesn't.
    assert_eq!(should_begin_drain(true, 0, false, &QuitState::Running, true), None);
    // Still correctly refuses to drain for every other reason, whether or
    // not background-service mode is on — same answer either way.
    assert_eq!(should_begin_drain(true, 1, false, &QuitState::Running, true), None);
    assert_eq!(should_begin_drain(true, 0, true, &QuitState::Running, true), None);
    assert_eq!(should_begin_drain(false, 0, false, &QuitState::Running, true), None);
    assert_eq!(
        should_begin_drain(
            true,
            0,
            false,
            &QuitState::Draining { reason: QuitReason::External },
            true,
        ),
        None
    );
}

/// Only USER-initiated creations block drain; pool/pane background creations don't.
#[test]
fn user_creation_in_flight_ignores_background_labels() {
    let mut state = HostState::default();
    // Pool refill, browser-pane, warm floating-pool, AND broad floating- tear-off
    // are all background — none blocks drain (mirrors the prefix exclusion in
    // orphan_reconcile.rs's `ui_thread_reconcile`).
    for bg in ["window-pool-abc", "browser-pane-1", "floating-pool-xyz", "floating-tearoff-7"] {
        update(
            &mut state,
            HostCommand::EnqueuePendingWindowCreation { entry: entry(bg) },
        );
    }
    assert!(
        !user_creation_in_flight(&state),
        "background (pool/pane/floating) creations must not block drain"
    );
    update(
        &mut state,
        HostCommand::EnqueuePendingWindowCreation { entry: entry("window-uuid-1234") },
    );
    assert!(
        user_creation_in_flight(&state),
        "a user-initiated window creation must block drain"
    );
}

/// A fresh (pre-first-window) state must NOT drain: `HostState::default()` is
/// unarmed. This pins the §1.E fix — before the arming bit, this exact state
/// (the startup gap) read as drainable, a latent startup-quit for any future
/// `request_drain` consumer.
#[test]
fn reconcile_quit_never_drains_unarmed_fresh_state() {
    let state = HostState::default();
    assert_eq!(
        reconcile_quit(&state),
        None,
        "unarmed (pre-first-window) state must never request a drain"
    );
}

/// After the last user window deregisters (no browsers, no pending creation),
/// reconcile drains. (Composed over real `HostState`; armed = a live user
/// window registered at some point this process, as `RegisterBrowser` does.)
#[test]
fn reconcile_quit_drains_when_no_windows_and_no_pending_creation() {
    let mut state = HostState::default();
    state.saw_live_user_window = true;
    assert_eq!(reconcile_quit(&state), Some(QuitReason::LastWindowClosed));
}

/// The premature-quit corner (spec §10.2): zero registered user windows but a
/// user "New Window" is mid-creation → must NOT drain; once it leaves the
/// pending queue (registered or aborted), reconcile drains on the next tick.
#[test]
fn reconcile_quit_deferred_while_user_creation_pending() {
    let mut state = HostState::default();
    state.saw_live_user_window = true;
    update(
        &mut state,
        HostCommand::EnqueuePendingWindowCreation { entry: entry("window-fresh-uuid") },
    );
    assert_eq!(reconcile_quit(&state), None, "must not quit mid user-window create");
    update(&mut state, HostCommand::DequeuePendingWindowCreation);
    assert_eq!(
        reconcile_quit(&state),
        Some(QuitReason::LastWindowClosed),
        "drains once the pending user creation clears"
    );
}

/// A background pool refill pending must NOT keep the host alive on last close.
#[test]
fn reconcile_quit_drains_despite_pending_pool_refill() {
    let mut state = HostState::default();
    state.saw_live_user_window = true;
    update(
        &mut state,
        HostCommand::EnqueuePendingWindowCreation { entry: entry("window-pool-refill-1") },
    );
    assert_eq!(reconcile_quit(&state), Some(QuitReason::LastWindowClosed));
}

/// `live_user_window_labels` must classify by the same rule as
/// `count_live_user_windows` — the explicit-quit path (`quit_app`) closes
/// exactly what it returns, so a pool window or pane leaking into the list
/// would have the tray's Quit closing background inventory as if it were the
/// user's windows.
/// ReAgent P1 on PR #2996: the creation-path guards narrow the
/// quit-vs-create race but cannot close it — a creation already in flight
/// when the drain begins still reaches registration. Registration is the
/// LAST step, so flagging it here is the only unraceable point.
#[test]
fn a_user_window_arriving_mid_drain_is_closed_on_arrival() {
    use super::quit::should_close_on_arrival;
    let user = BrowserKind::TopLevel { is_pool: false };
    for reason in [QuitReason::LauncherRequested, QuitReason::LastWindowClosed] {
        assert!(
            should_close_on_arrival(&user, &QuitState::Draining { reason: reason.clone() }),
            "a user window registering mid-drain must be closed on arrival"
        );
    }
}

/// The normal case must stay untouched: registering while Running never
/// flags, or every window the app opens would immediately close itself.
#[test]
fn registering_while_running_is_never_closed_on_arrival() {
    use super::quit::should_close_on_arrival;
    assert!(!should_close_on_arrival(
        &BrowserKind::TopLevel { is_pool: false },
        &QuitState::Running
    ));
}

/// Pool browsers legitimately register during a drain — the drain cascade
/// closes them itself — so closing them here would fight that machinery.
/// Panes and floaters are not top-level windows the quit owns either.
#[test]
fn background_browsers_arriving_mid_drain_are_left_alone() {
    use super::quit::should_close_on_arrival;
    let draining = QuitState::Draining { reason: QuitReason::LauncherRequested };
    for kind in [
        BrowserKind::TopLevel { is_pool: true },
        BrowserKind::Floater { is_pool: false },
        BrowserKind::Pane { block_id: "b1".into() },
    ] {
        assert!(
            !should_close_on_arrival(&kind, &draining),
            "{:?} is background inventory, not a user window the quit owns",
            kind
        );
    }
}

#[test]
fn live_user_window_labels_matches_the_live_count_classification() {
    let entries: Vec<(String, BrowserKind)> = vec![
        ("main".into(), BrowserKind::TopLevel { is_pool: false }),
        ("window-abc".into(), BrowserKind::TopLevel { is_pool: false }),
        // A promoted pool window keeps its `window-pool-` label forever but
        // has is_pool: false — it IS a real user window and must be closed
        // by an explicit quit (classification is by type, never by label).
        ("window-pool-promoted".into(), BrowserKind::TopLevel { is_pool: false }),
        ("window-pool-1".into(), BrowserKind::TopLevel { is_pool: true }),
        ("floating-xyz".into(), BrowserKind::Floater { is_pool: false }),
        ("browser-pane-b1-1".into(), BrowserKind::Pane { block_id: "b1".into() }),
    ];
    let mut labels = super::quit::live_user_window_labels_from(
        entries.iter().map(|(l, k)| (l, k)),
    );
    labels.sort();
    assert_eq!(
        labels,
        vec![
            "main".to_string(),
            "window-abc".to_string(),
            "window-pool-promoted".to_string(),
        ],
        "only real user top-levels; unpromoted pool windows, floaters and panes excluded"
    );
}

/// Workstream 0 Phase 1: with background-service mode on, closing the last
/// window (armed, zero live windows, nothing pending) must NOT drain —
/// `host` stays alive with zero windows instead of tearing itself (and, via
/// the launcher's teardown-on-clean-exit, `srv`) down.
#[test]
fn reconcile_quit_never_drains_when_background_service_enabled() {
    let mut state = HostState::default();
    state.saw_live_user_window = true;
    state.background_service_enabled = true;
    assert_eq!(
        reconcile_quit(&state),
        None,
        "background-service mode must suppress the last-window drain"
    );
}

/// Issue #2977 WS4 — the transition is level-triggered against the CURRENT
/// state, so it fires only on a real crossing and is idempotent under the
/// repeated zero-window dispatches pool churn produces.
#[test]
fn attention_transition_fires_only_on_a_real_crossing() {
    use super::quit::background_attention_transition as t;
    // attended -> no windows left
    assert_eq!(t(true, false, false, 0), Some(true));
    // unattended -> a window opened
    assert_eq!(t(true, false, true, 1), Some(false));
    // Already in that state: not a new event.
    assert_eq!(t(true, false, true, 0), None);
    assert_eq!(t(true, false, false, 3), None);
}

/// Quitting closes every window, but that is the app exiting, not going to the
/// background: no `went_unattended`. Otherwise the next launch reports the quit
/// as "kept running in the background since <quit time>" (seen 2026-09-27 on a
/// dev build restarted with task dev, whose launcher-requested quit drains).
#[test]
fn no_attention_transition_once_quitting() {
    use super::quit::background_attention_transition as t;
    assert_eq!(t(true, true, false, 0), None);
    assert_eq!(t(true, true, true, 1), None);
}

/// The reducer's own wiring (`attention_after`, what `update` calls): the last
/// user window going away (`live_after` 0) after a real `BeginDrain` through
/// `update` records nothing, while the same drop with the app still running
/// records `went_unattended`. The window count is passed in because unit tests
/// can't register a live window (a `BrowserHandle` needs a real `cef::Browser`).
#[test]
fn the_last_window_closing_during_a_drain_is_not_going_unattended() {
    let mut running = HostState::default();
    running.background_service_enabled = true;
    assert_eq!(super::attention_after(&running, 0), Some(true), "control: no quit, so it IS going unattended");

    let mut draining = HostState::default();
    draining.background_service_enabled = true;
    update(&mut draining, HostCommand::BeginDrain { reason: QuitReason::LauncherRequested });
    assert_eq!(super::attention_after(&draining, 0), None);
    assert!(!draining.background_unattended);
}

/// With background-service mode off, reaching zero windows means the app is
/// EXITING, not resting — there is no unattended period to tell the user
/// about, and reporting one would be actively misleading.
#[test]
fn no_attention_transitions_when_background_service_is_off() {
    use super::quit::background_attention_transition as t;
    assert_eq!(t(false, false, false, 0), None);
    assert_eq!(t(false, false, true, 1), None);
}

/// Idempotent: once draining, reconcile is a no-op (no double-drain).
#[test]
fn reconcile_quit_noop_once_draining() {
    let mut state = HostState::default();
    state.saw_live_user_window = true; // isolate the Draining gate from the arming gate
    update(&mut state, HostCommand::BeginDrain { reason: QuitReason::LastWindowClosed });
    assert_eq!(reconcile_quit(&state), None);
}

// ── Pillar 2 Stage 1 — the reducer hook (`is_quit_relevant` + `request_drain`) ──
//
// The hook is the wiring that makes the level-triggered decision fire after every
// quit-relevant transition. These pin (a) the negative guard's membership and
// (b) that `update` surfaces the decision via `DispatchOutput::request_drain`
// only for relevant commands. Stage 1 is behavior-neutral — nothing consumes
// `request_drain` yet — so these are the safety net for the wiring itself.

/// The negative guard: window/pool/pending/quit commands are relevant (default
/// true); the drag-opacity hot path and the browser-pane lifecycle are excluded.
#[test]
fn is_quit_relevant_guard_membership() {
    use super::is_quit_relevant;
    // Relevant — change live-window count / pending creations / quit_state.
    assert!(is_quit_relevant(&HostCommand::UnregisterBrowser { label: "w".into() }));
    assert!(is_quit_relevant(&HostCommand::DequeuePendingWindowCreation));
    assert!(is_quit_relevant(&HostCommand::PopAndPromoteFrontPoolWindow));
    assert!(is_quit_relevant(&HostCommand::BeginDrain {
        reason: QuitReason::LastWindowClosed
    }));
    // Irrelevant — hot path + pane lifecycle (panes never affect the window count).
    assert!(!is_quit_relevant(&HostCommand::EvictFloatingPaneWindowState { label: "f".into() }));
    assert!(!is_quit_relevant(&HostCommand::EnqueueBrowserPaneClose { block_id: "b".into(), now: t0() }));
}

/// `update` surfaces the level-triggered drain decision on a quit-relevant
/// command, and skips it on a quit-irrelevant one even when the state would drain.
#[test]
fn update_surfaces_request_drain_only_for_relevant_commands() {
    let mut state = HostState::default(); // Running, no windows, no pending
    state.saw_live_user_window = true; // armed → drainable
    let out = update(&mut state, HostCommand::DequeuePendingWindowCreation);
    assert_eq!(
        out.request_drain,
        Some(QuitReason::LastWindowClosed),
        "quit-relevant command must surface the reconcile decision"
    );
    let out2 = update(
        &mut state,
        HostCommand::EvictFloatingPaneWindowState { label: "missing".into() },
    );
    assert_eq!(
        out2.request_drain, None,
        "quit-irrelevant command must not compute a drain request"
    );
}

// ── Sanitize-then-decide Phase 0 (SPEC_PILLAR2_SANITIZE_THEN_DECIDE_2026_07_11) ──

/// `ReconcileQuit` is a pure poke: it mutates nothing (no events, no version
/// bump) and only surfaces the standing `reconcile_quit` verdict via
/// `request_drain` — the edge a level-triggered executor rides when it has no
/// state-changing dispatch of its own (§1.H).
#[test]
fn reconcile_quit_poke_surfaces_verdict_without_mutation() {
    let mut state = HostState::default();
    state.saw_live_user_window = true;
    let version_before = state.event_version;
    let out = update(&mut state, HostCommand::ReconcileQuit);
    assert_eq!(
        out.request_drain,
        Some(QuitReason::LastWindowClosed),
        "armed drainable state → poke surfaces the drain verdict"
    );
    assert!(out.events.is_empty(), "poke must emit no events");
    assert_eq!(state.event_version, version_before, "poke must not bump version");
}

/// The poke respects every gate the composed decision has: unarmed and
/// already-draining states both answer `None`.
#[test]
fn reconcile_quit_poke_respects_arming_and_monotonicity() {
    // Unarmed fresh state (the startup gap) → None.
    let mut fresh = HostState::default();
    let out = update(&mut fresh, HostCommand::ReconcileQuit);
    assert_eq!(out.request_drain, None, "unarmed poke must not drain");

    // Armed but already draining → None (monotonic).
    let mut draining = HostState::default();
    draining.saw_live_user_window = true;
    update(&mut draining, HostCommand::BeginDrain { reason: QuitReason::LastWindowClosed });
    let out2 = update(&mut draining, HostCommand::ReconcileQuit);
    assert_eq!(out2.request_drain, None, "poke after BeginDrain must not re-drain");
}

/// `ReconcileQuit` must never enter the `is_quit_relevant` exclusion list —
/// surfacing the verdict is its entire purpose.
#[test]
fn reconcile_quit_poke_is_quit_relevant() {
    use super::is_quit_relevant;
    assert!(is_quit_relevant(&HostCommand::ReconcileQuit));
}

// ── Browser-pane host→JS event routing ──────────────────────────────────────
// A pane's push events (nav-state / title-change / favicon-urls) must be
// delivered to the top-level window that OWNS the pane, never blindly to
// `main`: after a tab tear-off recreates the pane in a promoted pool window,
// `main` has no listener for that block_id, the events are silently dropped,
// and the pane's loading overlay never clears. See
// docs/reports/REPORT_BROWSER_PANE_EVENTS_MISROUTED_TO_MAIN_WINDOW_2026_09_06.md.

#[test]
fn browser_pane_event_target_is_the_owning_window_not_main() {
    let mut state = HostState::default();
    // The reported repro: pane recreated in a promoted pool window while
    // `main` is still alive.
    let out = update(
        &mut state,
        HostCommand::TryRegisterBrowserPaneLive {
            block_id: "b1".into(),
            pending: Some(pending_in("window-pool-930b71ac")),
        },
    );
    assert!(matches!(
        out.browser_pane_register_result,
        Some(RegisterResult::Fresh(_))
    ));
    assert_eq!(
        crate::events::browser_pane_event_target(&state, "b1").as_deref(),
        Some("window-pool-930b71ac"),
    );
}

#[test]
fn browser_pane_event_target_is_none_for_unknown_or_windowless_pane() {
    let mut state = HostState::default();
    assert_eq!(
        crate::events::browser_pane_event_target(&state, "nope"),
        None
    );
    // A pane registered without a pending create records no window: the
    // emitter must fall back to its previous behaviour rather than try to
    // deliver to an empty label.
    update(
        &mut state,
        HostCommand::TryRegisterBrowserPaneLive { block_id: "legacy".into(), pending: None },
    );
    assert_eq!(
        crate::events::browser_pane_event_target(&state, "legacy"),
        None
    );
}

// ── SetWindowOpacity ─────────────────────────────────────────────────

#[test]
fn set_window_opacity_stores_the_value_and_emits_applied() {
    let mut state = HostState::default();
    let out = update(&mut state, HostCommand::SetWindowOpacity { label: "w1".into(), opacity: 0.5 });
    assert!(matches!(
        out.events.as_slice(),
        [HostEvent::WindowOpacityApplied { label, opacity, version: 1 }] if label == "w1" && *opacity == 0.5
    ));
    assert_eq!(state.window_opacities.get("w1"), Some(&0.5));
    assert!(out.request_drain.is_none(), "opacity is not quit-relevant");
}

#[test]
fn set_window_opacity_clamps_below_zero() {
    let mut state = HostState::default();
    let out = update(&mut state, HostCommand::SetWindowOpacity { label: "w1".into(), opacity: -3.0 });
    assert!(matches!(
        out.events.as_slice(),
        [HostEvent::WindowOpacityApplied { opacity, .. }] if *opacity == 0.0
    ));
    assert_eq!(state.window_opacities.get("w1"), Some(&0.0));
}

#[test]
fn set_window_opacity_at_or_above_one_clears_the_entry() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::SetWindowOpacity { label: "w1".into(), opacity: 0.4 });
    for opacity in [1.0, 7.5] {
        let out = update(&mut state, HostCommand::SetWindowOpacity { label: "w1".into(), opacity });
        assert!(matches!(
            out.events.as_slice(),
            [HostEvent::WindowOpacityCleared { label, .. }] if label == "w1"
        ));
        assert!(!state.window_opacities.contains_key("w1"), "fully opaque = no entry");
    }
}

#[test]
fn set_window_opacity_is_per_label() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::SetWindowOpacity { label: "a".into(), opacity: 0.3 });
    update(&mut state, HostCommand::SetWindowOpacity { label: "b".into(), opacity: 0.7 });
    update(&mut state, HostCommand::SetWindowOpacity { label: "a".into(), opacity: 1.0 });
    assert!(!state.window_opacities.contains_key("a"));
    assert_eq!(state.window_opacities.get("b"), Some(&0.7));
}

// ── PopAndPromoteFrontPanePoolWindow ─────────────────────────────────

#[test]
fn pop_and_promote_front_pane_pool_window_pops_in_fifo_order() {
    let mut state = HostState::default();
    for label in ["pp1", "pp2"] {
        update(&mut state, HostCommand::PanePoolWindowSpawnStart { label: label.into() });
        update(&mut state, HostCommand::PanePoolWindowReady { label: label.into() });
    }
    assert_eq!(state.pane_pool.queue, ["pp1", "pp2"]);

    let out = update(&mut state, HostCommand::PopAndPromoteFrontPanePoolWindow);
    assert_eq!(out.promoted_pane_pool_label.as_deref(), Some("pp1"));
    assert_eq!(out.pane_pool_size_after, Some(1));
    assert!(out.events.is_empty());
    assert_eq!(state.pane_pool.queue, ["pp2"]);
    assert!(!state.pane_pool.unpromoted.contains("pp1"));

    let out = update(&mut state, HostCommand::PopAndPromoteFrontPanePoolWindow);
    assert_eq!(out.promoted_pane_pool_label.as_deref(), Some("pp2"));
    assert_eq!(out.pane_pool_size_after, Some(0));
}

#[test]
fn pop_and_promote_front_pane_pool_window_on_empty_queue_is_a_no_op() {
    let mut state = HostState::default();
    // A spawn still in flight (unpromoted, not yet ready) must not be promoted.
    update(&mut state, HostCommand::PanePoolWindowSpawnStart { label: "pp1".into() });
    let out = update(&mut state, HostCommand::PopAndPromoteFrontPanePoolWindow);
    assert_eq!(out.promoted_pane_pool_label, None);
    assert_eq!(out.pane_pool_size_after, None);
    assert!(state.pane_pool.unpromoted.contains("pp1"));
    assert!(state.pane_pool.respawn_in_flight);
}

#[test]
fn pop_and_promote_and_eviction_never_claim_the_same_label() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::PanePoolWindowSpawnStart { label: "pp1".into() });
    update(&mut state, HostCommand::PanePoolWindowReady { label: "pp1".into() });
    let promoted = update(&mut state, HostCommand::PopAndPromoteFrontPanePoolWindow);
    let evicted = update(&mut state, HostCommand::PopFrontPanePoolWindowForEviction);
    assert_eq!(promoted.promoted_pane_pool_label.as_deref(), Some("pp1"));
    assert_eq!(evicted.evicted_pane_pool_label, None, "already claimed by the promote");
}

// ── Purity: nothing in the reducer reads the clock, a global or the env ──

#[test]
fn pane_labels_come_from_state_so_two_fresh_states_agree() {
    let register = |state: &mut HostState, block_id: &str| match update(
        state,
        HostCommand::TryRegisterBrowserPaneLive { block_id: block_id.into(), pending: None },
    )
    .browser_pane_register_result
    {
        Some(RegisterResult::Fresh(l)) => l,
        other => panic!("expected Fresh, got {:?}", other),
    };
    let mut a = HostState::default();
    let mut b = HostState::default();
    assert_eq!(register(&mut a, "x"), "browser-pane-x-1");
    assert_eq!(register(&mut a, "y"), "browser-pane-y-2");
    // A second state is not affected by the first, as it was with the old
    // process-global counter.
    assert_eq!(register(&mut b, "x"), "browser-pane-x-1");
}

#[test]
fn enqueue_browser_pane_close_stamps_the_callers_instant() {
    let mut state = HostState::default();
    update(
        &mut state,
        HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into(), pending: None },
    );
    let when = t0() + std::time::Duration::from_secs(5);
    update(&mut state, HostCommand::EnqueueBrowserPaneClose { block_id: "b1".into(), now: when });
    assert!(matches!(
        state.browser_panes["b1"].lifecycle,
        BrowserPaneLifecycle::Closing { since } if since == when
    ));
}

#[test]
fn background_service_mode_comes_from_the_constructor() {
    assert!(!HostState::default().background_service_enabled);
    assert!(!HostState::new(false).background_service_enabled);
    assert!(HostState::new(true).background_service_enabled);
}

#[test]
fn host_bump_version_saturates_instead_of_wrapping() {
    let mut state = HostState { event_version: u64::MAX - 1, ..HostState::default() };
    assert_eq!(state.bump_version(), u64::MAX);
    assert_eq!(state.bump_version(), u64::MAX, "must not wrap to 0");
}

// ── Attended/unattended flag: every write goes through the reducer ──

#[test]
fn seed_background_unattended_sets_the_flag_without_reporting_a_crossing() {
    let mut state = HostState::new(true);
    let out = update(&mut state, HostCommand::SeedBackgroundUnattended { unattended: true });
    assert!(state.background_unattended);
    assert_eq!(out.background_attention, None, "the log already holds this period");
    assert!(out.events.is_empty());
    assert!(out.request_drain.is_none());
}

#[test]
fn start_hidden_hold_and_release_report_one_crossing_each() {
    // Background-service mode off: a held login start is still an unattended
    // period (that is how start_hidden always recorded it).
    let mut state = HostState::default();
    let out = update(&mut state, HostCommand::SetStartHiddenHeld { held: true });
    assert_eq!(out.background_attention, Some(true));
    assert!(state.background_unattended);
    assert!(state.start_hidden_held);

    // Repeating it is not a new period.
    let out = update(&mut state, HostCommand::SetStartHiddenHeld { held: true });
    assert_eq!(out.background_attention, None);

    let out = update(&mut state, HostCommand::SetStartHiddenHeld { held: false });
    assert_eq!(out.background_attention, Some(false));
    assert!(!state.background_unattended);
    assert!(!state.start_hidden_held);
}

#[test]
fn start_hidden_hold_over_a_seeded_period_is_not_a_new_crossing() {
    let mut state = HostState::new(true);
    update(&mut state, HostCommand::SeedBackgroundUnattended { unattended: true });
    let out = update(&mut state, HostCommand::SetStartHiddenHeld { held: true });
    assert_eq!(out.background_attention, None);
    assert!(state.background_unattended);
}

#[test]
fn window_count_rule_stands_down_while_main_is_held() {
    // "main" is registered (live count 1) but hidden; without the hold gate the
    // next quit-relevant dispatch would flip the flag back to attended.
    let mut state = HostState::new(true);
    update(&mut state, HostCommand::SetStartHiddenHeld { held: true });
    assert_eq!(attention_after(&state, 1), None);
    assert_eq!(attention_after(&state, 0), None);
    update(&mut state, HostCommand::SetStartHiddenHeld { held: false });
    // Released with no live window (unit tests can't register one) in
    // background-service mode: still unattended, until a window counts.
    assert!(state.background_unattended);
    assert_eq!(attention_after(&state, 1), Some(false));
}

#[test]
fn start_hidden_attention_transition_truth_table() {
    use super::quit::start_hidden_attention_transition as t;
    // (held, bg_enabled, quitting, currently_unattended, live) -> crossing
    assert_eq!(t(true, false, false, false, 1), Some(true));
    assert_eq!(t(true, true, true, false, 1), Some(true), "a hold is unattended even mid-quit");
    assert_eq!(t(true, false, false, true, 1), None);
    assert_eq!(t(false, false, false, true, 1), Some(false));
    assert_eq!(t(false, false, false, true, 0), Some(false), "no window rule without background mode");
    assert_eq!(t(false, true, false, true, 1), Some(false));
    assert_eq!(t(false, true, false, true, 0), None, "background mode, no window: still unattended");
    assert_eq!(t(false, true, true, true, 0), Some(false), "quitting is never unattended");
    assert_eq!(t(false, false, false, false, 1), None);
}

#[test]
fn attention_commands_are_not_quit_relevant() {
    use super::is_quit_relevant;
    assert!(!is_quit_relevant(&HostCommand::SeedBackgroundUnattended { unattended: true }));
    assert!(!is_quit_relevant(&HostCommand::SetStartHiddenHeld { held: true }));
}

// ── Outer-window HWND registry and drop-target classification (R5/R6) ────

#[test]
fn bind_window_hwnd_replaces_unless_asked_to_keep() {
    let mut state = HostState::default();
    let bind = |label: &str, hwnd: isize, replace: bool| HostCommand::BindWindowHwnd {
        label: label.to_string(),
        hwnd,
        replace,
    };
    update(&mut state, bind("floating-a", 0x10, true));
    assert_eq!(state.window_hwnds.get("floating-a"), Some(&0x10));
    // A guess (capture_hwnd_for_label) never overwrites the creator's bind.
    update(&mut state, bind("floating-a", 0x20, false));
    assert_eq!(state.window_hwnds.get("floating-a"), Some(&0x10));
    update(&mut state, bind("main", 0x30, false));
    assert_eq!(state.window_hwnds.get("main"), Some(&0x30));
    update(&mut state, bind("floating-a", 0x20, true));
    assert_eq!(state.window_hwnds.get("floating-a"), Some(&0x20));
}

#[test]
fn evict_window_hwnd_returns_the_binding_once() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::BindWindowHwnd { label: "main".into(), hwnd: 0x30, replace: true });
    let out = update(&mut state, HostCommand::EvictWindowHwnd { label: "main".into() });
    assert_eq!(out.evicted_window_hwnd, Some(0x30));
    assert!(out.events.is_empty());
    let again = update(&mut state, HostCommand::EvictWindowHwnd { label: "main".into() });
    assert_eq!(again.evicted_window_hwnd, None, "idempotent");
}

/// `state::window_hwnds`'s shim calls `update` directly; that is only the same
/// as `host_dispatch` while these arms stay out of the quit reconcile.
#[test]
fn window_hwnd_commands_are_not_quit_relevant() {
    use super::is_quit_relevant;
    assert!(!is_quit_relevant(&HostCommand::BindWindowHwnd { label: "main".into(), hwnd: 1, replace: true }));
    assert!(!is_quit_relevant(&HostCommand::EvictWindowHwnd { label: "main".into() }));
}

#[cfg(target_os = "windows")]
#[test]
fn failed_relabel_leaves_the_hwnd_binding_alone() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::BindWindowHwnd { label: "floating-pool-a".into(), hwnd: 0x10, replace: true });
    // No browser under the old label, so the relabel fails and moves nothing.
    let out = update(
        &mut state,
        HostCommand::RelabelBrowser { old_label: "floating-pool-a".into(), new_label: "floating-a".into() },
    );
    assert!(!out.relabel_ok);
    assert_eq!(state.window_hwnds.get("floating-pool-a"), Some(&0x10));
    assert!(!state.window_hwnds.contains_key("floating-a"));
}

#[test]
fn labels_at_hwnd_is_sorted_and_exact() {
    let hwnds: HashMap<String, isize> =
        [("window-b", 7), ("main", 7), ("floating-c", 8)].into_iter().map(|(l, h)| (l.to_string(), h)).collect();
    assert_eq!(super::labels_at_hwnd(&hwnds, 7), vec!["main", "window-b"]);
    assert_eq!(super::labels_at_hwnd(&hwnds, 8), vec!["floating-c"]);
    assert!(super::labels_at_hwnd(&hwnds, 9).is_empty());
}

mod drop_target {
    use super::super::window_hwnds::classify_drop_target;
    use super::super::DropTarget;
    use crate::state::BrowserKind;

    const USER: BrowserKind = BrowserKind::TopLevel { is_pool: false };

    /// Classify `labels` where every label has `kind`, "main" has a frame of
    /// its own, and `with_backend` lists the labels whose renderer registered
    /// a backend window.
    fn classify(labels: &[&str], kind: Option<BrowserKind>, main_has_frame: bool, with_backend: &[&str]) -> DropTarget {
        classify_drop_target(labels, &|_| kind.clone(), main_has_frame, &|l| with_backend.contains(&l))
    }

    #[test]
    fn floaters_are_never_a_target_whatever_their_label() {
        for label in ["floating-a", "floating-pool-a", "window-a"] {
            for is_pool in [false, true] {
                assert_eq!(
                    classify(&[label], Some(BrowserKind::Floater { is_pool }), true, &[label]),
                    DropTarget::Floater(label.to_string()),
                );
            }
        }
    }

    #[test]
    fn a_user_window_is_a_target_by_type_not_label() {
        assert_eq!(classify(&["main"], Some(USER), true, &["main"]), DropTarget::Window("main".into()));
        assert_eq!(classify(&["window-a"], Some(USER), true, &["main", "window-a"]), DropTarget::Window("window-a".into()));
        // Promoted from the pool: keeps its `window-pool-*` label, is a user window.
        assert_eq!(
            classify(&["window-pool-a"], Some(USER), true, &["main", "window-pool-a"]),
            DropTarget::Window("window-pool-a".into()),
        );
        // A tear-off whose renderer hasn't registered yet, while main has its frame.
        assert_eq!(classify(&["window-a"], Some(USER), true, &["main"]), DropTarget::Window("window-a".into()));
    }

    #[test]
    fn nothing_else_takes_a_drop() {
        let not_targets = [
            Some(BrowserKind::TopLevel { is_pool: true }),
            Some(BrowserKind::Pane { block_id: "b".into() }),
            Some(BrowserKind::Popup),
            None, // bound at window creation, browser not registered yet
        ];
        for kind in not_targets {
            assert_eq!(classify(&["window-pool-a"], kind.clone(), true, &["main"]), DropTarget::Nothing, "{kind:?}");
        }
    }

    #[test]
    fn untracked_and_ambiguous_hwnds() {
        assert_eq!(classify(&[], Some(USER), true, &["main"]), DropTarget::Untracked);
        // Two labels on one HWND: one is stale, and nothing says which.
        assert_eq!(classify(&["main", "window-a"], Some(USER), true, &["main", "window-a"]), DropTarget::Nothing);
    }

    #[test]
    fn a_window_serving_the_primary_is_main() {
        // Its renderer registered as "main", it has no backend window of its
        // own, and "main" has no frame of its own.
        assert_eq!(classify(&["window-pool-a"], Some(USER), false, &["main"]), DropTarget::Window("main".into()));
        assert_eq!(classify(&["window-a"], Some(USER), false, &["main"]), DropTarget::Window("main".into()));
        // Any one of those facts missing: its own label.
        assert_eq!(classify(&["window-pool-a"], Some(USER), true, &["main"]), DropTarget::Window("window-pool-a".into()));
        assert_eq!(
            classify(&["window-pool-a"], Some(USER), false, &["main", "window-pool-a"]),
            DropTarget::Window("window-pool-a".into()),
        );
        assert_eq!(classify(&["window-pool-a"], Some(USER), false, &[]), DropTarget::Window("window-pool-a".into()));
    }

    #[test]
    fn drop_target_at_reads_the_registry() {
        let mut state = super::HostState::default();
        super::update(
            &mut state,
            super::HostCommand::BindWindowHwnd { label: "main".into(), hwnd: 0x30, replace: true },
        );
        let none: &dyn Fn(&str) -> bool = &|_| false;
        assert_eq!(super::super::drop_target_at(&state, 0x99, none), DropTarget::Untracked);
        // Bound, but no browser registered under the label (unit tests can't
        // register one: a `BrowserHandle` needs a real `cef::Browser`).
        assert_eq!(super::super::drop_target_at(&state, 0x30, none), DropTarget::Nothing);
    }
}
