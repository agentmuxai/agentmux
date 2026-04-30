// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.1a — saga coordinator infrastructure.
//
// A saga is a state machine that orchestrates a multi-step,
// multi-reducer flow (e.g. tear-off touches host pool + srv
// workspace + launcher window registration). Sagas exist where
// per-flow correctness needs explicit coordination — distributed
// subscriber callbacks aren't enough.
//
// This module provides the framework. Phase E.5 adds the first
// concrete saga consumer (tear-off). E.1a is framework-only — no
// actual sagas yet.
//
// Design (per `docs/specs/SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` §7):
//
//   trait Saga {
//       fn start(&mut self, ctx: &SagaCtx) -> SagaAction;
//       fn on_event(&mut self, event: &Event, ctx: &SagaCtx) -> SagaAction;
//       fn name(&self) -> &'static str;
//   }
//
//   enum SagaAction {
//       IssueCmd { target: PipeTarget, cmd: Command },
//       Done,
//       Failed { reason: String },
//       Wait,
//   }
//
// The `SagaCoordinator` task subscribes to the broadcast bus,
// routes events to in-flight sagas by saga-correlation, dispatches
// IssueCmd actions to the appropriate pipe, and emits
// `SagaStarted` / `SagaCompleted` / `SagaFailed` so subscribers
// (renderer) can buffer-until-complete.
//
// Per-variant `saga_id` tagging on existing Command/Event variants
// is deferred to E.5; for E.1a, only the lifecycle events carry
// `saga_id`. This keeps the wire change small and lets E.1a ship
// before any saga consumer exists to validate the field's use.
//
// Saga state is in-memory. Launcher restart abandons in-flight
// sagas; renderer-side timeouts cover the visible consequence.

use std::sync::Arc;

use agentmux_common::ipc::{Command, Event};

/// Where a `SagaAction::IssueCmd` should be dispatched.
///
/// `LauncherSelf` means "feed this command to the launcher's own
/// reducer" (in-process); `Host` and `Srv` mean "forward to the
/// peer's pipe." Phase E.5 adds the actual dispatch wiring; E.1a
/// stops at the SagaAction enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // pub variants used by Phase E.5 sagas
pub enum PipeTarget {
    LauncherSelf,
    Host,
    Srv,
}

/// What a saga decides to do next, returned from `start` /
/// `on_event`. The coordinator drives the saga forward by reacting
/// to this enum.
#[derive(Debug)]
#[allow(dead_code)] // variants used by Phase E.5 sagas
pub enum SagaAction {
    /// Dispatch a command on the target pipe; saga remains in flight.
    IssueCmd {
        target: PipeTarget,
        cmd: Command,
    },
    /// Saga succeeded. Coordinator removes it from `in_flight`
    /// and emits `Event::SagaCompleted`.
    Done,
    /// Saga failed irrecoverably. Coordinator emits `Event::SagaFailed`
    /// after dispatching any compensation IssueCmds the saga issued
    /// before returning Failed.
    Failed { reason: String },
    /// Saga is waiting for an event it hasn't seen yet. No-op for
    /// the coordinator until the next bus event.
    Wait,
}

/// Read-only context passed to saga callbacks. Currently unused
/// (sagas don't need launcher state); kept for forward-compatibility
/// so `Saga` impls don't need to change shape when E.5 needs to
/// expose state to sagas.
#[allow(dead_code)] // pub fields used by Phase E.5 sagas
#[derive(Debug, Clone, Copy)]
pub struct SagaCtx {
    pub saga_id: u64,
}

/// A multi-step, multi-reducer state machine. Implementations
/// describe one logical operation (tear-off, pool-respawn, etc.).
///
/// Lifecycle: coordinator calls `start` once when the saga is
/// added to `in_flight`. After that, every event on the bus is
/// passed to `on_event`. The saga inspects the event, advances
/// its internal state, returns the next action.
///
/// **Identification:** sagas know which events belong to them by
/// inspecting saga_id in lifecycle events (E.1a) or by matching
/// patterns in event payloads (e.g. specific labels — E.5 sagas).
/// Per-variant `saga_id` tagging on commands/events is E.5 work.
#[allow(dead_code)] // trait used by Phase E.5 sagas
pub trait Saga: Send {
    fn start(&mut self, ctx: &SagaCtx) -> SagaAction;
    fn on_event(&mut self, event: &Event, ctx: &SagaCtx) -> SagaAction;
    fn name(&self) -> &'static str;
}

/// Saga coordinator task.
///
/// Owns the registry of in-flight sagas, allocates saga ids,
/// routes events to sagas, dispatches IssueCmd actions to the
/// appropriate pipes, and emits lifecycle events on the broadcast
/// bus.
///
/// E.1a ships the type and the bus-subscription loop; the
/// in-flight registry is empty (no sagas yet). E.5 adds the
/// `start_saga` API and the dispatch wiring.
#[allow(dead_code)] // fields populated by Phase E.5
pub struct SagaCoordinator {
    /// Monotonic saga-id allocator.
    next_saga_id: std::sync::atomic::AtomicU64,
    /// In-flight sagas keyed by saga_id. E.1a: always empty.
    in_flight: tokio::sync::Mutex<std::collections::HashMap<u64, Box<dyn Saga>>>,
    /// Reference to the broadcast bus so the coordinator can emit
    /// `SagaStarted` / `SagaCompleted` / `SagaFailed`.
    events_tx: tokio::sync::broadcast::Sender<Event>,
    /// Reference to the launcher's reducer state for `bump_version`
    /// when emitting saga lifecycle events.
    state: Arc<tokio::sync::Mutex<crate::state::State>>,
}

impl SagaCoordinator {
    pub fn new(
        events_tx: tokio::sync::broadcast::Sender<Event>,
        state: Arc<tokio::sync::Mutex<crate::state::State>>,
    ) -> Self {
        Self {
            next_saga_id: std::sync::atomic::AtomicU64::new(1),
            in_flight: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            events_tx,
            state,
        }
    }

    /// Allocate the next saga_id. Public for E.5's saga-spawning
    /// callers; E.1a holds it unused.
    #[allow(dead_code)]
    pub fn next_id(&self) -> u64 {
        self.next_saga_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }
}

/// Run the coordinator's bus-subscription loop. E.1a: observes
/// every event, holds the registry empty (no sagas to drive).
/// E.5+: routes events to sagas, dispatches IssueCmd actions,
/// emits lifecycle events.
///
/// **Receiver is passed in, not subscribed inside.** Subscribing
/// before `tokio::spawn` (in `main.rs`) ensures events emitted
/// between coordinator construction and the first `recv()` aren't
/// lost to the race window. Same pattern as `event_log::run_disk_writer`.
/// (reagent P2 PR #609 — preempt E.5 saga drops once real consumers
/// land.)
pub async fn run_coordinator(
    coord: Arc<SagaCoordinator>,
    mut events_rx: tokio::sync::broadcast::Receiver<Event>,
) {
    let _ = coord; // E.1a: registry empty; coord ref held for E.5+
    crate::log("[saga] coordinator started (no in-flight sagas — E.1a is framework-only)");

    loop {
        match events_rx.recv().await {
            Ok(_event) => {
                // E.1a: observe but don't act. E.5 adds:
                //   - lookup sagas correlating to this event
                //   - call saga.on_event(&event, &ctx)
                //   - dispatch resulting SagaAction
                //   - on Done/Failed: emit lifecycle event, remove
                //     from in_flight
                //
                // Empty registry means there's nothing to drive
                // forward; the loop exists so the subscription is
                // active (otherwise the coordinator wouldn't be
                // observable in `--diag`).
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                crate::log(&format!("[saga] coordinator lagged, missed {} events", n));
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                crate::log("[saga] coordinator stopping (bus closed)");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentmux_common::ipc::{ClientKind, LifecyclePhase};

    /// Smoke test: coordinator subscribes to the bus and drains
    /// events without panicking. E.1a doesn't have sagas to test
    /// state-machine logic against; E.5 adds those.
    #[tokio::test]
    async fn coordinator_drains_bus_without_panic() {
        let (events_tx, _rx) = tokio::sync::broadcast::channel::<Event>(64);
        let state = Arc::new(tokio::sync::Mutex::new(crate::state::State::default()));
        let coord = Arc::new(SagaCoordinator::new(events_tx.clone(), Arc::clone(&state)));
        // Subscribe BEFORE spawn (per reagent P2 PR #609) so the
        // pattern in `main.rs` is exercised here too.
        let coord_rx = events_tx.subscribe();
        let handle = tokio::spawn(run_coordinator(Arc::clone(&coord), coord_rx));

        // Push a few events; coordinator should observe them
        // without acting (no sagas in flight).
        for v in 1..=5 {
            let _ = events_tx.send(Event::LifecyclePhaseChanged {
                from: LifecyclePhase::Starting,
                to: LifecyclePhase::Running,
                version: v,
            });
        }
        // Push a non-lifecycle event too.
        let _ = events_tx.send(Event::ProcessSpawned {
            pid: 42,
            kind: ClientKind::Tool,
            client_version: "test".into(),
            version: 6,
        });

        // Brief delay so the coordinator's recv loop drains.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        // Drop the sender; coordinator should observe Closed and exit.
        drop(events_tx);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), handle).await;
    }

    #[test]
    fn next_id_is_monotonic() {
        let (events_tx, _) = tokio::sync::broadcast::channel::<Event>(64);
        let state = Arc::new(tokio::sync::Mutex::new(crate::state::State::default()));
        let coord = SagaCoordinator::new(events_tx, state);
        let a = coord.next_id();
        let b = coord.next_id();
        let c = coord.next_id();
        assert!(a < b);
        assert!(b < c);
    }
}
