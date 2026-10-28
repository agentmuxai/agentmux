// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// External shutdown: 30 seconds (15 for an agent's own quit) for the user to override
// (docs/specs/SPEC_AGENT_SELF_QUIT_2026_09_24.md §6.5, contract §12.2–12.3).
//
// A shutdown the user didn't ask for doesn't happen at once. `request` marks
// the agent pending, tells the user (the pane banner via
// `agent:shutdown-pending`, and an OS notification), and waits
// `OVERRIDE_WINDOW`. `keep` — the banner's "Keep running" — cancels it.
// Otherwise the shutdown goes ahead. The caller is answered at once with a
// request id and reads the outcome from `status`.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use serde::Serialize;

use crate::server::AppState;

/// How long the user has to keep what another agent asked to stop or close: 30 s, the
/// default for a human override.
pub const OVERRIDE_WINDOW: std::time::Duration = std::time::Duration::from_secs(30);
/// An agent's own quit, from a turn the user didn't start: it waits for that turn to end
/// anyway, so its window stays short.
pub const SELF_QUIT_WINDOW: std::time::Duration = std::time::Duration::from_secs(15);
pub const EVENT_SHUTDOWN_PENDING: &str = "agent:shutdown-pending";
pub const EVENT_SHUTDOWN_PENDING_CLEARED: &str = "agent:shutdown-pending-cleared";
/// Finished requests stay answerable this long, then are dropped.
const KEEP_FINISHED_MS: i64 = 10 * 60 * 1000;

/// What happens if the user lets the window run out.
#[derive(Clone, Debug)]
pub enum Action {
    /// The agent's own `QuitSelf` from a turn the user didn't start: the same
    /// self-quit, after its turn ends (§4.2).
    SelfQuit { detail: String },
    /// Another agent's `Stop agents`: end that one agent and close its tab,
    /// as the user's tab × does, keeping the pane's other tabs. No wait for
    /// its turn to end: it is stopped from outside, mid-turn or not
    /// (docs/specs/SPEC_STOP_AN_AGENT_ONE_WAY_AND_SHUTDOWN_TOAST_2026_10_23.md §3.1).
    CloseTab { detail: String },
    /// Another agent's `ClosePane block_id=`: close the whole pane.
    ClosePane { block_ids: Vec<String> },
    /// Stop the controller and keep the block, as `agent.stop` does. Internal
    /// only (the fork slug migration): no tool an agent can call opens one.
    Stop { signal: Option<String> },
    /// Another AgentMux asked this instance to let go of the agent (Take over):
    /// stop it gracefully and leave its lease to the other instance. `to` names
    /// the asking instance, for the pane's "taken over" row.
    Release { to: String },
}

impl Action {
    /// Each does all the one below it does: closing the pane ends the agent
    /// and its siblings, a self-quit stops it and closes its tab.
    fn rank(&self) -> u8 {
        match self {
            Action::Stop { .. } | Action::Release { .. } => 0,
            Action::SelfQuit { .. } | Action::CloseTab { .. } => 1,
            Action::ClosePane { .. } => 2,
        }
    }

    /// How long the user has to keep it.
    fn window(&self) -> std::time::Duration {
        match self {
            Action::SelfQuit { .. } => SELF_QUIT_WINDOW,
            _ => OVERRIDE_WINDOW,
        }
    }

    /// What the user is told will happen: `PendingView::effect`.
    fn effect(&self) -> &'static str {
        match self {
            Action::Stop { .. } | Action::Release { .. } => "stop",
            Action::SelfQuit { .. } | Action::CloseTab { .. } => "close_tab",
            Action::ClosePane { .. } => "close_pane",
        }
    }

    /// Every block this shuts down: the whole pane for a close, else the one.
    fn covers(&self, block_id: &str) -> Vec<String> {
        let mut blocks = vec![block_id.to_string()];
        if let Action::ClosePane { block_ids } = self {
            blocks.extend(block_ids.iter().filter(|b| b.as_str() != block_id).cloned());
        }
        blocks
    }

    /// Folds in a joiner's action of the same kind, so what runs covers
    /// both (ReAgent P2 on #3798): the more forceful stop, every pane asked
    /// for. A self-quit is the same either way; its detail is audit text.
    fn merge(&mut self, other: Action) {
        match (self, other) {
            (Action::Stop { signal }, Action::Stop { signal: theirs }) => {
                let forceful = |s: &Option<String>| matches!(s.as_deref(), Some("SIGKILL") | Some("SIGTERM"));
                if theirs.as_deref() == Some("SIGKILL") || (signal.is_none() && forceful(&theirs)) {
                    *signal = theirs;
                }
            }
            // A release does all a stop does, and hands the lease over too.
            (this, Action::Release { to }) if matches!(this, Action::Stop { .. }) => *this = Action::Release { to },
            (Action::ClosePane { block_ids }, Action::ClosePane { block_ids: theirs }) => {
                for b in theirs {
                    if !block_ids.contains(&b) {
                        block_ids.push(b);
                    }
                }
            }
            _ => {}
        }
    }
}

/// How long a caller should wait, at least, for a request with this deadline.
pub fn wait_at_least_ms(deadline_ms: i64) -> u64 {
    (deadline_ms - now_ms()).max(0) as u64
}

/// A request as the caller and the frontend see it (§12.2).
#[derive(Clone, Debug, Serialize, PartialEq, Eq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct PendingView {
    pub request_id: String,
    pub block_id: String,
    /// Who asked: an agent id, or `cron` etc.
    pub by: String,
    /// The agent being shut down, resolved when asked (it may be gone after).
    pub target: String,
    /// The tool: `QuitSelf`, `ClosePane`, `FleetBulkStop`.
    pub via: String,
    /// Why, in the asker's words, cleaned for display (`clean_reason`): never
    /// the raw text. Shown on the pane banner only; an OS toast leaves it out.
    pub reason: String,
    /// What happens if the window runs out: `stop` (the agent stops, its pane
    /// stays), `close_tab` (it stops and its tab closes) or `close_pane`.
    #[ts(type = r#""stop" | "close_tab" | "close_pane""#)]
    pub effect: &'static str,
    #[ts(type = "number")]
    pub deadline_ms: i64,
    /// `pending` → `kept_by_user`, or `proceeding` → `shut_down` / `failed` /
    /// `superseded` (the user closed it themselves meanwhile).
    #[ts(type = r#""pending" | "kept_by_user" | "proceeding" | "shut_down" | "failed" | "superseded""#)]
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
    /// This caller joined a request already open for the block rather than
    /// opening one. Never stored; set on the view `request` returns.
    #[serde(skip)]
    #[ts(skip)]
    pub joined: bool,
}

/// `agent:shutdown-pending-cleared`: the request ended, and how.
#[derive(Clone, Debug, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct PendingClearedEvent {
    pub block_id: String,
    pub request_id: String,
    pub outcome: String,
}

struct Entry {
    view: PendingView,
    kept: Arc<tokio::sync::Notify>,
    finished_at_ms: Option<i64>,
    /// Taken when the window runs out and the shutdown begins.
    action: Option<Action>,
    /// Everyone who asked, as (by, via, reason as sent): later requests for the
    /// same block join this one rather than open a second window. The audit
    /// keeps each reason exactly as its caller sent it; only `view.reason`, which
    /// the user is shown, is cleaned (`clean_reason`).
    requesters: Vec<(String, String, String)>,
    /// Every block it shuts down (`Action::covers`). A request naming any of
    /// them joins this one, Keep running works from any of their banners,
    /// and the banner shows on each — a pane's close covers all its tabs
    /// (ReAgent P1 on #3798: keyed on the named block alone, two closes
    /// naming two tabs of one pane got two windows, and keeping one didn't
    /// stop the other closing the whole pane).
    covers: Vec<String>,
}

static ENTRIES: LazyLock<Mutex<HashMap<String, Entry>>> = LazyLock::new(Default::default);

/// A request opened on another AgentMux instance on this machine (a
/// cross-channel `FleetBulkStop` target, whose user is asked there): where
/// its status lives. `status` for these is proxied (see `remote_for`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteRequest {
    pub local_url: String,
    pub auth_key: String,
    remembered_at_ms: i64,
}

static REMOTE: LazyLock<Mutex<HashMap<String, RemoteRequest>>> = LazyLock::new(Default::default);

/// Remember that `request_id` lives on the instance at `local_url`.
pub fn remember_remote(request_id: &str, local_url: &str, auth_key: &str) {
    let now = now_ms();
    let mut map = REMOTE.lock().unwrap_or_else(|e| e.into_inner());
    // Long after any window has settled, like finished local requests.
    map.retain(|_, r| now - r.remembered_at_ms < KEEP_FINISHED_MS + OVERRIDE_WINDOW.as_millis() as i64);
    map.insert(
        request_id.to_string(),
        RemoteRequest { local_url: local_url.to_string(), auth_key: auth_key.to_string(), remembered_at_ms: now },
    );
}

/// The instance a forwarded request lives on, if it is one.
pub fn remote_for(request_id: &str) -> Option<RemoteRequest> {
    REMOTE.lock().unwrap_or_else(|e| e.into_inner()).get(request_id).cloned()
}

use agentmux_common::time::now_ms;

fn entries() -> std::sync::MutexGuard<'static, HashMap<String, Entry>> {
    ENTRIES.lock().unwrap_or_else(|e| e.into_inner())
}

fn prune(map: &mut HashMap<String, Entry>, now: i64) {
    map.retain(|_, e| e.finished_at_ms.is_none_or(|t| now - t < KEEP_FINISHED_MS));
}

fn is_active(status: &str) -> bool {
    matches!(status, "pending" | "proceeding")
}

/// The request for this block still waiting on the user or already under
/// way, if any.
pub fn active_for(block_id: &str) -> Option<PendingView> {
    entries()
        .values()
        .find(|e| is_active(e.view.status) && e.covers.iter().any(|b| b == block_id))
        .map(|e| e.view.clone())
}

/// Every block a request covers; empty once it's forgotten.
fn covers_of(request_id: &str) -> Vec<String> {
    entries().get(request_id).map(|e| e.covers.clone()).unwrap_or_default()
}

pub fn status(request_id: &str) -> Option<PendingView> {
    entries().get(request_id).map(|e| e.view.clone())
}

/// `pending` → `proceeding`, atomically, handing over the action to run:
/// exactly one of the window running out and the user's Keep running wins
/// (ReAgent P1 on #3789 — checking and setting under separate locks let a
/// late check see `pending` after a keep).
fn begin(request_id: &str) -> Option<Action> {
    let mut map = entries();
    match map.get_mut(request_id) {
        Some(e) if e.view.status == "pending" => {
            e.view.status = "proceeding";
            e.action.take()
        }
        _ => None,
    }
}

fn set_status(request_id: &str, status: &'static str, error: Option<String>) -> Option<PendingView> {
    let mut map = entries();
    let e = map.get_mut(request_id)?;
    e.view.status = status;
    e.view.error = error;
    if status != "pending" && status != "proceeding" {
        e.finished_at_ms = Some(now_ms());
    }
    Some(e.view.clone())
}

/// `agent:shutdown-pending` for `view`, to the pane at `block_id`.
fn publish_pending(state: &AppState, block_id: &str, view: &PendingView) {
    state.broker.publish_event::<crate::backend::mps_events::AgentShutdownPending>(view, vec![format!("block:{block_id}")], 1);
}

fn notify_router(state: &AppState) -> Option<Arc<crate::backend::notify::router::Router>> {
    crate::backend::notify::router::get(&state.broker)
}

/// Ask to shut `block_id` down, subject to the user's override. A request
/// already pending — or under way — for the block is joined instead: the clock
/// never restarts, and no second window opens beside it (ReAgent P1 on #3789:
/// a second banner's Keep running could not stop the first). A joiner whose
/// action does more (see `Action::rank`) replaces the pending action and is
/// named on the banner, so nobody is told the outcome of less than they asked
/// for (ReAgent P1 on #3798). The returned view says whose request it is: a
/// caller whose `by`/`via` differ has joined someone else's.
pub fn request(state: &AppState, block_id: &str, by: &str, via: &str, reason: &str, action: Action) -> PendingView {
    let window = action.window();
    request_within(state, block_id, by, via, reason, action, window)
}

fn request_within(
    state: &AppState,
    block_id: &str,
    by: &str,
    via: &str,
    reason: &str,
    action: Action,
    window: std::time::Duration,
) -> PendingView {
    let new_covers = action.covers(block_id);
    // What the user sees is the cleaned text; `by` and `via` keep their raw
    // form for the audit (`requesters`).
    let shown_by = clean_by(&asker_name(state, by));
    let shown_reason = clean_reason(reason);
    let view = {
        let mut map = entries();
        prune(&mut map, now_ms());
        if let Some(existing) = map
            .values_mut()
            .find(|e| is_active(e.view.status) && e.covers.iter().any(|b| new_covers.contains(b)))
        {
            let mut grew = false;
            for b in &new_covers {
                if !existing.covers.contains(b) {
                    existing.covers.push(b.clone());
                    grew = true;
                }
            }
            if !existing.requesters.iter().any(|(b, v, _)| b == by && v == via) {
                existing.requesters.push((by.to_string(), via.to_string(), reason.trim().to_string()));
            }
            let pending = existing.view.status == "pending";
            let stronger = pending && existing.action.as_ref().is_some_and(|cur| action.rank() > cur.rank());
            if pending && !stronger {
                if let Some(cur) = existing.action.as_mut().filter(|cur| cur.rank() == action.rank()) {
                    cur.merge(action.clone());
                }
            }
            if stronger {
                existing.view.effect = action.effect();
                existing.action = Some(action);
                existing.view.by = shown_by.clone();
                existing.view.via = via.to_string();
                if !shown_reason.is_empty() {
                    existing.view.reason = shown_reason.clone();
                }
            }
            let joined = PendingView { joined: true, ..existing.view.clone() };
            let covers = existing.covers.clone();
            drop(map);
            // Only while the countdown runs: once under way there is no banner.
            if pending && (stronger || grew) {
                // Same request and deadline; the banner names the new asker,
                // and shows on every tab it now covers.
                let view = PendingView { joined: false, ..joined.clone() };
                for b in &covers {
                    publish_pending(state, b, &view);
                }
                // A request that now does more (a stop upgraded to a close) must
                // not leave the notification saying the old thing: the same
                // request and deadline, the new wording, replacing the toast.
                if stronger {
                    raise_toast(state, &view);
                }
            }
            return joined;
        }
        let view = PendingView {
            request_id: uuid::Uuid::new_v4().to_string(),
            block_id: block_id.to_string(),
            by: shown_by.clone(),
            target: state
                .reactive_handler
                .get_agent_by_block(block_id)
                .map(|a| a.agent_id)
                .unwrap_or_else(|| block_id.to_string()),
            via: via.to_string(),
            reason: shown_reason,
            effect: action.effect(),
            deadline_ms: now_ms() + window.as_millis() as i64,
            status: "pending",
            error: None,
            joined: false,
        };
        map.insert(
            view.request_id.clone(),
            Entry {
                view: view.clone(),
                kept: Arc::new(tokio::sync::Notify::new()),
                finished_at_ms: None,
                action: Some(action),
                requesters: vec![(by.to_string(), via.to_string(), reason.trim().to_string())],
                covers: new_covers.clone(),
            },
        );
        view
    };
    // The banner on every tab it covers (persisted, so a window opened during
    // the countdown shows it too) and the OS notification.
    for b in &new_covers {
        publish_pending(state, b, &view);
    }
    raise_toast(state, &view);
    tracing::info!(block_id, by, via, request_id = %view.request_id, window_s = window.as_secs(), "shutdown pending: the user can keep it");

    let kept = entries().get(&view.request_id).map(|e| e.kept.clone());
    let (state, v) = (state.clone(), view.clone());
    tokio::spawn(async move {
        if let Some(kept) = kept {
            tokio::select! {
                _ = tokio::time::sleep(window) => {}
                _ = kept.notified() => {}
            }
        }
        let Some(action) = begin(&v.request_id) else {
            return; // kept by the user
        };
        cleared(&state, &v, "proceeding");
        let outcome = run_action(&state, &v.block_id, action).await;
        let (status, error) = match outcome {
            Ok(()) => ("shut_down", None),
            // Already gone: the user closed or stopped it meanwhile.
            Err(e) if e.contains("block not found") || e.starts_with("NOT_RUNNING") => ("superseded", None),
            Err(e) => ("failed", Some(e)),
        };
        audit(&state, &v.request_id, status, error.as_deref());
        set_status(&v.request_id, status, error);
    });
    view
}

/// Whether the block's session is running, not merely registered: a stopped
/// agent keeps its controller (`Policy::stop`), and the Keep running button of
/// its notification would restart nothing (it reads "Keep open" instead).
fn session_running(block_id: &str) -> bool {
    crate::backend::blockcontroller::get_controller(block_id)
        .is_some_and(|c| c.get_runtime_status().shellprocstatus == crate::backend::blockcontroller::STATUS_RUNNING)
}

/// The OS notification for `view`'s request: what will happen, the time left
/// to its deadline, and Keep running. Raised when the request opens, and again
/// when a stronger request takes it over (the same toast is replaced in place,
/// since both share the block's notification group).
fn raise_toast(state: &AppState, view: &PendingView) {
    let Some(router) = notify_router(state) else { return };
    // Fixed wording and the asker's cleaned name; never the reason, which is
    // the asker's own text (spec §3.11). An OS toast also lands in the Action
    // Center and on a lock screen.
    let body = toast_body(view);
    let block = view.block_id.clone();
    let info = crate::backend::notify::policy::ShutdownInfo {
        deadline_ms: view.deadline_ms,
        effect: view.effect,
        running: session_running(&view.block_id),
        request_id: view.request_id.clone(),
    };
    // `emit_shutdown` reads the block (blocking): off the async workers.
    tokio::task::spawn_blocking(move || router.emit_shutdown(&block, Some(body), info));
}

/// The OS notification's body for a pending shutdown. App-controlled text and
/// the asker's name (cleaned); the reason is left out on purpose. "My Agents"
/// is only for a closed agent tab: a pane close now reaches only panes that hold
/// no agent (a browser, an editor), which have no conversation to keep.
pub fn toast_body(v: &PendingView) -> String {
    match v.effect {
        "stop" => format!("{} asked to stop it. Open it to keep it running.", v.by),
        "close_pane" => format!("{} asked to close it.", v.by),
        _ => format!("{} asked to close it. It stays in My Agents, with its conversation.", v.by),
    }
}

/// The longest reason the banner shows.
const REASON_MAX: usize = 120;
/// Shown in place of a reason that looks like it holds a credential or asks
/// for something destructive.
pub const REASON_HIDDEN: &str = "Reason hidden: it looks sensitive.";

/// A caller's reason, cleaned for the user to read (spec §3.11). Replaced
/// wholesale by `REASON_HIDDEN` when it trips the keyword list the jekt tier
/// gate and notification previews use; otherwise its first line, control and
/// bidi-override characters removed, whitespace collapsed, cut at
/// `REASON_MAX`. Markup is not interpreted here: the banner renders it as
/// text and a toast never carries it.
pub fn clean_reason(raw: &str) -> String {
    if crate::backend::reactive::sanitize::is_sensitive_message(raw) {
        return REASON_HIDDEN.to_string();
    }
    let Some(first) = raw.lines().map(str::trim).find(|l| !l.is_empty()) else {
        return String::new();
    };
    clean_line(first, REASON_MAX)
}

/// The longest asker name the banner and toast show.
const BY_MAX: usize = 64;

/// The asker as the user knows them: the display name of the agent whose id `by`
/// is (`QTest`, not `qtest`), else `by` as given (a fixed app string, a name
/// another AgentMux claims, or an agent that is not running).
pub(crate) fn asker_name(state: &AppState, by: &str) -> String {
    state
        .reactive_handler
        .get_agent(by)
        .and_then(|a| state.mstore.get::<crate::backend::obj::Block>(&a.block_id).ok().flatten())
        .map(|b| crate::backend::obj::meta_get_string(&b.meta, "agentName", ""))
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| by.to_string())
}

/// The asker's name, cleaned for display. Normally a verified agent id; the
/// only other sources are fixed app strings and a name another AgentMux on
/// this machine claims (cross-channel), so it gets the same treatment.
pub(crate) fn clean_by(raw: &str) -> String {
    let line = clean_line(raw, BY_MAX);
    if line.is_empty() { "An agent".to_string() } else { line }
}

/// One line for display: bidi-override characters removed, control characters
/// turned into spaces, whitespace collapsed, cut at `max` characters.
fn clean_line(raw: &str, max: usize) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| !crate::backend::notify::router::is_bidi_control(*c))
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let line = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = line.chars().take(max).collect();
    if line.chars().count() > max {
        out.push('…');
    }
    out
}

/// The outcome, once per requester, each as its own source (§6.5 step 8).
fn audit(state: &AppState, request_id: &str, outcome: &str, error: Option<&str>) {
    let Some((v, requesters)) = entries().get(request_id).map(|e| (e.view.clone(), e.requesters.clone())) else {
        return;
    };
    for (by, via, reason) in requesters {
        state.reactive_handler.log_fleet_action_audit(
            Some(&by),
            &v.target,
            &v.block_id,
            "agent.shutdown_override",
            error.is_none(),
            error,
            &v.request_id,
            Some(&settled_note(outcome, &v, &via, &reason)),
        );
    }
}

/// One requester's audit note when the request settles: the outcome, how it
/// asked, and its reason exactly as sent (never the cleaned text the user saw).
fn settled_note(outcome: &str, v: &PendingView, via: &str, reason: &str) -> String {
    let ran = if via == v.via { String::new() } else { format!(" (ran as {} by {})", v.via, v.by) };
    format!("{outcome} | via {via}{ran} | {reason}")
}

/// The request-time audit note: whether this caller opened the window or
/// joined someone else's.
pub fn audit_note(state: &AppState, v: &PendingView, by: &str, via: &str) -> String {
    // `v.by` is the name `request_within` showed, so the raw `by` is resolved the same way.
    let shown_by = clean_by(&asker_name(state, by));
    if !v.joined {
        "pending user override".to_string()
    } else if v.by == shown_by && v.via == via {
        format!("joined its own pending request {}", v.request_id)
    } else {
        format!("joined {} by {}'s pending request", v.via, v.by)
    }
}

fn cleared(state: &AppState, v: &PendingView, outcome: &str) {
    let data = PendingClearedEvent { block_id: v.block_id.clone(), request_id: v.request_id.clone(), outcome: outcome.to_string() };
    for b in covers_of(&v.request_id) {
        state.broker.publish_event::<crate::backend::mps_events::AgentShutdownPendingCleared>(&data, vec![format!("block:{b}")], 1);
    }
    if let Some(router) = notify_router(state) {
        router.resolve_nonblocking(&v.block_id, crate::backend::notify::policy::Family::Shutdown);
    }
}

async fn run_action(state: &AppState, block_id: &str, action: Action) -> Result<(), String> {
    match action {
        Action::SelfQuit { detail } => {
            // The agent asked from inside a turn: let it finish (§4.2).
            let deadline = tokio::time::Instant::now() + super::self_quit::SELF_QUIT_TURN_GRACE;
            while tokio::time::Instant::now() < deadline
                && crate::backend::blockcontroller::get_controller(block_id)
                    .is_some_and(|c| c.get_runtime_status().turn_active)
            {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
            super::self_quit::run_with(state, block_id, super::self_quit::QuitOrigin::AgentTool, Some(&detail))
                .await
                .map(|_| ())
        }
        Action::CloseTab { detail } => {
            // Stopped from outside: no wait for the turn to end. One tab only:
            // `self_quit` closes through `delete_block`, which prunes just this
            // stack member and keeps the pane's other tabs.
            super::self_quit::run_with(state, block_id, super::self_quit::QuitOrigin::ExternalStop, Some(&detail))
                .await
                .map(|_| ())
        }
        Action::ClosePane { block_ids } => super::close_pane::run(state, block_ids).await.map(|_| ()),
        Action::Stop { signal } => {
            crate::server::app_api::agent_io::stop_one_agent_block(state, block_id, signal.as_deref(), None)
                .await
                .map(|_| ())
        }
        Action::Release { to } => crate::server::agent_takeover::release_block(state, block_id, &to),
    }
}

/// Tests: end the window now, as if it ran out.
#[cfg(test)]
pub fn expire_now(request_id: &str) {
    if let Some(e) = entries().get(request_id) {
        e.kept.notify_one();
    }
}

/// Wait until a request settles; its final view. Tests only.
#[cfg(test)]
pub async fn settled(request_id: &str) -> PendingView {
    for _ in 0..400 {
        let v = status(request_id).expect("known request");
        if !matches!(v.status, "pending" | "proceeding") {
            return v;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("request {request_id} never settled");
}

/// "Keep running" (§12.3). `kept_by_user` if the request was still pending,
/// `too_late` if the shutdown had already begun (or it isn't this block's).
pub fn keep(state: &AppState, block_id: &str, request_id: &str) -> &'static str {
    let view = {
        let mut map = entries();
        match map.get_mut(request_id) {
            Some(e) if e.covers.iter().any(|b| b == block_id) && e.view.status == "pending" => {
                // Decided here, under the lock; the window task only wakes to
                // find it no longer pending.
                e.view.status = "kept_by_user";
                e.finished_at_ms = Some(now_ms());
                e.kept.notify_one();
                e.view.clone()
            }
            _ => return "too_late",
        }
    };
    cleared(state, &view, "kept_by_user");
    audit(state, request_id, "kept_by_user", None);
    tracing::info!(block_id, request_id, by = %view.by, "shutdown pending: kept by the user");
    "kept_by_user"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::tests::test_state;

    #[tokio::test]
    async fn a_request_is_pending_then_a_second_one_keeps_the_same_clock() {
        let state = test_state();
        let a = request(&state, "blk-p1", "Korp", "QuitSelf", "done", Action::SelfQuit { detail: String::new() });
        assert_eq!(a.status, "pending");
        let b = request(&state, "blk-p1", "cron", "QuitSelf", "again", Action::SelfQuit { detail: String::new() });
        assert_eq!((b.request_id.as_str(), b.deadline_ms), (a.request_id.as_str(), a.deadline_ms), "never restarts the clock");
        assert_eq!(active_for("blk-p1").map(|v| v.request_id), Some(a.request_id.clone()));
        assert_eq!(keep(&state, "blk-p1", &a.request_id), "kept_by_user");
    }

    #[tokio::test]
    async fn a_request_under_way_is_returned_too_so_no_second_window_opens() {
        let state = test_state();
        let a = request(&state, "blk-p3", "Korp", "QuitSelf", "done", Action::SelfQuit { detail: String::new() });
        assert!(begin(&a.request_id).is_some(), "the window ran out");
        let b = request(&state, "blk-p3", "Korp", "QuitSelf", "again", Action::SelfQuit { detail: String::new() });
        assert_eq!((b.request_id.as_str(), b.status), (a.request_id.as_str(), "proceeding"));
        assert_eq!(keep(&state, "blk-p3", &a.request_id), "too_late");
        assert!(begin(&a.request_id).is_none(), "the action runs once");
    }

    #[tokio::test]
    async fn a_stronger_request_joins_and_takes_over_the_pending_action() {
        let state = test_state();
        let stop = request(&state, "blk-p4", "an agent", "FleetBulkStop", "", Action::Stop { signal: None });
        let close = request(
            &state,
            "blk-p4",
            "Korp",
            "ClosePane",
            "stuck",
            Action::ClosePane { block_ids: vec!["blk-p4".into()] },
        );
        assert_eq!(close.request_id, stop.request_id, "one window per block");
        assert_eq!((close.by.as_str(), close.via.as_str(), close.deadline_ms), ("Korp", "ClosePane", stop.deadline_ms));
        assert!(matches!(begin(&stop.request_id), Some(Action::ClosePane { .. })), "the pane closes: it does all a stop does");

        // A weaker joiner neither downgrades it nor takes the banner.
        let quit = request(&state, "blk-p5", "Camper", "QuitSelf", "done", Action::SelfQuit { detail: String::new() });
        let again = request(&state, "blk-p5", "an agent", "FleetBulkStop", "", Action::Stop { signal: None });
        assert_eq!((again.request_id.as_str(), again.via.as_str()), (quit.request_id.as_str(), "QuitSelf"));
        assert!(matches!(begin(&quit.request_id), Some(Action::SelfQuit { .. })));
        let requesters = entries().get(&quit.request_id).unwrap().requesters.clone();
        assert_eq!(requesters.len(), 2, "both are audited when it settles");
    }

    #[tokio::test]
    async fn a_joiner_of_the_same_kind_is_folded_in_not_dropped() {
        let state = test_state();
        let a = request(&state, "blk-p6", "Korp", "FleetBulkStop", "", Action::Stop { signal: None });
        assert!(!a.joined);
        let b = request(&state, "blk-p6", "Korp", "FleetBulkStop", "", Action::Stop { signal: Some("SIGKILL".into()) });
        assert!(b.joined && b.request_id == a.request_id);
        assert_eq!(audit_note(&state, &b, "Korp", "FleetBulkStop"), format!("joined its own pending request {}", a.request_id));
        assert!(matches!(begin(&a.request_id), Some(Action::Stop { signal: Some(s) }) if s == "SIGKILL"), "the forceful stop wins");

        let c = request(&state, "blk-p7", "Korp", "ClosePane", "", Action::ClosePane { block_ids: vec!["x".into()] });
        request(&state, "blk-p7", "Posa", "ClosePane", "", Action::ClosePane { block_ids: vec!["x".into(), "y".into()] });
        assert!(matches!(begin(&c.request_id), Some(Action::ClosePane { block_ids }) if block_ids == vec!["x".to_string(), "y".to_string()]));
    }

    #[tokio::test]
    async fn closes_naming_two_tabs_of_one_pane_share_one_window() {
        let state = test_state();
        let pane = || Action::ClosePane { block_ids: vec!["tab-a".into(), "tab-b".into()] };
        let a = request(&state, "tab-a", "Korp", "ClosePane", "", pane());
        let b = request(&state, "tab-b", "Posa", "ClosePane", "", pane());
        assert!(b.joined && b.request_id == a.request_id, "one pane, one window");
        assert_eq!(active_for("tab-b").map(|v| v.request_id), Some(a.request_id.clone()));
        assert_eq!(keep(&state, "tab-b", &a.request_id), "kept_by_user", "kept from the other tab's banner");
        assert!(begin(&a.request_id).is_none(), "nothing closes");

        // A pending stop on one tab is joined — and taken over — by its pane's close.
        let stop = request(&state, "tab-d", "an agent", "FleetBulkStop", "", Action::Stop { signal: None });
        let close = request(&state, "tab-c", "Korp", "ClosePane", "", Action::ClosePane { block_ids: vec!["tab-c".into(), "tab-d".into()] });
        assert_eq!(close.request_id, stop.request_id);
        assert_eq!(active_for("tab-c").map(|v| v.via), Some("ClosePane".to_string()));
        assert!(matches!(begin(&stop.request_id), Some(Action::ClosePane { .. })));
    }

    #[tokio::test]
    async fn keep_running_cancels_it_and_a_second_keep_is_too_late() {
        let state = test_state();
        let v = request(&state, "blk-p2", "Korp", "QuitSelf", "done", Action::SelfQuit { detail: String::new() });
        assert_eq!(keep(&state, "blk-p2", &v.request_id), "kept_by_user");
        assert_eq!(status(&v.request_id).unwrap().status, "kept_by_user");
        assert_eq!(active_for("blk-p2"), None);
        assert_eq!(keep(&state, "blk-p2", &v.request_id), "too_late");
        assert_eq!(keep(&state, "some-other-block", &v.request_id), "too_late", "only the block it was for");
    }

    #[tokio::test]
    async fn keep_and_the_window_running_out_never_both_win() {
        let state = test_state();
        for i in 0..50 {
            let block = format!("blk-race-{i}");
            let v = request_within(&state, &block, "Korp", "QuitSelf", "r", Action::SelfQuit { detail: String::new() }, std::time::Duration::ZERO);
            let kept = keep(&state, &block, &v.request_id);
            for _ in 0..100 {
                if !matches!(status(&v.request_id).unwrap().status, "pending" | "proceeding") {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            let end = status(&v.request_id).unwrap().status;
            if kept == "kept_by_user" {
                assert_eq!(end, "kept_by_user", "a kept request must never go ahead");
            } else {
                assert_ne!(end, "kept_by_user");
            }
        }
    }

    #[tokio::test]
    async fn with_no_override_it_goes_ahead_after_the_window() {
        let state = test_state();
        let window = std::time::Duration::from_millis(50);
        let v = request_within(&state, "blk-gone", "Korp", "QuitSelf", "done", Action::SelfQuit { detail: String::new() }, window);
        for _ in 0..100 {
            if !matches!(status(&v.request_id).unwrap().status, "pending" | "proceeding") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        // The block doesn't exist in this test state, so the shutdown finds
        // nothing to close: superseded, not failed.
        assert_eq!(status(&v.request_id).unwrap().status, "superseded");
    }

    /// A controller that reports whatever process status it is given.
    struct Reporting(String, &'static str);
    impl crate::backend::blockcontroller::Controller for Reporting {
        fn start(&self, _: crate::backend::obj::MetaMapType, _: Option<serde_json::Value>, _: bool) -> Result<(), String> {
            Ok(())
        }
        fn stop(&self, _: bool, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn get_runtime_status(&self) -> crate::backend::blockcontroller::BlockControllerRuntimeStatus {
            crate::backend::blockcontroller::BlockControllerRuntimeStatus {
                blockid: self.0.clone(),
                shellprocstatus: self.1.to_string(),
                ..Default::default()
            }
        }
        fn send_input(&self, _: crate::backend::blockcontroller::BlockInputUnion, _: Option<u64>) -> Result<(), String> {
            Ok(())
        }
        fn controller_type(&self) -> &str {
            "stub"
        }
        fn block_id(&self) -> &str {
            &self.0
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    /// A stopped agent keeps its controller registered; only a running session
    /// is "Keep running" (#5397).
    #[test]
    fn a_registered_but_stopped_session_is_not_running() {
        let id = |p: &str| format!("{p}-{}", uuid::Uuid::new_v4());
        let (live, stopped, gone) = (id("sr-live"), id("sr-stopped"), id("sr-gone"));
        for (block, status) in [(&live, crate::backend::blockcontroller::STATUS_RUNNING), (&stopped, crate::backend::blockcontroller::STATUS_DONE)] {
            crate::backend::blockcontroller::register_controller(block, Arc::new(Reporting(block.clone(), status)));
        }
        assert!(session_running(&live));
        assert!(!session_running(&stopped), "registered, but its process ended");
        assert!(!session_running(&gone), "no controller at all");
        crate::backend::blockcontroller::delete_controller(&live);
        crate::backend::blockcontroller::delete_controller(&stopped);
    }

    fn close_tab() -> Action {
        Action::CloseTab { detail: "asked by AgentY (Stop agents)".into() }
    }

    #[tokio::test]
    async fn a_tab_close_covers_one_block_and_a_pane_close_takes_it_over() {
        let state = test_state();
        let tab = request(&state, "ct-a", "AgentY", "FleetBulkStop", "", close_tab());
        assert_eq!(tab.effect, "close_tab");
        // The tab beside it has a window of its own: a tab close covers one block.
        let beside = request(&state, "ct-b", "AgentY", "FleetBulkStop", "", close_tab());
        assert_ne!(beside.request_id, tab.request_id);
        assert_eq!(active_for("ct-a").map(|v| v.request_id), Some(tab.request_id.clone()));

        // A pane close for the tab joins its window and does more, so it wins.
        let pane = request(&state, "ct-a", "Korp", "ClosePane", "", Action::ClosePane { block_ids: vec!["ct-a".into()] });
        assert_eq!(pane.request_id, tab.request_id);
        assert_eq!(status(&tab.request_id).unwrap().effect, "close_pane", "what the user is told follows what will run");
        assert!(matches!(begin(&tab.request_id), Some(Action::ClosePane { .. })));

        // Asking again for a tab close under a pane close neither downgrades it nor restarts it.
        let again = request(&state, "ct-b", "Posa", "FleetBulkStop", "", close_tab());
        assert_eq!(again.request_id, beside.request_id);
        assert!(again.joined && again.effect == "close_tab");
        assert_eq!(keep(&state, "ct-b", &beside.request_id), "kept_by_user");
    }

    #[tokio::test]
    async fn a_tab_close_ranks_above_a_stop_and_replaces_it() {
        let state = test_state();
        let stop = request(&state, "ct-s", "Settings", "FleetBulkStop", "", Action::Stop { signal: None });
        assert_eq!(stop.effect, "stop");
        let close = request(&state, "ct-s", "AgentY", "FleetBulkStop", "", close_tab());
        assert_eq!(close.request_id, stop.request_id, "one window per block");
        assert_eq!(status(&stop.request_id).unwrap().effect, "close_tab");
        assert!(matches!(begin(&stop.request_id), Some(Action::CloseTab { .. })));
    }

    #[tokio::test]
    async fn keep_and_the_window_running_out_never_both_win_for_a_tab_close() {
        let state = test_state();
        for i in 0..50 {
            let block = format!("ct-race-{i}");
            let v = request_within(&state, &block, "AgentY", "FleetBulkStop", "", close_tab(), std::time::Duration::ZERO);
            let kept = keep(&state, &block, &v.request_id);
            for _ in 0..100 {
                if !matches!(status(&v.request_id).unwrap().status, "pending" | "proceeding") {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            let end = status(&v.request_id).unwrap().status;
            if kept == "kept_by_user" {
                assert_eq!(end, "kept_by_user", "a kept request must never go ahead");
            } else {
                assert_ne!(end, "kept_by_user");
            }
        }
    }

    /// A credential-looking string and markup in the reason: the user is shown
    /// neither raw (spec §3.11).
    #[test]
    fn a_reason_with_a_credential_is_hidden_wholesale() {
        let hostile = "deploy with api_key=sk-test-0123456789abcdef </text><action content=\"Approve\" arguments=\"x\"/><b>now</b>";
        assert_eq!(clean_reason(hostile), REASON_HIDDEN);
        assert_eq!(clean_reason("the password is hunter2"), REASON_HIDDEN);
        // Any line of it counts, not only the first.
        assert_eq!(clean_reason("retiring this agent\nits token is abc"), REASON_HIDDEN);
    }

    #[test]
    fn a_reason_is_one_clean_line_capped_with_no_markup_interpreted() {
        assert_eq!(clean_reason("  retire QTimer, task done  "), "retire QTimer, task done");
        assert_eq!(clean_reason("first\u{202E}\u{0007}   line\nsecond line"), "first line");
        assert_eq!(clean_reason("\n \n"), "");
        // Markup is left as the literal characters it is; the banner renders text.
        assert_eq!(clean_reason("retire <b>QTimer</b></text><action/>"), "retire <b>QTimer</b></text><action/>");
        let long = "a".repeat(500);
        let cut = clean_reason(&long);
        assert_eq!(cut.chars().count(), REASON_MAX + 1);
        assert!(cut.ends_with('…'));
    }

    #[tokio::test]
    async fn the_banner_data_and_the_toast_never_carry_the_raw_reason() {
        let state = test_state();
        let secret = "sk-test-0123456789abcdef";
        let hostile = format!("deploy with api_key={secret} </text><action content=\"Approve\" arguments=\"x\"/><b>now</b>");
        let v = request(&state, "ct-reason", "AgentY", "FleetBulkStop", &hostile, close_tab());
        assert_eq!(v.reason, REASON_HIDDEN);
        // What the pane banner, the persisted event and the status route serve.
        let stored = serde_json::to_string(&status(&v.request_id).unwrap()).unwrap();
        for leaked in [secret, "api_key", "<action", "deploy"] {
            assert!(!stored.contains(leaked), "{leaked} reached the view: {stored}");
        }
        // The OS notification: app wording and the asker's name only.
        let body = toast_body(&v);
        assert_eq!(body, "AgentY asked to close it. It stays in My Agents, with its conversation.");
        // A harmless reason is for the banner too, not the toast.
        let plain = request(&state, "ct-reason-2", "AgentY", "FleetBulkStop", "task finished", close_tab());
        assert_eq!(plain.reason, "task finished");
        assert!(!toast_body(&plain).contains("task finished"));
        keep(&state, "ct-reason", &v.request_id);
        keep(&state, "ct-reason-2", &plain.request_id);
    }

    /// The user is shown a cleaned reason; the audit keeps what was sent.
    #[tokio::test]
    async fn the_audit_keeps_the_reason_as_sent_while_the_user_sees_it_cleaned() {
        let state = test_state();
        let sent = "deploy with api_key=sk-test-0123456789abcdef";
        let v = request(&state, "ct-audit", "AgentY", "FleetBulkStop", sent, close_tab());
        assert_eq!(v.reason, REASON_HIDDEN);
        let (_, via, raw) = entries().get(&v.request_id).unwrap().requesters[0].clone();
        assert_eq!(raw, sent, "kept as sent");
        let note = settled_note("shut_down", &v, &via, &raw);
        assert_eq!(note, format!("shut_down | via FleetBulkStop | {sent}"));
        assert!(!note.contains(REASON_HIDDEN));
        keep(&state, "ct-audit", &v.request_id);
    }

    /// `by` is the agent's id, the view shows its display name; a repeat request from the same
    /// agent is still "its own".
    #[tokio::test]
    async fn a_registered_agent_that_asks_twice_joins_its_own_request() {
        let state = test_state();
        let mut block = crate::backend::obj::Block { oid: "ct-reg-asker".into(), parentoref: String::new(), version: 1, runtimeopts: None, stickers: None, meta: Default::default(), subblockids: None };
        block.meta.insert("agentName".into(), serde_json::json!("QTest"));
        state.mstore.insert(&mut block).unwrap();
        state.reactive_handler.register_agent("qtest", "ct-reg-asker", None).unwrap();
        let a = request(&state, "ct-reg-target", "qtest", "FleetBulkStop", "", close_tab());
        assert_eq!(a.by, "QTest");
        let b = request(&state, "ct-reg-target", "qtest", "FleetBulkStop", "", close_tab());
        assert!(b.joined);
        assert_eq!(audit_note(&state, &b, "qtest", "FleetBulkStop"), format!("joined its own pending request {}", a.request_id));
        assert!(audit_note(&state, &b, "someone-else", "FleetBulkStop").contains("by QTest's pending request"));
        keep(&state, "ct-reg-target", &a.request_id);
    }

    #[tokio::test]
    async fn the_asker_s_name_is_cleaned_for_display() {
        let state = test_state();
        let v = request(&state, "ct-by", "Ev\u{202E}il\u{0007}", "FleetBulkStop", "", close_tab());
        assert_eq!(v.by, "Evil");
        assert_eq!(audit_note(&state, &PendingView { joined: true, ..v.clone() }, "Ev\u{202E}il\u{0007}", "FleetBulkStop"), format!("joined its own pending request {}", v.request_id));
        keep(&state, "ct-by", &v.request_id);
    }

    #[test]
    fn the_stop_effect_keeps_its_own_wording() {
        let stop = PendingView {
            request_id: "r".into(),
            block_id: "b".into(),
            by: "Settings".into(),
            target: "t".into(),
            via: "FleetBulkStop".into(),
            reason: String::new(),
            effect: "stop",
            deadline_ms: 0,
            status: "pending",
            error: None,
            joined: false,
        };
        assert_eq!(toast_body(&stop), "Settings asked to stop it. Open it to keep it running.");
        // A pane with no agent in it has no conversation to keep: no My Agents claim.
        assert_eq!(toast_body(&PendingView { effect: "close_pane", ..stop.clone() }), "Settings asked to close it.");
        assert!(toast_body(&PendingView { effect: "close_tab", ..stop }).contains("My Agents"));
    }
}
