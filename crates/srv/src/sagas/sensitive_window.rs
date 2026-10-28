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
//! A veto or an approve-now is the person's, and only the person's: the RPC
//! below accepts them only from the proven AgentMux window (`operator_only`),
//! never from an agent. The banner, toast and chime that tell the person arrive
//! with the next steps of the spec's §9; the events here carry what they show.
//!
//! The `Sensitive` routes start a countdown when the super agent approves. The
//! runners, the code that does each operation, arrive with the later steps of
//! that spec's §9; an operation with none is not approved.
#![allow(dead_code)]

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::sensitive_ops;
use crate::backend::sensitive_requests::{self as requests, Request, Status};
use crate::server::AppState;
use agentmux_common::time::now_ms;

pub const EVENT_SENSITIVE_PENDING: &str = "sensitive:pending";
pub const EVENT_SENSITIVE_CLEARED: &str = "sensitive:cleared";
/// The person's veto or approve-now of a countdown.
pub const COMMAND_SENSITIVE_DECIDE: &str = "sensitive.decide";
/// A settled window stays answerable this long, then is dropped.
const KEEP_FINISHED_MS: i64 = 10 * 60 * 1000;
/// How many of each event are kept for a window that opens mid-countdown: more than the
/// countdowns that can overlap in the longest wait.
const REPLAY_EVENTS: usize = 32;

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
    /// When the countdown began. A window that connects later is replayed the countdowns still
    /// running; this is what tells those from a new one.
    #[ts(type = "number")]
    pub opened_ms: i64,
    /// `pending` → `vetoed` or `cancelled`, or `proceeding` → `done` / `failed`.
    #[ts(type = r#""pending" | "vetoed" | "cancelled" | "proceeding" | "done" | "failed""#)]
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

/// Data for `sensitive.decide`: the countdown's `request_id` (from `sensitive:pending`).
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandSensitiveDecideData {
    pub request_id: String,
    #[ts(type = r#""veto" | "approve_now""#)]
    pub decision: String,
}

/// How the person's decision came out: `too_late` when the countdown had begun or ended.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct SensitiveDecideResult {
    #[ts(type = r#""vetoed" | "approved_now" | "too_late""#)]
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

/// Take back what `register_runner` said, for a test of an operation nothing runs.
#[cfg(test)]
pub fn unregister_runner(op: &str) {
    RUNNERS.lock().unwrap_or_else(|e| e.into_inner()).remove(op);
}

/// Whether something runs `op` when its window runs out.
pub fn has_runner(op: &str) -> bool {
    runner_for(op).is_some()
}

fn runner_for(op: &str) -> Option<Runner> {
    RUNNERS.lock().unwrap_or_else(|e| e.into_inner()).get(op).cloned()
}

pub fn status(request_id: &str) -> Option<SensitiveView> {
    entries().get(request_id).map(|e| e.view.clone())
}

/// `sensitive.decide`: only the AgentMux window, proven, may veto or approve a
/// countdown now. An agent's connection is refused whatever it holds.
pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_typed(COMMAND_SENSITIVE_DECIDE, move |cmd: CommandSensitiveDecideData, ctx| {
        let state = state.clone();
        async move {
            ctx.operator_only("only the person can veto or approve a sensitive request")?;
            let outcome = match cmd.decision.as_str() {
                "veto" => veto(&state, &cmd.request_id),
                "approve_now" => approve_now(&cmd.request_id),
                other => return Err(format!("INVALID_ARG: decision is 'veto' or 'approve_now', not '{}'", other.chars().take(20).collect::<String>())),
            };
            Ok(SensitiveDecideResult { outcome: outcome.to_string() })
        }
    });
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
            opened_ms: now,
            status: "pending",
            error: None,
        };
        map.insert(
            request.id.clone(),
            Entry { view: view.clone(), wake: Arc::new(tokio::sync::Notify::new()), now: false, finished_at_ms: None },
        );
        view
    };
    state.broker.publish_event::<crate::backend::mps_events::SensitivePending>(&view, vec![], REPLAY_EVENTS);
    raise_toast(state, &view);
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
        // A request withdrawn in the last moment is not run.
        let request = match requests::registry().start(&v.request_id) {
            Ok(request) => request,
            Err(_) => {
                set_status(&v.request_id, "cancelled", None);
                cleared(&state, &v.request_id, "cancelled");
                return;
            }
        };
        cleared(&state, &v.request_id, "proceeding");
        let outcome = run(&state, request).await;
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

/// Run the operation in a task of its own, so one that panics settles the
/// window as failed instead of leaving it counted as under way.
async fn run(state: &AppState, request: Request) -> Result<(), String> {
    let runner = runner_for(&request.op).ok_or_else(|| format!("nothing runs {} yet", request.op))?;
    let state = state.clone();
    // The runner is called inside the task too: one that panics before it hands back a future is caught the same way.
    match tokio::spawn(async move { runner(state, request).await }).await {
        Ok(result) => result,
        Err(e) => Err(format!("the operation stopped unexpectedly: {e}")),
    }
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

/// The request was withdrawn (see `Registry::cancel`) while the countdown ran:
/// end it, so the banner stops counting for an operation that will not run.
/// `cancelled`, or `too_late` once it has begun or ended.
pub fn cancelled(state: &AppState, request_id: &str) -> &'static str {
    {
        let mut map = entries();
        match map.get_mut(request_id) {
            Some(e) if e.view.status == "pending" => {
                e.view.status = "cancelled";
                e.finished_at_ms = Some(now_ms());
                e.wake.notify_one();
            }
            _ => return "too_late",
        }
    }
    cleared(state, request_id, "cancelled");
    "cancelled"
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
    if let Some(router) = crate::backend::notify::router::get(&state.broker) {
        router.resolve_nonblocking(&toast_group(request_id), crate::backend::notify::policy::Family::Shutdown);
    }
    let data = SensitiveClearedEvent { request_id: request_id.to_string(), outcome: outcome.to_string() };
    state.broker.publish_event::<crate::backend::mps_events::SensitiveCleared>(&data, vec![], REPLAY_EVENTS);
}

/// The notification group of a countdown's toast: there is no pane, so the request is its own.
pub const TOAST_GROUP_PREFIX: &str = "sensitive:";

fn toast_group(request_id: &str) -> String {
    format!("{TOAST_GROUP_PREFIX}{request_id}")
}

/// What the OS notification says: the title (what will happen, from the operation's table, not
/// from any request text) and the body (who asked and who approved, by display name). The
/// request's facts are left out on purpose: a toast also lands in the Action Center and on a
/// lock screen, and a fact can be a path or an address. The banner in AgentMux shows them.
fn toast_text(requester: &str, approver: Option<&str>, op: &str) -> (String, String) {
    let label = sensitive_ops::spec(op).map_or("A sensitive operation", |s| s.label);
    let mut title = label.to_string();
    if let Some(first) = title.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    let reviewed = match approver {
        Some(a) => format!("Approved by {a}."),
        None => "No super agent reviewed this.".to_string(),
    };
    (title, format!("{requester} asked. {reviewed} It happens when the countdown ends unless you veto it."))
}

/// The OS notification for a countdown: what will happen, the seconds left, and Veto.
fn raise_toast(state: &AppState, view: &SensitiveView) {
    let Some(router) = crate::backend::notify::router::get(&state.broker) else { return };
    let name = |slug: &str| crate::sagas::pending_shutdown::clean_by(&crate::sagas::pending_shutdown::asker_name(state, slug));
    let (title, body) = toast_text(&name(&view.requester), view.approver.as_deref().map(name).as_deref(), &view.op);
    let info = crate::backend::notify::policy::ShutdownInfo {
        deadline_ms: view.deadline_ms,
        effect: crate::backend::notify::policy::SENSITIVE_EFFECT,
        running: true,
        request_id: view.request_id.clone(),
    };
    router.emit_sensitive(&toast_group(&view.request_id), &title, Some(body), info);
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
