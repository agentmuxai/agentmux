// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.1a — saga coordinator infrastructure.
// Phase F.5 — first concrete saga consumer (`pool_respawn`).
//
// A saga is a state machine that orchestrates a multi-step,
// multi-reducer flow (e.g. tear-off touches host pool + srv
// workspace + launcher window registration). Sagas exist where
// per-flow correctness needs explicit coordination — distributed
// subscriber callbacks aren't enough.
//
// E.1a shipped this module as framework-only (no real sagas).
// **F.5 lights up the first concrete saga (`pool_respawn`) plus the
// minimal coordinator wiring needed to drive it**: the bus-
// subscription loop now starts sagas in response to trigger events,
// routes subsequent bus events into in-flight sagas via
// `Saga::on_event`, dispatches `SagaAction` results, and emits
// `SagaStarted` / `SagaCompleted` / `SagaFailed` brackets.
//
// Design (per `docs/specs/SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` §7
// + `docs/specs/SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md` §7.1):
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
// routes events to in-flight sagas, dispatches IssueCmd actions to
// the appropriate pipe, and emits `SagaStarted` / `SagaCompleted`
// / `SagaFailed` so subscribers (renderer) can buffer-until-complete.
//
// **Cross-process dispatch — F.5 caveat.** Today the launcher's
// named-pipe IPC is host→launcher only (host sends Commands up;
// launcher broadcasts Events back). A launcher→host command pipe
// doesn't exist yet. F.5's `pool_respawn` saga issues
// `Command::SpawnPoolWindow` with `target = PipeTarget::Host`, but
// the coordinator's IssueCmd handler currently logs the dispatch
// without transmitting it — the host's existing implicit
// `spawn_pool_window` call inside `promote_pool_window` produces
// the matching `Event::PoolWindowAdded` the saga waits for. F.6
// (or the cross-process-dispatch follow-up) replaces the log with
// a real wire-level send. See `pool_respawn.rs` module docstring
// for the full rationale and scope decision.
//
// Per-variant `saga_id` tagging on existing Command/Event variants
// is deferred; F.5's coordinator doesn't yet need it because:
//   - only one `pool_respawn` saga can be in flight per promote
//     (sagas are dispatched on `PoolWindowPromoted` 1:1);
//   - `Event::PoolWindowAdded` from the *implicit* refill is the
//     unique terminal signal — no foreign event type collides.
// When concurrent promotes can happen (and produce overlapping
// `PoolWindowAdded` streams), a future PR adds saga_id correlation.
//
// Saga state is in-memory. Launcher restart abandons in-flight
// sagas; renderer-side timeouts cover the visible consequence.

use std::sync::Arc;

use agentmux_common::ipc::{Command, Event};

pub mod pool_respawn;

/// Where a `SagaAction::IssueCmd` should be dispatched.
///
/// `LauncherSelf` means "feed this command to the launcher's own
/// reducer" (in-process); `Host` and `Srv` mean "forward to the
/// peer's pipe."
///
/// **F.5 status:** `Host` is wired only as a log target — the
/// launcher→host command pipe doesn't exist yet. Follow-up PRs
/// replace the log with the actual transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // pub variants used by sagas
pub enum PipeTarget {
    LauncherSelf,
    Host,
    Srv,
}

/// What a saga decides to do next, returned from `start` /
/// `on_event`. The coordinator drives the saga forward by reacting
/// to this enum.
#[derive(Debug)]
#[allow(dead_code)] // variants used by sagas
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

/// Read-only context passed to saga callbacks. Currently carries
/// only the saga_id; kept as a struct (rather than a bare u64) so
/// future fields can be added without touching every `Saga` impl.
#[allow(dead_code)] // pub fields used by sagas
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
/// inspecting saga_id in lifecycle events or by matching patterns
/// in event payloads (e.g. specific labels). F.5 sagas correlate
/// by event type only (`pool_respawn` matches any
/// `PoolWindowAdded` after start). Per-variant `saga_id` tagging
/// on commands/events is deferred until concurrent same-type
/// sagas are needed.
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
/// F.5 adds the first real consumer (`pool_respawn`). The
/// `in_flight` registry is now actually populated; the bus loop
/// dispatches events into it.
pub struct SagaCoordinator {
    /// Monotonic saga-id allocator.
    next_saga_id: std::sync::atomic::AtomicU64,
    /// In-flight sagas keyed by saga_id.
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

    /// Allocate the next saga_id. Monotonic per launcher run.
    pub fn next_id(&self) -> u64 {
        self.next_saga_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    /// Bump the launcher's `event_version` for a coordinator-emitted
    /// lifecycle event. Brief mutex hold — no I/O between lock and
    /// drop.
    async fn next_event_version(&self) -> u64 {
        let mut state = self.state.lock().await;
        state.bump_version()
    }

    /// Emit `Event::SagaStarted` after registering a saga.
    async fn emit_started(&self, saga_id: u64, name: &'static str) {
        let v = self.next_event_version().await;
        let _ = self.events_tx.send(Event::SagaStarted {
            saga_id,
            name: name.to_string(),
            version: v,
        });
    }

    /// Emit `Event::SagaCompleted` after a saga returns `Done`.
    async fn emit_completed(&self, saga_id: u64) {
        let v = self.next_event_version().await;
        let _ = self.events_tx.send(Event::SagaCompleted {
            saga_id,
            version: v,
        });
    }

    /// Emit `Event::SagaFailed` after a saga returns `Failed`.
    #[allow(dead_code)] // F.5 sagas don't fail; future sagas will.
    async fn emit_failed(&self, saga_id: u64, reason: String) {
        let v = self.next_event_version().await;
        let _ = self.events_tx.send(Event::SagaFailed {
            saga_id,
            reason,
            version: v,
        });
    }

    /// Apply a `SagaAction` returned by `start` or `on_event`. Returns
    /// `true` if the saga remains in flight (caller keeps it in
    /// `in_flight`); `false` if it terminated (caller removes it).
    ///
    /// **F.5 IssueCmd dispatch is logged-only for `Host` target.** No
    /// launcher→host command pipe exists yet; the saga relies on the
    /// host's existing implicit `spawn_pool_window` to produce the
    /// terminal `PoolWindowAdded`. `LauncherSelf` and `Srv` targets
    /// also fall back to log-only for now (no sagas use them yet —
    /// future work).
    async fn apply_action(&self, saga_id: u64, name: &'static str, action: SagaAction) -> bool {
        match action {
            SagaAction::IssueCmd { target, cmd } => {
                crate::log(&format!(
                    "[saga] saga_id={} name={} IssueCmd target={:?} cmd={:?} (F.5: dispatch is log-only; awaiting cross-process pipe)",
                    saga_id, name, target, cmd
                ));
                true
            }
            SagaAction::Wait => true,
            SagaAction::Done => {
                crate::log(&format!(
                    "[saga] saga_id={} name={} Done — emitting SagaCompleted",
                    saga_id, name
                ));
                self.emit_completed(saga_id).await;
                false
            }
            SagaAction::Failed { reason } => {
                crate::log(&format!(
                    "[saga] saga_id={} name={} Failed reason={} — emitting SagaFailed",
                    saga_id, name, reason
                ));
                self.emit_failed(saga_id, reason).await;
                false
            }
        }
    }

    /// Register a fresh saga, calling `start` and applying its first
    /// action. The caller has already determined that the saga
    /// should fire (e.g. matched a trigger event). Returns the saga's
    /// allocated id (logged + bracketed in `SagaStarted`).
    async fn spawn_saga(&self, mut saga: Box<dyn Saga>) -> u64 {
        let saga_id = self.next_id();
        let name = saga.name();
        crate::log(&format!(
            "[saga] starting saga_id={} name={}",
            saga_id, name
        ));
        // Emit SagaStarted FIRST so any subscriber buffering by
        // saga_id sees the bracket open before any per-step events.
        // Mirrors `agentmux-srv::sagas::emit_saga_started` ordering.
        self.emit_started(saga_id, name).await;
        let ctx = SagaCtx { saga_id };
        let action = saga.start(&ctx);
        let in_flight = self.apply_action(saga_id, name, action).await;
        if in_flight {
            self.in_flight.lock().await.insert(saga_id, saga);
        }
        saga_id
    }
}

/// Inspect a bus event for "should this start a fresh saga?" Returns
/// the constructed saga (boxed) on a hit; `None` otherwise.
///
/// F.5 wires one trigger: `Event::PoolWindowPromoted` →
/// `pool_respawn::PoolRespawn`. Future sagas extend this match.
fn match_trigger(event: &Event) -> Option<Box<dyn Saga>> {
    match event {
        Event::PoolWindowPromoted { label, .. } => {
            Some(Box::new(pool_respawn::PoolRespawn::new(label.clone())))
        }
        _ => None,
    }
}

/// Run the coordinator's bus-subscription loop.
///
/// **Receiver is passed in, not subscribed inside.** Subscribing
/// before `tokio::spawn` (in `main.rs`) ensures events emitted
/// between coordinator construction and the first `recv()` aren't
/// lost to the race window. Same pattern as `event_log::run_disk_writer`.
/// (reagent P2 PR #609.)
///
/// Dispatch order on each event:
///   1. Match against trigger table → start any new sagas via
///      `spawn_saga` (which emits `SagaStarted` and applies the
///      saga's initial action).
///   2. Feed the same event into every in-flight saga's `on_event`.
///      Sagas returning `Done` / `Failed` are removed from
///      `in_flight` after their lifecycle event is emitted.
///
/// **Self-emitted events (`SagaStarted` / `SagaCompleted` /
/// `SagaFailed`) are NOT re-fed into sagas.** A saga reacting to its
/// own start event would loop. Filtered at the top of the dispatch
/// path.
pub async fn run_coordinator(
    coord: Arc<SagaCoordinator>,
    mut events_rx: tokio::sync::broadcast::Receiver<Event>,
) {
    crate::log("[saga] coordinator started");

    loop {
        match events_rx.recv().await {
            Ok(event) => {
                // Skip our own lifecycle events to avoid loops.
                if matches!(
                    event,
                    Event::SagaStarted { .. } | Event::SagaCompleted { .. } | Event::SagaFailed { .. }
                ) {
                    continue;
                }

                // Step 1 — start any new sagas this event triggers.
                if let Some(saga) = match_trigger(&event) {
                    coord.spawn_saga(saga).await;
                }

                // Step 2 — feed the event into every in-flight saga.
                // Two-pass to avoid holding the registry lock across
                // `apply_action` (which itself locks state to bump
                // version when emitting SagaCompleted/Failed).
                let actions: Vec<(u64, &'static str, SagaAction)> = {
                    let mut in_flight = coord.in_flight.lock().await;
                    let mut out = Vec::new();
                    for (saga_id, saga) in in_flight.iter_mut() {
                        let ctx = SagaCtx { saga_id: *saga_id };
                        let action = saga.on_event(&event, &ctx);
                        out.push((*saga_id, saga.name(), action));
                    }
                    out
                };
                for (saga_id, name, action) in actions {
                    let still_in_flight = coord.apply_action(saga_id, name, action).await;
                    if !still_in_flight {
                        coord.in_flight.lock().await.remove(&saga_id);
                    }
                }
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

    /// Smoke test from E.1a, retained: coordinator drains unrelated
    /// events without panicking. F.5 expanded the loop body but the
    /// invariant — non-trigger events are no-ops — still holds.
    #[tokio::test]
    async fn coordinator_drains_unrelated_events_without_panic() {
        let (events_tx, _rx) = tokio::sync::broadcast::channel::<Event>(64);
        let state = Arc::new(tokio::sync::Mutex::new(crate::state::State::default()));
        let coord = Arc::new(SagaCoordinator::new(events_tx.clone(), Arc::clone(&state)));
        let coord_rx = events_tx.subscribe();
        let handle = tokio::spawn(run_coordinator(Arc::clone(&coord), coord_rx));

        for v in 1..=5 {
            let _ = events_tx.send(Event::LifecyclePhaseChanged {
                from: LifecyclePhase::Starting,
                to: LifecyclePhase::Running,
                version: v,
            });
        }
        let _ = events_tx.send(Event::ProcessSpawned {
            pid: 42,
            kind: ClientKind::Tool,
            client_version: "test".into(),
            version: 6,
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        // No sagas should be in flight (no trigger events).
        assert!(coord.in_flight.lock().await.is_empty());
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

    /// F.5 — verify the trigger table picks up `PoolWindowPromoted`.
    /// The end-to-end coordinator test lives in
    /// `pool_respawn::tests::coordinator_brackets_promote_with_saga_lifecycle_events`
    /// where the pool-respawn saga's expected behavior is asserted.
    #[test]
    fn match_trigger_pool_window_promoted_starts_pool_respawn() {
        let event = Event::PoolWindowPromoted {
            label: "window-pool-abc".into(),
            version: 1,
        };
        let saga = match_trigger(&event).expect("PoolWindowPromoted should trigger a saga");
        assert_eq!(saga.name(), "pool_respawn_on_promote");
    }

    #[test]
    fn match_trigger_returns_none_for_non_trigger_events() {
        let cases = vec![
            Event::PoolWindowAdded {
                label: "window-pool-abc".into(),
                version: 1,
            },
            Event::PoolWindowRemoved {
                label: "window-pool-abc".into(),
                version: 1,
            },
            Event::WindowOpened {
                label: "main".into(),
                kind: agentmux_common::ipc::WindowKind::FullInstance,
                parent_label: None,
                version: 1,
            },
            Event::LifecyclePhaseChanged {
                from: LifecyclePhase::Starting,
                to: LifecyclePhase::Running,
                version: 1,
            },
        ];
        for event in cases {
            assert!(
                match_trigger(&event).is_none(),
                "non-trigger event spawned a saga: {:?}",
                event
            );
        }
    }

    /// Saga lifecycle events on the bus must not feed back into
    /// sagas (would cause loops). Sanity-check the filter directly.
    #[tokio::test]
    async fn coordinator_does_not_self_trigger_on_lifecycle_events() {
        let (events_tx, _) = tokio::sync::broadcast::channel::<Event>(64);
        let state = Arc::new(tokio::sync::Mutex::new(crate::state::State::default()));
        let coord = Arc::new(SagaCoordinator::new(events_tx.clone(), Arc::clone(&state)));
        let coord_rx = events_tx.subscribe();
        let handle = tokio::spawn(run_coordinator(Arc::clone(&coord), coord_rx));
        tokio::task::yield_now().await;

        // Push a SagaStarted-shaped event; if the coordinator
        // re-spawned, it would eventually exhaust the saga_id atomic
        // by looping. We verify simpler: in_flight stays empty.
        let _ = events_tx.send(Event::SagaStarted {
            saga_id: 999,
            name: "foreign_saga".into(),
            version: 1,
        });
        let _ = events_tx.send(Event::SagaCompleted {
            saga_id: 999,
            version: 2,
        });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(coord.in_flight.lock().await.is_empty());
        drop(events_tx);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), handle).await;
    }
}
