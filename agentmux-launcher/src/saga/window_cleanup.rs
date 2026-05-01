// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase F.6 — window-cleanup cascade saga.
//
// **What this saga does**
//
// When a user-visible top-level window closes, two cleanup steps
// happen implicitly in the host today:
//
//   1. Browser-pane HWNDs that belong to the closing window get
//      reaped (lifecycle entries drained, pane HWND map cleared,
//      subwindow cascade closes children).
//   2. If that window was the LAST user-visible window, Stage 1 of
//      the wrr two-stage close cascade in
//      `agentmux-cef::client::on_before_close` posts WM_CLOSE to
//      every warm-pool browser so the app can drain to zero.
//      Otherwise the pool stays warm.
//
// Both steps fire inside the same `on_before_close` body. Renderers
// that want to buffer "this window is closing AND its post-close
// cleanup is settling" atomically have nothing to pivot on today —
// the launcher's `Event::WindowClosed` is the only signal, and it
// fires BEFORE the cleanup steps complete.
//
// This saga formalizes the implicit cleanup as an explicit state
// machine so the renderer sees a `SagaStarted` / `SagaCompleted`
// bracket. Implementing the spec sketch from
// `docs/specs/SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md` §7.2:
//
//   saga WindowCleanupCascade(closed_label):
//       Step 1 — on Event::WindowClosed { label }:
//                 issue Command::ReapPanes { label } → host
//                 wait for: Event::PanesReaped { label }
//       Step 2 — issue Command::DrainPoolIfLast { label } → host
//                 wait for: Event::PoolDrained or Event::PoolNotLast
//       Step 3 — Done
//
//       Compensation: none (cleanup failure is logged; next close
//                     retries).
//
// **Scope of F.6 (this PR)**
//
// F.6 ships:
//   - the saga state machine itself (this file),
//   - the wire types (`Command::ReapPanes`,
//     `Command::DrainPoolIfLast`, `Command::ReportPanesReaped`,
//     `Command::ReportPoolDrainDecision`, `Event::PanesReaped`,
//     `Event::PoolDrained`, `Event::PoolNotLast`),
//   - host-side `report_panes_reaped` + `report_pool_drain_decision`
//     calls inside `on_before_close`,
//   - launcher reducer arms translating the reports into the typed
//     events,
//   - coordinator wiring that registers the saga as a consumer of
//     `Event::WindowClosed`.
//
// F.6 does NOT ship:
//   - The actual cross-process dispatch of `Command::ReapPanes` /
//     `Command::DrainPoolIfLast` to the host. Today the launcher's
//     named-pipe IPC is host→launcher only (host sends Commands up;
//     launcher broadcasts Events back). A launcher→host command
//     pipe doesn't exist yet. Both `IssueCmd::Host` actions are
//     logged-only; the saga relies on the host's existing implicit
//     cleanup flow inside `on_before_close` to produce the
//     `Event::PanesReaped` + `Event::PoolDrained`/`PoolNotLast` it
//     waits for. **This is acceptable scope per the F-spec § 7
//     ("If the cross-process plumbing is too heavy a lift for one
//     PR, scope down: implement the saga shape + the launcher-side
//     coordinator entry, even if the actual cross-process dispatch
//     is initially in-process").** The follow-up PR replaces the
//     log with a real wire-level send — same trajectory as F.5's
//     `SpawnPoolWindow`.
//   - F.7 (cleanup audit + property tests).
//   - Launcher-side saga durability — separate concern.
//
// **Why a saga at all if the IssueCmds are currently passive?**
//
// Same reasoning as F.5: the renderer-visible value is the
// `SagaStarted` / `SagaCompleted` bracket. Subscribers see "saga
// foo running" and can buffer related events for that saga_id
// until the bracket closes. That works the same way whether the
// launcher actively dispatches the cleanup or merely observes the
// host doing it. When the cross-process pipe lands, only the
// saga's `IssueCmd` handling needs to change — the renderer-facing
// semantics are stable.
//
// **What if a terminal event never arrives?**
//
// Same behavior as F.5: the saga waits forever until the bus
// closes (saga abandoned on launcher restart) or — once
// per-saga timeouts land — a deadline force-fails it. F.6 keeps
// behavior deliberately simple: if cleanup genuinely fails, the
// next window close starts a fresh saga; the prior failed saga
// stays in `in_flight` until the launcher exits or it gets
// evicted by a same-kind successor (see `saga::mod.rs`'s
// evict-and-replace policy from F.5 round 4).
//
// **Known limitation: concurrent-correlation** (inherited from
// F.5). If two windows close simultaneously, the coordinator
// broadcasts every event to every in-flight saga. The first
// `PanesReaped` (or `PoolDrained`/`PoolNotLast`) advances ALL
// matching sagas regardless of which window's close they belong
// to — early `SagaCompleted` for one window's bracket, the other
// window's terminal event left unbracketed.
//
// In practice this requires the user to close two windows in
// rapid succession (under the host's `on_before_close` execution
// latency, ~few ms). The user-visible consequence is a brief
// renderer-side bracketing inconsistency; reducer + SQLite state
// remain correct.
//
// Mitigation today: `saga::mod.rs::run_coordinator`'s
// evict-and-replace policy (F.5 round 4) ensures only ONE
// window-cleanup-cascade saga is in flight at a time. The second
// `WindowClosed` evicts the first saga (emitting `SagaFailed`
// with "evicted" reason), then starts a fresh one. Renderer-
// visible: one premature `SagaFailed` + one correct
// `SagaCompleted`. Same trade-off as F.5 documented for
// concurrent promotes.
//
// Proper fix requires coordinator-level FIFO routing or per-saga
// sequence-number correlation. Both depend on the cross-process
// dispatch landing first (so the saga's `IssueCmd` is actually
// causal). Closed alongside the launcher→host command pipe in a
// future phase.

use agentmux_common::ipc::{Command, Event};

use super::{PipeTarget, Saga, SagaAction, SagaCtx};

/// State of one in-flight window-cleanup-cascade saga.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    /// Saga just constructed — waiting for the coordinator to call
    /// `start`. The first `start` call transitions to `ReapingPanes`
    /// and emits the `IssueCmd` for `ReapPanes`.
    Initial,
    /// `ReapPanes` has been issued; waiting for any
    /// `Event::PanesReaped` that matches our window label.
    ReapingPanes,
    /// `DrainPoolIfLast` has been issued; waiting for either
    /// `Event::PoolDrained` or `Event::PoolNotLast`. Both are
    /// terminal — the saga doesn't care WHICH branch fires, only
    /// that ONE of them does.
    DrainingPool,
}

/// Window-cleanup-cascade saga: fires once per window close, drives
/// the implicit pane-reap + pool-drain-decision flow into an
/// explicit two-step state machine, then completes.
pub struct WindowCleanupCascade {
    /// Label of the window that closed (the trigger event's payload).
    /// The saga uses this for label-matching on terminal events:
    /// `PanesReaped { label }` only advances Step 1 when `label`
    /// matches; same for `PoolDrained`/`PoolNotLast` in Step 2.
    ///
    /// Note: under the coordinator's evict-and-replace policy, only
    /// one window-cleanup-cascade saga is ever in flight at a time,
    /// so label-matching is technically redundant for correctness.
    /// Keep it anyway as cheap defense-in-depth + a clear
    /// invariant-statement: "this saga belongs to *this* window's
    /// cleanup, not whoever's terminal event happens to land first."
    closed_label: String,
    /// Whether the host's drain decision said "yes, that was the
    /// last window" (`PoolDrained` arm) or "no, more windows
    /// remain" (`PoolNotLast` arm). `None` until Step 2 resolves.
    /// Exported for tests.
    drained_pool: Option<bool>,
    phase: Phase,
}

impl WindowCleanupCascade {
    /// Construct a fresh saga for a close of `closed_label`.
    /// Coordinator allocates the saga_id and calls `start` once.
    pub fn new(closed_label: String) -> Self {
        Self {
            closed_label,
            drained_pool: None,
            phase: Phase::Initial,
        }
    }

    /// Whether the host's drain decision flagged this close as
    /// "last user-visible window" (`true` → `PoolDrained` branch
    /// fired) or not (`false` → `PoolNotLast` branch fired). `None`
    /// before Step 2 resolves. Exported for tests.
    #[cfg(test)]
    pub fn drained_pool(&self) -> Option<bool> {
        self.drained_pool
    }

    /// Label this saga is tracking. Exported for tests.
    #[cfg(test)]
    pub fn closed_label(&self) -> &str {
        &self.closed_label
    }
}

impl Saga for WindowCleanupCascade {
    fn name(&self) -> &'static str {
        "window_cleanup_cascade"
    }

    fn start(&mut self, _ctx: &SagaCtx) -> SagaAction {
        // Step 1 — issue the ReapPanes command, transition into the
        // wait state. F.6 routes the action to `PipeTarget::Host`
        // for forward compatibility with the cross-process dispatch
        // follow-up; today the coordinator logs the IssueCmd and
        // doesn't actually transmit it on a launcher→host pipe (no
        // such pipe exists yet — see module docstring).
        debug_assert_eq!(self.phase, Phase::Initial);
        self.phase = Phase::ReapingPanes;
        SagaAction::IssueCmd {
            target: PipeTarget::Host,
            cmd: Command::ReapPanes {
                label: self.closed_label.clone(),
            },
        }
    }

    fn on_event(&mut self, event: &Event, _ctx: &SagaCtx) -> SagaAction {
        match (&self.phase, event) {
            // Step 1 → Step 2 transition. `PanesReaped` arrives from
            // the host (`report_panes_reaped` inside
            // `on_before_close`). Match on the label so a stray
            // event from another window's close doesn't advance us
            // (defense-in-depth — see `closed_label` field doc).
            (Phase::ReapingPanes, Event::PanesReaped { label, .. })
                if label == &self.closed_label =>
            {
                self.phase = Phase::DrainingPool;
                SagaAction::IssueCmd {
                    target: PipeTarget::Host,
                    cmd: Command::DrainPoolIfLast {
                        label: self.closed_label.clone(),
                    },
                }
            }
            // Step 2 terminal — drain happened (last window closed).
            (Phase::DrainingPool, Event::PoolDrained { label, .. })
                if label == &self.closed_label =>
            {
                self.drained_pool = Some(true);
                SagaAction::Done
            }
            // Step 2 terminal — drain skipped (other windows remain).
            // Equally a success: the saga's job is to bracket the
            // drain *decision*, not enforce a particular outcome.
            (Phase::DrainingPool, Event::PoolNotLast { label, .. })
                if label == &self.closed_label =>
            {
                self.drained_pool = Some(false);
                SagaAction::Done
            }
            // Anything else: still waiting; or — for `Initial` —
            // coordinator hasn't called `start` yet, in which case
            // we're not supposed to be receiving events. Either way:
            // no-op.
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
    fn start_issues_reap_panes_to_host() {
        let mut saga = WindowCleanupCascade::new("main".into());
        let action = saga.start(&ctx(7));
        match action {
            SagaAction::IssueCmd { target, cmd } => {
                assert_eq!(target, PipeTarget::Host);
                match cmd {
                    Command::ReapPanes { label } => assert_eq!(label, "main"),
                    other => panic!("expected ReapPanes, got {:?}", other),
                }
            }
            other => panic!(
                "expected IssueCmd(ReapPanes) on start, got {:?}",
                other
            ),
        }
        assert_eq!(saga.closed_label(), "main");
    }

    #[test]
    fn panes_reaped_advances_to_drain_pool_step() {
        let mut saga = WindowCleanupCascade::new("main".into());
        let _ = saga.start(&ctx(7));
        let action = saga.on_event(
            &Event::PanesReaped {
                label: "main".into(),
                version: 100,
            },
            &ctx(7),
        );
        match action {
            SagaAction::IssueCmd { target, cmd } => {
                assert_eq!(target, PipeTarget::Host);
                match cmd {
                    Command::DrainPoolIfLast { label } => assert_eq!(label, "main"),
                    other => panic!("expected DrainPoolIfLast, got {:?}", other),
                }
            }
            other => panic!(
                "expected IssueCmd(DrainPoolIfLast) on PanesReaped, got {:?}",
                other
            ),
        }
        // Drain decision not yet known.
        assert_eq!(saga.drained_pool(), None);
    }

    #[test]
    fn pool_drained_completes_the_saga_with_drain_flag_true() {
        let mut saga = WindowCleanupCascade::new("main".into());
        let _ = saga.start(&ctx(7));
        let _ = saga.on_event(
            &Event::PanesReaped {
                label: "main".into(),
                version: 100,
            },
            &ctx(7),
        );
        let action = saga.on_event(
            &Event::PoolDrained {
                label: "main".into(),
                version: 101,
            },
            &ctx(7),
        );
        assert!(matches!(action, SagaAction::Done));
        assert_eq!(saga.drained_pool(), Some(true));
    }

    #[test]
    fn pool_not_last_completes_the_saga_with_drain_flag_false() {
        let mut saga = WindowCleanupCascade::new("secondary".into());
        let _ = saga.start(&ctx(7));
        let _ = saga.on_event(
            &Event::PanesReaped {
                label: "secondary".into(),
                version: 100,
            },
            &ctx(7),
        );
        let action = saga.on_event(
            &Event::PoolNotLast {
                label: "secondary".into(),
                version: 101,
            },
            &ctx(7),
        );
        assert!(matches!(action, SagaAction::Done));
        assert_eq!(saga.drained_pool(), Some(false));
    }

    #[test]
    fn unrelated_events_keep_saga_waiting() {
        let mut saga = WindowCleanupCascade::new("main".into());
        let _ = saga.start(&ctx(7));

        // WindowOpened — wholly unrelated.
        let unrelated = Event::WindowOpened {
            label: "other".into(),
            kind: agentmux_common::ipc::WindowKind::FullInstance,
            parent_label: None,
            version: 50,
        };
        let action = saga.on_event(&unrelated, &ctx(7));
        assert!(matches!(action, SagaAction::Wait));

        // PanesReaped for a DIFFERENT label — must not advance us
        // (defense-in-depth label match).
        let action = saga.on_event(
            &Event::PanesReaped {
                label: "different-window".into(),
                version: 51,
            },
            &ctx(7),
        );
        assert!(matches!(action, SagaAction::Wait));

        // PoolDrained while we're still in Step 1 — must not advance
        // us (wrong phase).
        let action = saga.on_event(
            &Event::PoolDrained {
                label: "main".into(),
                version: 52,
            },
            &ctx(7),
        );
        assert!(matches!(action, SagaAction::Wait));

        // The trigger itself (WindowClosed) is what the coordinator
        // uses to START the saga; the saga MUST NOT mistake a stray
        // WindowClosed (e.g. from another close) for a step
        // advancement.
        let action = saga.on_event(
            &Event::WindowClosed {
                label: "main".into(),
                version: 53,
            },
            &ctx(7),
        );
        assert!(matches!(action, SagaAction::Wait));
    }

    #[test]
    fn label_mismatch_in_drain_phase_keeps_saga_waiting() {
        let mut saga = WindowCleanupCascade::new("main".into());
        let _ = saga.start(&ctx(7));
        let _ = saga.on_event(
            &Event::PanesReaped {
                label: "main".into(),
                version: 100,
            },
            &ctx(7),
        );
        // Now in DrainingPool. A drain event for a different label
        // must NOT terminate this saga.
        let action = saga.on_event(
            &Event::PoolDrained {
                label: "other".into(),
                version: 101,
            },
            &ctx(7),
        );
        assert!(matches!(action, SagaAction::Wait));
        let action = saga.on_event(
            &Event::PoolNotLast {
                label: "other".into(),
                version: 102,
            },
            &ctx(7),
        );
        assert!(matches!(action, SagaAction::Wait));
        // Drain decision still unresolved.
        assert_eq!(saga.drained_pool(), None);
    }

    /// End-to-end coordinator integration: emit `WindowClosed`,
    /// watch the coordinator start the saga, observe the IssueCmds
    /// via log, emit synthetic `PanesReaped` then
    /// `PoolDrained`/`PoolNotLast`, watch the coordinator emit
    /// `SagaStarted` then `SagaCompleted` bracket events on the bus.
    #[tokio::test]
    async fn coordinator_brackets_close_with_saga_lifecycle_events() {
        use crate::saga::{run_coordinator, SagaCoordinator};

        let (events_tx, _) = tokio::sync::broadcast::channel::<Event>(64);
        let state = Arc::new(tokio::sync::Mutex::new(crate::state::State::default()));
        let coord = Arc::new(SagaCoordinator::new(events_tx.clone(), Arc::clone(&state)));

        let mut witness = events_tx.subscribe();
        let coord_rx = events_tx.subscribe();
        let _handle = tokio::spawn(run_coordinator(Arc::clone(&coord), coord_rx));

        // Yield so coordinator's recv loop is parked before publish.
        tokio::task::yield_now().await;

        // Trigger.
        let _ = events_tx.send(Event::WindowClosed {
            label: "main".into(),
            version: 1,
        });
        // Step 1 terminal.
        let _ = events_tx.send(Event::PanesReaped {
            label: "main".into(),
            version: 2,
        });
        // Step 2 terminal — pick the "drained" branch.
        let _ = events_tx.send(Event::PoolDrained {
            label: "main".into(),
            version: 3,
        });

        let mut saw_started = false;
        let mut saw_completed_after_started = false;
        let mut saga_id_started: Option<u64> = None;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            match tokio::time::timeout(std::time::Duration::from_millis(50), witness.recv()).await
            {
                Ok(Ok(Event::SagaStarted { saga_id, name, .. })) => {
                    if name == "window_cleanup_cascade" {
                        saw_started = true;
                        saga_id_started = Some(saga_id);
                    }
                }
                Ok(Ok(Event::SagaCompleted { saga_id, .. })) => {
                    if saw_started && Some(saga_id) == saga_id_started {
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
            "expected coordinator to emit Event::SagaStarted for window_cleanup_cascade"
        );
        assert!(
            saw_completed_after_started,
            "expected coordinator to emit Event::SagaCompleted after SagaStarted"
        );
    }

    /// Concurrent-window-close scenario: two `WindowClosed` events
    /// arrive in close succession. Per the F.5 evict-and-replace
    /// policy in `saga::mod.rs::run_coordinator`, the second close
    /// evicts the first saga (emitting `SagaFailed` with "evicted"
    /// reason) and starts a fresh one. Both close events still end
    /// with terminal lifecycle activity:
    ///   - first close → `SagaFailed` (evicted)
    ///   - second close → `SagaStarted` + `SagaCompleted`
    /// We assert that we observe at least one of each lifecycle
    /// type on the bus.
    #[tokio::test]
    async fn coordinator_evicts_on_concurrent_window_close() {
        use crate::saga::{run_coordinator, SagaCoordinator};

        let (events_tx, _) = tokio::sync::broadcast::channel::<Event>(64);
        let state = Arc::new(tokio::sync::Mutex::new(crate::state::State::default()));
        let coord = Arc::new(SagaCoordinator::new(events_tx.clone(), Arc::clone(&state)));

        let mut witness = events_tx.subscribe();
        let coord_rx = events_tx.subscribe();
        let _handle = tokio::spawn(run_coordinator(Arc::clone(&coord), coord_rx));

        tokio::task::yield_now().await;

        // First close → starts saga A.
        let _ = events_tx.send(Event::WindowClosed {
            label: "main".into(),
            version: 1,
        });
        // Second close BEFORE saga A's terminals arrive → evicts A,
        // starts saga B.
        let _ = events_tx.send(Event::WindowClosed {
            label: "secondary".into(),
            version: 2,
        });
        // Saga B's terminals.
        let _ = events_tx.send(Event::PanesReaped {
            label: "secondary".into(),
            version: 3,
        });
        let _ = events_tx.send(Event::PoolNotLast {
            label: "secondary".into(),
            version: 4,
        });

        let mut started_count = 0u32;
        let mut completed_count = 0u32;
        let mut failed_evict_count = 0u32;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            match tokio::time::timeout(std::time::Duration::from_millis(50), witness.recv()).await
            {
                Ok(Ok(Event::SagaStarted { name, .. })) => {
                    if name == "window_cleanup_cascade" {
                        started_count += 1;
                    }
                }
                Ok(Ok(Event::SagaCompleted { .. })) => {
                    completed_count += 1;
                }
                Ok(Ok(Event::SagaFailed { reason, .. })) => {
                    if reason.contains("evicted") {
                        failed_evict_count += 1;
                    }
                }
                Ok(Ok(_)) => {}
                Ok(Err(_)) => break,
                Err(_) => {
                    // No new events for 50ms — break early once we've
                    // seen the expected pattern.
                    if started_count >= 2 && completed_count >= 1 && failed_evict_count >= 1 {
                        break;
                    }
                }
            }
        }
        assert!(
            started_count >= 2,
            "expected ≥2 SagaStarted (one per close), got {}",
            started_count
        );
        assert!(
            failed_evict_count >= 1,
            "expected ≥1 SagaFailed with 'evicted' reason from evict-and-replace policy, got {}",
            failed_evict_count
        );
        assert!(
            completed_count >= 1,
            "expected ≥1 SagaCompleted (the surviving saga), got {}",
            completed_count
        );
    }
}
