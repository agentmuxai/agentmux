// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// External shutdown: 15 seconds for the user to override
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

use crate::backend::mps::MuxEvent;
use crate::server::AppState;

pub const OVERRIDE_WINDOW: std::time::Duration = std::time::Duration::from_secs(15);
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
    /// Another agent's `ClosePane block_id=`: close the whole pane.
    ClosePane { block_ids: Vec<String> },
    /// An agent's `FleetBulkStop`: stop the controller, as `agent.stop` does.
    Stop { signal: Option<String> },
}

impl Action {
    /// Each does all the one below it does: closing the pane ends the agent
    /// and its siblings, a self-quit stops it and closes its tab.
    fn rank(&self) -> u8 {
        match self {
            Action::Stop { .. } => 0,
            Action::SelfQuit { .. } => 1,
            Action::ClosePane { .. } => 2,
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

/// A request as the caller and the frontend see it (§12.2).
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct PendingView {
    pub request_id: String,
    pub block_id: String,
    /// Who asked: an agent id, or `cron` etc.
    pub by: String,
    /// The agent being shut down, resolved when asked (it may be gone after).
    pub target: String,
    /// The tool: `QuitSelf`, `ClosePane`, `FleetBulkStop`.
    pub via: String,
    pub reason: String,
    pub deadline_ms: i64,
    /// `pending` → `kept_by_user`, or `proceeding` → `shut_down` / `failed` /
    /// `superseded` (the user closed it themselves meanwhile).
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// This caller joined a request already open for the block rather than
    /// opening one. Never stored; set on the view `request` returns.
    #[serde(skip)]
    pub joined: bool,
}

struct Entry {
    view: PendingView,
    kept: Arc<tokio::sync::Notify>,
    finished_at_ms: Option<i64>,
    /// Taken when the window runs out and the shutdown begins.
    action: Option<Action>,
    /// Everyone who asked, as (by, via): later requests for the same block
    /// join this one rather than open a second window.
    requesters: Vec<(String, String)>,
    /// Every block it shuts down (`Action::covers`). A request naming any of
    /// them joins this one, Keep running works from any of their banners,
    /// and the banner shows on each — a pane's close covers all its tabs
    /// (ReAgent P1 on #3798: keyed on the named block alone, two closes
    /// naming two tabs of one pane got two windows, and keeping one didn't
    /// stop the other closing the whole pane).
    covers: Vec<String>,
}

static ENTRIES: LazyLock<Mutex<HashMap<String, Entry>>> = LazyLock::new(Default::default);

fn now_ms() -> i64 {
    agentmux_common::time::now_ms()
}

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

fn publish(state: &AppState, event: &str, block_id: &str, persist: usize, data: serde_json::Value) {
    state.broker.publish(MuxEvent {
        event: event.to_string(),
        scopes: vec![format!("block:{block_id}")],
        sender: String::new(),
        persist,
        data: Some(data),
    });
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
    request_within(state, block_id, by, via, reason, action, OVERRIDE_WINDOW)
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
            if !existing.requesters.iter().any(|(b, v)| b == by && v == via) {
                existing.requesters.push((by.to_string(), via.to_string()));
            }
            let pending = existing.view.status == "pending";
            let stronger = pending && existing.action.as_ref().is_some_and(|cur| action.rank() > cur.rank());
            if pending && !stronger {
                if let Some(cur) = existing.action.as_mut().filter(|cur| cur.rank() == action.rank()) {
                    cur.merge(action.clone());
                }
            }
            if stronger {
                existing.action = Some(action);
                existing.view.by = by.to_string();
                existing.view.via = via.to_string();
                if !reason.trim().is_empty() {
                    existing.view.reason = reason.trim().to_string();
                }
            }
            let joined = PendingView { joined: true, ..existing.view.clone() };
            let covers = existing.covers.clone();
            drop(map);
            // Only while the countdown runs: once under way there is no banner.
            if pending && (stronger || grew) {
                // Same request and deadline; the banner names the new asker,
                // and shows on every tab it now covers.
                let data = serde_json::to_value(PendingView { joined: false, ..joined.clone() }).unwrap_or_default();
                for b in &covers {
                    publish(state, EVENT_SHUTDOWN_PENDING, b, 1, data.clone());
                }
            }
            return joined;
        }
        let view = PendingView {
            request_id: uuid::Uuid::new_v4().to_string(),
            block_id: block_id.to_string(),
            by: by.to_string(),
            target: state
                .reactive_handler
                .get_agent_by_block(block_id)
                .map(|a| a.agent_id)
                .unwrap_or_else(|| block_id.to_string()),
            via: via.to_string(),
            reason: reason.trim().to_string(),
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
                requesters: vec![(by.to_string(), via.to_string())],
                covers: new_covers.clone(),
            },
        );
        view
    };
    // The banner on every tab it covers (persisted, so a window opened during
    // the countdown shows it too) and the OS notification.
    let data = serde_json::to_value(&view).unwrap_or_default();
    for b in &new_covers {
        publish(state, EVENT_SHUTDOWN_PENDING, b, 1, data.clone());
    }
    if let Some(router) = notify_router(state) {
        let why = if view.reason.is_empty() { String::new() } else { format!(": {}", view.reason) };
        let body = format!("{} asked to shut it down{why}. Open it to keep it running.", view.by);
        let block = block_id.to_string();
        // `emit_fixed` reads the block (blocking): off the async workers.
        tokio::task::spawn_blocking(move || {
            router.emit_fixed(crate::backend::notify::policy::NotifyKind::ShutdownPending, &block, Some(body))
        });
    }
    tracing::info!(block_id, by, via, request_id = %view.request_id, "shutdown pending: user has 15 s to keep it");

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

/// The outcome, once per requester, each as its own source (§6.5 step 8).
fn audit(state: &AppState, request_id: &str, outcome: &str, error: Option<&str>) {
    let Some((v, requesters)) = entries().get(request_id).map(|e| (e.view.clone(), e.requesters.clone())) else {
        return;
    };
    for (by, via) in requesters {
        let ran = if via == v.via { String::new() } else { format!(" (ran as {} by {})", v.via, v.by) };
        state.reactive_handler.log_fleet_action_audit(
            Some(&by),
            &v.target,
            &v.block_id,
            "agent.shutdown_override",
            error.is_none(),
            error,
            &v.request_id,
            Some(&format!("{outcome} | via {via}{ran} | {}", v.reason)),
        );
    }
}

/// The request-time audit note: whether this caller opened the window or
/// joined someone else's.
pub fn audit_note(v: &PendingView, by: &str, via: &str) -> String {
    if !v.joined {
        "pending user override".to_string()
    } else if v.by == by && v.via == via {
        format!("joined its own pending request {}", v.request_id)
    } else {
        format!("joined {} by {}'s pending request", v.via, v.by)
    }
}

fn cleared(state: &AppState, v: &PendingView, outcome: &str) {
    let data = serde_json::json!({ "block_id": v.block_id, "request_id": v.request_id, "outcome": outcome });
    for b in covers_of(&v.request_id) {
        publish(state, EVENT_SHUTDOWN_PENDING_CLEARED, &b, 1, data.clone());
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
        Action::ClosePane { block_ids } => super::close_pane::run(state, block_ids).await.map(|_| ()),
        Action::Stop { signal } => {
            crate::server::app_api::agent_io::stop_one_agent_block(block_id, signal.as_deref()).map(|_| ())
        }
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
        assert_eq!(audit_note(&b, "Korp", "FleetBulkStop"), format!("joined its own pending request {}", a.request_id));
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
}
