// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The countdown for an approved sensitive request
//! (`SPEC_SUPER_AGENT_2026_10_25.md` §5), beside `pending_shutdown`'s window for
//! stops and closes. `open` starts the clock; the person can veto it, or approve
//! it now to skip the wait. No click is needed to let it proceed: at the deadline
//! the operation runs as the super agent's approved act on behalf of the
//! requester, through the runner registered for its operation. Each operation's
//! own code path runs unchanged, so no new power is added; only who may trigger
//! it changes.
//!
//! A veto or an approve-now is the person's, and only the person's: the caller
//! of [`veto`] and [`approve_now`] has already proven the AgentMux window (the
//! RPC checks `operator_only`). The banner, toast and chime that tell the
//! person arrive with the next step of the spec's §9; the events here carry
//! what they show.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use serde::Serialize;

use crate::backend::sensitive_requests::{self as requests, Request, Status};
use crate::server::AppState;

pub const EVENT_SENSITIVE_PENDING: &str = "sensitive:pending";
pub const EVENT_SENSITIVE_CLEARED: &str = "sensitive:cleared";
/// A settled window stays answerable this long, then is dropped.
const KEEP_FINISHED_MS: i64 = 10 * 60 * 1000;

/// A countdown as the person's banner shows it.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct SensitiveView {
    pub request_id: String,
    pub op: String,
    /// What will happen, built by AgentMux from the request's facts.
    pub summary: String,
    /// The agent that asked.
    pub requester: String,
    /// The super agent that approved it; absent when none reviewed it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub approver: Option<String>,
    #[ts(type = "number")]
    pub deadline_ms: i64,
    /// `pending` → `vetoed`, or `proceeding` → `done` / `failed`.
    #[ts(type = r#""pending" | "vetoed" | "proceeding" | "done" | "failed""#)]
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
}

/// `sensitive:cleared`: the countdown ended, and how.
#[derive(Clone, Debug, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct SensitiveClearedEvent {
    pub request_id: String,
    pub outcome: String,
}

struct Entry {
    view: SensitiveView,
    wake: Arc<tokio::sync::Notify>,
    /// The person approved it now: run without waiting out the window.
    now: bool,
    finished_at_ms: Option<i64>,
}

static ENTRIES: LazyLock<Mutex<HashMap<String, Entry>>> = LazyLock::new(Default::default);

fn entries() -> std::sync::MutexGuard<'static, HashMap<String, Entry>> {
    ENTRIES.lock().unwrap_or_else(|e| e.into_inner())
}

/// What runs an operation when its window runs out.
pub type RunFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;
pub type Runner = Arc<dyn Fn(AppState, Request) -> RunFuture + Send + Sync>;

static RUNNERS: LazyLock<Mutex<HashMap<&'static str, Runner>>> = LazyLock::new(Default::default);

/// Say what runs `op` when its window runs out.
pub fn register_runner(op: &'static str, runner: Runner) {
    RUNNERS.lock().unwrap_or_else(|e| e.into_inner()).insert(op, runner);
}

fn runner_for(op: &str) -> Option<Runner> {
    RUNNERS.lock().unwrap_or_else(|e| e.into_inner()).get(op).cloned()
}

fn now_ms() -> i64 {
    agentmux_common::time::now_ms()
}

pub fn status(request_id: &str) -> Option<SensitiveView> {
    entries().get(request_id).map(|e| e.view.clone())
}

/// The countdowns still running, for a window that opens during one.
pub fn pending() -> Vec<SensitiveView> {
    entries().values().filter(|e| e.view.status == "pending").map(|e| e.view.clone()).collect()
}

/// Start the countdown for the approved request `request_id`, running `window`.
/// One already counting for the request is returned as it is: the clock never restarts.
pub fn open(state: &AppState, request_id: &str, window: Duration) -> Result<SensitiveView, String> {
    let request = requests::registry().get(request_id).ok_or_else(|| format!("no request {request_id}"))?;
    if request.status != Status::Pending {
        return Err(format!("request {request_id} has not been approved"));
    }
    let view = {
        let mut map = entries();
        let now = now_ms();
        map.retain(|_, e| e.finished_at_ms.is_none_or(|t| now - t < KEEP_FINISHED_MS));
        if let Some(existing) = map.get(request_id) {
            return Ok(existing.view.clone());
        }
        let view = SensitiveView {
            request_id: request.id.clone(),
            op: request.op.clone(),
            summary: request.summary.clone(),
            requester: request.requester_slug.clone(),
            approver: request.decided_by.clone(),
            deadline_ms: now + window.as_millis() as i64,
            status: "pending",
            error: None,
        };
        map.insert(
            request.id.clone(),
            Entry { view: view.clone(), wake: Arc::new(tokio::sync::Notify::new()), now: false, finished_at_ms: None },
        );
        view
    };
    state.broker.publish_event::<crate::backend::mps_events::SensitivePending>(&view, vec![], 1);
    tracing::info!(request_id, op = %view.op, requester = %view.requester, "sensitive request: the person has {} s to veto it", window.as_secs());

    let wake = entries().get(request_id).map(|e| e.wake.clone());
    let (state, v) = (state.clone(), view.clone());
    tokio::spawn(async move {
        if let Some(wake) = wake {
            tokio::select! {
                _ = tokio::time::sleep(window) => {}
                _ = wake.notified() => {}
            }
        }
        // Exactly one of the window running out and a veto wins.
        if !begin(&v.request_id) {
            return;
        }
        cleared(&state, &v.request_id, "proceeding");
        let outcome = run(&state, &v.request_id).await;
        let (status, error, end) = match outcome {
            Ok(()) => ("done", None, Status::Done),
            Err(e) => ("failed", Some(e), Status::Failed),
        };
        let _ = requests::registry().finish(&v.request_id, end, now_ms());
        set_status(&v.request_id, status, error.clone());
        audit(&state, &v, "sensitive.run", error.is_none(), error.as_deref(), status);
    });
    Ok(view)
}

/// `pending` → `proceeding`, atomically.
fn begin(request_id: &str) -> bool {
    let mut map = entries();
    match map.get_mut(request_id) {
        Some(e) if e.view.status == "pending" => {
            e.view.status = "proceeding";
            true
        }
        _ => false,
    }
}

fn set_status(request_id: &str, status: &'static str, error: Option<String>) {
    if let Some(e) = entries().get_mut(request_id) {
        e.view.status = status;
        e.view.error = error;
        e.finished_at_ms = Some(now_ms());
    }
}

async fn run(state: &AppState, request_id: &str) -> Result<(), String> {
    let request = requests::registry().get(request_id).ok_or_else(|| format!("no request {request_id}"))?;
    let runner = runner_for(&request.op).ok_or_else(|| format!("nothing runs {} yet", request.op))?;
    runner(state.clone(), request).await
}

/// The person vetoes the countdown: `vetoed`, or `too_late` once it has begun
/// or ended.
pub fn veto(state: &AppState, request_id: &str) -> &'static str {
    let view = {
        let mut map = entries();
        match map.get_mut(request_id) {
            Some(e) if e.view.status == "pending" => {
                // Decided here, under the lock; the window task only wakes to
                // find it no longer pending.
                e.view.status = "vetoed";
                e.finished_at_ms = Some(now_ms());
                e.wake.notify_one();
                e.view.clone()
            }
            _ => return "too_late",
        }
    };
    let _ = requests::registry().finish(request_id, Status::Vetoed, now_ms());
    cleared(state, request_id, "vetoed");
    audit(state, &view, "sensitive.veto", true, None, "vetoed");
    tracing::info!(request_id, "sensitive request: vetoed by the person");
    "vetoed"
}

/// The person approves it now, skipping the rest of the wait: `approved_now`,
/// or `too_late` once it has begun or ended.
pub fn approve_now(request_id: &str) -> &'static str {
    let mut map = entries();
    match map.get_mut(request_id) {
        Some(e) if e.view.status == "pending" && !e.now => {
            e.now = true;
            e.wake.notify_one();
            "approved_now"
        }
        _ => "too_late",
    }
}

fn cleared(state: &AppState, request_id: &str, outcome: &str) {
    let data = SensitiveClearedEvent { request_id: request_id.to_string(), outcome: outcome.to_string() };
    state.broker.publish_event::<crate::backend::mps_events::SensitiveCleared>(&data, vec![], 1);
}

/// One audit row for a step of the window: the person's veto, or the run.
fn audit(state: &AppState, v: &SensitiveView, action: &str, ok: bool, error: Option<&str>, outcome: &str) {
    let source = if action == "sensitive.veto" { "operator" } else { v.approver.as_deref().unwrap_or("no super agent") };
    let note = format!("{}: {} | requested by {} | {outcome}", v.op, v.summary, v.requester);
    state.reactive_handler.log_fleet_action_audit(Some(source), &v.requester, "", action, ok, error, &v.request_id, Some(&note));
}

#[cfg(test)]
#[path = "sensitive_window_tests.rs"]
mod tests;
