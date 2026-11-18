// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! What a headless server's registered external host asks of srv besides
//! vouching for a connection: whether the workspace is in use, a graceful shutdown, and the notice that it
//! is going to stop.
//!
//! These are host routes, never RPCs: an RPC is a command an agent can call, and these must not be. Every
//! agent on the machine holds the instance key, so the key does not open them. Each call is signed by the
//! registered host (`external_host::Host::check_call`), for this workspace, over its operation and
//! arguments, and answers 404 when no host is registered, so a desktop or a plain headless srv has none of
//! these routes.
//!
//! - `POST /api/v1/host/activity`: `{ busy, agentsWorking, lastOutputAt }`. A turn counts only while its pane
//!   has written output in the last ten minutes (`ACTIVE_WINDOW_MS`), so a hung turn cannot keep a workspace
//!   up for ever; a pane process that wrote that recently counts as busy too.
//! - `POST /api/v1/host/shutdown`: srv's own graceful teardown (the one SIGTERM runs: interrupt turns, save
//!   final state) and the answer when it is done, within the deadline given.
//! - `POST /api/v1/host/notice`: the stop time and why, as the fixed reason ids `person`, `timer`, `credit`
//!   and `cap`, never text: srv builds the phrase the page and the agents are shown, so nothing a person or an
//!   agent wrote can cross. `stopAt` and `reason` both null cancel it.

use std::sync::Mutex;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::external_host::{host, Refusal};
use super::AppState;
use crate::backend::blockcontroller::{get_all_controllers, shell::ShellController, STATUS_RUNNING};
use crate::backend::shutdown_events::WorkspaceStopView;

/// A turn or a pane process counts as working only if it wrote output this recently.
pub(crate) const ACTIVE_WINDOW_MS: i64 = 10 * 60 * 1000;
/// The longest graceful shutdown a caller may ask for.
const MAX_DEADLINE_SECS: u64 = 120;
/// The furthest ahead a stop may be set.
const MAX_AHEAD_MS: u64 = 7 * 24 * 60 * 60 * 1000;
/// How far in the past a stop time may be before it is refused (clock skew between host and srv).
const SKEW_MS: u64 = 60 * 1000;
/// The held-note key: a newer stop notice replaces an older one still waiting for an agent's next turn.
const NOTE_KEY: &str = "workspace-stop";
/// The last reminder to the agents, this long before the stop.
const REMIND_BEFORE_MS: u64 = 60 * 1000;

fn current_ms() -> u64 {
    agentmux_common::time::now_ms_u64()
}

fn refused((status, error): Refusal) -> Response {
    (status, Json(json!({ "ok": false, "error": error }))).into_response()
}

/// What one pane's controller says, for the activity answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Seen {
    pub turn_active: bool,
    pub last_output_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Activity {
    pub busy: bool,
    pub agents_working: usize,
    pub last_output_at: Option<i64>,
}

/// The workspace's activity from its running panes. A turn that has been silent for `ACTIVE_WINDOW_MS` is not
/// working (the guard against a stuck turn); a pane that wrote output within it is busy even with no turn.
pub(crate) fn summarize(running: &[Seen], now_ms: i64) -> Activity {
    let recent = |s: &Seen| s.last_output_ms.is_some_and(|t| now_ms - t <= ACTIVE_WINDOW_MS);
    let agents_working = running.iter().filter(|s| s.turn_active && recent(s)).count();
    Activity {
        busy: agents_working > 0 || running.iter().any(recent),
        agents_working,
        last_output_at: running.iter().filter_map(|s| s.last_output_ms).max(),
    }
}

fn running_panes(now_ms: i64) -> Vec<Seen> {
    get_all_controllers()
        .values()
        .filter_map(|ctrl| {
            let status = ctrl.get_runtime_status();
            if status.shellprocstatus != STATUS_RUNNING {
                return None;
            }
            let last_output_ms = match ctrl.as_any().downcast_ref::<ShellController>() {
                Some(shell) => shell.last_output_secs_ago().map(|secs| now_ms - secs as i64 * 1000),
                None => ctrl.last_output_ms(),
            };
            Some(Seen { turn_active: status.turn_active, last_output_ms })
        })
        .collect()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActivityRequest {
    v: u32,
    workspace: String,
    issued_at: u64,
    sig: String,
}

/// `POST /api/v1/host/activity`.
pub(crate) async fn handle_activity(Json(req): Json<ActivityRequest>) -> Response {
    let Some(host) = host() else { return not_registered() };
    if req.v != 1 {
        return refused((StatusCode::BAD_REQUEST, "unsupported version"));
    }
    if let Err(r) = host.check_call("activity", &req.workspace, req.issued_at, "", &req.sig, agentmux_common::time::now_secs_u64()) {
        return refused(r);
    }
    let now = current_ms() as i64;
    let a = summarize(&running_panes(now), now);
    Json(json!({ "ok": true, "busy": a.busy, "agentsWorking": a.agents_working, "lastOutputAt": a.last_output_at })).into_response()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ShutdownRequest {
    v: u32,
    workspace: String,
    issued_at: u64,
    deadline_seconds: u64,
    sig: String,
}

/// `POST /api/v1/host/shutdown`: srv's graceful teardown, and the answer once it is done (`done: false` when the
/// deadline came first, with the teardown still running). Calling it again is harmless: what is closed is gone.
pub(crate) async fn handle_shutdown(State(state): State<AppState>, Json(req): Json<ShutdownRequest>) -> Response {
    let Some(host) = host() else { return not_registered() };
    if req.v != 1 || !(1..=MAX_DEADLINE_SECS).contains(&req.deadline_seconds) {
        return refused((StatusCode::BAD_REQUEST, "a field is not valid"));
    }
    let args = req.deadline_seconds.to_string();
    if let Err(r) = host.check_call("shutdown", &req.workspace, req.issued_at, &args, &req.sig, agentmux_common::time::now_secs_u64()) {
        return refused(r);
    }
    tracing::info!(deadline_seconds = req.deadline_seconds, "the external host asked for a graceful shutdown");
    match tokio::time::timeout(Duration::from_secs(req.deadline_seconds), crate::sagas::agent_teardown::app_exit(&state)).await {
        Ok(closed) => Json(json!({ "ok": true, "done": true, "closed": closed })).into_response(),
        Err(_) => Json(json!({ "ok": true, "done": false })).into_response(),
    }
}

/// Why a workspace is going to stop: the only reasons srv will say, as ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reason {
    Person,
    Timer { idle_minutes: u32 },
    Credit,
    Cap,
}

impl Reason {
    pub(crate) fn id(self) -> &'static str {
        match self {
            Reason::Person => "person",
            Reason::Timer { .. } => "timer",
            Reason::Credit => "credit",
            Reason::Cap => "cap",
        }
    }

    /// The reason as a person and an agent are told it.
    pub(crate) fn phrase(self) -> String {
        match self {
            Reason::Person => "you asked".to_string(),
            Reason::Timer { idle_minutes } => format!("nothing has happened for {idle_minutes} minutes"),
            Reason::Credit => "your credit ran out".to_string(),
            Reason::Cap => "a daily cap was reached".to_string(),
        }
    }
}

/// A notice as srv holds it: the stop time and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stop {
    pub at_ms: u64,
    pub reason: Reason,
}

/// What the host sent, checked: a stop, or its cancelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Notice {
    Set(Stop),
    Clear,
}

/// The notice in a request, or why it is refused. A stop needs both a time and a reason id; neither cancels.
/// `idleMinutes` goes with `timer` and with nothing else.
pub(crate) fn parse_notice(stop_at: Option<u64>, reason: Option<&str>, idle_minutes: Option<u32>, now_ms: u64) -> Result<Notice, &'static str> {
    let (Some(at_ms), Some(reason)) = (stop_at, reason) else {
        return match (stop_at, reason, idle_minutes) {
            (None, None, None) => Ok(Notice::Clear),
            _ => Err("a stop needs both a time and a reason"),
        };
    };
    if at_ms.saturating_add(SKEW_MS) < now_ms || at_ms > now_ms.saturating_add(MAX_AHEAD_MS) {
        return Err("the stop time is not within a week ahead");
    }
    let reason = match (reason, idle_minutes) {
        ("timer", Some(m)) if (1..=1440).contains(&m) => Reason::Timer { idle_minutes: m },
        ("timer", _) => return Err("a timer stop needs idleMinutes between 1 and 1440"),
        (_, Some(_)) => return Err("idleMinutes goes only with a timer stop"),
        ("person", None) => Reason::Person,
        ("credit", None) => Reason::Credit,
        ("cap", None) => Reason::Cap,
        _ => return Err("the reason is not one of person, timer, credit, cap"),
    };
    Ok(Notice::Set(Stop { at_ms, reason }))
}

/// What to do with a notice, given the one held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Plan {
    /// The same as held (or nothing to cancel): nothing is said again.
    Nothing,
    Announce(Stop),
    Cancel,
}

pub(crate) fn plan(held: Option<Stop>, notice: Notice) -> Plan {
    match (held, notice) {
        (Some(h), Notice::Set(s)) if h == s => Plan::Nothing,
        (_, Notice::Set(s)) => Plan::Announce(s),
        (Some(_), Notice::Clear) => Plan::Cancel,
        (None, Notice::Clear) => Plan::Nothing,
    }
}

/// `HH:MM UTC` of a unix-ms time.
pub(crate) fn clock(at_ms: u64) -> String {
    let secs = at_ms / 1000;
    format!("{:02}:{:02} UTC", (secs / 3600) % 24, (secs / 60) % 60)
}

/// The note every agent is given when a stop is set.
pub(crate) fn stop_note(stop: Stop) -> String {
    format!("This workspace stops at {} because {}. Your conversation is saved.", clock(stop.at_ms), stop.reason.phrase())
}

/// The last reminder, sent a minute before.
pub(crate) fn remind_note(stop: Stop) -> String {
    format!("This workspace stops in about a minute, at {}, because {}. Your conversation is saved.", clock(stop.at_ms), stop.reason.phrase())
}

/// The note when a stop is cancelled.
pub(crate) const CANCEL_NOTE: &str = "The workspace's stop was cancelled; it keeps running.";

/// The notice srv holds, and a counter that tells a reminder task whether it is still the current one.
static HELD: Mutex<(Option<Stop>, u64)> = Mutex::new((None, 0));

/// The stop srv holds now, for what tells the agents.
pub(crate) fn held_stop() -> Option<Stop> {
    HELD.lock().unwrap_or_else(|e| e.into_inner()).0
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NoticeRequest {
    v: u32,
    workspace: String,
    issued_at: u64,
    stop_at: Option<u64>,
    reason: Option<String>,
    idle_minutes: Option<u32>,
    sig: String,
}

fn text_of(v: Option<impl ToString>) -> String {
    v.map_or_else(|| "-".to_string(), |x| x.to_string())
}

fn tell_agents(state: &AppState, text: &str) {
    for agent in crate::backend::reactive::get_global_handler().list_agents() {
        if let Err(e) = state.reactive_handler.hold_system_note(&agent.agent_id, NOTE_KEY, text) {
            tracing::debug!(agent = %agent.agent_id, "workspace stop note not held: {e}");
        }
    }
}

fn publish(state: &AppState, view: &WorkspaceStopView) {
    state.broker.publish_event::<crate::backend::mps_events::WorkspaceStop>(view, vec![], 1);
}

/// `POST /api/v1/host/notice`.
pub(crate) async fn handle_notice(State(state): State<AppState>, Json(req): Json<NoticeRequest>) -> Response {
    let Some(host) = host() else { return not_registered() };
    if req.v != 1 {
        return refused((StatusCode::BAD_REQUEST, "unsupported version"));
    }
    // Signed as `<stopAt>,<reason>,<idleMinutes>`, `-` for a null (a cancel is `-,-,-`).
    let args = format!("{},{},{}", text_of(req.stop_at), text_of(req.reason.as_deref()), text_of(req.idle_minutes));
    if let Err(r) = host.check_call("notice", &req.workspace, req.issued_at, &args, &req.sig, agentmux_common::time::now_secs_u64()) {
        return refused(r);
    }
    let notice = match parse_notice(req.stop_at, req.reason.as_deref(), req.idle_minutes, current_ms()) {
        Ok(n) => n,
        Err(why) => return refused((StatusCode::BAD_REQUEST, why)),
    };
    apply(&state, notice);
    Json(json!({ "ok": true })).into_response()
}

/// Hold the notice, show it, tell the agents, and remind them a minute before; or take it back.
fn apply(state: &AppState, notice: Notice) {
    let generation;
    let what = {
        let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
        let what = plan(held.0, notice);
        match what {
            Plan::Nothing => return,
            Plan::Announce(s) => held.0 = Some(s),
            Plan::Cancel => held.0 = None,
        }
        held.1 += 1;
        generation = held.1;
        what
    };
    match what {
        Plan::Announce(stop) => {
            tracing::info!(reason = stop.reason.id(), stop_at_ms = stop.at_ms, "the external host set a workspace stop");
            publish(state, &WorkspaceStopView { status: "pending", stop_at_ms: stop.at_ms as i64, reason_id: stop.reason.id().to_string(), reason: stop.reason.phrase() });
            tell_agents(state, &stop_note(stop));
            remind_later(state.clone(), stop, generation);
        }
        Plan::Cancel => {
            tracing::info!("the external host cancelled the workspace stop");
            publish(state, &WorkspaceStopView { status: "cleared", stop_at_ms: 0, reason_id: String::new(), reason: String::new() });
            tell_agents(state, CANCEL_NOTE);
        }
        Plan::Nothing => {}
    }
}

/// A minute before the stop, if it is still the stop that is held, the agents are reminded.
fn remind_later(state: AppState, stop: Stop, generation: u64) {
    let wait = stop.at_ms.saturating_sub(REMIND_BEFORE_MS).saturating_sub(current_ms());
    if wait == 0 {
        return;
    }
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(wait)).await;
        if HELD.lock().unwrap_or_else(|e| e.into_inner()).1 == generation {
            tell_agents(&state, &remind_note(stop));
        }
    });
}

fn not_registered() -> Response {
    refused((StatusCode::NOT_FOUND, "no external host is registered"))
}

#[cfg(test)]
#[path = "host_control_tests.rs"]
mod tests;
