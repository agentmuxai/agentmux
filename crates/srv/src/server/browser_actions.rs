// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What an agent did in a browser pane it drives, and the person's Pause
//! (docs/specs/SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md §5.5,
//! PLAN_BROWSER_DRIVER_FIXES_2026_10_17.md §5).
//!
//! The log is a ring of the last [`RING`] entries per pane, in memory only:
//! srv restarting clears it, closing the pane drops it, and Take over keeps
//! it, so the person can still read what happened. The newest [`PUBLISHED`]
//! are mirrored to the pane's `browser:actions` meta key (srv-only, like
//! `browser:bypass`) for the header's Log. An entry never holds what was
//! typed: a fill names its field, never the value, secret or not.
//!
//! Pause is the person's quick, reversible brake, where Take over is final:
//! while a pane is paused, every agent call that resolves it is refused, and
//! an Auto-approve grant's clock stands still. Only the person pauses or
//! resumes, through the host (`/api/v1/host/browser_pause`), so no agent can
//! lift a pause on itself.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use serde_json::{json, Value};

use crate::server::ui_handlers::{banner_for, err_response, from_host, verified_block_id};
use crate::server::AppState;

/// Block meta key the header's Log reads: `{count, entries}`.
pub(crate) const ACTIONS_META_KEY: &str = "browser:actions";
/// Block meta key the header reads to show a pane is paused.
pub(crate) const PAUSED_META_KEY: &str = "browser:paused";

/// Entries kept per pane.
pub(crate) const RING: usize = 200;
/// Entries mirrored to the pane's meta: the header shows the newest.
pub(crate) const PUBLISHED: usize = 50;

/// What was done. Kebab-case on the wire (`auto-approve-on`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Kind {
    Navigate,
    Back,
    Forward,
    Reload,
    Click,
    Fill,
    Select,
    Check,
    Key,
    Upload,
    Eval,
    Handoff,
    AutoApproveOn,
    AutoApproveOff,
    Pause,
    Resume,
}

/// How it went: who let it through, or why it didn't happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum How {
    /// Nothing needed asking.
    Allowed,
    /// The person approved it in the pane's banner.
    Approved,
    /// Made without asking under the person's Auto-approve.
    AutoApproved,
    Declined,
    Refused(String),
    TimedOut,
    /// The person's own doing (Pause, Resume, Auto-approve from the header).
    ByUser,
}

impl How {
    fn label(&self) -> String {
        match self {
            How::Allowed => "allowed".into(),
            How::Approved => "approved".into(),
            How::AutoApproved => "auto-approved".into(),
            How::Declined => "declined".into(),
            How::Refused(why) => format!("refused: {}", short_reason(why)),
            How::TimedOut => "timed out".into(),
            How::ByUser => "by the user".into(),
        }
    }

    /// A hand-off's or a request's answer.
    pub(crate) fn from_answer(a: crate::server::browser_attention::Answer) -> How {
        use crate::server::browser_attention::Answer;
        match a {
            Answer::Yes => How::Approved,
            Answer::Cancelled => How::Declined,
            Answer::TimedOut => How::TimedOut,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Entry {
    /// Milliseconds since the Unix epoch.
    pub ts: i64,
    pub agent: String,
    pub kind: Kind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub how: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Default)]
struct Log {
    entries: VecDeque<Entry>,
    /// Every entry the pane ever had, for the header's count.
    count: u64,
}

#[derive(Default)]
struct Table {
    logs: HashMap<String, Log>,
    /// Paused panes, and since when.
    paused: HashMap<String, Instant>,
}

fn table() -> std::sync::MutexGuard<'static, Table> {
    static T: OnceLock<Mutex<Table>> = OnceLock::new();
    T.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner())
}

/// `s` trimmed and cut to `n` characters.
fn clip(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// A refusal's gist for the log: its first clause.
fn short_reason(why: &str) -> String {
    let cut = [": ", ". ", "; "].iter().filter_map(|sep| why.find(sep)).min().unwrap_or(why.len());
    clip(&why[..cut], 100)
}

/// Add `e` to `pane`'s ring, dropping the oldest past [`RING`].
fn add(pane: &str, e: Entry) {
    let mut t = table();
    let log = t.logs.entry(pane.to_string()).or_default();
    log.entries.push_back(e);
    log.count += 1;
    while log.entries.len() > RING {
        log.entries.pop_front();
    }
}

/// `pane`'s entries, oldest first, and how many it ever had.
pub(crate) fn entries(pane: &str) -> (u64, Vec<Entry>) {
    table().logs.get(pane).map_or((0, Vec::new()), |l| (l.count, l.entries.iter().cloned().collect()))
}

/// One entry being described; [`Note::record`] adds it. Built by the
/// handlers, one line each.
#[must_use]
pub(crate) struct Note {
    target_pane: String,
    agent: String,
    kind: Kind,
    url: Option<String>,
    target: Option<String>,
    detail: Option<String>,
    how: Option<How>,
}

/// Describe `agent`'s `kind` on `pane` (a pane or a popup window id).
pub(crate) fn note(pane: &str, agent: &str, kind: Kind) -> Note {
    Note { target_pane: pane.to_string(), agent: agent.to_string(), kind, url: None, target: None, detail: None, how: None }
}

impl Note {
    /// An act request: click, fill, select or check, on its ref. A fill's
    /// text is never kept; a select's option and a check's state are.
    pub(crate) fn act(pane: &str, req: &agentmux_common::api_types::UiBrowserActRequest) -> Note {
        let kind = match req.action.as_str() {
            "fill" => Kind::Fill,
            "select" => Kind::Select,
            "check" => Kind::Check,
            _ => Kind::Click,
        };
        let n = note(pane, &req.auth.agent_id, kind).target(&req.ref_);
        match kind {
            Kind::Select => n.detail(&format!("option {:?}", clip(req.option.as_deref().unwrap_or(""), 80))),
            Kind::Check => n.detail(if req.checked == Some(false) { "uncheck" } else { "check" }),
            _ => n,
        }
    }

    pub(crate) fn url(mut self, url: &str) -> Self {
        self.url = Some(clip(url, 500)).filter(|u| !u.is_empty());
        self
    }

    pub(crate) fn target(mut self, target: &str) -> Self {
        self.target = Some(clip(target, 120)).filter(|t| !t.is_empty());
        self
    }

    pub(crate) fn detail(mut self, detail: &str) -> Self {
        self.detail = Some(clip(detail, 200)).filter(|d| !d.is_empty());
        self
    }

    pub(crate) fn how(mut self, how: How) -> Self {
        self.how = Some(how);
        self
    }

    /// The committing click a `needs_approval` describes: its label (from
    /// `Click "…"`) and the page it is on, keeping the ref as detail.
    pub(crate) fn asked(self, ask: &Value) -> Self {
        let what = ask.get("what").and_then(|v| v.as_str()).unwrap_or("");
        let label = what.strip_prefix("Click ").unwrap_or(what).to_string();
        let page = ask.get("page").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let r = self.target.clone().map(|r| format!("ref {r}")).unwrap_or_default();
        self.target(&label).url(&page).detail(&r)
    }

    fn entry(&self, how: How, window_url: Option<String>) -> Entry {
        Entry {
            ts: chrono::Utc::now().timestamp_millis(),
            agent: self.agent.clone(),
            kind: self.kind,
            url: self.url.clone().or(window_url),
            target: self.target.clone(),
            how: how.label(),
            detail: self.detail.clone(),
        }
    }

    /// Add the entry (as `Allowed` unless [`Note::how`] said otherwise) to
    /// the pane's log and show it in the header. Only a pane an agent drives
    /// has a log: an agent's own pane, which these routes also reach, has none.
    pub(crate) fn record(self, state: &AppState) {
        let how = self.how.clone().unwrap_or(How::Allowed);
        self.record_as(state, how);
    }

    fn record_as(self, state: &AppState, how: How) {
        // A popup window's actions go in its opener's log, with its address.
        let (pane, window_url) = banner_for(&self.target_pane);
        if crate::server::browser_owner::owner_of(&pane).is_none() {
            return;
        }
        add(&pane, self.entry(how, window_url));
        publish(state, &pane);
    }

    /// Record a host call's result and answer the agent with it: `how` (or
    /// `Allowed`) when it went through, `refused: <why>` when it didn't.
    pub(crate) fn outcome(self, state: &AppState, r: Result<Value, (StatusCode, String)>) -> Response {
        match r {
            Ok(data) => {
                self.record(state);
                (StatusCode::OK, Json(json!({ "ok": true, "data": data }))).into_response()
            }
            Err((code, e)) => {
                self.record_as(state, How::Refused(e.clone()));
                err_response(code, e)
            }
        }
    }

    /// Record a call that didn't go through (as `how`, else `refused: msg`)
    /// and answer the agent with `msg`.
    pub(crate) fn fail(self, state: &AppState, code: StatusCode, msg: String) -> Response {
        let how = self.how.clone().unwrap_or_else(|| How::Refused(msg.clone()));
        self.record_as(state, how);
        err_response(code, msg)
    }
}

/// Mirror `pane`'s newest entries to its meta, for the header's Log.
fn publish(state: &AppState, pane: &str) {
    let value = {
        let t = table();
        t.logs.get(pane).map_or(Value::Null, |l| {
            let skip = l.entries.len().saturating_sub(PUBLISHED);
            json!({ "count": l.count, "entries": l.entries.iter().skip(skip).collect::<Vec<_>>() })
        })
    };
    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert(ACTIONS_META_KEY.to_string(), value);
    if let Err(e) = crate::server::http_shell::broadcast_meta_update(state, pane, &meta) {
        tracing::debug!(pane = %pane, error = %e, "[browser-actions] header log not updated");
    }
}

pub(crate) fn is_paused(pane: &str) -> bool {
    table().paused.contains_key(pane)
}

/// For `target_block_id`: refuse every agent call on a pane the person paused.
pub(crate) fn not_paused(pane: &str) -> Result<(), (StatusCode, String)> {
    if !is_paused(&banner_for(pane).0) {
        return Ok(());
    }
    Err((
        StatusCode::CONFLICT,
        "the user paused this pane; your calls on it wait until they resume it (this one wasn't made): \
         try again later, or ask them in chat"
            .to_string(),
    ))
}

/// Pause (`on`) or resume `pane`. Returns whether that changed anything.
/// An Auto-approve grant's clock stops with it, and runs again on resume.
pub(crate) fn set_paused(state: &AppState, pane: &str, on: bool) -> bool {
    let held = {
        let mut t = table();
        match (on, t.paused.get(pane).copied()) {
            (true, None) => {
                t.paused.insert(pane.to_string(), Instant::now());
                None
            }
            (false, Some(since)) => {
                t.paused.remove(pane);
                Some(since.elapsed())
            }
            _ => return false,
        }
    };
    crate::server::browser_bypass::hold_clock(state, pane, on);
    let agent = crate::server::browser_owner::owner_of(pane).unwrap_or_default();
    let n = note(pane, &agent, if on { Kind::Pause } else { Kind::Resume }).how(How::ByUser);
    match held {
        Some(d) => n.detail(&format!("after {}", held_for(d))).record(state),
        None => n.record(state),
    }
    tracing::info!(pane = %pane, paused = on, "[browser-actions] the user {} the agent", if on { "paused" } else { "resumed" });
    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert(PAUSED_META_KEY.to_string(), if on { json!(true) } else { Value::Null });
    if let Err(e) = crate::server::http_shell::broadcast_meta_update(state, pane, &meta) {
        tracing::debug!(pane = %pane, error = %e, "[browser-actions] header pause not updated");
    }
    true
}

fn held_for(d: Duration) -> String {
    match d.as_secs() {
        s if s < 60 => format!("{s} s"),
        s => format!("{} min", s / 60),
    }
}

/// Where an Auto-approve grant's end moves to when its clock ran from
/// `paused_at` to `now` while paused: later by exactly the paused time, both
/// as an `Instant` and as the header's epoch milliseconds.
pub(crate) fn resumed_clock(expires: Instant, expires_at_ms: i64, paused_at: Instant, now: Instant) -> (Instant, i64) {
    let held = now.saturating_duration_since(paused_at);
    (expires + held, expires_at_ms + held.as_millis() as i64)
}

/// The agent no longer drives `pane` (Take over): the pause goes with it.
/// The log stays.
pub(crate) fn forget_pause(pane: &str) {
    table().paused.remove(pane);
}

/// `pane` was closed: its log and pause go.
pub(crate) fn drop_pane(pane: &str) {
    let mut t = table();
    t.logs.remove(pane);
    t.paused.remove(pane);
}

/// Clear the log and pause saved with panes that srv no longer has them for.
/// Called when the host registers: neither survives srv restarting, and the
/// header, which reads the saved meta, must not show a stale one.
pub(crate) fn reset_strips(state: &AppState) {
    let blocks = match state.mstore.get_all::<crate::backend::obj::Block>() {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "[browser-actions] couldn't list panes to clear stale logs");
            return;
        }
    };
    let set = |b: &crate::backend::obj::Block, key: &str| b.meta.get(key).is_some_and(|v| !v.is_null());
    for b in blocks {
        let (has_log, paused) = {
            let t = table();
            (t.logs.contains_key(&b.oid), t.paused.contains_key(&b.oid))
        };
        let mut meta = crate::backend::obj::MetaMapType::new();
        if set(&b, ACTIONS_META_KEY) && !has_log {
            meta.insert(ACTIONS_META_KEY.to_string(), Value::Null);
        }
        if set(&b, PAUSED_META_KEY) && !paused {
            meta.insert(PAUSED_META_KEY.to_string(), Value::Null);
        }
        if !meta.is_empty() {
            if let Err(e) = crate::server::http_shell::broadcast_meta_update(state, &b.oid, &meta) {
                tracing::debug!(pane = %b.oid, error = %e, "[browser-actions] stale log not cleared");
            }
        }
    }
}

/// `POST /api/v1/ui/browser/actions` — backs Browser's `log`: the pane's
/// action log, for the agent that drives it (or its own pane). Read-only.
/// Resolved without `target_block_id`, so it works while the pane is paused
/// or waiting on the person.
pub(crate) async fn handle_ui_browser_actions(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<agentmux_common::api_types::UiBrowserActionsRequest>,
) -> impl IntoResponse {
    let own = match verified_block_id(&state, caller.as_deref(), &req.auth) {
        Ok(b) => b,
        Err(e) => return err_response(StatusCode::UNAUTHORIZED, e),
    };
    let asked = req.pane.as_deref().map(str::trim).filter(|p| !p.is_empty()).unwrap_or(&own).to_string();
    let pane = banner_for(&asked).0;
    let yours = pane == own
        || crate::server::browser_owner::owner_of(&pane).is_some_and(|o| o.eq_ignore_ascii_case(&req.auth.agent_id));
    if !yours {
        return err_response(
            StatusCode::FORBIDDEN,
            format!("pane {asked:?} isn't a browser pane you drive: you can read the log of your own panes only"),
        );
    }
    let (count, entries) = entries(&pane);
    let data = json!({ "pane": pane, "paused": is_paused(&pane), "count": count, "entries": entries });
    (StatusCode::OK, Json(json!({ "ok": true, "data": data }))).into_response()
}

/// `POST /api/v1/host/browser_pause` — the person's Pause and Resume in an
/// agent-driven pane's header, relayed by the CEF host. Only the host can
/// call it: an agent holding the instance auth key gets 403, so no agent can
/// resume itself. Body `{block_id, paused}`.
pub(crate) async fn handle_host_browser_pause(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    if !from_host(&state, &headers).await {
        return err_response(StatusCode::FORBIDDEN, "only the AgentMux host can pause or resume an agent".to_string());
    }
    let block_id = body.get("block_id").and_then(|v| v.as_str()).unwrap_or("").trim();
    let Some(on) = body.get("paused").and_then(|v| v.as_bool()) else {
        return err_response(StatusCode::BAD_REQUEST, "say whether to pause (`paused`: true or false)".to_string());
    };
    let pane = banner_for(block_id).0;
    if !matches!(state.mstore.get::<crate::backend::obj::Block>(&pane), Ok(Some(_))) {
        return err_response(StatusCode::NOT_FOUND, "the pane is gone".to_string());
    }
    if on && crate::server::browser_owner::owner_of(&pane).is_none() {
        return err_response(StatusCode::CONFLICT, "no agent drives this pane any more".to_string());
    }
    set_paused(&state, &pane, on);
    (StatusCode::OK, Json(json!({ "ok": true, "data": { "paused": is_paused(&pane) } }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(n: usize) -> Entry {
        note("x", "naki", Kind::Click).target(&format!("e{n}")).entry(How::Allowed, None)
    }

    #[test]
    fn the_ring_keeps_the_newest_two_hundred() {
        for n in 0..250 {
            add("ba-ring", entry(n));
        }
        let (count, kept) = entries("ba-ring");
        assert_eq!(count, 250);
        assert_eq!(kept.len(), RING);
        assert_eq!(kept[0].target.as_deref(), Some("e50"));
        assert_eq!(kept[RING - 1].target.as_deref(), Some("e249"));
        drop_pane("ba-ring");
        assert_eq!(entries("ba-ring"), (0, Vec::new()));
    }

    #[test]
    fn a_fill_never_keeps_the_value() {
        let req: agentmux_common::api_types::UiBrowserActRequest = serde_json::from_value(json!({
            "agent_id": "naki", "ts_secs": 1, "sig": "s", "pane": "p", "ref": "e7", "action": "fill",
            "text": "hunter2-secret-value",
        }))
        .unwrap();
        let n = Note::act("p", &req);
        let e = n.entry(How::Allowed, None);
        assert_eq!(e.kind, Kind::Fill);
        assert_eq!(e.target.as_deref(), Some("e7"));
        let wire = serde_json::to_string(&e).unwrap();
        assert!(!wire.contains("hunter2"), "{wire}");
        // A failed fill's refusal reason comes from the host, never the text.
        let e = note("p", "naki", Kind::Fill).target("e7").entry(How::Refused("a secret field: use handoff".into()), None);
        assert_eq!(e.how, "refused: a secret field");
    }

    #[test]
    fn each_pane_has_its_own_log() {
        add("ba-one", entry(1));
        add("ba-two", entry(2));
        add("ba-two", entry(3));
        assert_eq!(entries("ba-one").1.len(), 1);
        assert_eq!(entries("ba-two").1.len(), 2);
        drop_pane("ba-one");
        assert_eq!(entries("ba-one").0, 0);
        assert_eq!(entries("ba-two").1.len(), 2, "closing one pane leaves the other's log");
        drop_pane("ba-two");
    }

    #[test]
    fn hows_and_kinds_read_as_the_log_shows_them() {
        assert_eq!(How::AutoApproved.label(), "auto-approved");
        assert_eq!(How::TimedOut.label(), "timed out");
        assert_eq!(How::Refused("off the pane's sites. Ask the user".into()).label(), "refused: off the pane's sites");
        assert_eq!(serde_json::to_value(Kind::AutoApproveOn).unwrap(), json!("auto-approve-on"));
        let e = note("p", "naki", Kind::Click)
            .target("e3")
            .asked(&json!({ "what": "Click \"Delete draft\"", "page": "https://github.com/x" }))
            .entry(How::Approved, None);
        assert_eq!(e.target.as_deref(), Some("\"Delete draft\""));
        assert_eq!(e.url.as_deref(), Some("https://github.com/x"));
        assert_eq!(e.detail.as_deref(), Some("ref e3"));
    }

    #[test]
    fn pausing_moves_the_grants_end_by_the_paused_time() {
        let t0 = Instant::now();
        let expires = t0 + Duration::from_secs(15 * 60);
        // Paused 5 minutes in, resumed 25 minutes later: 10 minutes were
        // left, and 10 minutes are left after resuming.
        let paused_at = t0 + Duration::from_secs(5 * 60);
        let now = paused_at + Duration::from_secs(25 * 60);
        let (end, end_ms) = resumed_clock(expires, 1_000_000, paused_at, now);
        assert_eq!(end, expires + Duration::from_secs(25 * 60));
        assert_eq!(end - now, Duration::from_secs(10 * 60));
        assert_eq!(end_ms, 1_000_000 + 25 * 60 * 1000);
        // A resume can't move the end back.
        assert_eq!(resumed_clock(expires, 5, now, paused_at), (expires, 5));
    }

    #[test]
    fn a_paused_pane_refuses_and_a_resumed_one_does_not() {
        table().paused.insert("ba-paused".into(), Instant::now());
        let (code, msg) = not_paused("ba-paused").unwrap_err();
        assert_eq!(code, StatusCode::CONFLICT);
        assert!(msg.starts_with("the user paused this pane"), "{msg}");
        forget_pause("ba-paused");
        assert!(not_paused("ba-paused").is_ok());
    }
}
