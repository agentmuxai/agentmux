// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.5.5 — srv-side saga coordinator.
//
// **Why srv, not launcher:** the existing E.1a coordinator
// framework lives in `agentmux-launcher::saga` (which still does
// nothing — no consumers). The original Phase E spec assumed
// sagas would fan out across host/launcher/srv via cross-process
// IPC; the actual implementation kept that fan-out in the frontend
// (`requestTearOff` calls srv-rpc and host-rpc directly), so every
// saga in the E.5 plan mutates only srv state. In-process oneshot
// dispatch beats an IPC round-trip on every saga step. See
// `docs/retro/saga-coordinator-location-analysis-2026-04-30.md` for
// the full reasoning, including the robustness trade-offs (which
// are the same for both placements).
//
// **Shape:** sagas are async functions that:
//   1. allocate a fresh saga_id via `alloc_saga_id`,
//   2. emit `Event::SagaStarted` via `emit_saga_started`,
//   3. drive their state machine via `SagaCtx::dispatch` /
//      `SagaCtx::compensate`,
//   4. emit `Event::SagaCompleted` or `Event::SagaFailed` via
//      `emit_terminal` once the inner work returns.
//
// `run_saga(state, name, future)` is a thin wrapper that does
// 1+2+4 + applies a 5 s timeout. Sagas pass the future directly
// (not a closure), avoiding the lifetime-of-SagaCtx complication
// that closure-style coordinators run into.
//
// **Compensation:** the saga's inner future is responsible for
// driving compensation before returning `Err`. `SagaCtx::compensate`
// is a best-effort dispatch that swallows errors (the saga is
// already failing; secondary failures get logged). Idempotency of
// compensating commands (`MoveTab` back to source, `DeleteWorkspace`,
// etc.) keeps the cleanup safe even if a step partially applied.
//
// What this module does NOT close (per the location analysis §4.2):
// * Per-step SQLite transactions in the subscriber (gap; F1.A).
// * Host pool-promote and renderer registration outside the saga
//   (gap; Phase F).
// * Saga state across srv restart (gap; Phase F+).

pub mod log;
pub mod promote_block_to_tab;
pub mod restore_torn_off_tab;
pub mod tear_off_block;
pub mod tear_off_tab;

use std::sync::atomic::{AtomicU32, Ordering};

use agentmux_common::ipc::{Command, Event};
use serde_json::Value;

use crate::sagas::log::{command_discriminant_name, SagaOutcome};
use crate::server::AppState;

/// Maximum wall-clock time a saga is allowed to run before the
/// coordinator force-fails it. Tear-off sagas should complete in
/// tens of milliseconds; the budget is generous to absorb SQLite
/// write spikes without flapping in CI.
const SAGA_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Read-only context passed to a saga's inner async function.
/// Wraps the AppState handle and the saga's allocated id.
///
/// Construct via [`SagaCtx::new`] — the durability log requires the
/// per-step counter to start at zero and be owned by the ctx (so
/// concurrent sagas don't interleave step indices).
pub struct SagaCtx<'a> {
    pub(crate) state: &'a AppState,
    pub(crate) saga_id: u64,
    /// Monotonic step index (0, 1, 2, ...) for this saga. Each
    /// `dispatch` / `compensate` call `fetch_add(1)`s and writes the
    /// resulting index into the saga log. Atomic because saga inner
    /// futures may parallelise dispatches in the future (today they
    /// don't, but the cost is one cache line).
    pub(crate) step_index: AtomicU32,
}

impl<'a> SagaCtx<'a> {
    /// Construct a fresh context for a saga that has just allocated
    /// its `saga_id` (via [`alloc_saga_id`]).
    pub fn new(state: &'a AppState, saga_id: u64) -> Self {
        Self {
            state,
            saga_id,
            step_index: AtomicU32::new(0),
        }
    }

    /// Saga-id this context belongs to. Used by sagas that need to
    /// log progress with the saga prefix.
    #[allow(dead_code)]
    pub fn saga_id(&self) -> u64 {
        self.saga_id
    }

    /// Acquire the reducer's state lock for read-only inspection.
    /// Used by sagas that need to inspect post-step state to decide
    /// the next step (e.g. RestoreTornOffTab checking whether the
    /// source workspace is now empty before issuing the cascade
    /// delete). Hold briefly — the reducer is single-mutex.
    pub async fn state_lock(&self) -> tokio::sync::MutexGuard<'_, crate::state::State> {
        self.state.srv_state.lock().await
    }

    /// Dispatch `cmd` through the srv reducer and apply the emitted
    /// events to SQLite + the broadcast bus, exactly like the
    /// in-handler reducer-dispatch helpers.
    ///
    /// Returns the emitted event vec on success. If the reducer
    /// emits any `Event::Error`, the error message is returned and
    /// SQLite/bus side-effects are skipped — the caller must then
    /// dispatch compensation for the saga's already-applied steps.
    pub async fn dispatch(&self, cmd: Command) -> Result<Vec<Event>, String> {
        // Saga durability — write a `pending` step row before
        // dispatch so a crash mid-dispatch leaves a recoverable
        // breadcrumb (PR 2's compensate-on-restart will see it).
        let idx = self.step_index.fetch_add(1, Ordering::Relaxed);
        let step_name = command_discriminant_name(&cmd);
        if let Err(e) = self
            .state
            .saga_log
            .start_step(self.saga_id, idx, &step_name, &cmd)
        {
            // Log-write failure is non-fatal: the in-memory saga
            // path is still authoritative for THIS srv run; we lose
            // crash-recovery for this step, but the user's command
            // shouldn't fail because durability hiccupped.
            tracing::warn!(
                saga_id = self.saga_id,
                step_index = idx,
                "[saga] start_step log write failed: {} — continuing without durable log for this step",
                e
            );
        }

        let events = crate::server::service::dispatch_to_reducer(self.state, cmd).await;
        if let Some(message) = events.iter().find_map(|e| match e {
            Event::Error { message, .. } => Some(message.clone()),
            _ => None,
        }) {
            if let Err(e) = self.state.saga_log.fail_step(self.saga_id, idx, &message) {
                tracing::warn!(
                    saga_id = self.saga_id,
                    step_index = idx,
                    "[saga] fail_step log write failed: {}",
                    e
                );
            }
            return Err(message);
        }
        for ev in &events {
            crate::persist_subscriber::apply_event_to_wstore(ev, &self.state.wstore)
                .map_err(|e| e.to_string())?;
        }
        if let Err(e) = self.state.saga_log.finish_step(self.saga_id, idx, &events) {
            tracing::warn!(
                saga_id = self.saga_id,
                step_index = idx,
                "[saga] finish_step log write failed: {}",
                e
            );
        }
        crate::server::service::publish_events(self.state, &events);
        Ok(events)
    }

    /// Best-effort compensating dispatch. Same as `dispatch` but
    /// SQLite-write failures are logged and swallowed. Intended for
    /// the unwind path: the saga is already returning an error to
    /// the caller; throwing on cleanup hides the original cause and
    /// prevents subsequent compensating commands from running.
    pub async fn compensate(&self, cmd: Command) {
        // Compensation gets its own step row so the durable log
        // distinguishes "step that succeeded forward" from "step
        // that ran in unwind". Index continues monotonically from
        // forward steps so `--diag sagas` shows the full sequence.
        let idx = self.step_index.fetch_add(1, Ordering::Relaxed);
        let step_name = command_discriminant_name(&cmd);
        if let Err(e) = self
            .state
            .saga_log
            .start_step(self.saga_id, idx, &step_name, &cmd)
        {
            tracing::warn!(
                saga_id = self.saga_id,
                step_index = idx,
                "[saga] compensate start_step log write failed: {}",
                e
            );
        }
        let events =
            crate::server::service::dispatch_to_reducer(self.state, cmd.clone()).await;
        if let Some(message) = events.iter().find_map(|e| match e {
            Event::Error { message, .. } => Some(message.clone()),
            _ => None,
        }) {
            tracing::warn!(
                saga_id = self.saga_id,
                "[saga] compensation rejected by reducer: {} (cmd discriminant: {:?})",
                message,
                std::mem::discriminant(&cmd),
            );
            if let Err(e) = self.state.saga_log.fail_step(self.saga_id, idx, &message) {
                tracing::warn!(
                    saga_id = self.saga_id,
                    step_index = idx,
                    "[saga] compensate fail_step log write failed: {}",
                    e
                );
            }
            return;
        }
        for ev in &events {
            if let Err(e) =
                crate::persist_subscriber::apply_event_to_wstore(ev, &self.state.wstore)
            {
                tracing::warn!(
                    saga_id = self.saga_id,
                    "[saga] compensation: SQLite write failed: {}",
                    e
                );
            }
        }
        if let Err(e) = self
            .state
            .saga_log
            .compensate_step(self.saga_id, idx, &events)
        {
            tracing::warn!(
                saga_id = self.saga_id,
                step_index = idx,
                "[saga] compensate_step log write failed: {}",
                e
            );
        }
        crate::server::service::publish_events(self.state, &events);
    }
}

/// Allocate the next saga_id. Monotonic per srv-process run.
pub fn alloc_saga_id(state: &AppState) -> u64 {
    state.saga_id_alloc.fetch_add(1, Ordering::Relaxed) + 1
}

/// Emit `Event::SagaStarted` for a freshly-allocated saga_id.
/// Sagas call this immediately after `alloc_saga_id` so subscribers
/// see the start record before any per-step events.
///
/// Also writes a `running` row to the durable saga log (PR 1 of
/// SPEC_SAGA_DURABILITY_2026-05-01.md). Log-write failures are
/// non-fatal — the in-memory saga path is still authoritative for
/// THIS srv run.
pub async fn emit_saga_started(state: &AppState, saga_id: u64, name: &'static str) {
    if let Err(e) =
        state
            .saga_log
            .start_saga(saga_id, name, &serde_json::Value::Null)
    {
        tracing::warn!(
            saga_id,
            name,
            "[saga] start_saga log write failed: {} — saga continues without durable record",
            e
        );
    }
    let v = state.srv_state.lock().await.bump_version();
    let _ = state.srv_events_tx.send(Event::SagaStarted {
        saga_id,
        name: name.to_string(),
        version: v,
    });
}

/// Emit the saga's terminal lifecycle event. Pass `Ok(())` for
/// success (emits `SagaCompleted`) or `Err(reason)` for failure
/// (emits `SagaFailed`).
///
/// Also writes the terminal row to the durable saga log. The
/// distinction between `Failed` and `Compensated` (spec §2.2) is
/// not visible to in-memory sagas today — the inner future drives
/// compensation before returning `Err`, so by the time we reach
/// here, compensation has already run. We default a non-Ok outcome
/// to `Compensated` since the existing sagas (`tear_off_tab`,
/// `restore_torn_off_tab`, etc.) all emit compensating dispatches
/// before returning errors. Sagas that genuinely fail without
/// compensation can be distinguished in PR 2 once the saga author
/// API exposes the post-compensation outcome explicitly.
pub async fn emit_terminal(state: &AppState, saga_id: u64, outcome: Result<(), &str>) {
    let log_outcome = match outcome {
        Ok(()) => SagaOutcome::Completed,
        Err(reason) => SagaOutcome::Compensated {
            reason: reason.to_string(),
        },
    };
    if let Err(e) = state.saga_log.terminate(saga_id, log_outcome) {
        tracing::warn!(
            saga_id,
            "[saga] terminate log write failed: {} — saga lifecycle row will look 'running' to PR 2's resume scan, which will then compensate it",
            e
        );
    }
    let v = state.srv_state.lock().await.bump_version();
    let event = match outcome {
        Ok(()) => Event::SagaCompleted {
            saga_id,
            version: v,
        },
        Err(reason) => Event::SagaFailed {
            saga_id,
            reason: reason.to_string(),
            version: v,
        },
    };
    let _ = state.srv_events_tx.send(event);
}

/// Run a saga's inner future under a 5 s timeout. The inner future
/// is responsible for emitting `SagaStarted` (the saga itself, since
/// it owns the saga_id allocation) and any compensation it needs;
/// `run_saga` only enforces the timeout and emits the terminal
/// `SagaCompleted` / `SagaFailed`.
///
/// Concrete usage (per saga):
/// ```ignore
/// pub async fn run(state: &AppState, ...) -> Result<Value, String> {
///     let saga_id = alloc_saga_id(state);
///     emit_saga_started(state, saga_id, "tear_off_tab").await;
///     let ctx = SagaCtx::new(state, saga_id);
///     let result = run_saga(run_inner(ctx, ...)).await;
///     emit_terminal(state, saga_id, match &result {
///         Ok(_) => Ok(()),
///         Err(r) => Err(r.as_str()),
///     }).await;
///     result
/// }
/// ```
pub async fn run_saga<Fut>(name: &'static str, fut: Fut) -> Result<Value, String>
where
    Fut: std::future::Future<Output = Result<Value, String>>,
{
    match tokio::time::timeout(SAGA_TIMEOUT, fut).await {
        Ok(r) => r,
        Err(_) => Err(format!("saga '{}' timed out after {:?}", name, SAGA_TIMEOUT)),
    }
}
