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
//   - F.6 window-cleanup cascade saga.
//   - Launcher-side saga durability (separate concern; srv-side
//     durability already shipped).
//
// **CPD-3 update.** Step 2's `SagaAction::IssueCmd` is now LIVE: the
// coordinator dispatches `Command::SpawnPoolWindow { saga_id }`
// through `HostPipe::send_command()` (see
// `agentmux-launcher/src/saga/mod.rs::apply_action`). The saga is
// structurally identical — what changed is the coordinator's
// `IssueCmd::Host` arm. The saga is no longer a passive narrator of
// the host's implicit refill: it now causally drives it.
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
    /// LSD-2 — also surfaces in `--diag sagas` via `input_snapshot`.
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

    /// LSD-2 — record the promote's source label for `--diag sagas`.
    /// `refilled_label` is None at start (only known after Step 2's
    /// echo lands) so we don't include it; the durable log captures
    /// the inputs the saga was constructed with, not its evolving
    /// state — that lives in step rows.
    fn input_snapshot(&self) -> serde_json::Value {
        serde_json::json!({ "promoted_label": self.promoted_label })
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
        // CPD-1 schema-only: the wire shape now carries saga_id, but
        // the saga's `apply_action` for `IssueCmd::Host` is still
        // log-only (CPD-3 wires real dispatch). We pass `0` here as
        // a placeholder; CPD-3's `inject_saga_id()` helper rewrites
        // it to the live saga's id at dispatch time.
        SagaAction::IssueCmd {
            target: PipeTarget::Host,
            cmd: Command::SpawnPoolWindow { saga_id: 0 },
        }
    }

    fn on_event(&mut self, event: &Event, ctx: &SagaCtx) -> SagaAction {
        // Only pivot on `PoolWindowAdded`. Every other event the bus
        // carries is unrelated to this saga's terminal condition.
        // A coordinator that drove this saga to Done because of an
        // unrelated event would fire the `SagaCompleted` bracket
        // before refill actually finished — the renderer's buffered
        // state would flush prematurely.
        //
        // **CPD-4 — per-saga event correlation.** Filter
        // `PoolWindowAdded` by `saga_id`: only the event tagged with
        // *this* saga's id advances us. Untagged events (organic pool
        // refills) and events from sibling concurrent sagas are
        // ignored. Retires the evict-and-replace workaround from
        // PR #634 — concurrent same-kind sagas now coexist without
        // false-positive `SagaCompleted` cross-talk.
        match (&self.phase, event) {
            (Phase::WaitingForRefill, Event::PoolWindowAdded { label, saga_id, .. })
                if *saga_id == Some(ctx.saga_id) =>
            {
                // The refill produced a new label. Record + complete.
                // (Spec § 7.1: Step 3 is just `Done`; no further
                // dispatch.)
                self.refilled_label = Some(label.clone());
                SagaAction::Done
            }
            // Still waiting; or — for `Initial` — coordinator
            // hasn't called `start` yet; or the event's saga_id
            // doesn't match (concurrent saga or organic refill).
            // Either way: no-op.
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
                assert!(matches!(cmd, Command::SpawnPoolWindow { .. }));
            }
            other => panic!(
                "expected IssueCmd(SpawnPoolWindow) on start, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn pool_window_added_completes_the_saga() {
        // CPD-4 — saga_id-tagged event for *this* saga advances it.
        let mut saga = PoolRespawn::new("window-pool-abc".into());
        let _ = saga.start(&ctx(7));
        let action = saga.on_event(
            &Event::PoolWindowAdded {
                label: "window-pool-xyz".into(),
                version: 100,
                saga_id: Some(7),
            },
            &ctx(7),
        );
        assert!(matches!(action, SagaAction::Done));
        assert_eq!(saga.refilled_label(), Some("window-pool-xyz"));
    }

    /// CPD-4 — `PoolWindowAdded` with `saga_id: None` (organic refill,
    /// no saga in flight on the host side) does NOT terminate the
    /// saga. Pre-CPD-4, ANY `PoolWindowAdded` would complete the saga;
    /// per-saga correlation now scopes terminal events to the
    /// originating saga only.
    #[test]
    fn organic_pool_window_added_does_not_complete_saga() {
        let mut saga = PoolRespawn::new("window-pool-abc".into());
        let _ = saga.start(&ctx(7));
        let action = saga.on_event(
            &Event::PoolWindowAdded {
                label: "window-pool-organic".into(),
                version: 100,
                saga_id: None,
            },
            &ctx(7),
        );
        assert!(matches!(action, SagaAction::Wait));
        assert!(saga.refilled_label().is_none());
    }

    /// CPD-4 — `PoolWindowAdded` tagged with a *different* saga_id
    /// (sibling concurrent saga's echo) is ignored. This is the
    /// invariant that retires evict-and-replace: two concurrent
    /// PoolRespawn sagas can coexist, each consuming only its own
    /// echo.
    #[test]
    fn pool_window_added_with_foreign_saga_id_is_ignored() {
        let mut saga = PoolRespawn::new("window-pool-abc".into());
        let _ = saga.start(&ctx(7));
        let action = saga.on_event(
            &Event::PoolWindowAdded {
                label: "window-pool-sibling".into(),
                version: 100,
                saga_id: Some(8),
            },
            &ctx(7),
        );
        assert!(matches!(action, SagaAction::Wait));
        assert!(saga.refilled_label().is_none());
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
        // The saga records the FIRST PoolWindowAdded *for its own
        // saga_id* after start as the refill. CPD-4 — the coordinator
        // dispatches the saga's IssueCmd with `saga_id` injected, so
        // the host's matching report carries `saga_id: Some(N)`. The
        // saga itself filters by `ctx.saga_id`; events with foreign
        // ids (sibling concurrent sagas) are ignored.
        let mut saga = PoolRespawn::new("window-pool-abc".into());
        let _ = saga.start(&ctx(7));
        let _ = saga.on_event(
            &Event::PoolWindowAdded {
                label: "window-pool-first".into(),
                version: 100,
                saga_id: Some(7),
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

        // CPD-4: wait for SagaStarted to learn the coordinator-
        // allocated saga_id before publishing the matching
        // PoolWindowAdded. Pre-CPD-4 we could send `saga_id: None`
        // because every PoolWindowAdded advanced every in-flight
        // saga; under per-saga correlation the saga only consumes
        // its own echo.
        let mut saga_id_started: Option<u64> = None;
        let mut saw_started = false;
        let mut saw_completed_after_started = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            match tokio::time::timeout(std::time::Duration::from_millis(50), witness.recv()).await
            {
                Ok(Ok(Event::SagaStarted { saga_id, name, .. })) => {
                    assert_eq!(name, "pool_respawn_on_promote");
                    saw_started = true;
                    saga_id_started = Some(saga_id);
                    // Step 2 — the saga waits for refill. Push a
                    // synthetic PoolWindowAdded tagged with the saga's
                    // allocated id (CPD-4 per-saga correlation).
                    let _ = events_tx.send(Event::PoolWindowAdded {
                        label: "window-pool-xyz".into(),
                        version: 2,
                        saga_id: Some(saga_id),
                    });
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
