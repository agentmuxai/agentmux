// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! In-process Chrome DevTools Protocol (CDP) client.
//!
//! One session = one CEF `Browser`, addressed by its host-state label
//! (`main`, `window-*`, `floating-*`, or a browser pane's label). Messages go
//! through CEF's own DevTools message API — `CefBrowserHost::SendDevToolsMessage`
//! out, a `CefDevToolsMessageObserver` back — which needs no DevTools front
//! end and no remote-debugging server. That is the point (#3681): the host no
//! longer has to run a TCP CDP server that any local user could connect to,
//! so release builds don't start one unless asked (`AGENTMUX_CDP_PORT`, see
//! `crate::cdp_port`).
//!
//! Threading: `SendDevToolsMessage` must be called on the CEF UI thread (it
//! returns false anywhere else), while callers are tokio tasks. Each `call`
//! registers a one-shot reply slot keyed by message id, posts the send to the
//! UI thread, and awaits the slot with a timeout, so a lost reply can't hang
//! a caller. The observer (one per browser, registered lazily on the UI
//! thread) completes the slot. When a browser closes, its observer
//! registration is dropped and its outstanding calls fail immediately
//! (`on_browser_closed`, called from the life-span handler).
//!
//! CDP protocol: every request is `{id, method, params}`; every reply with a
//! matching `id` is `{id, result}` or `{id, error}`. Events (no id) are left
//! to other observers, except the two that tell us about frames running in
//! another process (below).
//!
//! Frames (docs/specs/SPEC_BROWSER_PANE_FORMS_IN_FRAMES_AND_VISUAL_FALLBACK_2026_10_16.md
//! §4.1): an iframe from another site runs in its own process, and CDP
//! reaches it only through its own *session*: `Target.setAutoAttach` with
//! `flatten: true` makes Chromium announce it (`Target.attachedToTarget`,
//! carrying a `sessionId`), and a message with that `sessionId` at its top
//! level is routed to it. [`CdpSession::attach_frames`] turns that on and
//! returns the sessions; [`CdpSession::in_session`] addresses one. Replies
//! still match by `id` alone: ids are unique across every session.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use cef::*;
use parking_lot::Mutex;
use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::state::AppState;

pub type CdpError = String;

/// Same bound the old WebSocket client used. CDP replies are usually well
/// under 100 ms in-process; the generous bound covers pages stalled in JS and
/// large `Page.captureScreenshot` payloads.
const CALL_TIMEOUT: Duration = Duration::from_secs(10);

// ── Reply matching (pure; unit-tested below) ────────────────────────────────

type Reply = Result<Value, CdpError>;

struct PendingCall {
    browser_id: i32,
    method: String,
    tx: oneshot::Sender<Reply>,
}

#[derive(Default)]
struct PendingCalls {
    last_id: i32,
    calls: HashMap<i32, PendingCall>,
}

impl PendingCalls {
    /// Reserve a message id for `method` on `browser_id` and return the
    /// receiver its reply will arrive on.
    fn register(&mut self, browser_id: i32, method: &str) -> (i32, oneshot::Receiver<Reply>) {
        // Positive, wrapping, never an id that is still outstanding.
        let id = loop {
            self.last_id = if self.last_id >= i32::MAX - 1 { 1 } else { self.last_id + 1 };
            if !self.calls.contains_key(&self.last_id) {
                break self.last_id;
            }
        };
        let (tx, rx) = oneshot::channel();
        self.calls.insert(
            id,
            PendingCall {
                browser_id,
                method: method.to_string(),
                tx,
            },
        );
        (id, rx)
    }

    /// Route one raw DevTools message that `browser_id`'s observer received.
    /// Returns true if it was the reply to a call of ours ON THAT BROWSER
    /// (consumed), false for events, anyone else's replies, or an id that
    /// belongs to a call on a different browser — so a second DevTools sender
    /// or an id overlap can never complete the wrong call (Opaz's review of
    /// #3832).
    fn deliver(&mut self, browser_id: i32, message: &[u8]) -> bool {
        let Ok(msg) = serde_json::from_slice::<Value>(message) else {
            return false;
        };
        let Some(id) = msg.get("id").and_then(Value::as_i64).and_then(|i| i32::try_from(i).ok()) else {
            return false;
        };
        if self.calls.get(&id).map(|c| c.browser_id) != Some(browser_id) {
            return false;
        }
        let Some(call) = self.calls.remove(&id) else {
            return false;
        };
        let reply = match msg.get("error") {
            Some(err) => Err(format!(
                "CDP {} error: {}",
                call.method,
                err.get("message").and_then(Value::as_str).unwrap_or("unknown"),
            )),
            None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
        };
        let _ = call.tx.send(reply);
        true
    }

    fn fail(&mut self, id: i32, reason: &str) {
        if let Some(call) = self.calls.remove(&id) {
            let _ = call.tx.send(Err(format!("CDP {}: {reason}", call.method)));
        }
    }

    /// Fail every call still outstanding on `browser_id` — it closed, so no
    /// reply is ever coming.
    fn fail_browser(&mut self, browser_id: i32, reason: &str) {
        let ids: Vec<i32> = self
            .calls
            .iter()
            .filter(|(_, c)| c.browser_id == browser_id)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.fail(id, reason);
        }
    }

    /// Drop a call whose caller gave up (timeout), so a late reply is ignored.
    fn forget(&mut self, id: i32) {
        self.calls.remove(&id);
    }
}

static PENDING: LazyLock<Mutex<PendingCalls>> = LazyLock::new(|| Mutex::new(PendingCalls::default()));

// ── Frames running in their own process ─────────────────────────────────────

/// How many levels of frame-inside-frame are followed: auto-attach is not
/// recursive, so each level costs one more `Target.setAutoAttach`.
const MAX_FRAME_DEPTH: usize = 4;

/// A frame Chromium attached us to, with the session to reach it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachedTarget {
    pub session_id: String,
    /// For an iframe target this is the frame's id (`Page.getFrameTree`).
    pub target_id: String,
    /// `iframe` is the only kind asked for.
    pub kind: String,
    pub url: String,
    /// The session the attach was announced on: `None` for the page's own.
    pub parent_session: Option<String>,
}

#[derive(Default)]
struct Attached {
    /// Per browser id, in the order they attached.
    targets: HashMap<i32, Vec<AttachedTarget>>,
    /// `(browser id, session id)` pairs `Target.setAutoAttach` was sent on
    /// (`""` for the page's own session), so it is sent once.
    configured: HashSet<(i32, String)>,
}

impl Attached {
    /// Learn from one raw DevTools message `browser_id`'s observer received.
    /// Only the attach, detach and info-changed events matter; everything
    /// else is ignored.
    fn note_event(&mut self, browser_id: i32, message: &[u8]) {
        let Ok(msg) = serde_json::from_slice::<Value>(message) else {
            return;
        };
        let Some(method) = msg.get("method").and_then(Value::as_str) else {
            return;
        };
        let params = msg.get("params");
        let text = |v: Option<&Value>, key: &str| v.and_then(|p| p.get(key)).and_then(Value::as_str).map(str::to_string);
        match method {
            "Target.attachedToTarget" => {
                let info = params.and_then(|p| p.get("targetInfo"));
                let (Some(session_id), Some(target_id)) = (text(params, "sessionId"), text(info, "targetId")) else {
                    return;
                };
                let list = self.targets.entry(browser_id).or_default();
                list.retain(|t| t.session_id != session_id);
                list.push(AttachedTarget {
                    session_id,
                    target_id,
                    kind: text(info, "type").unwrap_or_default(),
                    url: text(info, "url").unwrap_or_default(),
                    parent_session: msg.get("sessionId").and_then(Value::as_str).map(str::to_string),
                });
            }
            "Target.detachedFromTarget" => {
                let Some(gone) = text(params, "sessionId") else {
                    return;
                };
                self.forget_session(browser_id, &gone);
            }
            "Target.targetInfoChanged" => {
                let info = params.and_then(|p| p.get("targetInfo"));
                let (Some(target_id), Some(url)) = (text(info, "targetId"), text(info, "url")) else {
                    return;
                };
                if let Some(t) = self.targets.get_mut(&browser_id).and_then(|l| l.iter_mut().find(|t| t.target_id == target_id)) {
                    t.url = url;
                }
            }
            _ => {}
        }
    }

    /// A session ended: drop it and every session attached through it.
    fn forget_session(&mut self, browser_id: i32, session_id: &str) {
        let mut gone = vec![session_id.to_string()];
        if let Some(list) = self.targets.get_mut(&browser_id) {
            while let Some(s) = gone.pop() {
                gone.extend(list.iter().filter(|t| t.parent_session.as_deref() == Some(s.as_str())).map(|t| t.session_id.clone()));
                list.retain(|t| t.session_id != s);
                self.configured.remove(&(browser_id, s));
            }
        }
    }

    /// An attached frame whose own frames were never looked for (discovery
    /// stopped at [`MAX_FRAME_DEPTH`]): frames may exist that nobody saw.
    fn has_unexplored(&self, browser_id: i32) -> bool {
        self.targets
            .get(&browser_id)
            .is_some_and(|l| l.iter().any(|t| !self.configured.contains(&(browser_id, t.session_id.clone()))))
    }

    fn forget_browser(&mut self, browser_id: i32) {
        self.targets.remove(&browser_id);
        self.configured.retain(|(b, _)| *b != browser_id);
    }

    /// The `(parent session)` list auto-attach still has to be switched on
    /// for: the page's own session first, then every attached session not yet
    /// configured. Marks them configured.
    fn to_configure(&mut self, browser_id: i32) -> Vec<Option<String>> {
        let mut out = Vec::new();
        if self.configured.insert((browser_id, String::new())) {
            out.push(None);
        }
        let sessions: Vec<String> =
            self.targets.get(&browser_id).map(|l| l.iter().map(|t| t.session_id.clone()).collect()).unwrap_or_default();
        for s in sessions {
            if self.configured.insert((browser_id, s.clone())) {
                out.push(Some(s));
            }
        }
        out
    }
}

static ATTACHED: LazyLock<Mutex<Attached>> = LazyLock::new(|| Mutex::new(Attached::default()));

// ── CEF side (UI thread) ────────────────────────────────────────────────────

thread_local! {
    /// One observer registration per browser id. Only ever touched on the CEF
    /// UI thread: registrations are created in `SendTask::execute` and dropped
    /// in `on_browser_closed` / agent detach, all UI-thread callbacks. Dropping
    /// a `Registration` is what removes the observer.
    static OBSERVERS: RefCell<HashMap<i32, Registration>> = RefCell::new(HashMap::new());
}

wrap_dev_tools_message_observer! {
    struct ReplyObserver {}

    impl DevToolsMessageObserver {
        fn on_dev_tools_message(
            &self,
            browser: Option<&mut Browser>,
            message: Option<&[u8]>,
        ) -> ::std::os::raw::c_int {
            match (browser, message) {
                (Some(b), Some(m)) => {
                    let id = b.identifier();
                    if PENDING.lock().deliver(id, m) {
                        1
                    } else {
                        // An event, or someone else's reply: learn about
                        // frames from it, and leave it for other observers.
                        ATTACHED.lock().note_event(id, m);
                        0
                    }
                }
                _ => 0,
            }
        }

        fn on_dev_tools_agent_detached(&self, browser: Option<&mut Browser>) {
            if let Some(b) = browser {
                release_browser(b.identifier(), "DevTools agent detached");
            }
        }
    }
}

wrap_task! {
    struct SendTask {
        state: Arc<AppState>,
        label: String,
        // The browser the call was registered against. The label is looked
        // up again here, on the UI thread; if it now names a different
        // browser (the pane was re-created in between), fail fast rather
        // than send to the new one (Opaz's review of #3832).
        browser_id: i32,
        id: i32,
        message: String,
    }

    impl Task {
        fn execute(&self) {
            let Some(browser) = self.state.get_browser(&self.label) else {
                PENDING.lock().fail(self.id, &format!("browser {:?} is gone", self.label));
                return;
            };
            let Some(host) = browser.host() else {
                PENDING.lock().fail(self.id, &format!("browser {:?} has no host", self.label));
                return;
            };
            let browser_id = browser.identifier();
            if browser_id != self.browser_id {
                PENDING.lock().fail(
                    self.id,
                    &format!("browser {:?} was re-created before the call was sent", self.label),
                );
                return;
            }
            OBSERVERS.with(|observers| {
                let mut observers = observers.borrow_mut();
                if !observers.contains_key(&browser_id) {
                    let mut observer = ReplyObserver::new();
                    if let Some(registration) = host.add_dev_tools_message_observer(Some(&mut observer)) {
                        observers.insert(browser_id, registration);
                    }
                }
            });
            if host.send_dev_tools_message(Some(self.message.as_bytes())) == 0 {
                PENDING.lock().fail(self.id, "SendDevToolsMessage was refused");
            }
        }
    }
}

fn release_browser(browser_id: i32, reason: &str) {
    OBSERVERS.with(|observers| {
        observers.borrow_mut().remove(&browser_id);
    });
    ATTACHED.lock().forget_browser(browser_id);
    PENDING.lock().fail_browser(browser_id, reason);
}

/// Called from the life-span handler's `on_before_close` (UI thread) for
/// every browser: drop its observer and fail its outstanding calls, so a
/// reply can't land on a dead browser and no caller waits out the timeout.
pub fn on_browser_closed(browser_id: i32) {
    release_browser(browser_id, "browser closed");
}

// ── Session ─────────────────────────────────────────────────────────────────

pub struct CdpSession {
    state: Arc<AppState>,
    label: String,
    /// Calls go to this attached frame's session; `None`: the page's own.
    session: Option<String>,
}

impl CdpSession {
    /// A session against the browser registered under `label` in host state.
    /// Nothing is opened here; each `call` finds the browser afresh, so a
    /// session never outlives its browser in a harmful way.
    pub fn attach(state: &Arc<AppState>, label: &str) -> Self {
        Self {
            state: state.clone(),
            label: label.to_string(),
            session: None,
        }
    }

    /// The same browser, with every call addressed to `session` (an attached
    /// frame's session id), or to the page's own when `None`.
    pub fn in_session(&self, session: Option<&str>) -> Self {
        Self {
            state: self.state.clone(),
            label: self.label.clone(),
            session: session.map(str::to_string),
        }
    }

    fn browser_id(&self) -> Option<i32> {
        self.state.get_browser(&self.label).map(|b| b.identifier())
    }

    /// Make Chromium announce every iframe running in its own process (and,
    /// level by level, the frames inside those), and return the sessions to
    /// reach them. Cheap when already done: `Target.setAutoAttach` is sent
    /// once per session, and its state lives as long as the DevTools session.
    /// After [`attach_frames`](Self::attach_frames): whether discovery stopped
    /// at its depth limit with frames left unexplored, so the frame list may be
    /// incomplete.
    pub fn frames_may_be_missing(&self) -> bool {
        self.browser_id().is_some_and(|id| ATTACHED.lock().has_unexplored(id))
    }

    pub async fn attach_frames(&mut self) -> Vec<AttachedTarget> {
        let Some(browser_id) = self.browser_id() else {
            return Vec::new();
        };
        for _ in 0..MAX_FRAME_DEPTH {
            let todo = ATTACHED.lock().to_configure(browser_id);
            if todo.is_empty() {
                break;
            }
            for session in todo {
                let mut on = self.in_session(session.as_deref());
                let reply = on
                    .call(
                        "Target.setAutoAttach",
                        json!({
                            "autoAttach": true,
                            "waitForDebuggerOnStart": false,
                            "flatten": true,
                            "filter": [{ "type": "iframe" }],
                        }),
                    )
                    .await;
                if reply.is_err() {
                    // Not switched on after all: let the next snapshot retry.
                    ATTACHED.lock().configured.remove(&(browser_id, session.clone().unwrap_or_default()));
                }
            }
        }
        ATTACHED.lock().targets.get(&browser_id).cloned().unwrap_or_default()
    }

    /// Send `{id, method, params}` and wait for the reply with the same id.
    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value, CdpError> {
        // `Browser` is an FFI handle and not Send: read the id and let it go
        // before the first await.
        let browser_id = {
            let Some(browser) = self.state.get_browser(&self.label) else {
                return Err(format!("CDP {method}: browser {:?} is gone", self.label));
            };
            browser.identifier()
        };
        let (id, rx) = PENDING.lock().register(browser_id, method);
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = &self.session {
            message["sessionId"] = json!(session);
        }
        let message = message.to_string();
        let mut task = SendTask::new(self.state.clone(), self.label.clone(), browser_id, id, message);
        if post_task(ThreadId::UI, Some(&mut task)) == 0 {
            PENDING.lock().forget(id);
            return Err(format!("CDP {method}: could not post to the CEF UI thread"));
        }
        match tokio::time::timeout(CALL_TIMEOUT, rx).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => Err(format!("CDP {method}: reply channel dropped")),
            Err(_) => {
                PENDING.lock().forget(id);
                Err(format!("CDP timeout waiting for reply to {method}"))
            }
        }
    }

    /// Kept for call-site symmetry with the old socket client; there is
    /// nothing to tear down per session.
    pub async fn close(self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(id: i32, body: &str) -> Vec<u8> {
        format!(r#"{{"id":{id},{body}}}"#).into_bytes()
    }

    #[test]
    fn a_reply_completes_its_own_call_with_the_result() {
        let mut p = PendingCalls::default();
        let (id, mut rx) = p.register(7, "Runtime.evaluate");
        assert!(p.deliver(7, &reply(id, r#""result":{"result":{"value":42}}"#)));
        assert_eq!(rx.try_recv().unwrap().unwrap()["result"]["value"], 42);
        assert!(p.calls.is_empty());
    }

    #[test]
    fn an_error_reply_fails_the_call_naming_the_method() {
        let mut p = PendingCalls::default();
        let (id, mut rx) = p.register(7, "Page.navigate");
        assert!(p.deliver(7, &reply(id, r#""error":{"code":-32000,"message":"Cannot navigate"}"#)));
        let err = rx.try_recv().unwrap().unwrap_err();
        assert_eq!(err, "CDP Page.navigate error: Cannot navigate");
    }

    #[test]
    fn events_unknown_ids_and_garbage_are_left_alone() {
        let mut p = PendingCalls::default();
        let (id, mut rx) = p.register(7, "Runtime.evaluate");
        assert!(!p.deliver(7, br#"{"method":"Page.loadEventFired","params":{}}"#));
        assert!(!p.deliver(7, &reply(id + 1000, r#""result":{}"#)));
        assert!(!p.deliver(7, b"not json"));
        assert!(rx.try_recv().is_err(), "still waiting");
        assert_eq!(p.calls.len(), 1);
    }

    #[test]
    fn a_reply_seen_by_a_different_browser_never_completes_the_call() {
        let mut p = PendingCalls::default();
        let (id, mut rx) = p.register(7, "Runtime.evaluate");
        assert!(!p.deliver(8, &reply(id, r#""result":{}"#)), "browser 8's observer must not consume browser 7's call");
        assert!(rx.try_recv().is_err(), "still waiting");
        assert!(p.deliver(7, &reply(id, r#""result":{}"#)), "its own browser's reply still completes it");
    }

    #[test]
    fn a_closed_browser_fails_only_its_own_calls() {
        let mut p = PendingCalls::default();
        let (_, mut on_closed) = p.register(1, "Runtime.evaluate");
        let (_, mut on_other) = p.register(2, "Runtime.evaluate");
        p.fail_browser(1, "browser closed");
        assert_eq!(on_closed.try_recv().unwrap().unwrap_err(), "CDP Runtime.evaluate: browser closed");
        assert!(on_other.try_recv().is_err(), "the other browser's call is untouched");
        assert_eq!(p.calls.len(), 1);
    }

    #[test]
    fn a_late_reply_after_the_caller_gave_up_is_ignored() {
        let mut p = PendingCalls::default();
        let (id, _rx) = p.register(7, "Runtime.evaluate");
        p.forget(id);
        assert!(!p.deliver(7, &reply(id, r#""result":{}"#)));
    }

    fn attach(session: &str, target: &str, url: &str, parent: Option<&str>) -> Vec<u8> {
        let mut m = json!({
            "method": "Target.attachedToTarget",
            "params": { "sessionId": session, "targetInfo": { "targetId": target, "type": "iframe", "url": url }, "waitingForDebugger": false },
        });
        if let Some(p) = parent {
            m["sessionId"] = json!(p);
        }
        m.to_string().into_bytes()
    }

    #[test]
    fn an_attach_event_records_the_frame_and_the_session_that_reaches_it() {
        let mut a = Attached::default();
        a.note_event(7, &attach("S1", "F1", "http://other.test/form", None));
        assert_eq!(
            a.targets[&7],
            vec![AttachedTarget {
                session_id: "S1".into(),
                target_id: "F1".into(),
                kind: "iframe".into(),
                url: "http://other.test/form".into(),
                parent_session: None,
            }]
        );
    }

    #[test]
    fn frames_are_kept_per_browser_and_a_repeated_attach_replaces_not_duplicates() {
        let mut a = Attached::default();
        a.note_event(7, &attach("S1", "F1", "http://a.test/", None));
        a.note_event(8, &attach("S9", "F9", "http://b.test/", None));
        a.note_event(7, &attach("S1", "F1", "http://a.test/again", None));
        assert_eq!(a.targets[&7].len(), 1);
        assert_eq!(a.targets[&7][0].url, "http://a.test/again");
        assert_eq!(a.targets[&8].len(), 1);
    }

    #[test]
    fn detaching_a_frame_also_drops_the_frames_attached_through_it() {
        let mut a = Attached::default();
        a.note_event(7, &attach("S1", "F1", "http://a.test/", None));
        a.note_event(7, &attach("S2", "F2", "http://b.test/", Some("S1")));
        a.note_event(7, &attach("S3", "F3", "http://c.test/", None));
        a.configured.insert((7, "S1".into()));
        a.configured.insert((7, "S2".into()));
        a.note_event(7, br#"{"method":"Target.detachedFromTarget","params":{"sessionId":"S1"}}"#);
        let left: Vec<&str> = a.targets[&7].iter().map(|t| t.session_id.as_str()).collect();
        assert_eq!(left, vec!["S3"]);
        assert!(!a.configured.contains(&(7, "S1".into())) && !a.configured.contains(&(7, "S2".into())));
    }

    #[test]
    fn a_frame_that_navigates_updates_its_url() {
        let mut a = Attached::default();
        a.note_event(7, &attach("S1", "F1", "http://a.test/one", None));
        a.note_event(7, br#"{"method":"Target.targetInfoChanged","params":{"targetInfo":{"targetId":"F1","url":"http://a.test/two"}}}"#);
        assert_eq!(a.targets[&7][0].url, "http://a.test/two");
    }

    #[test]
    fn other_events_and_garbage_change_nothing() {
        let mut a = Attached::default();
        a.note_event(7, br#"{"method":"Page.loadEventFired","params":{}}"#);
        a.note_event(7, br#"{"id":3,"result":{}}"#);
        a.note_event(7, b"not json");
        a.note_event(7, br#"{"method":"Target.attachedToTarget","params":{}}"#);
        assert!(a.targets.is_empty());
    }

    #[test]
    fn auto_attach_is_switched_on_once_per_session_page_first_then_each_new_frame() {
        let mut a = Attached::default();
        assert_eq!(a.to_configure(7), vec![None]);
        assert!(a.to_configure(7).is_empty(), "already on for the page");
        a.note_event(7, &attach("S1", "F1", "http://a.test/", None));
        assert_eq!(a.to_configure(7), vec![Some("S1".to_string())]);
        assert!(a.to_configure(7).is_empty());
        assert_eq!(a.to_configure(8), vec![None], "another browser starts over");
        a.forget_browser(7);
        assert_eq!(a.to_configure(7), vec![None], "a closed browser is forgotten");
    }

    #[test]
    fn a_frame_attached_but_not_yet_explored_means_frames_may_be_missing() {
        let mut a = Attached::default();
        assert!(!a.has_unexplored(7));
        a.note_event(7, &attach("S1", "F1", "http://a.test/", None));
        assert!(a.has_unexplored(7), "S1's own frames were never looked for");
        a.to_configure(7);
        assert!(!a.has_unexplored(7));
    }

    #[test]
    fn ids_are_unique_positive_and_skip_outstanding_ones_on_wrap() {
        let mut p = PendingCalls::default();
        let (a, _ra) = p.register(1, "m");
        let (b, _rb) = p.register(1, "m");
        assert!(a > 0 && b > 0 && a != b);
        // Force a wrap onto an id that is still outstanding.
        p.last_id = i32::MAX - 1;
        let (c, _rc) = p.register(1, "m");
        assert_eq!((a, b, c), (1, 2, 3), "wraps to 1; 1 and 2 are outstanding, so 3");
    }
}
