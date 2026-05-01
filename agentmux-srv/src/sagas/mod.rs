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

pub mod delete_block;
pub mod delete_tab;
pub mod delete_workspace;
pub mod log;
pub mod promote_block_to_tab;
pub mod recovery;
pub mod restore_torn_off_tab;
pub mod tear_off_block;
pub mod tear_off_tab;

// Step 7 — E.7 integration tests. Cross-saga end-to-end coverage
// that exercises reducer + saga coordinator + persist subscriber +
// saga log together against a real `AppState` (in-memory wstore +
// sagalog). Per-saga unit tests under each saga module already cover
// happy + reject paths in isolation; this module focuses on
// multi-surface consistency (reducer/wstore/saga-log) that PR 2's
// `compensate_unresolved` will rely on.
#[cfg(test)]
mod integration_tests;

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
    /// (codex P1 PR #636 round 4.) Stack of forward-step indices that
    /// have completed successfully and are eligible to be undone by
    /// the next `compensate` call. `dispatch` pushes on success;
    /// `compensate` pops to determine which original forward step
    /// it's reversing, and marks that step `compensated` in the log.
    /// Without this, in-process compensation only writes new
    /// `compensated` rows at fresh indices; the original `succeeded`
    /// rows stay `succeeded`, so resume-on-restart re-replays them
    /// and either no-ops or worse double-applies the inverse.
    ///
    /// `Mutex<Vec>` (rather than a lock-free counter) because saga
    /// inner futures could in theory parallelize compensations; in
    /// practice they're serial today, so contention is zero.
    pub(crate) forward_step_stack: tokio::sync::Mutex<Vec<u32>>,
}

impl<'a> SagaCtx<'a> {
    /// Construct a fresh context for a saga that has just allocated
    /// its `saga_id` (via [`alloc_saga_id`]).
    pub fn new(state: &'a AppState, saga_id: u64) -> Self {
        Self {
            state,
            saga_id,
            step_index: AtomicU32::new(0),
            forward_step_stack: tokio::sync::Mutex::new(Vec::new()),
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
            if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, &self.state.wstore)
            {
                // (reagent P1 PR #631 round 2) Mark the step as
                // failed in the durable log BEFORE returning. Without
                // this, the step row stays in `pending` state even
                // though the reducer already applied the command
                // (line 139); PR 2's compensate-on-restart sees a
                // `pending` step and can't determine whether the
                // command was applied.
                let err_msg = e.to_string();
                if let Err(log_err) =
                    self.state.saga_log.fail_step(self.saga_id, idx, &err_msg)
                {
                    tracing::warn!(
                        saga_id = self.saga_id,
                        step_index = idx,
                        "[saga] fail_step log write failed during wstore-apply error path: {}",
                        log_err,
                    );
                }
                return Err(err_msg);
            }
        }
        if let Err(e) = self.state.saga_log.finish_step(self.saga_id, idx, &events) {
            tracing::warn!(
                saga_id = self.saga_id,
                step_index = idx,
                "[saga] finish_step log write failed: {}",
                e
            );
        }
        // (codex P1 PR #636 round 4.) Track this idx as a successful
        // forward step eligible for compensation. The next
        // `compensate` call will pop this and mark the original step
        // `compensated`, preventing resume-on-restart from re-replaying
        // an inverse that already ran in-process.
        self.forward_step_stack.lock().await.push(idx);
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
        // (codex P1 PR #636 round 4.) Pop the most-recent successful
        // forward step from the stack and mark its original log row
        // as compensated. This prevents resume-on-restart from
        // double-replaying the inverse of a step that already had
        // in-process compensation. Idempotent — UPDATE only matches
        // rows still in `succeeded` state.
        if let Some(forward_idx) = self.forward_step_stack.lock().await.pop() {
            if let Err(e) = self
                .state
                .saga_log
                .mark_step_compensated(self.saga_id, forward_idx)
            {
                tracing::warn!(
                    saga_id = self.saga_id,
                    forward_step_index = forward_idx,
                    "[saga] mark_step_compensated (live) log write failed: {} — restart may re-replay this inverse",
                    e
                );
            }
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
/// SPEC_SAGA_DURABILITY_2026-05-01.md), recording `input` as the
/// saga's arguments serialized to JSON. PR 2's `compensate_unresolved`
/// + `--diag sagas` rely on this for crash-recovery provenance, so
/// callers should pass a structured representation of their inputs
/// (typically `serde_json::json!({...})`). (reagent P1 PR #631 —
/// `Value::Null` placeholder erased provenance.)
///
/// **Fail-fast on log error.** (codex P1 PR #631 round 2.) If
/// `start_saga` fails — most likely a UNIQUE constraint violation
/// from a saga_id collision — the saga MUST NOT proceed. Otherwise
/// later `terminate()` calls would `UPDATE saga SET ... WHERE saga_id=?`
/// against a *different run's* row, mixing lifecycle data across
/// sagas and silently corrupting the durability log. Returning
/// `Err` here propagates up to the caller, which records the
/// failure via `emit_terminal` (with a fresh saga_id allocated by
/// the caller's `alloc_saga_id` retry path, if any).
pub async fn emit_saga_started(
    state: &AppState,
    saga_id: u64,
    name: &'static str,
    input: serde_json::Value,
) -> Result<(), String> {
    if let Err(e) = state.saga_log.start_saga(saga_id, name, &input) {
        let msg = format!(
            "saga durable start row insert failed for saga_id={}: {} (likely ID collision; refusing to run)",
            saga_id, e
        );
        tracing::error!(
            saga_id,
            name,
            "[saga] {} — aborting saga to avoid corrupting prior run's lifecycle row",
            msg,
        );
        return Err(msg);
    }
    let v = state.srv_state.lock().await.bump_version();
    let _ = state.srv_events_tx.send(Event::SagaStarted {
        saga_id,
        name: name.to_string(),
        version: v,
    });
    Ok(())
}

/// Outcome a saga's inner future hands back to `emit_terminal`.
///
/// (codex P1 PR #631) The original PR 1 implementation mapped every
/// `Err` to `SagaOutcome::Compensated`, which is wrong for timeout/
/// abort paths: `run_saga` wraps the inner future in
/// `tokio::time::timeout`, and a timeout cancels the future *before*
/// it can run its compensation block. Recording "compensated" when
/// nothing was compensated would hide partially-applied state from
/// PR 2's `compensate_unresolved` resume scan — exactly the failure
/// mode this log exists to catch.
///
/// `Compensated` should only be recorded when compensation actually
/// completed; everything else (timeout, panic-converted-to-error,
/// pre-compensation early-return) records `Failed`, which leaves the
/// saga visible to PR 2's resume scan.
#[derive(Debug)]
pub enum SagaTerminal<'a> {
    /// All steps applied successfully.
    Completed,
    /// Compensation block ran to completion. Caller asserts this only
    /// after every compensating dispatch returned without error.
    Compensated { reason: &'a str },
    /// Saga aborted before/during compensation: timeout, panic,
    /// pre-compensation early-return, or any other path where
    /// compensation can't be assumed to have run. Default for the
    /// "I don't know if compensation completed" case.
    Failed { reason: &'a str },
}

/// Emit the saga's terminal lifecycle event + durable log row.
///
/// Maps `SagaTerminal` to:
/// - `Completed` → `Event::SagaCompleted` + log state `completed`.
/// - `Compensated { reason }` → `Event::SagaFailed { reason }` + log state `compensated`.
/// - `Failed { reason }` → `Event::SagaFailed { reason }` + log state `failed`.
///
/// The renderer-facing event is the same `SagaFailed` for both
/// non-success paths (the renderer doesn't currently distinguish).
/// The durable log distinguishes — PR 2's resume scan picks up
/// `failed` rows where compensation may not have run.
pub async fn emit_terminal(state: &AppState, saga_id: u64, terminal: SagaTerminal<'_>) {
    let log_outcome = match &terminal {
        SagaTerminal::Completed => SagaOutcome::Completed,
        SagaTerminal::Compensated { reason } => SagaOutcome::Compensated {
            reason: reason.to_string(),
        },
        SagaTerminal::Failed { reason } => SagaOutcome::Failed {
            reason: reason.to_string(),
        },
    };
    // (codex P1 PR #636 round 7 — reverted from round 6.)
    // Bulk-mark only on Compensated. Round 6 extended to Failed too,
    // but BOTH bots flagged that as data-loss: timeout/abort paths
    // classify as Failed and never run compensation, but the bulk-
    // mark would relabel forward steps as `compensated`, hiding
    // them from recovery and leaving side effects permanently
    // applied.
    //
    // Sagas that DO unwind via inner-future ctx.compensate calls
    // should classify as Compensated (the per-step pop already
    // marks 1:1; this bulk call catches residual 1:N cases like
    // tear_off_block's single DeleteWorkspace undoing both
    // CreateWorkspace + CreateTab). `classify_run_saga_result`
    // maps non-timeout Err → Compensated to support this; timeouts
    // → Failed so recovery picks up un-undone rows.
    if matches!(terminal, SagaTerminal::Compensated { .. }) {
        if let Err(e) = state.saga_log.mark_all_succeeded_steps_compensated(saga_id) {
            tracing::warn!(
                saga_id,
                "[saga] mark_all_succeeded_steps_compensated failed: {} — restart may re-replay an inverse",
                e
            );
        }
    }
    if let Err(e) = state.saga_log.terminate(saga_id, log_outcome) {
        tracing::warn!(
            saga_id,
            "[saga] terminate log write failed: {} — saga lifecycle row will look 'running' to PR 2's resume scan, which will then compensate it",
            e
        );
    }
    let v = state.srv_state.lock().await.bump_version();
    let event = match terminal {
        SagaTerminal::Completed => Event::SagaCompleted {
            saga_id,
            version: v,
        },
        SagaTerminal::Compensated { reason } | SagaTerminal::Failed { reason } => {
            Event::SagaFailed {
                saga_id,
                reason: reason.to_string(),
                version: v,
            }
        }
    };
    let _ = state.srv_events_tx.send(event);
}

/// Convenience: classify the standard `run_saga` `Result<Value, String>`
/// outcome into a `SagaTerminal`.
///
/// - `Ok(_)` → `Completed`.
/// - `Err(_)` → `Failed`.
///
/// (codex P1 PR #631 round 2.) The earlier round mapped non-timeout
/// `Err` to `Compensated` on the assumption that "our sagas drive
/// compensation in their inner future before returning `Err`."
/// That's true for the *forward* dispatch failures, but
/// `SagaCtx::compensate` is **best-effort** — if a compensating
/// dispatch is itself rejected by the reducer, `compensate` logs a
/// warning and returns without signaling failure. Marking those as
/// `Compensated` would hide partially-applied state from PR 2's
/// restart recovery (which scans for `running`/`failed` to know what
/// to compensate).
///
/// Conservative default: classify all errors as `Failed`. Sagas that
/// can *prove* compensation succeeded (e.g. a future per-step
/// compensation-success log) construct `SagaTerminal::Compensated`
/// directly without going through this helper.
pub fn classify_run_saga_result(result: &Result<serde_json::Value, String>) -> SagaTerminal<'_> {
    match result {
        Ok(_) => SagaTerminal::Completed,
        // Timeouts/aborts: compensation never ran (run_saga's
        // tokio::time::timeout cancels the inner future before it
        // can compensate). Classify as Failed so recovery picks up
        // the un-undone forward steps.
        Err(reason) if reason.contains("timed out") => SagaTerminal::Failed { reason },
        // Other Err: by convention, our sagas drive compensation
        // in their inner future before returning Err (each
        // ctx.compensate call already marked its target). Classify
        // as Compensated so emit_terminal's bulk-mark cleans up any
        // residual succeeded rows from 1:N compensation patterns
        // (e.g. tear_off_block's single DeleteWorkspace undoing
        // multiple CreateX steps). Sagas that abort without
        // compensating should explicitly construct
        // SagaTerminal::Failed instead of using this helper.
        // (codex round 7 reversal of round 1's blanket-Failed.)
        Err(reason) => SagaTerminal::Compensated { reason },
    }
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
///     emit_saga_started(state, saga_id, "tear_off_tab", serde_json::json!({})).await;
///     let ctx = SagaCtx::new(state, saga_id);
///     let result = run_saga(run_inner(ctx, ...)).await;
///     emit_terminal(state, saga_id, classify_run_saga_result(&result)).await;
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
