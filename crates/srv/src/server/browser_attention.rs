// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent's browser pane asking the human for something: a hand-off
//! (sign in, a CAPTCHA, anything the agent mustn't do) or approval of a
//! committing action (submit, send, pay, delete)
//! (docs/specs/SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md §5.2, §5.4).
//!
//! The request is shown as a banner in the pane through the block's
//! `browser:attention` meta key. The human's answer comes back only through
//! the CEF host (`/api/v1/host/browser_attention`, authenticated with the
//! host's IPC token, which agents never see): an agent can't answer its own
//! request, even though it holds the instance auth key every other route
//! trusts.
//!
//! A pane's slot has two levels (REPORT_BROWSER_PANE_BLANK_AFTER_BLOCKED_SIGN_IN_REDIRECT_RCA_2026_10_16.md
//! §6.1): below, one hand-off, approval, auto-approve question or page
//! dialog; above it, one question or notice about a navigation the pane
//! stopped. A sign-in redirect during a hand-off is asked about over the
//! hand-off, whose banner returns when it's answered.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::server::AppState;

/// Block meta key the browser pane's banner reads.
pub(crate) const ATTENTION_META_KEY: &str = "browser:attention";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Handoff,
    Approval,
    /// The page tried to leave the sites the pane is limited to
    /// (SPEC_BROWSER_PANE_ALLOWED_ORIGINS_2026_10_09.md §4).
    Navigation,
    /// The agent asks to act without asking for a while
    /// (SPEC_BROWSER_PANE_BYPASS_2026_10_16.md §6).
    Bypass,
    /// The page's own confirm or "Leave site?" box
    /// (SPEC_BROWSER_PANE_NATIVE_DIALOGS_2026_10_17.md §5).
    Dialog,
    /// Why a navigation the pane stopped didn't load. Nothing waits on it:
    /// it doesn't pause the agent's tools or time out.
    Notice,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Handoff => "handoff",
            Kind::Approval => "approval",
            Kind::Navigation => "navigation",
            Kind::Bypass => "bypass",
            Kind::Dialog => "dialog",
            Kind::Notice => "notice",
        }
    }

    /// The slot's upper level: about a navigation the pane stopped, shown
    /// over whatever the pane already waits on.
    fn on_top(self) -> bool {
        matches!(self, Kind::Navigation | Kind::Notice)
    }

    /// Why the agent's tools on `pane` are refused while a request of this
    /// kind is up.
    pub(crate) fn tools_paused(self, pane: &str) -> String {
        let why = match self {
            Kind::Handoff => format!("the user is working in pane {pane:?} (your Browser handoff)"),
            Kind::Approval => format!("pane {pane:?} is waiting for the user to approve your click"),
            Kind::Navigation => format!(
                "the page in pane {pane:?} tried to leave its allowed_origins and the user is being asked whether to allow it"
            ),
            Kind::Bypass => format!("pane {pane:?} is asking the user whether to auto-approve your clicks"),
            Kind::Dialog => format!(
                "the page in pane {pane:?} is showing its own question (a confirm or \"Leave site?\" box) and the user is being asked to answer it"
            ),
            // Never pauses anything (`waiting_on_user`); named for completeness.
            Kind::Notice => format!("pane {pane:?} is showing the user why a navigation stopped"),
        };
        let until = if self == Kind::Handoff { "they click Done or Cancel" } else { "they answer" };
        format!("{why}; your tools on it are paused until {until}")
    }
}

/// The human's answer, or the lack of one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Answer {
    /// Done (hand-off) or Approve (approval).
    Yes,
    Cancelled,
    TimedOut,
}

impl Answer {
    pub(crate) fn parse(decision: &str) -> Option<Answer> {
        match decision {
            "done" | "approve" => Some(Answer::Yes),
            "cancel" => Some(Answer::Cancelled),
            _ => None,
        }
    }
}

/// Why a request couldn't be put up. Callers handle each: a busy slot must
/// never turn into silence (RCA §7.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AskError {
    /// The pane's slot for this kind is taken, by a request of `kind`.
    Busy { kind: Kind },
    /// The banner couldn't be written to the pane.
    NotShown(String),
}

impl std::fmt::Display for AskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AskError::Busy { kind } => write!(f, "this pane is already waiting for the user (its pending request: {})", kind.as_str()),
            AskError::NotShown(e) => write!(f, "couldn't show the request in the pane: {e}"),
        }
    }
}

impl AskError {
    /// What a route answers the agent with: wait and retry, or a fault.
    pub(crate) fn status(&self) -> axum::http::StatusCode {
        match self {
            AskError::Busy { .. } => axum::http::StatusCode::CONFLICT,
            AskError::NotShown(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

struct Pending {
    block_id: String,
    kind: Kind,
    /// Taken by the first answer. The entry itself stays until the asker's
    /// [`Waiting`] is dropped, so the pane stays locked until it's done.
    tx: Option<oneshot::Sender<Answer>>,
    /// The banner it shows, kept so it can come back when a question shown
    /// over it is answered.
    banner: Value,
    /// Arrival order: on one level, the newest request shows.
    seq: u64,
}

fn pending() -> &'static Mutex<HashMap<String, Pending>> {
    static P: OnceLock<Mutex<HashMap<String, Pending>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

fn next_seq() -> u64 {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

/// What `block_id` is waiting on the human for, if anything. While it
/// waits, the agent's tools on that pane are refused: during a hand-off so
/// it can't watch what the human types (§5.2), during an approval so it
/// can't change the form the human is reading (§5.4). A notice holds
/// nothing. With a question over another request, the one under it is named:
/// it holds the pane longest.
pub(crate) fn waiting_on_user(block_id: &str) -> Option<Kind> {
    pending()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .filter(|p| p.block_id == block_id && p.kind != Kind::Notice)
        .min_by_key(|p| p.kind.on_top())
        .map(|p| p.kind)
}

/// The panes with a request of `kind` up.
pub(crate) fn asking(kind: Kind) -> Vec<String> {
    pending()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .filter(|p| p.kind == kind)
        .map(|p| p.block_id.clone())
        .collect()
}

/// Resolve request `id` on `block_id` with the human's answer. Called only
/// by the host-authenticated route.
pub(crate) fn resolve(id: &str, block_id: &str, answer: Answer) -> Result<(), String> {
    let mut map = pending().lock().unwrap_or_else(|p| p.into_inner());
    let tx = match map.get_mut(id) {
        Some(p) if p.block_id == block_id => p.tx.take(),
        Some(_) => return Err("that request belongs to another pane".to_string()),
        None => return Err("no such request (it may have timed out)".to_string()),
    };
    let Some(tx) = tx else {
        return Err("that request was already answered".to_string());
    };
    let _ = tx.send(answer);
    Ok(())
}

/// Answer every request waiting on `block_id` with Cancelled: the user took
/// the pane over, which ends whatever the agent was waiting for there.
pub(crate) fn cancel_for(block_id: &str) {
    let mut map = pending().lock().unwrap_or_else(|p| p.into_inner());
    for p in map.values_mut().filter(|p| p.block_id == block_id) {
        if let Some(tx) = p.tx.take() {
            let _ = tx.send(Answer::Cancelled);
        }
    }
}

/// Answer `block_id`'s unanswered request of `kind`, if any, with Cancelled:
/// what it asked about ended on its own (a page dialog whose page navigated).
pub(crate) fn cancel_kind_for(block_id: &str, kind: Kind) {
    let mut map = pending().lock().unwrap_or_else(|p| p.into_inner());
    for p in map.values_mut().filter(|p| p.block_id == block_id && p.kind == kind) {
        if let Some(tx) = p.tx.take() {
            let _ = tx.send(Answer::Cancelled);
        }
    }
}

/// A request that is still up. Dropping it takes the request down: it
/// leaves the pending map, which unlocks the pane, and the banner shows what
/// is left (the request under it, or nothing). That happens on every path,
/// including when the waiting HTTP handler is cancelled because its caller
/// went away.
pub(crate) struct Waiting {
    state: AppState,
    id: String,
    block_id: String,
    kind: Kind,
}

impl Drop for Waiting {
    fn drop(&mut self) {
        // Removing the entry and showing what's left happen under one hold
        // of the lock, so a new request on the pane can't publish its banner
        // in between and lose it.
        let mut map = pending().lock().unwrap_or_else(|p| p.into_inner());
        map.remove(&self.id);
        if let Err(e) = show(&self.state, &map, &self.block_id) {
            tracing::debug!(pane = %self.block_id, error = %e, "[browser-attention] banner not updated");
        }
        drop(map);
        if self.kind == Kind::Handoff {
            crate::server::browser_owner::changed().notify_one();
        }
    }
}

/// Show `block_id`'s banner from its requests: the newest unanswered one,
/// the upper level first. With none unanswered, the banner already shown
/// stays while its request is still being acted on (an approved click being
/// made), so an answered banner never comes back. Called with the map
/// locked, so two changes can't publish out of order.
fn show(state: &AppState, map: &HashMap<String, Pending>, block_id: &str) -> Result<(), String> {
    let shown = state
        .mstore
        .get::<crate::backend::obj::Block>(block_id)
        .ok()
        .flatten()
        .and_then(|b| b.meta.get(ATTENTION_META_KEY).and_then(|v| v.get("id")).and_then(|v| v.as_str()).map(str::to_string));
    let top = map
        .iter()
        .filter(|(_, p)| p.block_id == block_id && p.tx.is_some())
        .max_by_key(|(_, p)| (p.kind.on_top(), p.seq));
    let want = match top {
        Some((id, p)) => Some((id.as_str(), p.banner.clone())),
        None if shown.as_deref().is_some_and(|s| map.get(s).is_some_and(|p| p.block_id == block_id)) => return Ok(()),
        None => None,
    };
    if want.as_ref().map(|(id, _)| *id) == shown.as_deref() {
        return Ok(());
    }
    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert(ATTENTION_META_KEY.to_string(), want.map(|(_, b)| b).unwrap_or(Value::Null));
    crate::server::http_shell::broadcast_meta_update(state, block_id, &meta).map(|_| ())
}

/// The human answered a banner srv has no request for (srv restarted, so its
/// requests are gone but the block meta persisted): take that banner down,
/// so its buttons don't sit there doing nothing. Host-authenticated callers only.
pub(crate) fn clear_stale(state: &AppState, block_id: &str, id: &str) {
    let map = pending().lock().unwrap_or_else(|p| p.into_inner());
    if map.contains_key(id) {
        return;
    }
    let _ = show(state, &map, block_id);
}

/// Whether a new request of `kind` on `block_id` must wait, and on which
/// kind. Each level holds one request at a time, except that a page dialog
/// may follow one already answered (the approved click opened it). A notice
/// holds nothing and is replaced by anything new, but doesn't cover a
/// navigation question still unanswered.
fn slot_taken(map: &HashMap<String, Pending>, block_id: &str, kind: Kind) -> Option<Kind> {
    map.values()
        .filter(|p| p.block_id == block_id && p.kind != Kind::Notice)
        .find(|p| match kind {
            Kind::Notice => p.kind == Kind::Navigation && p.tx.is_some(),
            Kind::Dialog => !p.kind.on_top() && p.tx.is_some(),
            _ => p.kind.on_top() == kind.on_top(),
        })
        .map(|p| p.kind)
}

/// A request put up in its pane's banner, not yet answered.
pub(crate) struct Raised {
    rx: oneshot::Receiver<Answer>,
    waiting: Waiting,
    _asking: Option<crate::backend::user_attention::Asking>,
}

impl Raised {
    /// The human's answer, after up to `timeout` (`None`: no limit). A
    /// request taken down without one (replaced, or its pane closed) reads
    /// as Cancelled. The pane stays as it is until the returned [`Waiting`]
    /// is dropped.
    pub(crate) async fn answer(self, timeout: Option<Duration>) -> (Answer, Waiting) {
        let Raised { rx, waiting, _asking } = self;
        let answer = match timeout {
            Some(t) => match tokio::time::timeout(t, rx).await {
                Ok(Ok(a)) => a,
                Ok(Err(_)) => Answer::Cancelled,
                Err(_) => Answer::TimedOut,
            },
            None => rx.await.unwrap_or(Answer::Cancelled),
        };
        (answer, waiting)
    }
}

/// Put `payload` up in `block_id`'s banner as a request of `kind`, without
/// waiting for the answer: the caller learns at once whether the slot was
/// free. A notice on the pane is replaced.
pub(crate) fn raise(state: &AppState, block_id: &str, agent_id: &str, kind: Kind, payload: Value) -> Result<Raised, AskError> {
    let id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel();
    let mut banner = json!({ "id": id, "kind": kind.as_str(), "agent": agent_id });
    if let (Some(b), Some(p)) = (banner.as_object_mut(), payload.as_object()) {
        for (k, v) in p {
            b.insert(k.clone(), v.clone());
        }
    }
    let mut map = pending().lock().unwrap_or_else(|p| p.into_inner());
    if let Some(busy) = slot_taken(&map, block_id, kind) {
        return Err(AskError::Busy { kind: busy });
    }
    // Its sender goes with it: the notice's own wait ends as dismissed.
    map.retain(|_, p| !(p.block_id == block_id && p.kind == Kind::Notice));
    let pending = Pending { block_id: block_id.to_string(), kind, tx: Some(tx), banner: banner.clone(), seq: next_seq() };
    map.insert(id.clone(), pending);
    let waiting = Waiting { state: state.clone(), id, block_id: block_id.to_string(), kind };
    let shown = show(state, &map, block_id);
    // Unlocked before `waiting` can drop, which locks it again.
    drop(map);
    shown.map_err(AskError::NotShown)?;
    // The host leaves a page's dialogs to the person during a hand-off
    // (dialogs spec §4.3): it learns of one from the next push.
    if kind == Kind::Handoff {
        crate::server::browser_owner::changed().notify_one();
    }
    // The pane waits on the user until they answer or it times out: the
    // waiting tone and notification for it
    // (REPORT_AGENT_ATTENTION_CTA_CONTRAST_AND_TONE_2026_10_10.md §2). A
    // notice waits on nobody.
    let _asking = (kind != Kind::Notice).then(|| {
        let what = ["reason", "what", "origin"]
            .iter()
            .find_map(|k| banner.get(*k).and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();
        let who = if agent_id.is_empty() { "An agent" } else { agent_id };
        let text = if what.is_empty() { format!("{who} needs you") } else { format!("{who} needs you: {what}") };
        crate::backend::user_attention::Asking::start(&state.broker, &state.mstore, block_id, "browser", &text)
    });
    Ok(Raised { rx, waiting, _asking })
}

/// Show `payload` in `block_id`'s banner and wait for the human, up to
/// `timeout`. The pane stays locked until the returned [`Waiting`] is
/// dropped, so a caller that acts on a Yes holds it until that action is
/// done. See [`slot_taken`] for what may be up at the same time.
pub(crate) async fn ask(
    state: &AppState,
    block_id: &str,
    agent_id: &str,
    kind: Kind,
    payload: Value,
    timeout: Duration,
) -> Result<(Answer, Waiting), AskError> {
    Ok(raise(state, block_id, agent_id, kind, payload)?.answer(Some(timeout)).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn park(id: &str, block: &str, kind: Kind) -> oneshot::Receiver<Answer> {
        let (tx, rx) = oneshot::channel();
        let p = Pending { block_id: block.to_string(), kind, tx: Some(tx), banner: json!({ "id": id }), seq: next_seq() };
        pending().lock().unwrap().insert(id.to_string(), p);
        rx
    }

    /// A browser pane in `state`'s store, for the banner to be written to.
    fn browser_block(state: &AppState, id: &str) {
        let mut meta = crate::backend::obj::MetaMapType::new();
        meta.insert("view".to_string(), json!("browser"));
        let mut block = crate::backend::obj::Block {
            oid: id.to_string(),
            parentoref: String::new(),
            version: 1,
            runtimeopts: None,
            stickers: None,
            meta,
            subblockids: None,
        };
        state.mstore.insert(&mut block).unwrap();
    }

    fn banner(state: &AppState, id: &str) -> Value {
        let block = state.mstore.must_get::<crate::backend::obj::Block>(id).unwrap();
        block.meta.get(ATTENTION_META_KEY).cloned().unwrap_or(Value::Null)
    }

    #[test]
    fn a_navigation_question_may_come_over_a_handoff_but_not_over_another() {
        let _h = park("attn-test-10", "pane-k", Kind::Handoff);
        let taken = |kind: Kind| slot_taken(&pending().lock().unwrap(), "pane-k", kind);
        assert_eq!(taken(Kind::Navigation), None, "the upper level is free");
        assert_eq!(taken(Kind::Approval), Some(Kind::Handoff), "the lower one isn't");
        let _n = park("attn-test-11", "pane-k", Kind::Navigation);
        assert_eq!(taken(Kind::Navigation), Some(Kind::Navigation), "one question about a navigation at a time");
        assert_eq!(taken(Kind::Notice), Some(Kind::Navigation), "a notice doesn't cover an unanswered question");
        // The hand-off is what the pane is locked for.
        assert_eq!(waiting_on_user("pane-k"), Some(Kind::Handoff));
        pending().lock().unwrap().remove("attn-test-10");
        assert_eq!(waiting_on_user("pane-k"), Some(Kind::Navigation));
        pending().lock().unwrap().remove("attn-test-11");
    }

    #[test]
    fn a_notice_locks_nothing_and_holds_no_slot() {
        let _rx = park("attn-test-12", "pane-l", Kind::Notice);
        assert_eq!(waiting_on_user("pane-l"), None, "the agent's tools aren't paused");
        let taken = |kind: Kind| slot_taken(&pending().lock().unwrap(), "pane-l", kind);
        for kind in [Kind::Handoff, Kind::Approval, Kind::Navigation, Kind::Dialog, Kind::Notice] {
            assert_eq!(taken(kind), None, "{kind:?}");
        }
        pending().lock().unwrap().remove("attn-test-12");
    }

    #[test]
    fn a_busy_slot_names_what_holds_it() {
        let e = AskError::Busy { kind: Kind::Handoff };
        assert!(e.to_string().contains("already waiting") && e.to_string().contains("handoff"), "{e}");
    }

    // The RCA's headline (§6.4), at this level: a question over a hand-off
    // shows, and when it's answered the hand-off's banner returns while the
    // hand-off itself still waits.
    #[tokio::test]
    async fn the_handoff_banner_returns_after_a_question_over_it() {
        let state = crate::server::tests::test_state();
        let pane = uuid::Uuid::new_v4().to_string();
        browser_block(&state, &pane);
        let handoff = raise(&state, &pane, "agent-m", Kind::Handoff, json!({ "reason": "sign in" })).unwrap();
        let h_id = banner(&state, &pane)["id"].as_str().unwrap().to_string();
        let handoff_wait = tokio::spawn(handoff.answer(Some(Duration::from_secs(30))));
        let question = raise(&state, &pane, "agent-m", Kind::Navigation, json!({ "origin": "https://b.example" })).unwrap();
        let shown = banner(&state, &pane);
        assert_eq!(shown["kind"], "navigation");
        assert!(matches!(
            raise(&state, &pane, "agent-m", Kind::Navigation, json!({})),
            Err(AskError::Busy { kind: Kind::Navigation })
        ));
        resolve(shown["id"].as_str().unwrap(), &pane, Answer::Yes).unwrap();
        let (answer, waiting) = question.answer(Some(Duration::from_secs(5))).await;
        assert_eq!(answer, Answer::Yes);
        drop(waiting);
        let back = banner(&state, &pane);
        assert_eq!((back["kind"].as_str(), back["id"].as_str()), (Some("handoff"), Some(h_id.as_str())));
        assert_eq!(back["reason"], "sign in");
        assert!(!handoff_wait.is_finished(), "the hand-off still waits");
        assert_eq!(waiting_on_user(&pane), Some(Kind::Handoff));
        resolve(&h_id, &pane, Answer::Yes).unwrap();
        let (answer, waiting) = handoff_wait.await.unwrap();
        assert_eq!(answer, Answer::Yes);
        drop(waiting);
        assert!(banner(&state, &pane).is_null());
    }

    #[tokio::test]
    async fn a_new_request_replaces_a_notice() {
        let state = crate::server::tests::test_state();
        let pane = uuid::Uuid::new_v4().to_string();
        browser_block(&state, &pane);
        let notice = raise(&state, &pane, "", Kind::Notice, json!({ "why": "You blocked it." })).unwrap();
        assert_eq!(banner(&state, &pane)["kind"], "notice");
        let notice_wait = tokio::spawn(notice.answer(None));
        let approval = raise(&state, &pane, "agent-n", Kind::Approval, json!({ "what": "Click \"Send\"" })).unwrap();
        assert_eq!(banner(&state, &pane)["kind"], "approval");
        // The notice's own wait ends, as dismissed, without taking the approval down.
        let (answer, waiting) = notice_wait.await.unwrap();
        assert_eq!(answer, Answer::Cancelled);
        drop(waiting);
        assert_eq!(banner(&state, &pane)["kind"], "approval");
        drop(approval);
        assert!(banner(&state, &pane).is_null());
    }

    #[test]
    fn decisions_parse() {
        assert_eq!(Answer::parse("done"), Some(Answer::Yes));
        assert_eq!(Answer::parse("approve"), Some(Answer::Yes));
        assert_eq!(Answer::parse("cancel"), Some(Answer::Cancelled));
        assert_eq!(Answer::parse("yes please"), None);
    }

    #[tokio::test]
    async fn resolve_answers_the_waiting_request_once() {
        let rx = park("attn-test-1", "pane-a", Kind::Approval);
        resolve("attn-test-1", "pane-a", Answer::Yes).unwrap();
        assert_eq!(rx.await.unwrap(), Answer::Yes);
        assert!(resolve("attn-test-1", "pane-a", Answer::Yes).is_err(), "single use");
        // Answered, but the asker hasn't finished acting on it: still locked.
        assert_eq!(waiting_on_user("pane-a"), Some(Kind::Approval));
        pending().lock().unwrap().remove("attn-test-1");
    }

    #[tokio::test]
    async fn take_over_cancels_what_the_pane_waits_for() {
        let rx = park("attn-test-5", "pane-f", Kind::Approval);
        let _other = park("attn-test-6", "pane-g", Kind::Approval);
        cancel_for("pane-f");
        assert_eq!(rx.await.unwrap(), Answer::Cancelled);
        // A late Approve finds the request already answered.
        assert!(resolve("attn-test-5", "pane-f", Answer::Yes).is_err());
        // Another pane's request is untouched.
        assert!(resolve("attn-test-6", "pane-g", Answer::Yes).is_ok());
        pending().lock().unwrap().remove("attn-test-5");
        pending().lock().unwrap().remove("attn-test-6");
    }

    #[test]
    fn a_request_is_answered_only_for_its_own_pane() {
        let _rx = park("attn-test-2", "pane-b", Kind::Approval);
        assert!(resolve("attn-test-2", "pane-other", Answer::Yes).is_err());
        pending().lock().unwrap().remove("attn-test-2");
    }

    #[test]
    fn a_page_dialog_may_follow_an_answered_request_only() {
        let _rx = park("attn-test-7", "pane-h", Kind::Approval);
        let busy = |kind: Kind| slot_taken(&pending().lock().unwrap(), "pane-h", kind).is_some();
        assert!(busy(Kind::Dialog), "the approval is still up");
        resolve("attn-test-7", "pane-h", Answer::Yes).unwrap();
        assert!(!busy(Kind::Dialog), "approved, and the click it allowed opened the dialog");
        assert!(busy(Kind::Handoff), "anything else still waits its turn");
        pending().lock().unwrap().remove("attn-test-7");
    }

    #[tokio::test]
    async fn a_dialog_that_ended_is_cancelled() {
        let rx = park("attn-test-8", "pane-i", Kind::Dialog);
        let _other = park("attn-test-9", "pane-j", Kind::Dialog);
        cancel_kind_for("pane-i", Kind::Approval);
        cancel_kind_for("pane-i", Kind::Dialog);
        assert_eq!(rx.await.unwrap(), Answer::Cancelled);
        assert!(resolve("attn-test-9", "pane-j", Answer::Yes).is_ok());
        pending().lock().unwrap().remove("attn-test-8");
        pending().lock().unwrap().remove("attn-test-9");
    }

    #[test]
    fn a_pending_request_of_either_kind_locks_the_pane() {
        let _rx = park("attn-test-3", "pane-c", Kind::Handoff);
        assert_eq!(waiting_on_user("pane-c"), Some(Kind::Handoff));
        assert_eq!(waiting_on_user("pane-d"), None);
        pending().lock().unwrap().remove("attn-test-3");
        assert_eq!(waiting_on_user("pane-c"), None);
        let _rx = park("attn-test-4", "pane-e", Kind::Approval);
        assert_eq!(waiting_on_user("pane-e"), Some(Kind::Approval));
        pending().lock().unwrap().remove("attn-test-4");
    }
}
