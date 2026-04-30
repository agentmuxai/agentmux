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

pub mod restore_torn_off_tab;
pub mod tear_off_block;
pub mod tear_off_tab;

use std::sync::atomic::Ordering;

use agentmux_common::ipc::{Command, Event};
use serde_json::Value;

use crate::server::AppState;

/// Maximum wall-clock time a saga is allowed to run before the
/// coordinator force-fails it. Tear-off sagas should complete in
/// tens of milliseconds; the budget is generous to absorb SQLite
/// write spikes without flapping in CI.
const SAGA_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Read-only context passed to a saga's inner async function.
/// Wraps the AppState handle and the saga's allocated id.
pub struct SagaCtx<'a> {
    pub(crate) state: &'a AppState,
    pub(crate) saga_id: u64,
}

impl<'a> SagaCtx<'a> {
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
        let events = crate::server::service::dispatch_to_reducer(self.state, cmd).await;
        if let Some(message) = events.iter().find_map(|e| match e {
            Event::Error { message, .. } => Some(message.clone()),
            _ => None,
        }) {
            return Err(message);
        }
        for ev in &events {
            crate::persist_subscriber::apply_event_to_wstore(ev, &self.state.wstore)
                .map_err(|e| e.to_string())?;
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
pub async fn emit_saga_started(state: &AppState, saga_id: u64, name: &'static str) {
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
pub async fn emit_terminal(state: &AppState, saga_id: u64, outcome: Result<(), &str>) {
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
///     let ctx = SagaCtx { state, saga_id };
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
