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
pub mod window_cleanup;

// LSD-1 (PR LSD-1) — durable launcher saga log + API. Foundations
// only: the coordinator does NOT call any of these methods yet.
// Module is declared here so it compiles + tests run; PR LSD-2 wires
// the coordinator to write through `LauncherSagaLog` on every state
// transition. See `docs/specs/SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md`
// §4 PR1 for the staged-rollout rationale.
mod log;
#[allow(unused_imports)] // re-export consumed by PR LSD-2; keeps import path stable.
pub use log::LauncherSagaLog;

/// Where a `SagaAction::IssueCmd` should be dispatched.
///
/// `LauncherSelf` means "feed this command to the launcher's own
/// reducer" (in-process); `Host` and `Srv` mean "forward to the
/// peer's pipe."
///
/// **F.5 status:** `Host` is wired only as a log target — the
/// launcher→host command pipe doesn't exist yet. Follow-up PRs
/// replace the log with the actual transport.
///
/// F.7 cleanup audit: only `Host` is constructed today (F.5/F.6 saga
/// IssueCmds). `LauncherSelf` and `Srv` are framework slots reserved
/// for the cross-process dispatch follow-up phase per
/// `SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md` §4.3. Allow stays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // LauncherSelf + Srv reserved for future cross-process sagas
pub enum PipeTarget {
    LauncherSelf,
    Host,
    Srv,
}

/// What a saga decides to do next, returned from `start` /
/// `on_event`. The coordinator drives the saga forward by reacting
/// to this enum.
///
/// F.7 cleanup audit: `IssueCmd`, `Done`, `Wait` are all constructed
/// by F.5 + F.6 sagas. `Failed` is consumed in `apply_action` (and
/// its corresponding `emit_failed` is now actively called by the
/// evict-and-replace path), but no shipped saga *constructs*
/// `Failed` yet — sagas today only succeed or wait. Variant-level
/// allow scopes the dead-code suppression precisely to that one
/// reserved variant.
#[derive(Debug)]
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
    /// before returning Failed. Reserved for sagas with explicit
    /// failure conditions (e.g. cross-process dispatch follow-up
    /// + saga timeouts).
    #[allow(dead_code)] // reserved for sagas that explicitly fail; F.5/F.6 only succeed or wait
    Failed { reason: String },
    /// Saga is waiting for an event it hasn't seen yet. No-op for
    /// the coordinator until the next bus event.
    Wait,
}

/// Read-only context passed to saga callbacks. Currently carries
/// only the saga_id; kept as a struct (rather than a bare u64) so
/// future fields can be added without touching every `Saga` impl.
///
/// F.7 cleanup audit: `saga_id` is unread by the shipped sagas
/// (F.5/F.6 don't need it — coordinator already routes events
/// per-registry). Reserved for the per-event saga_id correlation
/// follow-up; allow stays.
#[allow(dead_code)] // saga_id reserved for per-event correlation follow-up
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

    /// Emit `Event::SagaFailed` after a saga returns `Failed` or
    /// after evict-and-replace cancels a same-kind in-flight saga.
    /// F.7 cleanup audit: prior `#[allow(dead_code)]` removed — F.6
    /// evict-and-replace policy now actively dispatches this on
    /// every concurrent same-kind retrigger.
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
            let mut registry = self.in_flight.lock().await;
            // (codex P1 PR #634) Observability for the known
            // concurrent-correlation limitation. With more than one
            // saga of the same kind in flight, broadcast event
            // routing mis-correlates: the first matching event
            // completes ALL of them. Logged here so operators can
            // spot the pattern in `--diag wrr` output. Closed when
            // F.6/F.7 adds proper FIFO routing or saga-id event
            // correlation.
            let same_kind_count = registry
                .values()
                .filter(|s| s.name() == name)
                .count();
            if same_kind_count >= 1 {
                crate::log(&format!(
                    "[saga] WARN: starting {} saga_id={} while {} other(s) of same kind in flight; concurrent-correlation limitation may produce premature SagaCompleted events (PR #634 / codex P1 known issue)",
                    name, saga_id, same_kind_count,
                ));
            }
            registry.insert(saga_id, saga);
        }
        saga_id
    }
}

/// Inspect a bus event for "should this start a fresh saga?" Returns
/// the constructed saga (boxed) on a hit; `None` otherwise.
///
/// Triggers wired to date:
/// * F.5: `Event::PoolWindowPromoted` → `pool_respawn::PoolRespawn`
/// * F.6: `Event::WindowClosed` →
///   `window_cleanup::WindowCleanupCascade`
///
/// Future sagas extend this match.
///
/// Note: this returns a *candidate* saga. The coordinator may still
/// evict a same-kind in-flight saga via the evict-and-replace
/// serialization gate before `spawn_saga` (codex P1 PR #634 round 3
/// — see `run_coordinator`).
fn match_trigger(event: &Event) -> Option<Box<dyn Saga>> {
    match event {
        Event::PoolWindowPromoted { label, .. } => {
            Some(Box::new(pool_respawn::PoolRespawn::new(label.clone())))
        }
        // (codex P1 PR #637.) Only fire the cleanup cascade on
        // CLEAN closes. `Event::WindowClosed { crash_detected: true }`
        // comes from `wrr::apply_hwnd_destroyed` after a host/renderer
        // crash; the host never sent `ReportPanesReaped` /
        // `ReportPoolDrainDecision`, so the saga would stay
        // in-flight indefinitely (only cleared by a later same-kind
        // eviction or launcher restart) leaving a SagaStarted
        // bracket dangling for subscribers that buffer on lifecycle.
        Event::WindowClosed { label, crash_detected: false, .. } => Some(Box::new(
            window_cleanup::WindowCleanupCascade::new(label.clone()),
        )),
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
                // (codex P1 PR #634 round 3.) Evict-and-replace: if a
                // saga of the same kind is already in flight, EVICT
                // it (mark Failed + remove from registry) and start
                // the new one. Round 2's silent-drop fix had a
                // permanent-deadlock failure mode: a stalled saga
                // (refill never arrived) would block all future
                // promote sagas for the rest of the process.
                //
                // Trade-offs of evict-and-replace:
                // - Pros: no permanent block; each new promote gets
                //   a fresh bracket; reducer + SQLite stay correct.
                // - Cons: when both promotes are healthy in quick
                //   succession, the first promote's bracket is cut
                //   short with SagaFailed (the late refill event for
                //   that promote completes the new saga instead).
                //   Renderer-visible: one premature SagaFailed +
                //   one correct SagaCompleted.
                // Proper FIFO routing / per-event saga_id correlation
                // ships in F.6/F.7 alongside cross-process dispatch.
                if let Some(saga) = match_trigger(&event) {
                    let new_kind = saga.name();
                    let evict_ids: Vec<u64> = {
                        let registry = coord.in_flight.lock().await;
                        registry
                            .iter()
                            .filter(|(_, s)| s.name() == new_kind)
                            .map(|(id, _)| *id)
                            .collect()
                    };
                    for evict_id in evict_ids {
                        crate::log(&format!(
                            "[saga] evicting prior {} saga_id={} to make room for new trigger (codex P1 #634 round 3 evict-and-replace)",
                            new_kind, evict_id,
                        ));
                        coord
                            .emit_failed(
                                evict_id,
                                "evicted: same-kind saga restarted (codex P1 #634 round 3)".to_string(),
                            )
                            .await;
                        coord.in_flight.lock().await.remove(&evict_id);
                    }
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

    /// F.6 — verify the trigger table picks up `WindowClosed`. The
    /// end-to-end coordinator test lives in
    /// `window_cleanup::tests::coordinator_brackets_close_with_saga_lifecycle_events`.
    #[test]
    fn match_trigger_window_closed_starts_window_cleanup_cascade() {
        let event = Event::WindowClosed {
            label: "main".into(),
            version: 1,
            crash_detected: false,
        };
        let saga = match_trigger(&event).expect("WindowClosed should trigger a saga");
        assert_eq!(saga.name(), "window_cleanup_cascade");
    }

    /// (codex P1 PR #637 round 2.) Crash-detected closes (originating
    /// from `wrr::apply_hwnd_destroyed`) must NOT trigger the saga —
    /// the host never sent the cleanup reports, so the saga would
    /// stay in-flight forever.
    #[test]
    fn match_trigger_skips_crash_detected_window_closed() {
        let event = Event::WindowClosed {
            label: "crashed-window".into(),
            version: 1,
            crash_detected: true,
        };
        assert!(
            match_trigger(&event).is_none(),
            "crash-detected close should NOT spawn a saga",
        );
    }

    #[test]
    fn match_trigger_returns_none_for_non_trigger_events() {
        let cases = vec![
            Event::PoolWindowAdded {
                label: "window-pool-abc".into(),
                version: 1,
                saga_id: None,
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
            // F.6 step-1/step-2 terminal events are NOT triggers
            // themselves — the saga consumes them, but receiving one
            // when no saga is in flight should be a no-op.
            Event::PanesReaped {
                label: "main".into(),
                version: 1,
                saga_id: None,
            },
            Event::PoolDrained {
                label: "main".into(),
                version: 1,
                saga_id: None,
            },
            Event::PoolNotLast {
                label: "main".into(),
                version: 1,
                saga_id: None,
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

    // ---------- Phase F.7 — saga-lifecycle property tests ----------
    //
    // Random sequences of `WindowClosed` / `PoolWindowPromoted`
    // events fed through `match_trigger`. Asserts:
    //   1. `WindowClosed { crash_detected: true }` NEVER produces a
    //      saga (F.6 round-2 gate, codex P1 #637).
    //   2. `WindowClosed { crash_detected: false }` ALWAYS produces
    //      exactly one window_cleanup_cascade saga.
    //   3. `PoolWindowPromoted` ALWAYS produces exactly one
    //      pool_respawn_on_promote saga.
    //   4. The saga's name (the (kind, key) tuple's "kind" component)
    //      is one of the two known names — no foreign saga slips
    //      through the trigger table.
    //
    // The end-to-end coordinator-level "exactly one terminal lifecycle
    // event per saga" assertion lives in
    // `pool_respawn::tests::coordinator_brackets_promote_with_saga_lifecycle_events`
    // and `window_cleanup::tests::coordinator_brackets_close_with_saga_lifecycle_events`
    // (already shipped in F.5 + F.6). The proptests here add the
    // randomized-input dimension those happy-path tests don't cover.
    //
    // Cases capped at 64 (default 1024 too slow for CI). Same bound
    // the srv reducer's E.7 proptest uses.

    use proptest::prelude::*;

    /// Trigger event variants the F.5/F.6 sagas can be triggered by,
    /// plus crash-detected close (the negative case).
    #[derive(Debug, Clone)]
    enum F7TriggerEvent {
        /// Should spawn `window_cleanup_cascade`.
        WindowClosedClean { label: String },
        /// Should NOT spawn anything (F.6 round-2 gate).
        WindowClosedCrashed { label: String },
        /// Should spawn `pool_respawn_on_promote`.
        PoolWindowPromoted { label: String },
        /// A non-trigger event — must produce no saga.
        Unrelated,
    }

    fn f7_trigger_strategy() -> impl Strategy<Value = F7TriggerEvent> {
        prop_oneof![
            3 => "[a-c]{1,3}".prop_map(|label| F7TriggerEvent::WindowClosedClean { label }),
            1 => "[a-c]{1,3}".prop_map(|label| F7TriggerEvent::WindowClosedCrashed { label }),
            3 => "[a-c]{1,3}".prop_map(|label| F7TriggerEvent::PoolWindowPromoted { label }),
            2 => Just(F7TriggerEvent::Unrelated),
        ]
    }

    fn make_event(t: F7TriggerEvent, version: u64) -> Event {
        match t {
            F7TriggerEvent::WindowClosedClean { label } => Event::WindowClosed {
                label,
                version,
                crash_detected: false,
            },
            F7TriggerEvent::WindowClosedCrashed { label } => Event::WindowClosed {
                label,
                version,
                crash_detected: true,
            },
            F7TriggerEvent::PoolWindowPromoted { label } => {
                Event::PoolWindowPromoted { label, version }
            }
            F7TriggerEvent::Unrelated => Event::Pong { nonce: 0, version },
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 64,
            ..ProptestConfig::default()
        })]

        /// For every event in a random sequence, `match_trigger`
        /// returns the expected saga (or None for crash / unrelated).
        /// Exhaustive shape check — catches accidental regressions
        /// in the trigger table.
        #[test]
        fn f7_match_trigger_matches_expected_saga_kind(
            triggers in prop::collection::vec(f7_trigger_strategy(), 0..40)
        ) {
            for (i, t) in triggers.into_iter().enumerate() {
                let event = make_event(t.clone(), (i + 1) as u64);
                let saga = match_trigger(&event);
                match (&t, &saga) {
                    (F7TriggerEvent::WindowClosedClean { .. }, Some(s)) => {
                        prop_assert_eq!(s.name(), "window_cleanup_cascade");
                    }
                    (F7TriggerEvent::PoolWindowPromoted { .. }, Some(s)) => {
                        prop_assert_eq!(s.name(), "pool_respawn_on_promote");
                    }
                    (F7TriggerEvent::WindowClosedCrashed { .. }, None) => {
                        // Expected — crash-detected close must never
                        // spawn a saga (F.6 codex P1 #637).
                    }
                    (F7TriggerEvent::Unrelated, None) => {
                        // Expected — non-trigger event.
                    }
                    (other_t, other_s) => {
                        prop_assert!(
                            false,
                            "trigger {:?} produced unexpected saga match: {:?}",
                            other_t,
                            other_s.as_ref().map(|s| s.name()),
                        );
                    }
                }
            }
        }

        /// Crash-detected `WindowClosed` events NEVER produce a
        /// saga, regardless of label. Reinforces invariant #1
        /// independently — a stricter shrinker target.
        #[test]
        fn f7_crash_detected_close_never_spawns_saga(
            label in "[a-c]{1,3}",
            version in 1u64..1_000_000,
        ) {
            let event = Event::WindowClosed { label, version, crash_detected: true };
            prop_assert!(
                match_trigger(&event).is_none(),
                "crash-detected WindowClosed must not spawn a saga",
            );
        }

        /// Clean-close `WindowClosed` ALWAYS produces a
        /// window_cleanup_cascade saga, regardless of label.
        #[test]
        fn f7_clean_close_always_spawns_window_cleanup_cascade(
            label in "[a-c]{1,3}",
            version in 1u64..1_000_000,
        ) {
            let event = Event::WindowClosed { label, version, crash_detected: false };
            let saga = match_trigger(&event)
                .expect("clean-close should spawn a saga");
            prop_assert_eq!(saga.name(), "window_cleanup_cascade");
        }
    }

    /// Coordinator-level invariant: under random sequences of
    /// trigger events, no two same-kind sagas are ever active
    /// simultaneously — the evict-and-replace policy guarantees at
    /// most one in-flight saga per kind. Drives the coordinator with
    /// synthetic events and asserts the `in_flight` registry has at
    /// most one entry per name (≤2 total: pool_respawn +
    /// window_cleanup).
    ///
    /// Run as a single tokio test (proptest can't drive an async
    /// closure directly). Iterates several seeded sequences inline —
    /// enough surface area to catch concurrent-overlap regressions
    /// without ballooning test time. Uses a hand-rolled LCG so we
    /// don't pull in `rand` as a dev-dependency for one test.
    #[tokio::test]
    async fn f7_evict_and_replace_keeps_one_saga_per_kind() {
        // Tiny LCG for deterministic per-seed sequences. Glibc's
        // constants — quality irrelevant; we just need reproducible
        // bit patterns across CI runs.
        fn lcg_next(state: &mut u64) -> u64 {
            *state = state
                .wrapping_mul(1_103_515_245)
                .wrapping_add(12_345);
            *state
        }

        let seeds: &[u64] = &[1, 7, 42, 99, 1234, 5678, 0xDEADBEEF, 0xC0FFEE];
        for seed in seeds {
            let (events_tx, _) = tokio::sync::broadcast::channel::<Event>(256);
            let state = Arc::new(tokio::sync::Mutex::new(crate::state::State::default()));
            let coord = Arc::new(SagaCoordinator::new(events_tx.clone(), Arc::clone(&state)));
            let coord_rx = events_tx.subscribe();
            let _handle = tokio::spawn(run_coordinator(Arc::clone(&coord), coord_rx));
            tokio::task::yield_now().await;

            // Build a deterministic sequence from the seed.
            let mut rng = *seed;
            let labels = ["a", "b", "c"];
            for v in 1u64..=20 {
                let label_idx = (lcg_next(&mut rng) % labels.len() as u64) as usize;
                let label = labels[label_idx].to_string();
                let pick = lcg_next(&mut rng) % 3;
                let event = match pick {
                    0 => Event::WindowClosed {
                        label,
                        version: v,
                        crash_detected: false,
                    },
                    1 => Event::PoolWindowPromoted { label, version: v },
                    _ => Event::Pong { nonce: v, version: v },
                };
                let _ = events_tx.send(event);
                // Yield so the coordinator processes one event before
                // we send the next. Tightens the concurrent-overlap
                // window.
                tokio::task::yield_now().await;
            }
            // Brief settling — give the coordinator time to process
            // the last few events + emit terminal lifecycles.
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;

            // Invariant: at most one saga per name in flight.
            let registry = coord.in_flight.lock().await;
            let mut counts = std::collections::HashMap::<&str, u32>::new();
            for s in registry.values() {
                *counts.entry(s.name()).or_insert(0) += 1;
            }
            for (name, count) in &counts {
                assert!(
                    *count <= 1,
                    "seed {} — saga kind {:?} has {} concurrent in-flight; evict-and-replace policy violated",
                    seed,
                    name,
                    count,
                );
            }
            // And the 2-kind ceiling (we only register two saga kinds).
            assert!(
                registry.len() <= 2,
                "seed {} — registry has {} sagas, expected ≤2 (one per kind)",
                seed,
                registry.len(),
            );
        }
    }

    /// Saga forward-path emission contract: every saga that
    /// terminates emits exactly ONE terminal lifecycle event
    /// (`SagaCompleted` xor `SagaFailed`). Drives a small batch of
    /// triggers through the coordinator and counts emitted
    /// lifecycle events on the bus. Catches regressions where a
    /// saga's `Done` action somehow fires both `emit_completed`
    /// and `emit_failed`, or neither.
    #[tokio::test]
    async fn f7_each_saga_emits_exactly_one_terminal_event() {
        let (events_tx, _) = tokio::sync::broadcast::channel::<Event>(256);
        let state = Arc::new(tokio::sync::Mutex::new(crate::state::State::default()));
        let coord = Arc::new(SagaCoordinator::new(events_tx.clone(), Arc::clone(&state)));
        let mut witness = events_tx.subscribe();
        let coord_rx = events_tx.subscribe();
        let _handle = tokio::spawn(run_coordinator(Arc::clone(&coord), coord_rx));
        tokio::task::yield_now().await;

        // Drive ONE clean cleanup-cascade saga to completion.
        let _ = events_tx.send(Event::WindowClosed {
            label: "main".into(),
            version: 1,
            crash_detected: false,
        });
        let _ = events_tx.send(Event::PanesReaped {
            label: "main".into(),
            version: 2,
            saga_id: None,
        });
        let _ = events_tx.send(Event::PoolDrained {
            label: "main".into(),
            version: 3,
            saga_id: None,
        });
        // And ONE pool-respawn saga to completion.
        let _ = events_tx.send(Event::PoolWindowPromoted {
            label: "pool-1".into(),
            version: 4,
        });
        let _ = events_tx.send(Event::PoolWindowAdded {
            label: "pool-2".into(),
            version: 5,
            saga_id: None,
        });

        // Drain the bus with a deadline. Count one terminal event
        // (SagaCompleted xor SagaFailed) per saga_id.
        let mut started_ids = std::collections::HashSet::<u64>::new();
        let mut terminal_for_id = std::collections::HashMap::<u64, &'static str>::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            match tokio::time::timeout(std::time::Duration::from_millis(50), witness.recv()).await
            {
                Ok(Ok(Event::SagaStarted { saga_id, .. })) => {
                    started_ids.insert(saga_id);
                }
                Ok(Ok(Event::SagaCompleted { saga_id, .. })) => {
                    let prev = terminal_for_id.insert(saga_id, "completed");
                    assert!(
                        prev.is_none(),
                        "saga_id {} got two terminal events: prior={:?} now=completed",
                        saga_id,
                        prev,
                    );
                }
                Ok(Ok(Event::SagaFailed { saga_id, .. })) => {
                    let prev = terminal_for_id.insert(saga_id, "failed");
                    assert!(
                        prev.is_none(),
                        "saga_id {} got two terminal events: prior={:?} now=failed",
                        saga_id,
                        prev,
                    );
                }
                Ok(Ok(_)) => {}
                Ok(Err(_)) => break,
                Err(_) => {
                    // No new events for 50ms — we've drained.
                    if started_ids.len() == 2
                        && started_ids.iter().all(|id| terminal_for_id.contains_key(id))
                    {
                        break;
                    }
                }
            }
        }
        // Both sagas started.
        assert_eq!(
            started_ids.len(),
            2,
            "expected 2 SagaStarted events (one per saga), got {}: {:?}",
            started_ids.len(),
            started_ids,
        );
        // Each got exactly one terminal event.
        for id in &started_ids {
            assert!(
                terminal_for_id.contains_key(id),
                "saga_id {} never got a terminal lifecycle event",
                id,
            );
        }
        // No spurious terminal events for sagas we didn't observe
        // start.
        for id in terminal_for_id.keys() {
            assert!(
                started_ids.contains(id),
                "saga_id {} got a terminal event but no SagaStarted observed",
                id,
            );
        }
    }
}
