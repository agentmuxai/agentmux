// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A page's own dialogs (alert, confirm, prompt, "Leave site?") in a browser
//! pane an agent drives (docs/specs/SPEC_BROWSER_PANE_NATIVE_DIALOGS_2026_10_17.md).
//! The host parks each one and asks here (`/api/v1/host/browser_dialog`); this
//! decides by policy (§5), or asks the person in the pane and answers the host
//! later. The agent can never accept a confirm itself: only Auto-approve, for
//! the confirm that follows its own click, or the person.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};

use crate::server::browser_attention::{self, Answer, Kind};
use crate::server::ui_handlers::{banner_for, err_response, from_host, get_host_ipc, proxy_ack, verified_block_id};
use crate::server::AppState;

/// How long after a committing click a confirm still counts as its own (§5.2).
const PAIR_WINDOW: Duration = Duration::from_secs(5);
/// How long the person has to answer. The host gives up a minute later.
const ASK_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Dialog outcomes kept per agent until its next `Browser` call reads them.
const LOG_MAX: usize = 20;
/// The page's words kept and shown: no more than this.
const MESSAGE_MAX: usize = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum How {
    /// Made under Auto-approve, without asking.
    Auto,
    /// The person approved it in the banner.
    Approved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DialogKind {
    Alert,
    Confirm,
    Prompt,
    BeforeUnload,
}

impl DialogKind {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "alert" => Some(Self::Alert),
            "confirm" => Some(Self::Confirm),
            "prompt" => Some(Self::Prompt),
            "beforeunload" => Some(Self::BeforeUnload),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Alert => "alert",
            Self::Confirm => "confirm",
            Self::Prompt => "prompt",
            Self::BeforeUnload => "beforeunload",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Decision {
    Accept(String),
    Cancel(String),
    Ask(String),
}

struct Click {
    at: Instant,
    label: String,
    how: How,
    paired: bool,
}

fn clicks() -> std::sync::MutexGuard<'static, HashMap<String, Click>> {
    static C: OnceLock<Mutex<HashMap<String, Click>>> = OnceLock::new();
    C.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner())
}

/// Remember a committing click about to be made on `target` (a pane or popup
/// window), so the confirm it opens can be matched to it. Called before the
/// click is sent: the page shows its confirm while the click is still running.
pub(crate) fn note_click(target: &str, label: &str, how: How) {
    let mut c = clicks();
    c.retain(|_, k| k.at.elapsed() < PAIR_WINDOW);
    c.insert(target.to_string(), Click { at: Instant::now(), label: label.to_string(), how, paired: false });
}

/// [`note_click`] for a click the person approved in the banner; `ask` is the
/// approval it was shown as.
pub(crate) fn note_approved_click(target: &str, ask: &Value) {
    note_click(target, ask.get("what").and_then(|v| v.as_str()).unwrap_or(""), How::Approved);
}

/// The click a dialog on `target` belongs to: one made there within the
/// window and not matched to a dialog yet. One click, one confirm.
fn pair(target: &str) -> Option<(How, String)> {
    let mut c = clicks();
    let k = c.get_mut(target)?;
    if k.paired || k.at.elapsed() >= PAIR_WINDOW {
        return None;
    }
    k.paired = true;
    Some((k.how, k.label.clone()))
}

/// Words that mean a dialog is about a secret. Such a dialog is never
/// answered for the agent: it only ever turns an accept into a question (§5.3).
fn about_a_secret(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    let words: Vec<&str> = m.split(|c: char| !c.is_ascii_alphanumeric()).filter(|s| !s.is_empty()).collect();
    let word = |w: &str| words.contains(&w);
    ["password", "passcode", "passphrase", "pin", "otp", "cvv", "cvc", "2fa", "mfa"].iter().any(|w| word(w))
        || ["security code", "verification code", "one-time", "one time code", "secret"].iter().any(|p| m.contains(p))
}

/// The policy (§5.1). `paired` is the click the dialog follows, if any;
/// `covered` says whether Auto-approve covers the site that showed it.
pub(crate) fn decide(kind: DialogKind, message: &str, paired: Option<(How, &str)>, covered: bool) -> Decision {
    match kind {
        DialogKind::Alert => Decision::Accept("it only informs".to_string()),
        DialogKind::Prompt => Decision::Cancel("it asks for typed text, which isn't answered for an agent".to_string()),
        DialogKind::Confirm | DialogKind::BeforeUnload => match paired {
            None => Decision::Ask("the page asked on its own, not right after a click of yours".to_string()),
            Some((How::Approved, _)) => {
                Decision::Ask("the user approved your click, not the page's question that followed it".to_string())
            }
            Some((How::Auto, _)) if !covered => {
                Decision::Ask("it came from a site auto-approve doesn't cover".to_string())
            }
            Some((How::Auto, _)) if crate::server::browser_bypass::is_payment(message) => {
                Decision::Ask("it is about a payment".to_string())
            }
            Some((How::Auto, _)) if about_a_secret(message) => Decision::Ask("it is about a secret".to_string()),
            Some((How::Auto, label)) => Decision::Accept(format!("auto-approve, right after your click on {label:?}")),
        },
    }
}

fn log() -> std::sync::MutexGuard<'static, HashMap<String, VecDeque<Value>>> {
    static L: OnceLock<Mutex<HashMap<String, VecDeque<Value>>>> = OnceLock::new();
    L.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner())
}

fn record(agent: &str, target: &str, kind: &str, message: &str, decision: &str, reason: &str) {
    tracing::info!(agent_id = %agent, target = %target, kind = %kind, decision = %decision, reason = %reason, "[browser-dialog] page dialog");
    if agent.is_empty() {
        return;
    }
    let mut l = log();
    let q = l.entry(agent.to_string()).or_default();
    if q.len() >= LOG_MAX {
        q.pop_front();
    }
    q.push_back(json!({
        "pane": target, "kind": kind, "message": message, "decision": decision, "reason": reason,
        "at": chrono::Utc::now().timestamp_millis(),
    }));
}

/// The dialogs `agent`'s panes showed since it last asked, oldest first.
pub(crate) fn drain(agent: &str) -> Vec<Value> {
    log().remove(agent).map(Vec::from).unwrap_or_default()
}

fn clip(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= MESSAGE_MAX {
        return s.to_string();
    }
    let mut out: String = s.chars().take(MESSAGE_MAX).collect();
    out.push('…');
    out
}

/// Tell the host how a parked dialog was answered.
async fn answer_host(state: &AppState, id: &str, accept: bool) {
    let Ok(host) = get_host_ipc(state).await else { return };
    if proxy_ack(state, &host, "dialog_answer", json!({ "id": id, "accept": accept })).await.is_err() {
        tracing::warn!(id = %id, "[browser-dialog] couldn't answer the host; its own timer will cancel the dialog");
    }
}

/// `POST /api/v1/host/browser_dialog` — the host parked a page's dialog in an
/// agent-driven pane and asks what to do (dialogs spec §4.2). Body
/// `{id, target, kind, message, origin}`; answers `accept`, `cancel`, or
/// `parked` (the person is asked; the answer goes to the host's
/// `dialog_answer` later). `{id, target, ended: true}`: the dialog ended on
/// its own (navigation, close), so a question about it comes down. `suppressed`:
/// the host cancelled it as one of a flood, and only reports it. Host only.
pub(crate) async fn handle_host_browser_dialog(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    if !from_host(&state, &headers).await {
        return err_response(StatusCode::FORBIDDEN, "only the AgentMux host can report a page's dialog".to_string());
    }
    let s = |k: &str| body.get(k).and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let flag = |k: &str| body.get(k).and_then(|v| v.as_bool()) == Some(true);
    let (id, target) = (s("id"), s("target"));
    let (banner_at, window) = banner_for(&target);
    let agent = crate::server::browser_owner::owner_of(&banner_at).unwrap_or_default();
    let answered = |decision: &str| (StatusCode::OK, Json(json!({ "ok": true, "data": { "decision": decision } }))).into_response();
    let message = clip(&s("message"));
    let kind_str = s("kind");
    if flag("ended") {
        browser_attention::cancel_kind_for(&banner_at, Kind::Dialog);
        record(&agent, &target, &kind_str, &message, "ended", "the page navigated or closed before it was answered");
        return answered("ok");
    }
    if flag("suppressed") {
        record(&agent, &target, &kind_str, &message, "cancelled", "the page showed more than 5 dialogs in 10 seconds");
        return answered("cancel");
    }
    let Some(kind) = DialogKind::parse(&kind_str) else {
        return answered("cancel");
    };
    let origin = s("origin");
    let paired = pair(&target);
    let covered = crate::server::browser_bypass::active_for(&banner_at, &agent).is_some_and(|g| g.covers(&origin));
    let reason = match decide(kind, &message, paired.as_ref().map(|(h, l)| (*h, l.as_str())), covered) {
        Decision::Accept(why) => {
            record(&agent, &target, kind.as_str(), &message, "accepted", &why);
            return answered("accept");
        }
        Decision::Cancel(why) => {
            record(&agent, &target, kind.as_str(), &message, "cancelled", &why);
            return answered("cancel");
        }
        Decision::Ask(why) => why,
    };
    record(&agent, &target, kind.as_str(), &message, "asking the user", &reason);
    let st = state.clone();
    let site = agentmux_common::allowed_origins::target_origin(&origin).unwrap_or(origin);
    tokio::spawn(async move {
        let asked = browser_attention::ask(
            &st,
            &banner_at,
            &agent,
            Kind::Dialog,
            json!({
                "what": "answer a question from the page",
                "message": message, "origin": site, "dialogKind": kind.as_str(), "window": window,
            }),
            ASK_TIMEOUT,
        )
        .await;
        let (accept, decision, why) = match &asked {
            Ok((Answer::Yes, _)) => (true, "accepted", "the user answered OK".to_string()),
            Ok((Answer::Cancelled, _)) => (false, "cancelled", "the user answered Cancel".to_string()),
            Ok((Answer::TimedOut, _)) => (false, "cancelled", "the user didn't answer within 10 minutes".to_string()),
            Err(e) => (false, "cancelled", format!("couldn't ask the user: {e}")),
        };
        answer_host(&st, &id, accept).await;
        record(&agent, &target, kind.as_str(), &message, decision, &why);
        // The pane stays paused until the host has the answer.
        drop(asked);
    });
    answered("parked")
}

/// `POST /api/v1/ui/browser/dialogs` — the page dialogs the calling agent's
/// panes showed since it last asked, and what became of each. Its `Browser`
/// tool reads it after every action, so the agent hears of each one; it works
/// while a pane is paused, which is when it matters.
pub(crate) async fn handle_ui_browser_dialogs(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<agentmux_common::api_types::UiBrowserDialogsRequest>,
) -> impl IntoResponse {
    if let Err(e) = verified_block_id(&state, caller.as_deref(), &req.auth) {
        return err_response(StatusCode::UNAUTHORIZED, e);
    }
    (StatusCode::OK, Json(json!({ "ok": true, "data": { "dialogs": drain(&req.auth.agent_id) } }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alerts_are_accepted_and_prompts_cancelled() {
        assert!(matches!(decide(DialogKind::Alert, "Saved", None, false), Decision::Accept(_)));
        assert!(matches!(decide(DialogKind::Prompt, "Name?", Some((How::Auto, "Go")), true), Decision::Cancel(_)));
    }

    #[test]
    fn a_confirm_is_accepted_only_after_an_auto_approved_click_on_a_covered_site() {
        let auto = Some((How::Auto, "Delete"));
        assert!(matches!(decide(DialogKind::Confirm, "Are you sure?", auto, true), Decision::Accept(_)));
        assert!(matches!(decide(DialogKind::BeforeUnload, "", auto, true), Decision::Accept(_)));
        // On its own, after a person-approved click, or off the granted sites: asks.
        assert!(matches!(decide(DialogKind::Confirm, "Are you sure?", None, true), Decision::Ask(_)));
        assert!(matches!(decide(DialogKind::Confirm, "Are you sure?", Some((How::Approved, "Delete")), true), Decision::Ask(_)));
        assert!(matches!(decide(DialogKind::Confirm, "Are you sure?", auto, false), Decision::Ask(_)));
    }

    #[test]
    fn money_and_secrets_always_ask() {
        let auto = Some((How::Auto, "Confirm"));
        assert!(matches!(decide(DialogKind::Confirm, "Pay $40 now?", auto, true), Decision::Ask(_)));
        assert!(matches!(decide(DialogKind::Confirm, "Send your password to support?", auto, true), Decision::Ask(_)));
        assert!(matches!(decide(DialogKind::Confirm, "Enter the verification code we sent", auto, true), Decision::Ask(_)));
        // "pin" as a word only: "shipping" isn't about a secret.
        assert!(matches!(decide(DialogKind::Confirm, "Delete this shipping address?", auto, true), Decision::Accept(_)));
    }

    #[test]
    fn one_click_pairs_with_one_dialog() {
        note_click("dlg-test-pane-1", "Click \"Delete\"", How::Auto);
        assert_eq!(pair("dlg-test-pane-1"), Some((How::Auto, "Click \"Delete\"".to_string())));
        assert_eq!(pair("dlg-test-pane-1"), None, "a second confirm isn't the click's");
        assert_eq!(pair("dlg-test-pane-2"), None, "another pane's click doesn't count");
    }

    #[test]
    fn a_click_too_long_ago_doesnt_pair() {
        note_click("dlg-test-pane-3", "Delete", How::Auto);
        clicks().get_mut("dlg-test-pane-3").unwrap().at -= PAIR_WINDOW;
        assert_eq!(pair("dlg-test-pane-3"), None);
    }

    #[test]
    fn the_log_is_drained_per_agent_and_bounded() {
        for i in 0..(LOG_MAX + 3) {
            record("dlg-test-agent", "p", "alert", &format!("m{i}"), "accepted", "it only informs");
        }
        record("dlg-test-other", "p", "alert", "x", "accepted", "it only informs");
        let got = drain("dlg-test-agent");
        assert_eq!(got.len(), LOG_MAX);
        assert_eq!(got[0]["message"], "m3", "the oldest go first");
        assert!(drain("dlg-test-agent").is_empty());
        assert_eq!(drain("dlg-test-other").len(), 1);
    }

    #[test]
    fn long_page_text_is_clipped() {
        let long = "x".repeat(MESSAGE_MAX + 50);
        assert_eq!(clip(&long).chars().count(), MESSAGE_MAX + 1);
        assert_eq!(clip("  short  "), "short");
    }
}
