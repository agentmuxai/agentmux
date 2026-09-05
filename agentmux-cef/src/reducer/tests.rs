// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::state::WindowKind;

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

#[test]
fn enqueue_during_shutdown_is_rejected() {
    let mut state = HostState::default();
    state.lifecycle = HostLifecyclePhase::ShuttingDown;
    let out = update(
        &mut state,
        HostCommand::EnqueuePendingWindowCreation { entry: entry("w1") },
    );
    assert!(matches!(
        out.events.as_slice(),
        [HostEvent::Error { .. }]
    ));
    assert_eq!(state.pending_window_creations.len(), 0);
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
        HostEvent::BrowserPaneCreateRequested { version, .. } => *version,
        HostEvent::BrowserPaneLive { version, .. } => *version,
        HostEvent::BrowserPaneClosing { version, .. } => *version,
        HostEvent::BrowserPaneClosed { version, .. } => *version,
        HostEvent::BrowserPaneCreationFailed { version, .. } => *version,
        HostEvent::BrowserRegistered { version, .. } => *version,
        HostEvent::BrowserUnregistered { version, .. } => *version,
        HostEvent::DragStarted { version, .. } => *version,
        HostEvent::DragEnded { version, .. } => *version,
        HostEvent::PoolWindowEntered { version, .. } => *version,
        HostEvent::PoolWindowLeft { version, .. } => *version,
        HostEvent::PoolEmpty { version } => *version,
        HostEvent::QuitDraining { version, .. } => *version,
        HostEvent::QuitReady { version } => *version,
        HostEvent::TopLevelCreationRequested { version, .. } => *version,
        HostEvent::TopLevelCreationStarted { version, .. } => *version,
        HostEvent::TopLevelCreationCompleted { version, .. } => *version,
        HostEvent::TopLevelCreationFailed { version, .. } => *version,
        HostEvent::TopLevelQueueLengthChanged { version, .. } => *version,
        HostEvent::WindowOpacityApplied { version, .. } => *version,
        HostEvent::WindowOpacityCleared { version, .. } => *version,
        HostEvent::PaneWindowStateChanged { version, .. } => *version,
        HostEvent::Effect { version, .. } => *version,
        HostEvent::Error { version, .. } => *version,
    };
    let v1 = extract_version(&out1.events);
    let v2 = extract_version(&out2.events);
    let v3 = extract_version(&out3.events);
    assert!(v1 < v2);
    assert!(v2 < v3);
}

// ── Phase H foundations tests ───────────────────────────────────────

fn browser_pane_request(block_id: &str, label: &str) -> HostCommand {
    HostCommand::EnqueueBrowserPaneCreate {
        block_id: block_id.to_string(),
        label: label.to_string(),
    }
}

fn top_level_request(label: &str, source: TopLevelSource) -> TopLevelCreationRequest {
    TopLevelCreationRequest {
        label: label.to_string(),
        kind: WindowKind::FullInstance,
        parent_instance_id: None,
        url: format!("https://example.test/{}", label),
        pos: (0, 0),
        size: (800, 600),
        frameless: true,
        source,
    }
}

// ── H.1 panes ────────────────────────────────────────────────────────

#[test]
fn enqueue_browser_pane_create_inserts_live() {
    let mut state = HostState::default();
    let out = update(&mut state, browser_pane_request("b1", "browser-pane-b1-1"));
    assert!(state.browser_panes.contains_key("b1"));
    assert!(matches!(state.browser_panes["b1"].lifecycle, BrowserPaneLifecycle::Live));
    assert!(matches!(out.events[0], HostEvent::BrowserPaneCreateRequested { .. }));
}

#[test]
fn enqueue_browser_pane_create_duplicate_rejected() {
    let mut state = HostState::default();
    update(&mut state, browser_pane_request("b1", "browser-pane-b1-1"));
    let out = update(&mut state, browser_pane_request("b1", "browser-pane-b1-2"));
    assert!(matches!(out.events[0], HostEvent::Error { .. }));
    assert_eq!(state.browser_panes.len(), 1);
}

#[test]
fn pane_close_lifecycle() {
    let mut state = HostState::default();
    update(&mut state, browser_pane_request("b1", "browser-pane-b1-1"));
    update(&mut state, HostCommand::EnqueueBrowserPaneClose { block_id: "b1".into() });
    assert!(matches!(
        state.browser_panes["b1"].lifecycle,
        BrowserPaneLifecycle::Closing { .. }
    ));
    update(&mut state, HostCommand::CompleteBrowserPaneClose { block_id: "b1".into() });
    assert!(!state.browser_panes.contains_key("b1"));
}

#[test]
fn pane_abort_removes_entry() {
    let mut state = HostState::default();
    update(&mut state, browser_pane_request("b1", "browser-pane-b1-1"));
    let out = update(
        &mut state,
        HostCommand::AbortBrowserPaneCreate {
            block_id: "b1".into(),
            reason: "test".into(),
        },
    );
    assert!(!state.browser_panes.contains_key("b1"));
    assert!(matches!(out.events[0], HostEvent::BrowserPaneCreationFailed { .. }));
}

#[test]
fn pane_close_idempotent_for_missing() {
    let mut state = HostState::default();
    let out = update(&mut state, HostCommand::EnqueueBrowserPaneClose { block_id: "missing".into() });
    assert!(out.events.is_empty()); // idempotent no-op
    // The load-bearing property for issue #2218 B.4:
    // BrowserPaneManager::close() (browser_panes.rs) checks exactly this
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
    assert!(label.starts_with("browser-pane-b1-"));
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
    update(&mut state, HostCommand::EnqueueBrowserPaneClose { block_id: "b1".into() });
    let out = update(
        &mut state,
        HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into(), pending: None },
    );
    assert!(matches!(out.browser_pane_register_result, Some(RegisterResult::Closing)));
}

#[test]
fn try_register_browser_pane_live_during_shutdown_errors() {
    let mut state = HostState::default();
    state.lifecycle = HostLifecyclePhase::ShuttingDown;
    let out = update(
        &mut state,
        HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into(), pending: None },
    );
    assert!(out.browser_pane_register_result.is_none());
    assert!(matches!(out.events[0], HostEvent::Error { .. }));
    assert!(!state.browser_panes.contains_key("b1"));
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
        HostCommand::EnqueueBrowserPaneClose { block_id: "b1".into() },
    );
    assert_eq!(out.closed_browser_pane_label, Some(label));
}

#[test]
fn enqueue_browser_pane_close_returns_none_for_already_closing() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into(), pending: None });
    update(&mut state, HostCommand::EnqueueBrowserPaneClose { block_id: "b1".into() });
    let out = update(
        &mut state,
        HostCommand::EnqueueBrowserPaneClose { block_id: "b1".into() },
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

    update(&mut state, HostCommand::EnqueueBrowserPaneClose { block_id: "b1".into() });
    // Closing pane present → gate is CLOSED
    assert!(state.browser_panes.values().any(|e| matches!(e.lifecycle, BrowserPaneLifecycle::Closing { .. })));

    update(&mut state, HostCommand::CompleteBrowserPaneClose { block_id: "b1".into() });
    // Drained → gate is OPEN again
    assert!(!state.browser_panes.values().any(|e| matches!(e.lifecycle, BrowserPaneLifecycle::Closing { .. })));
}

#[test]
fn drain_after_close_recreate_does_not_evict_new_entry() {
    // The exact bug BROWSER_PANE_LABEL_SEQ defends against: register → close →
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
    update(&mut state, HostCommand::EnqueueBrowserPaneClose { block_id: "b1".into() });
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
    update(&mut state, HostCommand::EnqueueBrowserPaneClose { block_id: "b1".into() });

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
    update(&mut state, HostCommand::ConfirmDrained);
    assert_eq!(state.quit_state, QuitState::Quit);
    // Subsequent BeginDrain is a no-op (monotonic).
    let out = update(&mut state, HostCommand::BeginDrain { reason: QuitReason::External });
    assert!(out.events.is_empty());
    assert_eq!(state.quit_state, QuitState::Quit);
}

// ── H.6 top-level runner (singleton + fail-fast) ─────────────────────

#[test]
fn enqueue_top_level_when_idle_starts_immediately() {
    let mut state = HostState::default();
    let out = update(
        &mut state,
        HostCommand::EnqueueTopLevelWindow {
            request: top_level_request("a", TopLevelSource::User),
        },
    );
    assert!(state.top_level_creation.in_flight.is_some());
    assert_eq!(state.top_level_creation.in_flight.as_ref().unwrap().label, "a");
    let begin_count = out
        .events
        .iter()
        .filter(|e| matches!(
            e,
            HostEvent::Effect { effect: EffectKind::PostCreateWindow { .. }, .. }
        ))
        .count();
    assert_eq!(begin_count, 1);
}

#[test]
fn user_initiated_when_busy_fails_fast() {
    let mut state = HostState::default();
    update(
        &mut state,
        HostCommand::EnqueueTopLevelWindow {
            request: top_level_request("a", TopLevelSource::User),
        },
    );
    // Second user-initiated request: in-flight is occupied → error.
    let out = update(
        &mut state,
        HostCommand::EnqueueTopLevelWindow {
            request: top_level_request("b", TopLevelSource::User),
        },
    );
    assert!(matches!(out.events[0], HostEvent::Error { .. }));
    assert_eq!(state.top_level_creation.queue.len(), 0); // not queued
}

#[test]
fn background_when_busy_queues_silently() {
    let mut state = HostState::default();
    update(
        &mut state,
        HostCommand::EnqueueTopLevelWindow {
            request: top_level_request("a", TopLevelSource::User),
        },
    );
    // Background request queues even though in-flight occupied.
    update(
        &mut state,
        HostCommand::EnqueueTopLevelWindow {
            request: top_level_request("b", TopLevelSource::Background),
        },
    );
    assert_eq!(state.top_level_creation.queue.len(), 1);
    assert_eq!(state.top_level_creation.in_flight.as_ref().unwrap().label, "a");
}

#[test]
fn callback_fired_advances_queue() {
    let mut state = HostState::default();
    update(
        &mut state,
        HostCommand::EnqueueTopLevelWindow {
            request: top_level_request("a", TopLevelSource::User),
        },
    );
    update(
        &mut state,
        HostCommand::EnqueueTopLevelWindow {
            request: top_level_request("b", TopLevelSource::Background),
        },
    );
    let out = update(
        &mut state,
        HostCommand::TopLevelCallbackFired { label: "a".into() },
    );
    // a archived to history; b now in-flight.
    assert_eq!(state.top_level_creation.history.len(), 1);
    assert_eq!(state.top_level_creation.in_flight.as_ref().unwrap().label, "b");
    assert!(out.events.iter().any(|e| matches!(e, HostEvent::TopLevelCreationCompleted { .. })));
}

#[test]
fn renderer_terminated_fails_in_flight() {
    let mut state = HostState::default();
    update(
        &mut state,
        HostCommand::EnqueueTopLevelWindow {
            request: top_level_request("a", TopLevelSource::User),
        },
    );
    update(
        &mut state,
        HostCommand::TopLevelRendererTerminated {
            label: "a".into(),
            status: "killed".into(),
        },
    );
    assert!(state.top_level_creation.in_flight.is_none());
    assert_eq!(state.top_level_creation.history.len(), 1);
    assert!(matches!(
        state.top_level_creation.history.back().unwrap().outcome,
        TopLevelCreationOutcome::RendererTerminated { .. }
    ));
}

#[test]
fn callback_fired_with_unknown_label_is_noop_or_orphan_close() {
    let mut state = HostState::default();
    // No in-flight, no browser registered for this label.
    let out = update(
        &mut state,
        HostCommand::TopLevelCallbackFired { label: "ghost".into() },
    );
    assert!(out.events.is_empty()); // pure no-op when no orphan to close
}

#[test]
fn enqueue_top_level_during_quit_rejected() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::BeginDrain { reason: QuitReason::LastWindowClosed });
    let out = update(
        &mut state,
        HostCommand::EnqueueTopLevelWindow {
            request: top_level_request("a", TopLevelSource::User),
        },
    );
    assert!(matches!(out.events[0], HostEvent::Error { .. }));
    assert!(state.top_level_creation.in_flight.is_none());
}

#[test]
fn history_caps_at_50() {
    let mut state = HostState::default();
    for i in 0..60 {
        let label = format!("w{}", i);
        update(
            &mut state,
            HostCommand::EnqueueTopLevelWindow {
                request: top_level_request(&label, TopLevelSource::Background),
            },
        );
        update(
            &mut state,
            HostCommand::TopLevelCallbackFired { label },
        );
    }
    assert_eq!(state.top_level_creation.history.len(), TOP_LEVEL_CREATION_HISTORY_CAP);
    assert_eq!(state.top_level_creation.history.front().unwrap().label, "w10");
    assert_eq!(state.top_level_creation.history.back().unwrap().label, "w59");
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
fn pool_drain_clears_all() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::PoolWindowSpawnStart { label: "p1".into() });
    update(&mut state, HostCommand::PoolWindowReady { label: "p1".into() });
    update(&mut state, HostCommand::PoolWindowSpawnStart { label: "p2".into() });
    let out = update(&mut state, HostCommand::PoolDrainAll);
    assert!(state.pool.queue.is_empty());
    assert!(state.pool.unpromoted.is_empty());
    assert!(out.events.iter().any(|e| matches!(e, HostEvent::PoolEmpty { .. })));
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
    assert!(out.pane_pool_destroyed_was_unpromoted, "must report a real removal, not a no-op");
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
    assert!(!out.pane_pool_destroyed_was_unpromoted);
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

/// Regression test for reagent P2 on PR #654 round 3.
///
/// `handle_promote_pool_window` should be idempotent for truly unknown
/// labels (matching `handle_pool_destroyed_before_promote`'s pattern).
/// A stale promote command — e.g., racing with destroy — must not emit
/// a phantom `PoolWindowLeft` event that observers might act on.
#[test]
fn pool_promote_with_unknown_label_is_noop() {
    let mut state = HostState::default();
    let out = update(&mut state, HostCommand::PromotePoolWindow { label: "ghost".into() });
    assert!(out.events.is_empty(), "promote of unknown label must be no-op");
    assert!(state.pool.queue.is_empty());
    assert!(state.pool.unpromoted.is_empty());
}

/// Confirms promote DOES emit when the label was in queue (the normal flow).
#[test]
fn pool_promote_with_known_label_emits_event() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::PoolWindowSpawnStart { label: "p1".into() });
    update(&mut state, HostCommand::PoolWindowReady { label: "p1".into() });
    let out = update(&mut state, HostCommand::PromotePoolWindow { label: "p1".into() });
    assert!(state.pool.queue.is_empty());
    assert!(out.events.iter().any(|e| matches!(
        e,
        HostEvent::PoolWindowLeft { reason: PoolLeaveReason::Promoted, .. }
    )));
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

/// Regression test for codex P1 on PR #654 round 1.
///
/// Setup: in-flight User creation, Background queued behind it. Begin
/// drain. Complete the in-flight. The queued Background request must
/// NOT be started — even though it was enqueued before drain, starting
/// it would create a new window mid-shutdown and prevent drain completion.
#[test]
fn queued_background_does_not_start_after_drain_begins() {
    let mut state = HostState::default();
    // Step 1: User-initiated creation goes in-flight.
    update(
        &mut state,
        HostCommand::EnqueueTopLevelWindow {
            request: top_level_request("user-window", TopLevelSource::User),
        },
    );
    assert!(state.top_level_creation.in_flight.is_some());
    // Step 2: Background pool refill queues behind it.
    update(
        &mut state,
        HostCommand::EnqueueTopLevelWindow {
            request: top_level_request("pool-refill", TopLevelSource::Background),
        },
    );
    assert_eq!(state.top_level_creation.queue.len(), 1);
    // Step 3: User triggers shutdown (last window closed). Drain begins.
    update(&mut state, HostCommand::BeginDrain { reason: QuitReason::LastWindowClosed });
    assert!(matches!(state.quit_state, QuitState::Draining { .. }));
    // Step 4: The in-flight user-window's CEF callback fires. Normally
    // this would pop the queued Background request and start it. With
    // the quit gate it must NOT.
    let out = update(
        &mut state,
        HostCommand::TopLevelCallbackFired { label: "user-window".into() },
    );
    assert!(state.top_level_creation.in_flight.is_none(), "in-flight cleared after callback");
    assert_eq!(state.top_level_creation.queue.len(), 1, "queued background still queued");
    // CRITICAL: no PostCreateWindow effect emitted.
    let post_create_count = out
        .events
        .iter()
        .filter(|e| matches!(
            e,
            HostEvent::Effect { effect: EffectKind::PostCreateWindow { .. }, .. }
        ))
        .count();
    assert_eq!(post_create_count, 0, "no PostCreateWindow effect during drain");
    // The completion event for the user-window should still fire.
    assert!(
        out.events.iter().any(|e| matches!(e, HostEvent::TopLevelCreationCompleted { .. })),
        "user-window completion still emitted"
    );
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
    assert_eq!(should_begin_drain(true, 0, false, &QuitState::Quit, false), None);
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
    assert_eq!(should_begin_drain(true, 0, false, &QuitState::Quit, true), None);
}

/// Only USER-initiated creations block drain; pool/pane background creations don't.
#[test]
fn user_creation_in_flight_ignores_background_labels() {
    let mut state = HostState::default();
    // Pool refill, browser-pane, warm floating-pool, AND broad floating- tear-off
    // are all background — none blocks drain (mirrors orphan_reconcile.rs:302-304).
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
    assert!(should_close_on_arrival(&user, &QuitState::Quit));
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
    assert!(is_quit_relevant(&HostCommand::PoolDrainAll));
    assert!(is_quit_relevant(&HostCommand::BeginDrain {
        reason: QuitReason::LastWindowClosed
    }));
    // Irrelevant — hot path + pane lifecycle (panes never affect the window count).
    assert!(!is_quit_relevant(&HostCommand::EvictFloatingPaneWindowState { label: "f".into() }));
    assert!(!is_quit_relevant(&HostCommand::EnqueueBrowserPaneClose { block_id: "b".into() }));
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
