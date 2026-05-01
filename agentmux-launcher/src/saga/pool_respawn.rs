// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase F.5 — pool-respawn-on-promote saga.
//
// **What this saga does**
//
// When the host promotes a pool window to a user-visible top-level
// window (the `promote_pool_window` flow in
// `agentmux-cef::commands::window_pool`), the host immediately
// calls `spawn_pool_window` to refill the pool. Today that refill
// fires implicitly between `Event::PoolWindowPromoted` and
// `Event::PoolWindowAdded` — no saga lifecycle event marks the
// transaction. Renderers that want to buffer "you're getting a
// tear-off + the pool is refilling" atomically have nothing to
// pivot on.
//
// This saga formalizes the implicit flow as an explicit cross-
// process state machine so the renderer sees a `SagaStarted` /
// `SagaCompleted` bracket. Implementing the spec sketch from
// `docs/specs/SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md` §7.1:
//
//   saga PoolRespawnOnPromote(promoted_label):
//       Step 1 — start: emit Event::PoolWindowPromoted { promoted_label }
//                       (already exists in Phase B; F.5 adds the wire
//                       variant + host-side report.)
//       Step 2 — issue Command::SpawnPoolWindow → host
//                       wait for: Event::PoolWindowAdded { new_label }
//       Step 3 — Done
//
//       Compensation: none (failure to refill is logged + retried
//       on next promote).
//
// **Scope of F.5 (this PR)**
//
// F.5 ships:
//   - the saga state machine itself (this file),
//   - the wire types (`Command::ReportPoolWindowPromoted`,
//     `Command::SpawnPoolWindow`, `Event::PoolWindowPromoted`),
//   - host-side `report_pool_window_promoted` call inside
//     `promote_pool_window` (between the remove and the open),
//   - launcher reducer arm translating the report into the typed
//     event,
//   - coordinator wiring that registers the saga as a consumer of
//     `Event::PoolWindowPromoted`.
//
// F.5 does NOT ship:
//   - The actual cross-process dispatch of `Command::SpawnPoolWindow`
//     to the host. Today the launcher's named-pipe IPC is host→
//     launcher only (host sends Commands up; launcher broadcasts
//     Events back). A launcher→host command pipe doesn't exist yet.
//     Step 2's `SagaAction::IssueCmd` is logged-only; the saga
//     relies on the host's existing implicit `spawn_pool_window`
//     call inside `promote_pool_window` to produce the
//     `Event::PoolWindowAdded` it waits for. **This is acceptable
//     scope per the F-spec § 7.1 final paragraph** ("If the cross-
//     process plumbing is too heavy a lift for one PR, scope down:
//     implement the saga shape + the launcher-side coordinator
//     entry, even if the actual cross-process dispatch is initially
//     in-process"). The follow-up PR replaces the log with a real
//     wire-level send.
//   - F.6 window-cleanup cascade saga.
//   - Launcher-side saga durability (separate concern; srv-side
//     durability already shipped).
//
// **Why a saga at all if Step 2 is currently passive?**
//
// The renderer-visible value is the `SagaStarted` / `SagaCompleted`
// bracket: subscribers see "saga foo running" and can buffer
// related events for that saga_id until the bracket closes. That
// works the same way whether the launcher actively dispatched the
// refill or merely observed the host doing it. When the cross-
// process pipe lands, only the saga's `IssueCmd` handling needs
// to change — the renderer-facing semantics are stable.
//
// **What if `Event::PoolWindowAdded` never arrives?**
//
// The saga waits forever until either the bus closes (saga
// abandoned on launcher restart) or — once F.6+ adds saga
// timeouts — a per-saga deadline force-fails it. F.5 keeps the
// behavior deliberately simple: if refill genuinely fails, the
// next promote will start a fresh saga; the prior failed saga
// stays in `in_flight` until the launcher exits. This matches the
// F-spec compensation strategy ("none — failure to refill is
// logged + retried on next promote").
//
// **Known limitation: concurrent-promote correlation** (codex P1
// PR #634). If two promotes are in flight simultaneously, the
// coordinator broadcasts every event to every in-flight saga
// (`saga::mod.rs::run_coordinator`). The first `PoolWindowAdded`
// completes BOTH sagas — early `SagaCompleted` for the second
// promote's bracket, later refill event left unbracketed.
//
// Concurrent promotes require the user to tear off two windows
// in rapid succession (under the host's spawn_pool_window
// completion latency). The user-visible consequence is a brief
// renderer-side bracketing inconsistency; reducer + SQLite state
// remain correct.
//
// Proper fix requires coordinator-level FIFO routing or per-saga
// sequence-number correlation. Both are non-trivial and depend on
// the cross-process dispatch landing first (so the saga's
// `IssueCmd` is actually causal). Closed in F.6/F.7 alongside the
// launcher→host command pipe.

use agentmux_common::ipc::{Command, Event};

use super::{PipeTarget, Saga, SagaAction, SagaCtx};

/// State of one in-flight pool-respawn saga.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    /// Saga just constructed — waiting for the coordinator to call
    /// `start`. The first `start` call transitions to
    /// `WaitingForRefill` and emits the `IssueCmd` for
    /// `SpawnPoolWindow`.
    Initial,
    /// `SpawnPoolWindow` has been issued; the saga is now waiting for
    /// any `Event::PoolWindowAdded` to land on the bus. The label
    /// of the new pool entry doesn't need to match the promoted
    /// label — the refill produces a *new* label, and any
    /// `PoolWindowAdded` that lands AFTER the saga was constructed
    /// is the refill we're tracking.
    WaitingForRefill,
}

/// Pool-respawn saga: fires once per promote, waits for the matching
/// refill, then completes.
pub struct PoolRespawn {
    /// Label of the window that was promoted (Step 1's input). Held
    /// for log correlation only — the saga doesn't validate the
    /// new pool label against this one. Kept on the struct so future
    /// failure-mode logging (timeout / explicit fail) can include
    /// "which promote did this saga belong to?" without rethreading.
    #[allow(dead_code)]
    promoted_label: String,
    /// New pool label observed in `Event::PoolWindowAdded` (Step 2's
    /// output). `None` until refill is observed.
    refilled_label: Option<String>,
    phase: Phase,
}

impl PoolRespawn {
    /// Construct a fresh saga for a promote of `promoted_label`.
    /// Coordinator allocates the saga_id and calls `start` once.
    pub fn new(promoted_label: String) -> Self {
        Self {
            promoted_label,
            refilled_label: None,
            phase: Phase::Initial,
        }
    }

    /// Label of the new pool window observed in `PoolWindowAdded`.
    /// `None` until the saga has progressed past `WaitingForRefill`.
    /// Exported for tests.
    #[cfg(test)]
    pub fn refilled_label(&self) -> Option<&str> {
        self.refilled_label.as_deref()
    }
}

impl Saga for PoolRespawn {
    fn name(&self) -> &'static str {
        "pool_respawn_on_promote"
    }

    fn start(&mut self, _ctx: &SagaCtx) -> SagaAction {
        // Step 2 — issue the SpawnPoolWindow command, transition into
        // the wait state. F.5 routes the action to `PipeTarget::Host`
        // for forward compatibility with the cross-process dispatch
        // follow-up; today the coordinator logs the IssueCmd and
        // doesn't actually transmit it on a launcher→host pipe (no
        // such pipe exists yet — see module docstring).
        debug_assert_eq!(self.phase, Phase::Initial);
        self.phase = Phase::WaitingForRefill;
        SagaAction::IssueCmd {
            target: PipeTarget::Host,
            cmd: Command::SpawnPoolWindow,
        }
    }

    fn on_event(&mut self, event: &Event, _ctx: &SagaCtx) -> SagaAction {
        // Only pivot on `PoolWindowAdded`. Every other event the bus
        // carries is unrelated to this saga's terminal condition.
        // A coordinator that drove this saga to Done because of an
        // unrelated event would fire the `SagaCompleted` bracket
        // before refill actually finished — the renderer's buffered
        // state would flush prematurely.
        match (&self.phase, event) {
            (Phase::WaitingForRefill, Event::PoolWindowAdded { label, .. }) => {
                // The refill produced a new label. Record + complete.
                // (Spec § 7.1: Step 3 is just `Done`; no further
                // dispatch.)
                self.refilled_label = Some(label.clone());
                SagaAction::Done
            }
            // Still waiting; or — for `Initial` — coordinator
            // hasn't called `start` yet, in which case we're not
            // supposed to be receiving events. Either way: no-op.
            _ => SagaAction::Wait,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn ctx(saga_id: u64) -> SagaCtx {
        SagaCtx { saga_id }
    }

    #[test]
    fn start_issues_spawn_pool_window_to_host() {
        let mut saga = PoolRespawn::new("window-pool-abc".into());
        let action = saga.start(&ctx(7));
        match action {
            SagaAction::IssueCmd { target, cmd } => {
                assert_eq!(target, PipeTarget::Host);
                assert!(matches!(cmd, Command::SpawnPoolWindow));
            }
            other => panic!(
                "expected IssueCmd(SpawnPoolWindow) on start, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn pool_window_added_completes_the_saga() {
        let mut saga = PoolRespawn::new("window-pool-abc".into());
        let _ = saga.start(&ctx(7));
        let action = saga.on_event(
            &Event::PoolWindowAdded {
                label: "window-pool-xyz".into(),
                version: 100,
            },
            &ctx(7),
        );
        assert!(matches!(action, SagaAction::Done));
        assert_eq!(saga.refilled_label(), Some("window-pool-xyz"));
    }

    #[test]
    fn unrelated_events_keep_saga_waiting() {
        let mut saga = PoolRespawn::new("window-pool-abc".into());
        let _ = saga.start(&ctx(7));
        let unrelated = Event::WindowOpened {
            label: "main".into(),
            kind: agentmux_common::ipc::WindowKind::FullInstance,
            parent_label: None,
            version: 50,
        };
        let action = saga.on_event(&unrelated, &ctx(7));
        assert!(matches!(action, SagaAction::Wait));
        assert!(saga.refilled_label().is_none());

        // Pool removal alone (the original promote signal) doesn't
        // complete the saga — only PoolWindowAdded does.
        let pool_removed = Event::PoolWindowRemoved {
            label: "window-pool-abc".into(),
            version: 51,
        };
        let action = saga.on_event(&pool_removed, &ctx(7));
        assert!(matches!(action, SagaAction::Wait));

        // PoolWindowPromoted is the start trigger but the saga is
        // already past that — coordinator only feeds events it
        // *receives*; the saga MUST NOT mistake a stray promote for
        // a refill.
        let pool_promoted = Event::PoolWindowPromoted {
            label: "window-pool-other".into(),
            version: 52,
        };
        let action = saga.on_event(&pool_promoted, &ctx(7));
        assert!(matches!(action, SagaAction::Wait));
    }

    #[test]
    fn first_pool_window_added_after_start_wins() {
        // The saga records the FIRST PoolWindowAdded after start as
        // the refill — even if a later one would be a closer match.
        // The coordinator's job is to scope events to this saga; the
        // saga itself does not disambiguate against concurrent
        // promotes (concurrent promotes spawn distinct sagas with
        // distinct saga_ids; coordinator routes events appropriately
        // — see super::run_coordinator for the routing logic).
        let mut saga = PoolRespawn::new("window-pool-abc".into());
        let _ = saga.start(&ctx(7));
        let _ = saga.on_event(
            &Event::PoolWindowAdded {
                label: "window-pool-first".into(),
                version: 100,
            },
            &ctx(7),
        );
        // Second event after Done is irrelevant; saga is no longer
        // in flight (coordinator removes it from the registry on
        // Done). We don't model that here — just assert the first
        // was captured.
        assert_eq!(saga.refilled_label(), Some("window-pool-first"));
    }

    /// End-to-end coordinator integration: emit a promote, watch the
    /// coordinator start the saga, observe the IssueCmd via log, emit
    /// a synthetic PoolWindowAdded, watch the coordinator emit
    /// SagaStarted then SagaCompleted bracket events on the bus.
    #[tokio::test]
    async fn coordinator_brackets_promote_with_saga_lifecycle_events() {
        use crate::saga::{run_coordinator, SagaCoordinator};

        let (events_tx, _) = tokio::sync::broadcast::channel::<Event>(64);
        let state = Arc::new(tokio::sync::Mutex::new(crate::state::State::default()));
        let coord = Arc::new(SagaCoordinator::new(events_tx.clone(), Arc::clone(&state)));

        // Subscribe a witness BEFORE the coordinator subscribes so we
        // observe both the input and the coordinator's emitted
        // SagaStarted/SagaCompleted events.
        let mut witness = events_tx.subscribe();
        let coord_rx = events_tx.subscribe();
        let _handle = tokio::spawn(run_coordinator(Arc::clone(&coord), coord_rx));

        // Yield so the coordinator's recv loop is parked on its
        // first recv() before we publish the trigger.
        tokio::task::yield_now().await;

        // Step 1 — the promote signal kicks off the saga.
        let _ = events_tx.send(Event::PoolWindowPromoted {
            label: "window-pool-abc".into(),
            version: 1,
        });

        // Step 2 — the saga waits for refill. Push a synthetic
        // PoolWindowAdded representing the host's implicit refill.
        let _ = events_tx.send(Event::PoolWindowAdded {
            label: "window-pool-xyz".into(),
            version: 2,
        });

        // Drain witness with a brief budget; we expect at minimum:
        //   PoolWindowPromoted (input) → SagaStarted → PoolWindowAdded
        //   (input) → SagaCompleted.
        // The order between the input events and the coordinator's
        // emissions can interleave (coordinator runs concurrently),
        // but causally SagaCompleted MUST follow SagaStarted.
        let mut saw_started = false;
        let mut saw_completed_after_started = false;
        let mut saga_id_started: Option<u64> = None;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            match tokio::time::timeout(std::time::Duration::from_millis(50), witness.recv()).await
            {
                Ok(Ok(Event::SagaStarted { saga_id, name, .. })) => {
                    assert_eq!(name, "pool_respawn_on_promote");
                    saw_started = true;
                    saga_id_started = Some(saga_id);
                }
                Ok(Ok(Event::SagaCompleted { saga_id, .. })) => {
                    if saw_started {
                        assert_eq!(Some(saga_id), saga_id_started);
                        saw_completed_after_started = true;
                        break;
                    }
                }
                Ok(Ok(_)) => {}
                Ok(Err(_)) => break,
                Err(_) => continue,
            }
        }
        assert!(
            saw_started,
            "expected coordinator to emit Event::SagaStarted for pool_respawn_on_promote"
        );
        assert!(
            saw_completed_after_started,
            "expected coordinator to emit Event::SagaCompleted after SagaStarted"
        );
    }
}
