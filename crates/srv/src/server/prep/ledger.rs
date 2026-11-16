// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The open windows, one per agent. A window is read and changed under the lock and nowhere else, and
//! its closure is taken out under the same lock by the window's task alone, so it runs exactly once
//! however `Agents ready`, "Switch now", a cancel and the deadline race.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use tokio::time::Instant;

use serde_json::Value;
use tokio::sync::Notify;

use crate::backend::continuity_handoff::{self, Handoff};
use crate::server::AppState;

use super::meta::{prep_value, set_block_meta};
use super::{note, window, How, Prep, Proceed, AUDIT_ANSWERED, AUDIT_REQUESTED, EXTENDED_WINDOW, META_PREP, WINDOW};

pub(super) struct Entry {
    pub id: String,
    pub prep: Prep,
    pub started: Instant,
    pub started_ms: u64,
    pub until: Instant,
    pub extended: bool,
    pub why: Option<String>,
    /// The agent answered with a handoff: the closure runs once its turn has ended.
    pub ready: bool,
    /// The person pressed "Switch now".
    pub now: bool,
    /// How long the handoff the agent left is, for the audit row.
    pub handoff_chars: Option<usize>,
    pub proceed: Option<Proceed>,
    pub wake: Arc<Notify>,
}

impl Entry {
    fn until_ms(&self) -> u64 {
        self.started_ms + u64::try_from((self.until - self.started).as_millis()).unwrap_or(0)
    }

    pub(super) fn meta(&self) -> Value {
        prep_value(&self.id, self.prep.action, self.until_ms(), self.extended, self.why.as_deref())
    }
}

static LEDGER: LazyLock<Mutex<HashMap<String, Entry>>> = LazyLock::new(Default::default);

fn ledger() -> MutexGuard<'static, HashMap<String, Entry>> {
    LEDGER.lock().unwrap_or_else(|e| e.into_inner())
}

/// Read or change the window `id` of `def_id`; `None` when it is gone (it ended, or was replaced).
pub(super) fn with<R>(def_id: &str, id: &str, f: impl FnOnce(&mut Entry) -> R) -> Option<R> {
    ledger().get_mut(def_id).filter(|e| e.id == id).map(f)
}

/// Agents whose window has ended and whose closure is still running. They count as open, so
/// `apply_pending` doesn't apply the same switch a second time while the closure does.
static CLOSING: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);

/// Held while a window's closure runs; the agent is open until it is dropped.
pub(super) struct Closing(String);

impl Drop for Closing {
    fn drop(&mut self) {
        CLOSING.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.0);
    }
}

/// Take the window `id` of `def_id` out of the ledger: whoever gets it runs its closure.
pub(super) fn take(def_id: &str, id: &str) -> Option<(Entry, Closing)> {
    let mut guard = ledger();
    if !guard.get(def_id).is_some_and(|e| e.id == id) {
        return None;
    }
    // Both under the ledger's lock, so `is_open` never sees the agent in neither.
    CLOSING.lock().unwrap_or_else(|e| e.into_inner()).insert(def_id.to_string());
    guard.remove(def_id).map(|e| (e, Closing(def_id.to_string())))
}

/// Whether a window is open for the agent, or its closure is still running: the account change it
/// waited on is the window's to apply, not `apply_pending`'s.
pub fn is_open(def_id: &str) -> bool {
    let open = ledger().contains_key(def_id);
    open || CLOSING.lock().unwrap_or_else(|e| e.into_inner()).contains(def_id)
}

/// An open window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    pub id: String,
    /// When it ends, unless the agent asks for more time (epoch milliseconds).
    pub until_ms: u64,
}

/// One audit row for the request `id`, recorded as `actor`.
pub(super) fn audit(state: &AppState, prep: &Prep, id: &str, actor: &str, action: &str, reason: &str) {
    let block = prep.blocks.first().map_or("", String::as_str);
    state.reactive_handler.log_fleet_action_audit(Some(actor), &prep.agent_name, block, action, true, None, id, Some(reason));
}

fn refs(blocks: &[String]) -> Vec<&str> {
    blocks.iter().map(String::as_str).collect()
}

/// Clear the chip on the agent's panes.
pub(super) fn clear_meta(state: &AppState, prep: &Prep) {
    set_block_meta(state, &refs(&prep.blocks), &[(META_PREP, Value::Null)]);
}

/// Open the window for `prep` and ask the agent; `proceed` runs once when it ends. Asking again for the
/// same target changes nothing; for another one replaces the closure and keeps the window and its
/// deadline. BLOCKING (store writes); needs a tokio runtime for the window's task.
pub fn open(state: &AppState, prep: Prep, proceed: Proceed) -> Opened {
    let mut guard = ledger();
    if let Some(e) = guard.get_mut(&prep.def_id) {
        if e.prep.target != prep.target {
            e.prep = prep;
            e.proceed = Some(proceed);
        }
        return Opened { id: e.id.clone(), until_ms: e.until_ms() };
    }
    let id = format!("p-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let started = Instant::now();
    let entry = Entry {
        id: id.clone(),
        prep: prep.clone(),
        started,
        started_ms: agentmux_common::time::now_ms_u64(),
        until: started + WINDOW,
        extended: false,
        why: None,
        ready: false,
        now: false,
        handoff_chars: None,
        proceed: Some(proceed),
        wake: Arc::new(Notify::new()),
    };
    let (opened, meta) = (Opened { id: id.clone(), until_ms: entry.until_ms() }, entry.meta());
    guard.insert(prep.def_id.clone(), entry);
    drop(guard);
    set_block_meta(state, &refs(&prep.blocks), &[(META_PREP, meta)]);
    audit(state, &prep, &id, &prep.actor, AUDIT_REQUESTED, &format!("{}: {}", prep.action, prep.what));
    tokio::spawn(window::run(state.clone(), prep.def_id.clone(), id));
    opened
}

/// The change is not going to happen (the person chose the account the agent is on): end the window
/// without running the closure, take back what the agent left and tell it to carry on. BLOCKING.
pub fn cancel(state: &AppState, def_id: &str) {
    let Some(entry) = ledger().remove(def_id) else { return };
    entry.wake.notify_one();
    clear_meta(state, &entry.prep);
    continuity_handoff::drop_for_request(def_id, &entry.id);
    audit(state, &entry.prep, &entry.id, "agentmux", AUDIT_ANSWERED, "cancelled");
    let route = agentmux_common::slug::agent_route_name(&entry.prep.agent_name);
    if let Err(e) = window::send_note(state, &route, &note::cancelled(&entry.id)) {
        tracing::debug!(agent = %entry.prep.agent_name, error = %e, "prep: the agent was not told the change was cancelled");
    }
}

/// What an agent answers with `Agents ready`.
#[derive(Debug, Default)]
pub struct Answer {
    pub handoff: Option<String>,
    pub more_time: bool,
    pub why: Option<String>,
}

/// The agent `def_id` (its slug `slug`) answers the window open for it. The handoff is stored at once, so
/// it is kept even if the window runs out; a request for more time moves the deadline, once.
pub(crate) fn answer(state: &AppState, def_id: &str, slug: &str, a: Answer) -> Result<Value, String> {
    let text = a.handoff.as_deref().map(continuity_handoff::clamp).filter(|t| !t.is_empty());
    if text.is_none() && !a.more_time {
        return Err("say what your next session must know: handoff (at most about 10 lines), or more_time true with why".to_string());
    }
    let mut guard = ledger();
    let Some(e) = guard.get_mut(def_id) else {
        return Err("no change is waiting on you: nothing to hand off (it may have already happened)".to_string());
    };
    if let Some(text) = &text {
        let handoff = Handoff { prep_id: e.id.clone(), at_ms: agentmux_common::time::now_ms(), actor: e.prep.actor.clone(), action: e.prep.action.to_string(), text: text.clone() };
        continuity_handoff::store(def_id, &handoff)?;
        e.handoff_chars = Some(text.chars().count());
    }
    let mut said = Vec::new();
    if a.more_time {
        if e.extended {
            return Err("you already asked for more time: it is once. Call Agents ready with your handoff".to_string());
        }
        e.extended = true;
        e.until = e.started + EXTENDED_WINDOW;
        e.why = a.why.as_deref().map(crate::backend::reactive::sanitize::quote_in_system_note).filter(|w| !w.is_empty());
        said.push(format!("more time: until {} seconds from the change", EXTENDED_WINDOW.as_secs()));
    } else {
        e.ready = true;
        said.push("handoff stored: the change runs when your turn ends".to_string());
    }
    let (meta, prep, id, wake) = (e.meta(), e.prep.clone(), e.id.clone(), e.wake.clone());
    drop(guard);
    set_block_meta(state, &refs(&prep.blocks), &[(META_PREP, meta)]);
    tracing::info!(agent = %slug, prep = %id, more_time = a.more_time, handoff = text.is_some(), "prep: the agent answered");
    wake.notify_one();
    Ok(serde_json::json!({ "status": if a.more_time { "more_time" } else { "ready" }, "note": said.join("; "), "prep": id }))
}

/// The person pressed "Switch now" for the agent `def_id`: the window ends and the closure runs. BLOCKING.
pub fn person_says_now(def_id: &str) -> Result<(), String> {
    let mut guard = ledger();
    let e = guard.get_mut(def_id).ok_or("nothing is waiting for that agent")?;
    e.now = true;
    e.wake.notify_one();
    Ok(())
}

/// The `prep.answered` row of a window that ended as `how`: who it is recorded as (the agent when it
/// answered, the person when they pressed "Switch now", AgentMux otherwise) and why.
pub(super) fn finish_row(e: &Entry, how: How) -> (String, String) {
    let who = match how {
        How::Ready => agentmux_common::slug::agent_route_name(&e.prep.agent_name),
        How::PersonSaidNow => "operator".to_string(),
        How::TimedOut | How::NotAsked => "agentmux".to_string(),
    };
    let handoff = e.handoff_chars.map_or_else(String::new, |n| format!(", handoff {n} characters"));
    let more = if e.extended { ", asked for more time" } else { "" };
    (who, format!("{}{handoff}{more}", how.reason()))
}
