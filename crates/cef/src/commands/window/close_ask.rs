// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A close that comes from the OS asks the window's own page first.
//!
//! Alt+F4, the taskbar's Close and a window manager's close reach the host without passing the
//! page, so they skipped what the window's own close button does first: the warning that the
//! close stops agents that are working (`frontend/app/window/window-close.ts`). Such a close is
//! now held back and the page is told (`window-close-requested`). The page closes the window the
//! way its button does, warning first when it would, and that close comes back as the
//! `close_window` command.
//!
//! A page that does not say it got the request within [`ANSWER_WAIT_MS`] is taken for hung or not
//! loaded, and the window closes as it did before.
//!
//! Every close the host itself decided on goes through `ui_tasks::post_close_window`, which marks
//! the window ([`approve`]); a marked window is never asked. So the page's own close, a quit, a
//! merge and the timeout's close all pass, and only a close nobody decided on is asked about.
//!
//! macOS is left as it was: its windows have no OS close of their own (the buttons are the page's),
//! and what reaches the host there is the app quitting, which is not a window's question.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use cef::*;
use parking_lot::Mutex;

use crate::state::AppState;

/// The event the window's page gets, with `{ seq }`.
pub(crate) const EVENT: &str = "window-close-requested";

/// How long the page has to say it got the request (`close_request_seen`).
const ANSWER_WAIT_MS: i64 = 1500;

/// How long a close the host decided on stays decided. The close's own steps run within a few
/// seconds. A host label can be used again (a pool window promoted again, `main` reopened from
/// the tray), and that window's close is a new question: [`page_registered`] ends the decision
/// when the new window's page comes up, and this is the limit for one that never does.
const APPROVAL: Duration = Duration::from_secs(30);

/// What to do with a close the OS asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OsClose {
    /// Hold it back: the page was asked.
    Ask,
    /// Let it go on as before.
    Proceed,
}

/// Whether a close the OS asked for is put to the page. Only a user's window can be asked (`main`
/// and `window-*`), only once its page is up (`page_ready`: it registered its srv window), never
/// during a quit, and never when the host already decided the close.
pub(crate) fn decide(label: &str, page_ready: bool, quitting: bool, approved: bool) -> OsClose {
    let users_window = label == "main" || label.starts_with("window-");
    if users_window && page_ready && !quitting && !approved {
        OsClose::Ask
    } else {
        OsClose::Proceed
    }
}

/// The closes decided on, and the requests the page has not answered yet.
#[derive(Default)]
struct Asks {
    approved: HashMap<String, Instant>,
    waiting: HashMap<String, u64>,
    last_seq: u64,
}

impl Asks {
    fn approve(&mut self, label: &str, now: Instant) {
        self.approved.retain(|_, at| now.duration_since(*at) < APPROVAL);
        self.approved.insert(label.to_string(), now);
        self.waiting.remove(label);
    }

    fn is_approved(&self, label: &str, now: Instant) -> bool {
        self.approved.get(label).is_some_and(|at| now.duration_since(*at) < APPROVAL)
    }

    /// A new request for `label`; an earlier one still waiting is replaced.
    fn ask(&mut self, label: &str) -> u64 {
        self.last_seq += 1;
        self.waiting.insert(label.to_string(), self.last_seq);
        self.last_seq
    }

    /// The page got request `seq`: true when it was the one waiting.
    fn seen(&mut self, label: &str, seq: u64) -> bool {
        if self.waiting.get(label) == Some(&seq) {
            self.waiting.remove(label);
            return true;
        }
        false
    }

    fn forget(&mut self, label: &str) {
        self.approved.remove(label);
        self.waiting.remove(label);
    }
}

static ASKS: LazyLock<Mutex<Asks>> = LazyLock::new(|| Mutex::new(Asks::default()));

/// The host decided to close `label` (`ui_tasks::post_close_window`): its close is not asked about.
pub(crate) fn approve(label: &str) {
    ASKS.lock().approve(label, Instant::now());
}

/// The window is gone: its label starts again.
pub(crate) fn forget(label: &str) {
    ASKS.lock().forget(label);
}

/// The page of `label` registered srv window `window_id` (`register_backend_window`). A window id
/// the label did not have is a new window under it: a pool window closed and promoted again keeps
/// its label and its native window, so nothing else says the one before is gone. What was decided
/// for that one says nothing about this one. A page registering the id it has (it does so twice
/// while it loads, and again after a reload) changes nothing.
pub(crate) fn page_registered(state: &AppState, label: &str, window_id: &str) {
    tracing::info!(label = %label, window_id = %window_id, "[window] registered backend window ID");
    if is_new_window(state.backend_window_id(label).as_deref(), window_id) {
        forget(label);
    }
}

fn is_new_window(had: Option<&str>, registers: &str) -> bool {
    had != Some(registers)
}

/// For a close of `label` that came from the OS, on the UI thread. On `Ask` the caller holds the
/// close back: the page has been told, and closes the window itself or is given up on.
pub(crate) fn os_close(state: &Arc<AppState>, label: &str) -> OsClose {
    if cfg!(target_os = "macos") {
        return OsClose::Proceed;
    }
    let approved = ASKS.lock().is_approved(label, Instant::now());
    let page_ready = state.backend_window_id(label).is_some();
    if decide(label, page_ready, state.is_quitting(), approved) == OsClose::Proceed {
        return OsClose::Proceed;
    }
    let seq = ASKS.lock().ask(label);
    if !crate::events::emit_event_to_window(state, label, EVENT, &serde_json::json!({ "seq": seq })) {
        ASKS.lock().seen(label, seq);
        return OsClose::Proceed;
    }
    tracing::info!("[close-ask] an OS close of {label} is held back: its page is asked (#{seq})");
    let mut task = UnansweredTask::new(state.clone(), label.to_string(), seq);
    post_delayed_task(ThreadId::UI, Some(&mut task), ANSWER_WAIT_MS);
    OsClose::Ask
}

/// `close_request_seen` from the page: `{ label, seq }`. Its close, if it makes one, comes as `close_window`.
pub(crate) fn seen(args: &serde_json::Value) -> serde_json::Value {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
    let seq = args.get("seq").and_then(|v| v.as_u64()).unwrap_or(0);
    serde_json::json!(ASKS.lock().seen(label, seq))
}

wrap_task! {
    struct UnansweredTask {
        state: Arc<AppState>,
        label: String,
        seq: u64,
    }

    impl Task {
        fn execute(&self) {
            if !ASKS.lock().seen(&self.label, self.seq) {
                return; // the page answered, or the window closed another way
            }
            tracing::warn!(
                "[close-ask] the page of {} did not answer a close request in {ANSWER_WAIT_MS} ms: closing it without asking",
                self.label
            );
            let _ = super::close_window(&self.state, &serde_json::json!({ "label": self.label }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_users_window_with_its_page_up_is_asked_and_never_in_a_quit_or_a_decided_close() {
        assert_eq!(decide("main", true, false, false), OsClose::Ask);
        assert_eq!(decide("window-9f8e7d", true, false, false), OsClose::Ask);
        // The page has not registered (still loading, the pool, the "install broken" page).
        assert_eq!(decide("window-pool-abc123", false, false, false), OsClose::Proceed);
        // A quit closes every window: nothing to ask.
        assert_eq!(decide("main", true, true, false), OsClose::Proceed);
        // The page, or the host, already decided.
        assert_eq!(decide("main", true, false, true), OsClose::Proceed);
        // Not a user's window.
        assert_eq!(decide("browser-pane-1", true, false, false), OsClose::Proceed);
        assert_eq!(decide("floating-pane-1", true, false, false), OsClose::Proceed);
    }

    #[test]
    fn a_decided_close_stays_decided_for_a_while_and_then_the_label_is_asked_about_again() {
        let mut asks = Asks::default();
        let t0 = Instant::now();
        assert!(!asks.is_approved("main", t0));
        asks.approve("main", t0);
        assert!(asks.is_approved("main", t0 + Duration::from_secs(5)));
        assert!(!asks.is_approved("main", t0 + APPROVAL));
        assert!(!asks.is_approved("window-a", t0));
        asks.approve("window-a", t0);
        asks.forget("window-a");
        assert!(!asks.is_approved("window-a", t0));
    }

    #[test]
    fn a_label_used_again_for_another_window_starts_undecided() {
        // A pool window closed and promoted again within the limit: same label, a new srv window.
        assert!(is_new_window(Some("w1"), "w2"));
        assert!(is_new_window(None, "w1"));
        // The page's second registration while it loads, or after a reload onto its own row.
        assert!(!is_new_window(Some("w1"), "w1"));
        let mut asks = Asks::default();
        let t0 = Instant::now();
        asks.approve("window-pool-1", t0);
        asks.forget("window-pool-1");
        assert!(!asks.is_approved("window-pool-1", t0 + Duration::from_secs(1)));
    }

    #[test]
    fn a_request_is_answered_once_and_only_the_newest_one_counts() {
        let mut asks = Asks::default();
        let first = asks.ask("main");
        let second = asks.ask("main");
        assert_ne!(first, second);
        assert!(!asks.seen("main", first), "an older request's timeout must not close the window");
        assert!(asks.seen("main", second));
        assert!(!asks.seen("main", second), "answered: its timeout finds nothing");
        // A decided close ends the wait too.
        let third = asks.ask("main");
        asks.approve("main", Instant::now());
        assert!(!asks.seen("main", third));
    }
}
