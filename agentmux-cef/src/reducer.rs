// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase F.1 — host reducer.
//
// Third reducer in the multi-reducer architecture, after the launcher
// (Phase B) and srv (Phase E). Same pure-functional shape as the
// other two: `update(&mut HostState, HostCommand) -> Vec<HostEvent>`,
// no I/O, no async, sub-millisecond mutex hold time.
//
// **Scope of F.1 (this PR):** skeleton + the `pending_window_creations`
// arm. Per `docs/specs/SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md` §3.1,
// pending_window_creations is the lowest-risk migration:
// single-producer/single-consumer queue with a clean enqueue/dequeue
// lifecycle and no FFI handles inside.
//
// **NOT in F.1:** drag arms (F.3), tear-off hook arms (F.4 — folds
// into the tear-off spec). The CEF `browsers` map and warm pool stay
// outside the reducer indefinitely (snapshot-and-drop discipline at
// every read site, see spec §3.2 / §6).
//
// **Wire protocol:** F.1's `HostCommand` and `HostEvent` are
// host-internal. They do NOT cross IPC. When a future PR adds a
// command that needs frontend or launcher access, that PR promotes
// the relevant variants to `agentmux-common::ipc::Command` /
// `agentmux-common::ipc::Event` and adds the IPC plumbing. Keeping
// F.1 in-process avoids serializing `PendingWindowCreation` over a
// pipe just to satisfy a pattern that has no current consumer.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::state::{
    CompletedTopLevelCreation, InFlightTopLevelCreation, PendingWindowCreation,
    TopLevelCreationOutcome, TopLevelCreationPhase, TopLevelCreationRequest,
};

/// Per-creation deadline. A wedged CEF profile init or a hung renderer is
/// evicted after this many seconds and the queue advances. 30s is generous
/// enough to absorb realistic profile-init latency under load while still
/// bounding the worst case where today the freeze is unbounded.
///
/// Future: make configurable via `~/.agentmux/config.toml` — see
/// `docs/specs/SPEC_HOST_WINDOW_CREATION_RUNNER_2026-05-02.md` §"Configurable
/// knobs". For now, hard-coded.
pub const TOP_LEVEL_CREATION_DEADLINE: Duration = Duration::from_secs(30);

/// History ring buffer cap. Old entries evicted FIFO when this is reached.
/// Sized so `--diag windows` shows enough recent activity to be useful
/// without growing `HostState` unboundedly.
pub const TOP_LEVEL_CREATION_HISTORY_CAP: usize = 50;

/// Lifecycle phase of the host reducer. Mirrors the launcher and srv
/// reducers' phase enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostLifecyclePhase {
    /// Pre-init: AppState exists, but no commands accepted yet.
    Bootstrapping,
    /// Normal operation: all commands accepted.
    Running,
    /// Shutting down: only cleanup commands accepted; producers
    /// short-circuit with no-op events.
    ShuttingDown,
}

/// State owned by the host reducer.
///
/// Held inside `AppState.host_state: parking_lot::Mutex<HostState>`.
/// Locked briefly by `host_dispatch`; never held across CEF callbacks
/// or `SendMessage` (snapshot-and-drop discipline — see spec §6).
pub struct HostState {
    /// FIFO queue of pre-create handoffs. Pushed by callers
    /// (`pane/creation.rs`, `commands/window.rs::open_new_window`,
    /// `commands/drag.rs::tear_off`, `commands/window_pool.rs::spawn_pool_window`)
    /// before `post_create_window`. Popped by `client.rs::on_after_created`
    /// when CEF reports a new browser. Peeked at the back by
    /// `wrr/win_event.rs::handle_event` to label OS-level WM_CREATE
    /// events with the upcoming label.
    ///
    /// Invariants:
    /// - At most one entry per (in-flight) browser create.
    /// - `on_after_created` always pops the head it expects to find.
    /// - The "main" window is special-cased in `on_after_created` and
    ///   never has a corresponding entry here.
    pub pending_window_creations: VecDeque<PendingWindowCreation>,

    // ── Phase 2 (window-creation runner) ─────────────────────────────────
    // Serializes top-level window creation through a single in-flight slot
    // with a watchdog deadline. Routes around the CEF v146 Chrome profile-
    // init deadlock under concurrent load (see freeze investigation
    // 2026-05-02).
    //
    // Pane creation does NOT use these fields — pane code paths still
    // enqueue/dequeue through `pending_window_creations` only.

    /// Queue of top-level window creation requests waiting to start.
    /// Producer: `EnqueueTopLevelWindow`. Consumer: `StartNextTopLevelIfIdle`
    /// (auto-emitted by Enqueue when the in-flight slot is empty, and by
    /// `MarkTopLevelRendererReady` / `TopLevelTimeoutTick` /
    /// `AbortInFlightTopLevel` when an in-flight completes).
    pub top_level_creation_queue: VecDeque<TopLevelCreationRequest>,

    /// The creation currently being processed by CEF, or `None` if idle.
    /// Singleton invariant — the reducer enforces at most one across all
    /// arms.
    pub in_flight_top_level_creation: Option<InFlightTopLevelCreation>,

    /// Ring buffer of completed top-level creations (any outcome). Capped
    /// at `TOP_LEVEL_CREATION_HISTORY_CAP`; oldest evicted FIFO.
    pub top_level_creation_history: VecDeque<CompletedTopLevelCreation>,

    /// Monotonic id allocator for `creation_id` in `InFlightTopLevelCreation`.
    pub next_top_level_creation_id: u64,

    /// Lifecycle phase. `Running` is the operating state; the others
    /// gate command acceptance.
    pub lifecycle: HostLifecyclePhase,

    /// Monotonic event-version counter (per host-process run). Same
    /// invariant as launcher / srv reducers.
    pub event_version: u64,
}

impl Default for HostState {
    fn default() -> Self {
        Self {
            pending_window_creations: VecDeque::new(),
            top_level_creation_queue: VecDeque::new(),
            in_flight_top_level_creation: None,
            top_level_creation_history: VecDeque::new(),
            next_top_level_creation_id: 0,
            // Boot directly into Running — nothing in F.1 needs the
            // pre-init guard yet. Future PRs (drag, tear-off hooks)
            // will move boot through Bootstrapping → Running.
            lifecycle: HostLifecyclePhase::Running,
            event_version: 0,
        }
    }
}

impl HostState {
    /// Allocate the next event version. Called inside reducer arms
    /// when emitting an event.
    fn bump_version(&mut self) -> u64 {
        self.event_version += 1;
        self.event_version
    }
}

/// Commands handled by the host reducer.
#[derive(Debug, Clone)]
pub enum HostCommand {
    /// Append a pending-window-creation handoff. Producer side of the
    /// `pending_window_creations` queue (replaces the four direct
    /// `state.pending_window_creations.lock().push_back(...)` sites).
    EnqueuePendingWindowCreation { entry: PendingWindowCreation },

    /// Pop the head of `pending_window_creations`. Returns the popped
    /// entry via `HostEvent::PendingWindowDequeued`, or
    /// `HostEvent::PendingWindowQueueEmpty` if the queue was empty.
    /// Consumer side of the queue (replaces
    /// `client.rs::on_after_created`'s direct `pop_front`).
    DequeuePendingWindowCreation,

    // ── Phase 2 (window-creation runner) ─────────────────────────────────

    /// Append a top-level window creation request to the runner queue. If
    /// the in-flight slot is empty, the reducer auto-emits
    /// `BeginTopLevelCreationEffect` for this request (and pops it from
    /// the queue into the in-flight slot). If busy, it just queues.
    EnqueueTopLevelWindow { request: TopLevelCreationRequest },

    /// Try to start the next queued request. No-op if the in-flight slot
    /// is occupied or the queue is empty. Auto-dispatched internally by
    /// the reducer after Enqueue / RendererReady / TimeoutTick / Abort —
    /// callers don't normally dispatch this directly.
    StartNextTopLevelIfIdle,

    /// Move the in-flight phase forward (Started → BrowserCallbackFired →
    /// RendererReady). Refuses regression. Dispatched by lifecycle
    /// observers (e.g., `client::on_after_created` advances to
    /// BrowserCallbackFired).
    AdvanceTopLevelPhase {
        label: String,
        phase: TopLevelCreationPhase,
    },

    /// Mark the in-flight top-level creation as completed and advance the
    /// queue. Dispatched by `client::on_after_created` after
    /// `Registered browser` (post-renderer-init for our purposes — the
    /// CEF profile-init competition is past at that point).
    MarkTopLevelRendererReady { label: String },

    /// Watchdog tick — checks whether the in-flight creation has exceeded
    /// its deadline. If yes, evicts the in-flight slot, archives it as
    /// `TimedOut`, and auto-advances the queue. If no in-flight or not
    /// past deadline, no-op.
    TopLevelTimeoutTick { now: Instant },

    /// Explicit abort of the in-flight creation. Used by saga compensation
    /// paths (Phase 3). Archives the in-flight slot as `Aborted` and
    /// auto-advances the queue.
    AbortInFlightTopLevel { reason: String },
}

/// Events emitted by the host reducer.
///
/// F.1 keeps these in-host: subscribers log them via tracing for
/// observability, but no IPC propagation. When a future PR adds a
/// wire-level consumer (host→launcher event for cross-process saga
/// observability, frontend dispatcher in E.6), that PR promotes the
/// relevant variants to `agentmux-common::ipc::Event`.
#[derive(Debug, Clone)]
pub enum HostEvent {
    /// A `PendingWindowCreation` was enqueued. Carries a snapshot of
    /// the current queue length so observers can spot pile-ups.
    PendingWindowEnqueued {
        label: String,
        queue_len_after: usize,
        version: u64,
    },

    /// A `PendingWindowCreation` was dequeued. The popped entry
    /// travels back to the caller; observers see only the label and
    /// post-pop queue length.
    PendingWindowDequeued {
        label: String,
        queue_len_after: usize,
        version: u64,
    },

    /// `DequeuePendingWindowCreation` ran on an empty queue. Caller
    /// is responsible for the fallback (the legacy code paths
    /// synthesize a UUID-labelled FullInstance entry).
    PendingWindowQueueEmpty { version: u64 },

    // ── Phase 2 (window-creation runner) ─────────────────────────────────

    /// Side-effect bearer: the effect handler should call
    /// `ui_tasks::post_create_window` for this request. Emitted by the
    /// reducer when an `EnqueueTopLevelWindow` finds the in-flight slot
    /// empty, or when an in-flight terminal advances the queue.
    BeginTopLevelCreationEffect {
        creation_id: u64,
        request: TopLevelCreationRequest,
        version: u64,
    },

    /// In-flight creation reached `RendererReady` and was archived.
    TopLevelCreationCompleted {
        creation_id: u64,
        label: String,
        latency_ms: u64,
        version: u64,
    },

    /// In-flight creation exceeded its deadline; archived as TimedOut.
    /// The wedged renderer/browser is leaked from our perspective —
    /// recoverable across restarts. Operator-visible via `--diag windows`
    /// (Phase 3).
    TopLevelCreationTimedOut {
        creation_id: u64,
        label: String,
        last_phase: TopLevelCreationPhase,
        elapsed_ms: u64,
        version: u64,
    },

    /// In-flight creation was explicitly aborted (saga compensation path).
    TopLevelCreationAborted {
        creation_id: u64,
        label: String,
        reason: String,
        version: u64,
    },

    /// Top-level creation queue length changed. Diagnostic — for spotting
    /// pile-ups under load.
    TopLevelQueueLengthChanged { len: usize, version: u64 },

    /// A command was rejected. Mirrors `Event::Error` in srv/launcher
    /// reducers — kept generic for future arms.
    Error { message: String, version: u64 },
}

/// Output bundle returned from the reducer for the dequeue arm.
///
/// The dequeue arm is the only F.1 command whose caller needs the
/// popped value (an event is not sufficient — `client.rs::on_after_created`
/// uses the popped `PendingWindowCreation`'s fields to drive
/// `window_meta.insert` and `ReportWindowOpened`). Other arms can
/// rely on events alone; this struct keeps the dispatch return type
/// uniform.
#[derive(Debug, Default)]
pub struct DispatchOutput {
    pub events: Vec<HostEvent>,
    pub dequeued: Option<PendingWindowCreation>,
}

/// Pure functional core of the host reducer.
///
/// Returns the events emitted by the command. Side-effecting wiring
/// (logging, future event broadcast) lives in `host_dispatch` — this
/// function takes only `&mut HostState` and produces no I/O.
pub fn update(state: &mut HostState, cmd: HostCommand) -> DispatchOutput {
    match cmd {
        HostCommand::EnqueuePendingWindowCreation { entry } => {
            handle_enqueue_pending_window_creation(state, entry)
        }
        HostCommand::DequeuePendingWindowCreation => {
            handle_dequeue_pending_window_creation(state)
        }
        HostCommand::EnqueueTopLevelWindow { request } => {
            handle_enqueue_top_level(state, request)
        }
        HostCommand::StartNextTopLevelIfIdle => handle_start_next_top_level(state),
        HostCommand::AdvanceTopLevelPhase { label, phase } => {
            handle_advance_top_level_phase(state, label, phase)
        }
        HostCommand::MarkTopLevelRendererReady { label } => {
            handle_mark_top_level_renderer_ready(state, label)
        }
        HostCommand::TopLevelTimeoutTick { now } => handle_top_level_timeout_tick(state, now),
        HostCommand::AbortInFlightTopLevel { reason } => {
            handle_abort_in_flight_top_level(state, reason)
        }
    }
}

fn handle_enqueue_pending_window_creation(
    state: &mut HostState,
    entry: PendingWindowCreation,
) -> DispatchOutput {
    if state.lifecycle == HostLifecyclePhase::ShuttingDown {
        // No new windows accepted during shutdown. Mirrors the
        // launcher reducer's shutdown gating from B.9.3.
        let v = state.bump_version();
        return DispatchOutput {
            events: vec![HostEvent::Error {
                message: "enqueue_pending_window_creation: host is shutting down".to_string(),
                version: v,
            }],
            dequeued: None,
        };
    }
    let label = entry.label.clone();
    state.pending_window_creations.push_back(entry);
    let queue_len_after = state.pending_window_creations.len();
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::PendingWindowEnqueued {
            label,
            queue_len_after,
            version: v,
        }],
        dequeued: None,
    }
}

fn handle_dequeue_pending_window_creation(state: &mut HostState) -> DispatchOutput {
    match state.pending_window_creations.pop_front() {
        Some(entry) => {
            let queue_len_after = state.pending_window_creations.len();
            let v = state.bump_version();
            let label = entry.label.clone();
            DispatchOutput {
                events: vec![HostEvent::PendingWindowDequeued {
                    label,
                    queue_len_after,
                    version: v,
                }],
                dequeued: Some(entry),
            }
        }
        None => {
            let v = state.bump_version();
            DispatchOutput {
                events: vec![HostEvent::PendingWindowQueueEmpty { version: v }],
                dequeued: None,
            }
        }
    }
}

// ── Phase 2 (window-creation runner) reducer arms ────────────────────────

fn handle_enqueue_top_level(
    state: &mut HostState,
    request: TopLevelCreationRequest,
) -> DispatchOutput {
    if state.lifecycle == HostLifecyclePhase::ShuttingDown {
        let v = state.bump_version();
        return DispatchOutput {
            events: vec![HostEvent::Error {
                message: "enqueue_top_level_window: host is shutting down".to_string(),
                version: v,
            }],
            dequeued: None,
        };
    }
    state.top_level_creation_queue.push_back(request);
    let len = state.top_level_creation_queue.len();
    let v = state.bump_version();
    let mut out = DispatchOutput {
        events: vec![HostEvent::TopLevelQueueLengthChanged { len, version: v }],
        dequeued: None,
    };
    // Auto-advance: if we're idle, fire the start now. Inline so callers
    // see all emitted events (BeginTopLevelCreationEffect) on the same
    // dispatch return.
    if state.in_flight_top_level_creation.is_none() {
        let next = handle_start_next_top_level(state);
        out.events.extend(next.events);
    }
    out
}

fn handle_start_next_top_level(state: &mut HostState) -> DispatchOutput {
    if state.in_flight_top_level_creation.is_some() {
        // Already busy. No-op.
        return DispatchOutput::default();
    }
    let request = match state.top_level_creation_queue.pop_front() {
        Some(r) => r,
        None => return DispatchOutput::default(),
    };
    state.next_top_level_creation_id = state.next_top_level_creation_id.wrapping_add(1);
    let creation_id = state.next_top_level_creation_id;
    let now = Instant::now();
    state.in_flight_top_level_creation = Some(InFlightTopLevelCreation {
        creation_id,
        label: request.label.clone(),
        started_at: now,
        phase: TopLevelCreationPhase::Started,
        deadline: now + TOP_LEVEL_CREATION_DEADLINE,
    });
    let v = state.bump_version();
    let queue_len = state.top_level_creation_queue.len();
    DispatchOutput {
        events: vec![
            HostEvent::BeginTopLevelCreationEffect {
                creation_id,
                request,
                version: v,
            },
            HostEvent::TopLevelQueueLengthChanged {
                len: queue_len,
                version: state.bump_version(),
            },
        ],
        dequeued: None,
    }
}

fn handle_advance_top_level_phase(
    state: &mut HostState,
    label: String,
    phase: TopLevelCreationPhase,
) -> DispatchOutput {
    if let Some(ref mut inflight) = state.in_flight_top_level_creation {
        if inflight.label == label && phase > inflight.phase {
            inflight.phase = phase;
        }
    }
    // No event emission — pure state mutation for diag visibility. (Phase
    // 3 may add an event for saga subscribers.)
    DispatchOutput::default()
}

fn handle_mark_top_level_renderer_ready(state: &mut HostState, label: String) -> DispatchOutput {
    let inflight = match state.in_flight_top_level_creation.as_ref() {
        Some(c) if c.label == label => state.in_flight_top_level_creation.take().unwrap(),
        _ => {
            // Mismatched label or no in-flight. Ignore — could be a stale
            // signal from a previously-evicted (timed-out) creation.
            return DispatchOutput::default();
        }
    };
    let now = Instant::now();
    let latency_ms = now.duration_since(inflight.started_at).as_millis() as u64;
    let v_completed = state.bump_version();
    let completed_event = HostEvent::TopLevelCreationCompleted {
        creation_id: inflight.creation_id,
        label: inflight.label.clone(),
        latency_ms,
        version: v_completed,
    };
    push_top_level_history(
        state,
        CompletedTopLevelCreation {
            creation_id: inflight.creation_id,
            label: inflight.label,
            outcome: TopLevelCreationOutcome::Completed,
            started_at: inflight.started_at,
            finished_at: now,
            last_phase: TopLevelCreationPhase::RendererReady,
        },
    );
    let mut out = DispatchOutput {
        events: vec![completed_event],
        dequeued: None,
    };
    // Auto-advance: try to start the next queued request.
    let next = handle_start_next_top_level(state);
    out.events.extend(next.events);
    out
}

fn handle_top_level_timeout_tick(state: &mut HostState, now: Instant) -> DispatchOutput {
    let should_evict = matches!(
        state.in_flight_top_level_creation.as_ref(),
        Some(c) if now >= c.deadline
    );
    if !should_evict {
        return DispatchOutput::default();
    }
    let inflight = state.in_flight_top_level_creation.take().unwrap();
    let elapsed_ms = now.duration_since(inflight.started_at).as_millis() as u64;
    let last_phase = inflight.phase;
    let v = state.bump_version();
    let timed_out_event = HostEvent::TopLevelCreationTimedOut {
        creation_id: inflight.creation_id,
        label: inflight.label.clone(),
        last_phase,
        elapsed_ms,
        version: v,
    };
    push_top_level_history(
        state,
        CompletedTopLevelCreation {
            creation_id: inflight.creation_id,
            label: inflight.label,
            outcome: TopLevelCreationOutcome::TimedOut {
                reason: format!("exceeded {}s deadline at phase {:?}", TOP_LEVEL_CREATION_DEADLINE.as_secs(), last_phase),
            },
            started_at: inflight.started_at,
            finished_at: now,
            last_phase,
        },
    );
    let mut out = DispatchOutput {
        events: vec![timed_out_event],
        dequeued: None,
    };
    // Critical: advance queue even on timeout. A wedged CEF init must not
    // permanently block the queue.
    let next = handle_start_next_top_level(state);
    out.events.extend(next.events);
    out
}

fn handle_abort_in_flight_top_level(state: &mut HostState, reason: String) -> DispatchOutput {
    let inflight = match state.in_flight_top_level_creation.take() {
        Some(c) => c,
        None => return DispatchOutput::default(),
    };
    let now = Instant::now();
    let last_phase = inflight.phase;
    let v = state.bump_version();
    let aborted_event = HostEvent::TopLevelCreationAborted {
        creation_id: inflight.creation_id,
        label: inflight.label.clone(),
        reason: reason.clone(),
        version: v,
    };
    push_top_level_history(
        state,
        CompletedTopLevelCreation {
            creation_id: inflight.creation_id,
            label: inflight.label,
            outcome: TopLevelCreationOutcome::Aborted { reason },
            started_at: inflight.started_at,
            finished_at: now,
            last_phase,
        },
    );
    let mut out = DispatchOutput {
        events: vec![aborted_event],
        dequeued: None,
    };
    let next = handle_start_next_top_level(state);
    out.events.extend(next.events);
    out
}

fn push_top_level_history(state: &mut HostState, entry: CompletedTopLevelCreation) {
    if state.top_level_creation_history.len() >= TOP_LEVEL_CREATION_HISTORY_CAP {
        state.top_level_creation_history.pop_front();
    }
    state.top_level_creation_history.push_back(entry);
}

#[cfg(test)]
mod tests {
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

        let extract_version = |events: &[HostEvent]| match &events[0] {
            HostEvent::PendingWindowEnqueued { version, .. } => *version,
            HostEvent::PendingWindowDequeued { version, .. } => *version,
            HostEvent::PendingWindowQueueEmpty { version } => *version,
            HostEvent::BeginTopLevelCreationEffect { version, .. } => *version,
            HostEvent::TopLevelCreationCompleted { version, .. } => *version,
            HostEvent::TopLevelCreationTimedOut { version, .. } => *version,
            HostEvent::TopLevelCreationAborted { version, .. } => *version,
            HostEvent::TopLevelQueueLengthChanged { version, .. } => *version,
            HostEvent::Error { version, .. } => *version,
        };
        let v1 = extract_version(&out1.events);
        let v2 = extract_version(&out2.events);
        let v3 = extract_version(&out3.events);
        assert!(v1 < v2);
        assert!(v2 < v3);
    }

    // ── Phase 2 (window-creation runner) tests ───────────────────────────

    fn top_level_request(label: &str) -> TopLevelCreationRequest {
        TopLevelCreationRequest {
            label: label.to_string(),
            kind: WindowKind::FullInstance,
            parent_instance_id: None,
            url: format!("https://example.test/{}", label),
            pos: (0, 0),
            size: (800, 600),
            frameless: true,
        }
    }

    #[test]
    fn enqueue_when_idle_starts_creation_immediately() {
        let mut state = HostState::default();
        let out = update(
            &mut state,
            HostCommand::EnqueueTopLevelWindow {
                request: top_level_request("a"),
            },
        );
        assert!(state.in_flight_top_level_creation.is_some());
        assert_eq!(
            state.in_flight_top_level_creation.as_ref().unwrap().label,
            "a"
        );
        let begin_count = out
            .events
            .iter()
            .filter(|e| matches!(e, HostEvent::BeginTopLevelCreationEffect { .. }))
            .count();
        assert_eq!(begin_count, 1, "should emit exactly one BeginTopLevelCreationEffect");
    }

    #[test]
    fn enqueue_when_busy_only_queues() {
        let mut state = HostState::default();
        update(
            &mut state,
            HostCommand::EnqueueTopLevelWindow {
                request: top_level_request("a"),
            },
        );
        assert!(state.in_flight_top_level_creation.is_some());
        let out = update(
            &mut state,
            HostCommand::EnqueueTopLevelWindow {
                request: top_level_request("b"),
            },
        );
        assert_eq!(state.top_level_creation_queue.len(), 1, "b still queued");
        assert_eq!(
            state.in_flight_top_level_creation.as_ref().unwrap().label,
            "a",
            "a still in-flight"
        );
        let begin_count = out
            .events
            .iter()
            .filter(|e| matches!(e, HostEvent::BeginTopLevelCreationEffect { .. }))
            .count();
        assert_eq!(begin_count, 0, "no second start while busy");
    }

    #[test]
    fn renderer_ready_advances_queue() {
        let mut state = HostState::default();
        update(
            &mut state,
            HostCommand::EnqueueTopLevelWindow {
                request: top_level_request("a"),
            },
        );
        update(
            &mut state,
            HostCommand::EnqueueTopLevelWindow {
                request: top_level_request("b"),
            },
        );
        let out = update(
            &mut state,
            HostCommand::MarkTopLevelRendererReady {
                label: "a".into(),
            },
        );
        assert_eq!(
            state.in_flight_top_level_creation.as_ref().unwrap().label,
            "b",
            "b now in-flight"
        );
        assert_eq!(
            state.top_level_creation_history.back().unwrap().label,
            "a",
            "a archived to history"
        );
        let completed_count = out
            .events
            .iter()
            .filter(|e| matches!(e, HostEvent::TopLevelCreationCompleted { .. }))
            .count();
        assert_eq!(completed_count, 1);
        let begin_count = out
            .events
            .iter()
            .filter(|e| matches!(e, HostEvent::BeginTopLevelCreationEffect { .. }))
            .count();
        assert_eq!(begin_count, 1, "next request auto-started");
    }

    #[test]
    fn timeout_tick_evicts_and_advances_queue() {
        let mut state = HostState::default();
        update(
            &mut state,
            HostCommand::EnqueueTopLevelWindow {
                request: top_level_request("a"),
            },
        );
        update(
            &mut state,
            HostCommand::EnqueueTopLevelWindow {
                request: top_level_request("b"),
            },
        );
        let deadline = state
            .in_flight_top_level_creation
            .as_ref()
            .unwrap()
            .deadline;
        let past_deadline = deadline + Duration::from_millis(1);
        let out = update(
            &mut state,
            HostCommand::TopLevelTimeoutTick { now: past_deadline },
        );
        assert_eq!(
            state.in_flight_top_level_creation.as_ref().unwrap().label,
            "b",
            "queue advanced after timeout"
        );
        assert!(matches!(
            state.top_level_creation_history.back().unwrap().outcome,
            TopLevelCreationOutcome::TimedOut { .. }
        ));
        let timed_out_count = out
            .events
            .iter()
            .filter(|e| matches!(e, HostEvent::TopLevelCreationTimedOut { .. }))
            .count();
        assert_eq!(timed_out_count, 1);
    }

    #[test]
    fn timeout_tick_before_deadline_is_noop() {
        let mut state = HostState::default();
        update(
            &mut state,
            HostCommand::EnqueueTopLevelWindow {
                request: top_level_request("a"),
            },
        );
        let well_before = state
            .in_flight_top_level_creation
            .as_ref()
            .unwrap()
            .started_at;
        let out = update(
            &mut state,
            HostCommand::TopLevelTimeoutTick { now: well_before },
        );
        assert!(state.in_flight_top_level_creation.is_some(), "still in-flight");
        assert!(state.top_level_creation_history.is_empty());
        assert!(
            out.events.is_empty(),
            "no events from before-deadline tick"
        );
    }

    #[test]
    fn renderer_ready_with_wrong_label_is_noop() {
        let mut state = HostState::default();
        update(
            &mut state,
            HostCommand::EnqueueTopLevelWindow {
                request: top_level_request("a"),
            },
        );
        let out = update(
            &mut state,
            HostCommand::MarkTopLevelRendererReady {
                label: "wrong".into(),
            },
        );
        assert_eq!(
            state.in_flight_top_level_creation.as_ref().unwrap().label,
            "a"
        );
        assert!(out.events.is_empty());
    }

    #[test]
    fn abort_evicts_and_advances() {
        let mut state = HostState::default();
        update(
            &mut state,
            HostCommand::EnqueueTopLevelWindow {
                request: top_level_request("a"),
            },
        );
        update(
            &mut state,
            HostCommand::EnqueueTopLevelWindow {
                request: top_level_request("b"),
            },
        );
        let out = update(
            &mut state,
            HostCommand::AbortInFlightTopLevel {
                reason: "test reason".into(),
            },
        );
        assert_eq!(
            state.in_flight_top_level_creation.as_ref().unwrap().label,
            "b"
        );
        assert!(matches!(
            state.top_level_creation_history.back().unwrap().outcome,
            TopLevelCreationOutcome::Aborted { .. }
        ));
        let aborted_count = out
            .events
            .iter()
            .filter(|e| matches!(e, HostEvent::TopLevelCreationAborted { .. }))
            .count();
        assert_eq!(aborted_count, 1);
    }

    #[test]
    fn history_caps_at_50() {
        let mut state = HostState::default();
        for i in 0..60 {
            let label = format!("w{}", i);
            update(
                &mut state,
                HostCommand::EnqueueTopLevelWindow {
                    request: top_level_request(&label),
                },
            );
            update(
                &mut state,
                HostCommand::MarkTopLevelRendererReady { label },
            );
        }
        assert_eq!(
            state.top_level_creation_history.len(),
            TOP_LEVEL_CREATION_HISTORY_CAP
        );
        assert_eq!(
            state.top_level_creation_history.front().unwrap().label,
            "w10",
            "oldest 10 evicted (60 created, cap 50)"
        );
        assert_eq!(
            state.top_level_creation_history.back().unwrap().label,
            "w59"
        );
    }

    #[test]
    fn advance_phase_refuses_regression() {
        let mut state = HostState::default();
        update(
            &mut state,
            HostCommand::EnqueueTopLevelWindow {
                request: top_level_request("a"),
            },
        );
        // Started → BrowserCallbackFired
        update(
            &mut state,
            HostCommand::AdvanceTopLevelPhase {
                label: "a".into(),
                phase: TopLevelCreationPhase::BrowserCallbackFired,
            },
        );
        assert_eq!(
            state.in_flight_top_level_creation.as_ref().unwrap().phase,
            TopLevelCreationPhase::BrowserCallbackFired
        );
        // Try to regress to Started — should be ignored.
        update(
            &mut state,
            HostCommand::AdvanceTopLevelPhase {
                label: "a".into(),
                phase: TopLevelCreationPhase::Started,
            },
        );
        assert_eq!(
            state.in_flight_top_level_creation.as_ref().unwrap().phase,
            TopLevelCreationPhase::BrowserCallbackFired,
            "phase did not regress"
        );
    }

    #[test]
    fn enqueue_top_level_during_shutdown_is_rejected() {
        let mut state = HostState::default();
        state.lifecycle = HostLifecyclePhase::ShuttingDown;
        let out = update(
            &mut state,
            HostCommand::EnqueueTopLevelWindow {
                request: top_level_request("a"),
            },
        );
        assert!(matches!(out.events.as_slice(), [HostEvent::Error { .. }]));
        assert!(state.in_flight_top_level_creation.is_none());
        assert!(state.top_level_creation_queue.is_empty());
    }

    #[test]
    fn singleton_invariant_under_random_actions() {
        // Mini fuzz: hammer the reducer with a fixed-but-mixed sequence
        // and assert the in-flight singleton invariant + queue advancement
        // are both preserved.
        let mut state = HostState::default();
        let labels = ["a", "b", "c", "d", "e"];
        for label in labels {
            update(
                &mut state,
                HostCommand::EnqueueTopLevelWindow {
                    request: top_level_request(label),
                },
            );
            // After every enqueue, exactly one in-flight if queue+inflight
            // > 0.
            let total = state.top_level_creation_queue.len()
                + state.in_flight_top_level_creation.iter().count();
            assert!(total > 0);
            assert!(state.in_flight_top_level_creation.is_some());
        }
        // Drain by alternating ready + timeout.
        while state.in_flight_top_level_creation.is_some() {
            let label = state
                .in_flight_top_level_creation
                .as_ref()
                .unwrap()
                .label
                .clone();
            update(&mut state, HostCommand::MarkTopLevelRendererReady { label });
        }
        assert!(state.top_level_creation_queue.is_empty());
        assert_eq!(state.top_level_creation_history.len(), 5);
    }
}
