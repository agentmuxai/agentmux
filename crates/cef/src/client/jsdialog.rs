// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A page's own dialogs (alert, confirm, prompt, "Leave site?") in a browser
//! pane an agent drives (docs/specs/SPEC_BROWSER_PANE_NATIVE_DIALOGS_2026_10_17.md
//! §4.1). Without a handler CEF draws its own modal, which CDP can't see or
//! answer, so every later action on the pane waits behind it. Here the host
//! takes the dialog, parks CEF's callback and asks srv, which answers at once
//! by policy or asks the person in the pane and answers later
//! (`/agentmux/browser/dialog_answer`). Panes no agent drives, and panes
//! handed to the person, keep CEF's own dialog.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use cef::*;
use parking_lot::Mutex;

/// srv asks the person for up to 10 minutes; past that, nobody will answer.
const PARKED_TTL: Duration = Duration::from_secs(11 * 60);
/// More than this many dialogs from one pane within `FLOOD_WINDOW` are
/// cancelled without asking (§6.5).
const FLOOD_MAX: usize = 5;
const FLOOD_WINDOW: Duration = Duration::from_secs(10);

struct Parked {
    target: String,
    callback: JsdialogCallback,
}

fn parked() -> &'static Mutex<HashMap<String, Parked>> {
    static P: OnceLock<Mutex<HashMap<String, Parked>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

/// The owned panes handed to the person, as srv last pushed them: their
/// dialogs are the person's (§4.3).
fn handoff() -> &'static Mutex<HashSet<String>> {
    static H: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    H.get_or_init(Default::default)
}

pub(crate) fn set_handoff(panes: Vec<String>) {
    *handoff().lock() = panes.into_iter().collect();
}

fn recent() -> &'static Mutex<HashMap<String, VecDeque<Instant>>> {
    static R: OnceLock<Mutex<HashMap<String, VecDeque<Instant>>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

/// Count a dialog from `target` at `now`; how many it has shown within the
/// window, this one included.
fn count_recent(times: &mut VecDeque<Instant>, now: Instant) -> usize {
    while times.front().is_some_and(|t| now.duration_since(*t) >= FLOOD_WINDOW) {
        times.pop_front();
    }
    times.push_back(now);
    times.len()
}

/// The pane and target a dialog in `browser` is for, when an agent drives
/// that pane and hasn't handed it to the person.
fn driven(state: &Arc<crate::state::AppState>, browser: &Browser) -> Option<(String, String)> {
    let (pane, target) = super::allowed_origins::pane_and_target(state, browser)?;
    if !state.owned_panes.lock().contains(&pane) || handoff().lock().contains(&pane) {
        return None;
    }
    Some((pane, target))
}

/// Answer a parked dialog: `accept` is OK (or Leave), else Cancel (or Stay).
/// Returns whether it was still parked. CEF's callback may be run from any
/// thread; it moves to the UI thread itself.
pub(crate) fn answer(id: &str, accept: bool) -> bool {
    let Some(p) = parked().lock().remove(id) else { return false };
    p.callback.cont(accept as ::std::os::raw::c_int, None);
    true
}

/// Cancel every dialog parked for `block_id` (a pane, or a popup window by
/// its label): it is closing.
pub(crate) fn cancel_for_block(block_id: &str) {
    let ids: Vec<String> =
        parked().lock().iter().filter(|(_, p)| p.target == block_id).map(|(id, _)| id.clone()).collect();
    for id in ids {
        answer(&id, false);
    }
    recent().lock().remove(block_id);
}

/// Tell srv about a dialog and act on its decision, on a thread of its own:
/// the caller is a CEF callback that can't wait. `alert_fallback` is the
/// answer when srv can't be reached: an alert only informs, so OK; anything
/// else cancels, the page's own "no".
fn report(state: &Arc<crate::state::AppState>, id: String, body: serde_json::Value, alert_fallback: bool) {
    let web_endpoint = state.backend_endpoints.lock().web_endpoint.clone();
    let auth_key = state.auth_key.lock().clone();
    let ipc_token = state.ipc_token.clone();
    let spawned = std::thread::Builder::new().name("browser-dialog".into()).spawn({
        let id = id.clone();
        move || match super::backend_browser_dialog(&web_endpoint, &auth_key, &ipc_token, &body) {
            Ok(d) if d == "accept" => {
                answer(&id, true);
            }
            Ok(d) if d == "parked" => {}
            Ok(_) => {
                answer(&id, false);
            }
            Err(e) => {
                tracing::warn!(id = %id, error = %e, "browser pane: couldn't ask srv about a page dialog; answering it here");
                answer(&id, alert_fallback);
            }
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "couldn't start the browser-dialog thread");
        answer(&id, alert_fallback);
    }
}

/// Tell srv something it only records (a flood, a dialog that ended). Fire
/// and forget, on its own thread.
fn notify(state: &Arc<crate::state::AppState>, body: serde_json::Value) {
    let web_endpoint = state.backend_endpoints.lock().web_endpoint.clone();
    let auth_key = state.auth_key.lock().clone();
    let ipc_token = state.ipc_token.clone();
    let _ = std::thread::Builder::new().name("browser-dialog-note".into()).spawn(move || {
        if let Err(e) = super::backend_browser_dialog(&web_endpoint, &auth_key, &ipc_token, &body) {
            tracing::debug!(error = %e, "browser pane: couldn't tell srv about a page dialog");
        }
    });
}

/// Park `callback` under a new id, with a timer that cancels it if nobody
/// answers, and ask srv.
fn park(
    state: &Arc<crate::state::AppState>,
    target: &str,
    kind: &str,
    message: &str,
    origin: &str,
    callback: &JsdialogCallback,
) {
    let id = uuid::Uuid::new_v4().to_string();
    parked().lock().insert(id.clone(), Parked { target: target.to_string(), callback: callback.clone() });
    if let Some(rt) = crate::browser_pane::auth::runtime() {
        let id = id.clone();
        rt.spawn(async move {
            tokio::time::sleep(PARKED_TTL).await;
            if answer(&id, false) {
                tracing::warn!(id = %id, "browser pane: a page dialog nobody answered was cancelled");
            }
        });
    }
    tracing::info!(target = %target, kind = %kind, origin = %origin, "browser pane: a page dialog in an agent-driven pane, asking srv");
    let body = serde_json::json!({ "id": id, "target": target, "kind": kind, "message": message, "origin": origin });
    report(state, id, body, kind == "alert");
}

wrap_jsdialog_handler! {
    pub(crate) struct AgentMuxJsdialogHandler {
        state: Arc<crate::state::AppState>,
    }

    impl JsdialogHandler {
        fn on_jsdialog(
            &self,
            browser: Option<&mut Browser>,
            origin_url: Option<&CefString>,
            dialog_type: JsdialogType,
            message_text: Option<&CefString>,
            _default_prompt_text: Option<&CefString>,
            callback: Option<&mut JsdialogCallback>,
            suppress_message: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            let (Some(browser), Some(callback)) = (browser, callback) else { return 0 };
            let Some((_pane, target)) = driven(&self.state, browser) else { return 0 };
            let kind = if dialog_type == JsdialogType::ALERT {
                "alert"
            } else if dialog_type == JsdialogType::CONFIRM {
                "confirm"
            } else if dialog_type == JsdialogType::PROMPT {
                "prompt"
            } else {
                return 0;
            };
            let message = message_text.map(|s| s.to_string()).unwrap_or_default();
            let origin = origin_url.map(|s| s.to_string()).unwrap_or_default();
            let shown = count_recent(recent().lock().entry(target.clone()).or_default(), Instant::now());
            if shown > FLOOD_MAX {
                // Suppressed, not answered: CEF's way to stop a page spamming
                // dialogs. srv hears of the first one past the limit.
                if let Some(s) = suppress_message {
                    *s = 1;
                }
                if shown == FLOOD_MAX + 1 {
                    notify(&self.state, serde_json::json!({ "target": target, "kind": kind, "message": message, "suppressed": true }));
                }
                return 0;
            }
            park(&self.state, &target, kind, &message, &origin, callback);
            1
        }

        fn on_before_unload_dialog(
            &self,
            browser: Option<&mut Browser>,
            message_text: Option<&CefString>,
            _is_reload: ::std::os::raw::c_int,
            callback: Option<&mut JsdialogCallback>,
        ) -> ::std::os::raw::c_int {
            let (Some(browser), Some(callback)) = (browser, callback) else { return 0 };
            let Some((_pane, target)) = driven(&self.state, browser) else { return 0 };
            let message = message_text.map(|s| s.to_string()).unwrap_or_default();
            let origin = crate::browser_pane::callbacks::main_frame_url(browser);
            park(&self.state, &target, "beforeunload", &message, &origin, callback);
            1
        }

        fn on_reset_dialog_state(&self, browser: Option<&mut Browser>) {
            // The page navigated or went away: its dialogs are over, and CEF
            // drops their callbacks. Take them down here and in the pane.
            let Some(browser) = browser else { return };
            let Some((_pane, target)) = super::allowed_origins::pane_and_target(&self.state, browser) else { return };
            let ended: Vec<String> = {
                let mut p = parked().lock();
                let ids: Vec<String> = p.iter().filter(|(_, d)| d.target == target).map(|(id, _)| id.clone()).collect();
                ids.into_iter().filter(|id| p.remove(id).is_some()).collect()
            };
            for id in ended {
                notify(&self.state, serde_json::json!({ "id": id, "target": target, "ended": true }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flood_is_counted_within_its_window_only() {
        let mut times = VecDeque::new();
        let t0 = Instant::now();
        for i in 1..=FLOOD_MAX {
            assert_eq!(count_recent(&mut times, t0), i);
        }
        assert_eq!(count_recent(&mut times, t0), FLOOD_MAX + 1, "the next one is over");
        // After the window, the count starts again.
        assert_eq!(count_recent(&mut times, t0 + FLOOD_WINDOW), 1);
    }

    #[test]
    fn answering_an_unknown_dialog_does_nothing() {
        assert!(!answer("jsdialog-test-unknown", true));
        cancel_for_block("jsdialog-test-pane");
    }
}
