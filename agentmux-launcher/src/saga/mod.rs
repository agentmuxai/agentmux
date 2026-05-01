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
use std::time::Duration;

use agentmux_common::ipc::{Command, Event};

use crate::host_pipe::HostPipe;

pub mod pool_respawn;
pub mod window_cleanup;

#[cfg(test)]
mod integration_tests;

// LSD-1 (PR LSD-1) — durable launcher saga log + API. Foundations
// only: the coordinator does NOT call any of these methods yet.
// Module is declared here so it compiles + tests run; PR LSD-2 wires
// the coordinator to write through `LauncherSagaLog` on every state
// transition. See `docs/specs/SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md`
// §4 PR1 for the staged-rollout rationale.
mod log;
pub use log::LauncherSagaLog;
use log::SagaOutcome;

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
    /// LSD-2 — saga's input arguments serialized for the durable log's
    /// `input_json` column. The coordinator calls this once at
    /// `spawn_saga` time (before `start`) and writes the result via
    /// `LauncherSagaLog::start_saga`. Operators see this in
    /// `--diag sagas` output (e.g. `{"closed_label":"win-3"}`) so they
    /// can tell which window's cleanup a recovered-failed saga
    /// belonged to.
    ///
    /// Default `Value::Null` — sagas with no input fields can ignore
    /// it. Concrete sagas should override with a `serde_json::json!`
    /// of their constructor args.
    fn input_snapshot(&self) -> serde_json::Value {
        serde_json::Value::Null
    }

    /// CPD-3 — per-saga deadline budget for completing all
    /// `IssueCmd`+wait cycles. Coordinator arms a timer when the saga
    /// registers; if the saga is still in_flight when `timeout()`
    /// elapses, it is force-failed (`SagaFailed { reason: "saga
    /// timeout" }`) and removed from the registry.
    ///
    /// Default 5s — fits class-C single-step host dispatch (e.g.
    /// pool respawn). `WindowCleanupCascade` overrides to 30s
    /// because pane drain on a workspace with many panes can
    /// legitimately take that long. Per spec §3.10.
    fn timeout(&self) -> Duration {
        Duration::from_secs(5)
    }
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
/// LSD-2 — coordinator's per-saga book-keeping. Wraps the boxed saga
/// with the durable-log step bookkeeping the coordinator needs to call
/// `LauncherSagaLog::finish_step` when the awaited bus event lands.
///
/// `awaiting_step` is `Some(idx)` when the saga is parked on a
/// `Wait`-then-event pivot (the most recent `IssueCmd` allocated step
/// `idx`); `None` between dispatches and after termination. Tracking
/// it on the in-flight record (rather than inside each saga impl) keeps
/// the durability concern out of saga authors' hands — they only deal
/// with `SagaAction`.
///
/// `next_step_index` is the monotonic counter the coordinator
/// `fetch_add(1)`s on every `IssueCmd` for this saga. Mirrors srv's
/// `SagaCtx::step_index`. Lives per-saga (not coordinator-global) so
/// concurrent sagas don't interleave indices in the log.
struct InFlightSaga {
    saga: Box<dyn Saga>,
    awaiting_step: Option<u32>,
    next_step_index: u32,
}

pub struct SagaCoordinator {
    /// Monotonic saga-id allocator.
    next_saga_id: std::sync::atomic::AtomicU64,
    /// In-flight sagas keyed by saga_id, each wrapped with the
    /// durable-log bookkeeping (LSD-2: `awaiting_step` +
    /// `next_step_index`).
    in_flight: tokio::sync::Mutex<std::collections::HashMap<u64, InFlightSaga>>,
    /// Reference to the broadcast bus so the coordinator can emit
    /// `SagaStarted` / `SagaCompleted` / `SagaFailed`.
    events_tx: tokio::sync::broadcast::Sender<Event>,
    /// Reference to the launcher's reducer state for `bump_version`
    /// when emitting saga lifecycle events.
    state: Arc<tokio::sync::Mutex<crate::state::State>>,
    /// LSD-2 — durable saga log. `None` in tests that don't exercise
    /// the durability path (the saga then logs + remains in flight,
    /// preserving the pre-LSD-2 behavior tests rely on for end-to-end
    /// bracket assertions). When `Some`, every saga lifecycle
    /// transition (`spawn_saga` → `start_saga`, `IssueCmd` →
    /// `start_step`, awaited-event consumed → `finish_step`,
    /// `Done` → `terminate_saga(Completed)`, `Failed` / evicted →
    /// `terminate_saga(Failed)`) writes to this log.
    log: Option<Arc<LauncherSagaLog>>,
    /// CPD-3 — launcher → host pipe wrapper. `apply_action` for
    /// `IssueCmd::Host` dispatches via `host_pipe.send_command()` when
    /// installed. `None` in tests that don't exercise host dispatch
    /// (those drive sagas via synthetic terminal events on the bus).
    host_pipe: Option<Arc<HostPipe>>,
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
            log: None,
            host_pipe: None,
        }
    }

    /// CPD-3 — install the host pipe so `IssueCmd::Host` actions are
    /// dispatched live (instead of log-only). Builder-style setter so
    /// existing tests + the `next_id_is_monotonic` smoke don't have to
    /// construct a HostPipe. Production wiring (`main.rs`) calls this
    /// once before `run_coordinator` is spawned.
    pub fn with_host_pipe(mut self, host_pipe: Arc<HostPipe>) -> Self {
        self.host_pipe = Some(host_pipe);
        self
    }

    /// LSD-2 — install the durable saga log so saga lifecycle
    /// transitions are persisted to `~/.agentmux/launcher-sagas.db`.
    /// Builder-style setter rather than a constructor parameter so
    /// existing tests + the `next_id_is_monotonic` smoke don't have
    /// to construct an in-memory log. Production wiring (in
    /// `main.rs`) calls this once before `run_coordinator` is spawned.
    pub fn with_log(mut self, log: Arc<LauncherSagaLog>) -> Result<Self, crate::saga::log::LogError> {
        // Seed the saga_id allocator from the highest persisted id
        // so a launcher restart cannot reuse an id that already
        // exists in `launcher_saga`. Reusing would (a) fail new
        // INSERTs on duplicate-PK, and (b) silently mutate prior
        // saga rows via `terminate_saga` / `finish_step` UPDATEs
        // keyed by saga_id — corrupting saga history + recovery
        // diagnostics. (codex P1 PR #645 round 1.)
        //
        // On `max_saga_id()` error: return Err so caller (main.rs)
        // can fail launcher startup loudly. Continuing with a default
        // next_saga_id=1 while the log is still attached would leave
        // the coordinator in the exact failure mode this seed is
        // meant to prevent. (codex P1 PR #645 round 2.)
        let max = log.max_saga_id()?;
        self.next_saga_id
            .store(max + 1, std::sync::atomic::Ordering::Relaxed);
        if max > 0 {
            crate::log(&format!(
                "[saga] seeded next_saga_id={} from launcher_saga.max(saga_id)={}",
                max + 1,
                max
            ));
        }
        self.log = Some(log);
        Ok(self)
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
    /// an `ApplyOutcome` carrying:
    ///   - `in_flight`: true → caller keeps the saga in `in_flight`,
    ///     false → caller removes it.
    ///   - `awaiting_step`: `Some(idx)` if the action was an
    ///     `IssueCmd` that allocated step `idx` and the saga is now
    ///     parked waiting for the echo event. Caller stores this on
    ///     the InFlightSaga so the next non-`Wait` `on_event` return
    ///     can call `LauncherSagaLog::finish_step(saga_id, idx, ev)`.
    ///
    /// CPD-3 — `IssueCmd::Host` dispatches live through `HostPipe`
    /// when one is installed. Tests without a host_pipe (the existing
    /// F.5/F.6 unit + integration tests) fall back to log-only so
    /// synthetic-terminal-event drivers continue to work.
    /// `LauncherSelf` and `Srv` targets remain log-only — reserved
    /// for class-D/E sagas, no consumer today.
    ///
    /// LSD-2 — every state transition writes to `LauncherSagaLog`
    /// when one is installed: `IssueCmd` → `start_step`, `Done` →
    /// `terminate_saga(Completed)`, `Failed` →
    /// `terminate_saga(Failed)`.
    ///
    /// `step_index` is the caller-allocated index for THIS dispatch.
    /// On the first dispatch in `spawn_saga`, the caller passes 0;
    /// on subsequent dispatches from the event loop, the caller pulls
    /// + bumps the per-InFlightSaga counter.
    ///
    /// Terminal paths (`Done`, `Failed`, host-send-error) all go
    /// through `claim_terminal` so only one of {normal Done/Failed,
    /// timeout-task, SagaActionFailed listener, host-send-error}
    /// can win the terminal-event race for a given saga_id.
    /// (reagent P1 PR #644 round 1 + 2.)
    async fn apply_action(
        &self,
        saga_id: u64,
        name: &'static str,
        action: SagaAction,
        step_index: u32,
    ) -> ApplyOutcome {
        match action {
            SagaAction::IssueCmd { target, cmd } => {
                // LSD-2 — write the durable `pending` step row BEFORE
                // dispatch so a crash mid-dispatch leaves a recoverable
                // breadcrumb (LSD-3's walker upgrades this saga to
                // `failed_compensation` on next launcher startup).
                let step_name = derive_step_name(&cmd, target);
                if let Some(log) = self.log.as_ref() {
                    if let Err(e) =
                        log.start_step(saga_id, step_index, &step_name, target, &cmd)
                    {
                        crate::log(&format!(
                            "[saga] saga_id={} name={} start_step log write failed: {} — continuing (in-memory path remains authoritative for this run)",
                            saga_id, name, e
                        ));
                    }
                }

                match target {
                    PipeTarget::Host => {
                        // CPD-3 — dispatch live through the host pipe
                        // when installed. Tests without a host_pipe
                        // (most existing F.5/F.6 unit + integration
                        // tests) fall back to log-only so synthetic-
                        // terminal-event drivers continue to work.
                        let Some(host_pipe) = self.host_pipe.as_ref() else {
                            crate::log(&format!(
                                "[saga] saga_id={} name={} IssueCmd target=Host cmd={:?} (no host_pipe installed — log-only)",
                                saga_id, name, cmd
                            ));
                            return ApplyOutcome::in_flight_awaiting(step_index);
                        };
                        let cmd_with_id = inject_saga_id(cmd, saga_id);
                        match host_pipe.send_command(&cmd_with_id).await {
                            Ok(()) => {
                                crate::log(&format!(
                                    "[saga] saga_id={} name={} IssueCmd::Host dispatched cmd={:?}",
                                    saga_id, name, cmd_with_id
                                ));
                                ApplyOutcome::in_flight_awaiting(step_index)
                            }
                            Err(e) => {
                                // Same claim_terminal guard as the
                                // Done/Failed paths — host-send-failure
                                // is a terminal transition and must not
                                // race against the timeout task or the
                                // SagaActionFailed listener emitting
                                // their own terminal events for the same
                                // saga_id (codex P1 PR #644 round 2).
                                if self.claim_terminal(saga_id).await {
                                    crate::log(&format!(
                                        "[saga] saga_id={} name={} host pipe send failed: {} — emitting SagaFailed",
                                        saga_id, name, e
                                    ));
                                    let reason = format!("host pipe send failed: {}", e);
                                    if let Some(log) = self.log.as_ref() {
                                        if let Err(le) = log.terminate_saga(
                                            saga_id,
                                            SagaOutcome::Failed {
                                                reason: reason.clone(),
                                            },
                                        ) {
                                            crate::log(&format!(
                                                "[saga] saga_id={} terminate_saga(Failed) log write failed: {}",
                                                saga_id, le
                                            ));
                                        }
                                    }
                                    self.emit_failed(saga_id, reason).await;
                                }
                                ApplyOutcome::terminated()
                            }
                        }
                    }
                    PipeTarget::LauncherSelf | PipeTarget::Srv => {
                        // Reserved for class-D/E sagas; out of scope
                        // per SPEC_CROSS_PROCESS_DISPATCH §3.6. Log-
                        // only retains framework shape until a
                        // consumer arrives.
                        crate::log(&format!(
                            "[saga] saga_id={} name={} IssueCmd target={:?} cmd={:?} (target not yet wired; log-only)",
                            saga_id, name, target, cmd
                        ));
                        ApplyOutcome::in_flight_awaiting(step_index)
                    }
                }
            }
            SagaAction::Wait => ApplyOutcome::in_flight_no_change(),
            SagaAction::Done => {
                // Atomic claim: only one of {normal Done, timeout-task,
                // SagaActionFailed listener, host-send-error} can win
                // the terminal-event race. Without this guard a Done
                // could fire SagaCompleted while the timeout task
                // simultaneously fires SagaFailed, producing duplicate
                // terminal events for the same saga (reagent P1 PR
                // #644 round 1).
                if !self.claim_terminal(saga_id).await {
                    return ApplyOutcome::terminated();
                }
                crate::log(&format!(
                    "[saga] saga_id={} name={} Done — emitting SagaCompleted",
                    saga_id, name
                ));
                if let Some(log) = self.log.as_ref() {
                    if let Err(e) = log.terminate_saga(saga_id, SagaOutcome::Completed) {
                        crate::log(&format!(
                            "[saga] saga_id={} terminate_saga(Completed) log write failed: {}",
                            saga_id, e
                        ));
                    }
                }
                self.emit_completed(saga_id).await;
                ApplyOutcome::terminated()
            }
            SagaAction::Failed { reason } => {
                if !self.claim_terminal(saga_id).await {
                    return ApplyOutcome::terminated();
                }
                crate::log(&format!(
                    "[saga] saga_id={} name={} Failed reason={} — emitting SagaFailed",
                    saga_id, name, reason
                ));
                if let Some(log) = self.log.as_ref() {
                    if let Err(e) = log.terminate_saga(
                        saga_id,
                        SagaOutcome::Failed {
                            reason: reason.clone(),
                        },
                    ) {
                        crate::log(&format!(
                            "[saga] saga_id={} terminate_saga(Failed) log write failed: {}",
                            saga_id, e
                        ));
                    }
                }
                self.emit_failed(saga_id, reason).await;
                ApplyOutcome::terminated()
            }
        }
    }

    /// Atomically remove `saga_id` from `in_flight` and return whether
    /// the caller was the one who removed it (i.e. has the right to
    /// emit the terminal `SagaCompleted` / `SagaFailed`). Idempotent:
    /// a second caller for the same saga_id sees `false`. Used by the
    /// `SagaAction::Done` and `SagaAction::Failed` paths in
    /// `apply_action`, by the per-saga timeout task in `spawn_saga`,
    /// by the `Event::SagaActionFailed` listener in `run_coordinator`,
    /// by the host-send-failure path in `apply_action::IssueCmd::Host`,
    /// and by the eviction path in `run_coordinator`.
    /// (reagent P1 PR #644 round 1.)
    async fn claim_terminal(&self, saga_id: u64) -> bool {
        let mut registry = self.in_flight.lock().await;
        registry.remove(&saga_id).is_some()
    }

    /// Register a fresh saga, calling `start` and applying its first
    /// action. The caller has already determined that the saga
    /// should fire (e.g. matched a trigger event). Returns the saga's
    /// allocated id (logged + bracketed in `SagaStarted`).
    ///
    /// LSD-2 — also writes a `running` row to the durable saga log
    /// before invoking `saga.start()`, and (via `apply_action`)
    /// records the saga's first dispatched step.
    ///
    /// CPD-3 — also arms a per-saga deadline timer. If the saga is
    /// still in_flight when `saga.timeout()` elapses, a background
    /// task force-fails it (`SagaFailed { reason: "saga timeout" }`)
    /// and removes it from the registry. Per spec §3.10. Insert-then-
    /// start ordering is required so `apply_action`'s `claim_terminal`
    /// guard can succeed for immediate-completion sagas (codex P2 PR
    /// #644 round 2).
    async fn spawn_saga(self: &Arc<Self>, saga: Box<dyn Saga>) -> u64 {
        let saga_id = self.next_id();
        let name = saga.name();
        let input = saga.input_snapshot();
        let saga_timeout = saga.timeout();
        crate::log(&format!(
            "[saga] starting saga_id={} name={}",
            saga_id, name
        ));
        // LSD-2 — write the durable `running` row BEFORE
        // `emit_started` so a crash between this point and
        // `apply_action` still leaves a recoverable breadcrumb
        // (LSD-3's walker will upgrade it to `failed_compensation` on
        // next launcher startup). Mirrors srv's `emit_saga_started`
        // ordering: durable log before bus.
        if let Some(log) = self.log.as_ref() {
            if let Err(e) = log.start_saga(saga_id, name, &input) {
                crate::log(&format!(
                    "[saga] saga_id={} start_saga log write failed: {} — continuing (in-memory path remains authoritative for this run)",
                    saga_id, e
                ));
            }
        }
        // Emit SagaStarted FIRST so any subscriber buffering by
        // saga_id sees the bracket open before any per-step events.
        // Mirrors `agentmux-srv::sagas::emit_saga_started` ordering.
        self.emit_started(saga_id, name).await;

        // CPD-3 — insert the saga into in_flight BEFORE calling
        // start(), so that if start() returns SagaAction::Done or
        // SagaAction::Failed immediately, apply_action's terminal
        // paths can claim_terminal successfully and emit the matching
        // SagaCompleted/SagaFailed bracket. Pre-round-3 we started
        // saga.start() before insertion, which broke immediate-
        // completion sagas (claim_terminal returned false → no
        // terminal event → dangling SagaStarted bracket).
        // (codex P2 PR #644 round 2.)
        let action = {
            let mut registry = self.in_flight.lock().await;
            // (codex P1 PR #634) Observability for the known
            // concurrent-correlation limitation. With more than one
            // saga of the same kind in flight, broadcast event
            // routing mis-correlates: the first matching event
            // completes ALL of them. Logged here so operators can
            // spot the pattern in `--diag wrr` output. Closed when
            // CPD-4 adds saga-id event correlation.
            let same_kind_count = registry
                .values()
                .filter(|s| s.saga.name() == name)
                .count();
            if same_kind_count >= 1 {
                crate::log(&format!(
                    "[saga] WARN: starting {} saga_id={} while {} other(s) of same kind in flight; concurrent-correlation limitation may produce premature SagaCompleted events (PR #634 / codex P1 known issue)",
                    name, saga_id, same_kind_count,
                ));
            }
            registry.insert(
                saga_id,
                InFlightSaga {
                    saga,
                    awaiting_step: None,
                    next_step_index: 0,
                },
            );
            // Drive start() while still under the registry lock so
            // we have a mutable reference to the just-inserted saga.
            // start() is non-async and does no I/O — bounded hold time.
            let in_flight_saga = registry.get_mut(&saga_id).expect("just inserted");
            let ctx = SagaCtx { saga_id };
            in_flight_saga.saga.start(&ctx)
        };

        // First step on a fresh saga is index 0. apply_action may
        // remove the saga via claim_terminal (Done/Failed/host-send-
        // error) without re-locking the registry from here.
        let outcome = self.apply_action(saga_id, name, action, 0).await;
        if outcome.in_flight {
            // Update the bookkeeping for the freshly-issued step.
            // If the first action allocated step 0, the saga is now
            // parked awaiting that step's echo event; next allocation
            // is index 1. Otherwise (Wait — saga has no first
            // dispatch yet), keep the counter at 0.
            let mut registry = self.in_flight.lock().await;
            if let Some(in_flight_saga) = registry.get_mut(&saga_id) {
                in_flight_saga.awaiting_step = outcome.awaiting_step;
                in_flight_saga.next_step_index =
                    if outcome.awaiting_step.is_some() { 1 } else { 0 };
            }
            drop(registry);

            // CPD-3 — arm the per-saga deadline timer. If the saga
            // is still in_flight when `saga_timeout` elapses, fail
            // it out. Spawned task captures an Arc clone of the
            // coordinator so it can outlive the saga's deadline.
            // Uses `claim_terminal` so it doesn't race with the
            // normal Done/Failed path. (reagent P1 PR #644 round 1.)
            let coord_for_timeout = Arc::clone(self);
            tokio::spawn(async move {
                tokio::time::sleep(saga_timeout).await;
                if coord_for_timeout.claim_terminal(saga_id).await {
                    crate::log(&format!(
                        "[saga] saga_id={} name={} timed out after {:?} — emitting SagaFailed",
                        saga_id, name, saga_timeout
                    ));
                    let reason = "saga timeout".to_string();
                    if let Some(log) = coord_for_timeout.log.as_ref() {
                        if let Err(e) = log.terminate_saga(
                            saga_id,
                            SagaOutcome::Failed {
                                reason: reason.clone(),
                            },
                        ) {
                            crate::log(&format!(
                                "[saga] saga_id={} timeout terminate_saga(Failed) log write failed: {}",
                                saga_id, e
                            ));
                        }
                    }
                    coord_for_timeout
                        .emit_failed(saga_id, reason)
                        .await;
                }
            });
        }
        saga_id
    }
}

/// LSD-2 — outcome of `apply_action`. Combines the prior `bool` (is
/// the saga still in flight?) with the new `awaiting_step` book-
/// keeping the coordinator needs to call `LauncherSagaLog::finish_step`
/// when the awaited bus event lands.
#[derive(Debug, Clone, Copy)]
struct ApplyOutcome {
    in_flight: bool,
    /// If the action was an `IssueCmd`, the step index it allocated;
    /// the coordinator parks this on the InFlightSaga. `None` for
    /// `Wait` / `Done` / `Failed`.
    awaiting_step: Option<u32>,
}

impl ApplyOutcome {
    fn in_flight_awaiting(step_index: u32) -> Self {
        Self {
            in_flight: true,
            awaiting_step: Some(step_index),
        }
    }
    fn in_flight_no_change() -> Self {
        Self {
            in_flight: true,
            awaiting_step: None,
        }
    }
    fn terminated() -> Self {
        Self {
            in_flight: false,
            awaiting_step: None,
        }
    }
}

/// LSD-2 — short, greppable name for a `Command` dispatched as part
/// of a saga step. Mirrors srv's `command_discriminant_name` in spirit
/// (snake_case strings rather than `Debug` formatting) but prefixes
/// with `issue_cmd_<target>_<discriminant>` so `--diag sagas` output
/// makes the dispatch target obvious without a separate column lookup.
///
/// Falls back to `issue_cmd_<target>_unknown` for variants serde can't
/// stringify (shouldn't happen for the snake_case-tagged Command enum;
/// defensive default).
fn derive_step_name(cmd: &Command, target: PipeTarget) -> String {
    let target_str = match target {
        PipeTarget::LauncherSelf => "launcher_self",
        PipeTarget::Host => "host",
        PipeTarget::Srv => "srv",
    };
    let discriminant = match serde_json::to_value(cmd) {
        Ok(serde_json::Value::Object(map)) => map
            .get("cmd")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        _ => "unknown".to_string(),
    };
    format!("issue_cmd_{}_{}", target_str, discriminant)
}

/// CPD-3 — fill in the `saga_id` field on a host-bound `Command`
/// before dispatch. Sagas construct their `IssueCmd` actions with a
/// placeholder `saga_id: 0` (they don't know their coordinator-
/// allocated id at action-construction time); the coordinator
/// rewrites the field at dispatch time so the host can echo it back
/// on the matching `Report*`.
///
/// Exhaustive match: any host-bound Command variant added later that
/// forgets to plumb `saga_id` will refuse to compile here. Non-host-
/// bound variants (Report*, Identify, etc.) panic loudly because
/// sagas only emit `IssueCmd::Host` for the three host-bound
/// command kinds covered below.
fn inject_saga_id(cmd: Command, saga_id: u64) -> Command {
    match cmd {
        Command::SpawnPoolWindow { .. } => Command::SpawnPoolWindow { saga_id },
        Command::ReapPanes { label, .. } => Command::ReapPanes { label, saga_id },
        Command::DrainPoolIfLast { label, .. } => {
            Command::DrainPoolIfLast { label, saga_id }
        }
        // Defense-in-depth: every non-host-dispatched Command variant
        // (Report*, Identify, etc.) is a coding bug if it reaches
        // this point — the F.5/F.6 sagas only emit IssueCmd::Host
        // for the three command kinds above.
        other => panic!(
            "inject_saga_id called on non-host-bound Command variant: {:?}",
            other
        ),
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

                // CPD-3 — host reported via `Command::ReportSagaActionFailed`
                // that a saga-issued action failed. Launcher reducer
                // translates this into `Event::SagaActionFailed`
                // (CPD-1 schema). Terminate the matching saga
                // immediately rather than waiting for a Report* that
                // won't come.
                if let Event::SagaActionFailed { saga_id, reason, .. } = &event {
                    let saga_id = *saga_id;
                    let reason = reason.clone();
                    // claim_terminal: atomic take-ownership of the
                    // terminal-event slot, preventing duplicate-emission
                    // races with the saga's own Done/Failed path or its
                    // timeout task. (reagent P1 PR #644 round 1.)
                    if coord.claim_terminal(saga_id).await {
                        crate::log(&format!(
                            "[saga] saga_id={} terminating from host SagaActionFailed reason={}",
                            saga_id, reason
                        ));
                        let full_reason = format!("host action failed: {}", reason);
                        if let Some(log) = coord.log.as_ref() {
                            if let Err(e) = log.terminate_saga(
                                saga_id,
                                SagaOutcome::Failed {
                                    reason: full_reason.clone(),
                                },
                            ) {
                                crate::log(&format!(
                                    "[saga] saga_id={} SagaActionFailed terminate_saga(Failed) log write failed: {}",
                                    saga_id, e
                                ));
                            }
                        }
                        coord.emit_failed(saga_id, full_reason).await;
                    }
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
                            .filter(|(_, s)| s.saga.name() == new_kind)
                            .map(|(id, _)| *id)
                            .collect()
                    };
                    for evict_id in evict_ids {
                        // claim_terminal removes from in_flight AND
                        // ensures the timeout task / SagaActionFailed
                        // listener can't double-emit a terminal event
                        // for this saga_id. (reagent P1 PR #644
                        // round 1.)
                        if !coord.claim_terminal(evict_id).await {
                            // Lost the race — another path (timeout,
                            // SagaActionFailed) already terminated.
                            continue;
                        }
                        crate::log(&format!(
                            "[saga] evicting prior {} saga_id={} to make room for new trigger (codex P1 #634 round 3 evict-and-replace)",
                            new_kind, evict_id,
                        ));
                        let evict_reason =
                            "evicted: same-kind saga restarted (codex P1 #634 round 3)";
                        // LSD-2 — record the eviction in the durable
                        // saga row so `--diag sagas` shows it.
                        if let Some(log) = coord.log.as_ref() {
                            if let Err(e) = log.terminate_saga(
                                evict_id,
                                SagaOutcome::Failed {
                                    reason: evict_reason.to_string(),
                                },
                            ) {
                                crate::log(&format!(
                                    "[saga] saga_id={} evict terminate_saga(Failed) log write failed: {}",
                                    evict_id, e
                                ));
                            }
                        }
                        coord.emit_failed(evict_id, evict_reason.to_string()).await;
                    }
                    coord.spawn_saga(saga).await;
                }

                // Step 2 — feed the event into every in-flight saga.
                // Two-pass to avoid holding the registry lock across
                // `apply_action` (which itself locks state to bump
                // version when emitting SagaCompleted/Failed).
                //
                // LSD-2 — when a saga's `on_event` returns anything
                // OTHER than `Wait`, it has consumed its awaited bus
                // event; record the step's success in the durable log
                // (`finish_step(saga_id, awaiting_step, &event)`) before
                // dispatching the next action. The new action then
                // either parks the saga on a fresh step (next index
                // pulled from `next_step_index`) or terminates it.
                struct PendingAction {
                    saga_id: u64,
                    name: &'static str,
                    action: SagaAction,
                    /// Awaited step index that this on_event return
                    /// "consumed" — `Some` only when on_event returned
                    /// non-`Wait`. Caller will `finish_step` on it
                    /// before invoking `apply_action` for the new
                    /// action.
                    consumed_step: Option<u32>,
                    /// Next free step index for this saga; if the new
                    /// action is `IssueCmd`, `apply_action` writes to
                    /// this index.
                    next_idx: u32,
                }
                let actions: Vec<PendingAction> = {
                    let mut in_flight = coord.in_flight.lock().await;
                    let mut out = Vec::new();
                    for (saga_id, in_flight_saga) in in_flight.iter_mut() {
                        let ctx = SagaCtx { saga_id: *saga_id };
                        let action = in_flight_saga.saga.on_event(&event, &ctx);
                        // If the saga consumed its awaited event,
                        // capture the awaited index now and clear it.
                        // `Wait` keeps awaiting_step unchanged.
                        let consumed_step = if matches!(action, SagaAction::Wait) {
                            None
                        } else {
                            in_flight_saga.awaiting_step.take()
                        };
                        let next_idx = in_flight_saga.next_step_index;
                        out.push(PendingAction {
                            saga_id: *saga_id,
                            name: in_flight_saga.saga.name(),
                            action,
                            consumed_step,
                            next_idx,
                        });
                    }
                    out
                };
                for pending in actions {
                    let PendingAction {
                        saga_id,
                        name,
                        action,
                        consumed_step,
                        next_idx,
                    } = pending;
                    // LSD-2 — record the awaited step's success in
                    // the durable log. `event` is the event that
                    // caused the saga to advance.
                    if let Some(idx) = consumed_step {
                        if let Some(log) = coord.log.as_ref() {
                            if let Err(e) = log.finish_step(saga_id, idx, &event) {
                                crate::log(&format!(
                                    "[saga] saga_id={} finish_step log write failed: {}",
                                    saga_id, e
                                ));
                            }
                        }
                    }
                    let issued_cmd = matches!(action, SagaAction::IssueCmd { .. });
                    let outcome = coord.apply_action(saga_id, name, action, next_idx).await;
                    if !outcome.in_flight {
                        coord.in_flight.lock().await.remove(&saga_id);
                    } else if let Some(awaited) = outcome.awaiting_step {
                        // Update the saga's bookkeeping to reflect
                        // the freshly-issued step.
                        let mut registry = coord.in_flight.lock().await;
                        if let Some(in_flight_saga) = registry.get_mut(&saga_id) {
                            in_flight_saga.awaiting_step = Some(awaited);
                            // Bump only if this dispatch consumed the
                            // pre-allocated index (always true for
                            // IssueCmd; defensive guard).
                            if issued_cmd {
                                in_flight_saga.next_step_index = awaited + 1;
                            }
                        }
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
                *counts.entry(s.saga.name()).or_insert(0) += 1;
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

    // ---------- LSD-2 — coordinator <-> LauncherSagaLog wiring ------
    //
    // Three end-to-end tests that drive the coordinator with a real
    // in-memory `LauncherSagaLog` and assert lifecycle rows land in
    // the expected state. Mirror the test inventory pinned in
    // `docs/specs/SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md` §4 PR2.
    //
    // The tests use `pool_respawn::PoolRespawn` because its single-
    // step shape (one IssueCmd, one terminal echo) keeps assertions
    // tight; the `window_cleanup_cascade` two-step shape is exercised
    // implicitly by the existing F.6 coordinator tests + adds a
    // multi-step example in `saga_step_finish_records_event_payload`.

    use crate::saga::log::LauncherSagaLog;
    use std::sync::Arc as StdArc;

    /// Helper: spin up coordinator + in-memory log.
    fn spawn_coord_with_log()
        -> (StdArc<SagaCoordinator>, StdArc<LauncherSagaLog>, tokio::sync::broadcast::Sender<Event>)
    {
        let (events_tx, _) = tokio::sync::broadcast::channel::<Event>(256);
        let state = Arc::new(tokio::sync::Mutex::new(crate::state::State::default()));
        let log = StdArc::new(LauncherSagaLog::open_in_memory().expect("in-memory log"));
        let coord = StdArc::new(
            SagaCoordinator::new(events_tx.clone(), Arc::clone(&state))
                .with_log(StdArc::clone(&log))
                .expect("with_log on fresh in-memory log should succeed"),
        );
        (coord, log, events_tx)
    }

    /// LSD-2 — drive a saga (PoolRespawn) through to completion and
    /// verify the durable log records:
    ///   - `launcher_saga` row with state='completed', non-null
    ///     ended_at, no failure_reason, input_json round-trips the
    ///     `promoted_label`.
    ///   - one step row in 'succeeded' state with output_json
    ///     populated.
    #[tokio::test]
    async fn saga_completes_writes_lifecycle_to_log() {
        let (coord, log, events_tx) = spawn_coord_with_log();
        let coord_rx = events_tx.subscribe();
        let _handle = tokio::spawn(run_coordinator(StdArc::clone(&coord), coord_rx));
        tokio::task::yield_now().await;

        // Trigger the saga + give it the awaited terminal event.
        let _ = events_tx.send(Event::PoolWindowPromoted {
            label: "window-pool-abc".into(),
            version: 1,
        });
        let _ = events_tx.send(Event::PoolWindowAdded {
            label: "window-pool-xyz".into(),
            version: 2,
            saga_id: None,
        });

        // Settle: coordinator runs apply_action which awaits state
        // mutex + bus send. 200ms is generous for the in-process loop.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let snapshots = log.snapshot_recent(10).expect("snapshot_recent");
        assert_eq!(snapshots.len(), 1, "expected exactly one saga row");
        let s = &snapshots[0];
        assert_eq!(s.name, "pool_respawn_on_promote");
        assert_eq!(s.state, "completed");
        assert!(s.ended_at.is_some(), "ended_at must be set on completion");
        assert!(s.failure_reason.is_none());
        assert_eq!(s.step_count, 1, "expected exactly one succeeded step");
        // Input snapshot round-trips the saga's constructor arg.
        let parsed: serde_json::Value = serde_json::from_str(&s.input_json).unwrap();
        assert_eq!(parsed["promoted_label"], "window-pool-abc");
    }

    /// LSD-2 — when a saga is evicted by the same-kind concurrent
    /// gate (codex P1 PR #634), the durable log records its terminal
    /// state as 'failed' with a failure_reason. The replacement saga
    /// gets its own row.
    #[tokio::test]
    async fn saga_fails_writes_failed_state() {
        let (coord, log, events_tx) = spawn_coord_with_log();
        let coord_rx = events_tx.subscribe();
        let _handle = tokio::spawn(run_coordinator(StdArc::clone(&coord), coord_rx));
        tokio::task::yield_now().await;

        // First promote — kicks off saga A.
        let _ = events_tx.send(Event::PoolWindowPromoted {
            label: "window-pool-a".into(),
            version: 1,
        });
        // Second promote BEFORE A's terminal arrives — evicts A,
        // starts saga B. Saga A's durable row must transition to
        // 'failed' with the eviction reason.
        let _ = events_tx.send(Event::PoolWindowPromoted {
            label: "window-pool-b".into(),
            version: 2,
        });
        // Saga B's terminal so it completes (don't leave it dangling
        // — guards against the test's cleanup masking a real bug).
        let _ = events_tx.send(Event::PoolWindowAdded {
            label: "window-pool-c".into(),
            version: 3,
            saga_id: None,
        });

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let snapshots = log.snapshot_recent(10).expect("snapshot_recent");
        assert_eq!(snapshots.len(), 2, "expected two saga rows (evicted + replacement)");
        // One must be 'failed' (evicted), the other 'completed' (B).
        let states: Vec<_> = snapshots.iter().map(|s| s.state.as_str()).collect();
        assert!(states.contains(&"failed"), "missing 'failed' row: {:?}", states);
        assert!(states.contains(&"completed"), "missing 'completed' row: {:?}", states);
        let failed_row = snapshots.iter().find(|s| s.state == "failed").unwrap();
        assert!(
            failed_row
                .failure_reason
                .as_deref()
                .map(|r| r.contains("evicted"))
                .unwrap_or(false),
            "evicted saga's failure_reason should mention 'evicted', got {:?}",
            failed_row.failure_reason,
        );
    }

    /// LSD-2 — when a saga's `on_event` consumes its awaited bus
    /// event, the coordinator calls `LauncherSagaLog::finish_step`
    /// with the event payload. To inspect `output_json` directly via
    /// the public LSD-1 API, we drive a saga only PART of the way —
    /// the cleanup cascade's Step 1 lands its echo (`PanesReaped`)
    /// but Step 2 is left in `pending` so the saga stays unresolved.
    /// `unresolved_sagas` then exposes the full step list including
    /// Step 0's `output_json`.
    #[tokio::test]
    async fn saga_step_finish_records_event_payload() {
        let (coord, log, events_tx) = spawn_coord_with_log();
        let coord_rx = events_tx.subscribe();
        let _handle = tokio::spawn(run_coordinator(StdArc::clone(&coord), coord_rx));
        tokio::task::yield_now().await;

        // Trigger the cascade. Window-cleanup has two steps; we feed
        // only the first echo (`PanesReaped`) so Step 0 transitions
        // pending→succeeded but Step 1 (`DrainPoolIfLast`) stays
        // pending — the saga remains in_flight + queryable as
        // unresolved.
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
        // INTENTIONALLY no `PoolDrained` / `PoolNotLast`.

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let unresolved = log.unresolved_sagas().expect("unresolved_sagas");
        assert_eq!(unresolved.len(), 1, "expected one unresolved saga");
        let saga = &unresolved[0];
        assert_eq!(saga.name, "window_cleanup_cascade");
        assert_eq!(saga.state, "running");
        // Two step rows: Step 0 succeeded (PanesReaped echo), Step 1
        // pending (DrainPoolIfLast dispatched but no echo arrived).
        assert_eq!(saga.steps.len(), 2, "expected 2 step rows");
        let step0 = &saga.steps[0];
        assert_eq!(step0.step_index, 0);
        assert_eq!(step0.state, "succeeded");
        assert!(step0.ended_at.is_some(), "succeeded step needs ended_at");
        let output_json = step0
            .output_json
            .as_ref()
            .expect("output_json must be set after finish_step");
        // Round-trip — the JSON should deserialize to the awaited Event.
        let parsed: Event = serde_json::from_str(output_json).expect("event round-trips");
        match parsed {
            Event::PanesReaped { label, .. } => assert_eq!(label, "main"),
            other => panic!("expected PanesReaped, got {:?}", other),
        }
        let step1 = &saga.steps[1];
        assert_eq!(step1.step_index, 1);
        assert_eq!(step1.state, "pending");
        assert!(step1.output_json.is_none(), "pending step has no output");
    }
}
