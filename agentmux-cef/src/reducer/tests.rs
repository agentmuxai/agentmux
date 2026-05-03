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
}

// ── H.1.d (PR #5) — TryRegisterBrowserPaneLive / EnqueueBrowserPaneClose return-values
//   / DrainBrowserPaneByLabel ────────────────────────────────────────────────

#[test]
fn try_register_browser_pane_live_fresh_returns_label_and_inserts_live() {
    let mut state = HostState::default();
    let out = update(
        &mut state,
        HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into() },
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
        HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into() },
    ).browser_pane_register_result
    {
        Some(RegisterResult::Fresh(l)) => l,
        _ => unreachable!(),
    };
    let out = update(
        &mut state,
        HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into() },
    );
    match out.browser_pane_register_result {
        Some(RegisterResult::AlreadyLive(l)) => assert_eq!(l, first),
        other => panic!("expected AlreadyLive, got {:?}", other),
    }
    assert!(out.events.is_empty(), "no event for AlreadyLive — caller just navigates");
}

#[test]
fn try_register_browser_pane_live_closing_returns_closing() {
    let mut state = HostState::default();
    update(&mut state, HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into() });
    update(&mut state, HostCommand::EnqueueBrowserPaneClose { block_id: "b1".into() });
    let out = update(
        &mut state,
        HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into() },
    );
    assert!(matches!(out.browser_pane_register_result, Some(RegisterResult::Closing)));
}

#[test]
fn try_register_browser_pane_live_during_shutdown_errors() {
    let mut state = HostState::default();
    state.lifecycle = HostLifecyclePhase::ShuttingDown;
    let out = update(
        &mut state,
        HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into() },
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
        HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into() },
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
    update(&mut state, HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into() });
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
        HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into() },
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

    update(&mut state, HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into() });
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
        HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into() },
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
        HostCommand::TryRegisterBrowserPaneLive { block_id: "b1".into() },
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
