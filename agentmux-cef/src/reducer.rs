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

use crate::state::PendingWindowCreation;

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
            HostEvent::Error { version, .. } => *version,
        };
        let v1 = extract_version(&out1.events);
        let v2 = extract_version(&out2.events);
        let v3 = extract_version(&out3.events);
        assert!(v1 < v2);
        assert!(v2 < v3);
    }
}
