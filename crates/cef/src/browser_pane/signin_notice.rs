// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Noticing a sign-in in a browser pane
//! (SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §5.1).
//!
//! `signin_notice.js` runs in the isolated world `side_button_cdp` adds to every
//! document, and calls the binding [`BINDING`] with the user name and password a
//! sign-in sends. Here the host:
//! - takes that `Runtime.bindingCalled` out of the DevTools event stream before
//!   anything else reads it ([`take`]), so no generic parse of the event holds
//!   the password, and nothing logs it;
//! - names the frame's origin from its own map of the worlds it made, never from
//!   the payload, and refuses an opaque one (`about:blank`, `srcdoc`, `data:`);
//! - hands the pair to srv (`credential.Offer`), which holds it for the person's
//!   answer to "Save to Vault?". A navigation of the main frame, or the password
//!   field leaving the page, tells srv the sign-in went through
//!   (`credential.Settled`).

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Deserialize;
use zeroize::Zeroizing;

use crate::state::AppState;

/// The function `signin_notice.js` calls; added only to the pane's isolated world.
pub(crate) const BINDING: &str = "agentmuxSignIn";
/// The listener; see `signin_notice.js`.
pub(crate) const SCRIPT: &str = include_str!("signin_notice.js");
/// How long srv holds a sign-in for the person's answer: a navigation after that
/// tells it nothing.
const HELD: Duration = Duration::from_secs(120);
const SRV_WAIT: Duration = Duration::from_secs(5);

/// The world a frame's listener runs in: which DevTools session made it, its
/// execution context there, and the frame's origin when the world was made.
#[derive(Debug, Clone, PartialEq, Eq)]
struct World {
    session: Option<String>,
    context: i64,
    frame: String,
    origin: String,
}

/// The worlds made in each browser's frames, newest last.
static WORLDS: LazyLock<Mutex<HashMap<i32, Vec<World>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
/// When each browser last handed srv a sign-in: its navigations then tell srv it went through.
static NOTICED: LazyLock<Mutex<HashMap<i32, Instant>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
/// The frame origins each browser's page told srv it shows a sign-in form for: a
/// navigation takes the pane's key button down, and they are told again when the
/// host registers with an srv that has forgotten them ([`resend_forms`]).
static FORMS: LazyLock<Mutex<HashMap<i32, std::collections::BTreeSet<String>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// The frame each browser's last sign-in was sent from, and when: a sign-in in an
/// iframe can navigate only that iframe.
static SIGNIN_FRAME: LazyLock<Mutex<HashMap<i32, (String, Instant)>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// What goes to srv: the credential service's method and its arguments.
type Call = (&'static str, serde_json::Value);
/// Each browser's way to srv, one call at a time in the order they were seen: a
/// sign-in that navigates at once must reach srv before the `Settled` of that
/// navigation, or its offer would never show.
static SENDERS: LazyLock<Mutex<HashMap<i32, tokio::sync::mpsc::UnboundedSender<Call>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// The world `context` was made in frame `frame` of `browser_id`, through
/// `session` (`None` for the page's own), for a document of `origin`.
pub(crate) fn note_world(browser_id: i32, session: Option<&str>, frame: &str, context: i64, origin: &str) {
    let mut worlds = WORLDS.lock();
    let list = worlds.entry(browser_id).or_default();
    list.retain(|w| !(w.session.as_deref() == session && w.frame == frame));
    list.push(World { session: session.map(str::to_string), context, frame: frame.to_string(), origin: origin.to_string() });
}

/// Keep only the worlds of frames still in the page.
pub(crate) fn keep_frames(browser_id: i32, live: &std::collections::HashSet<String>) {
    if let Some(list) = WORLDS.lock().get_mut(&browser_id) {
        list.retain(|w| live.contains(&w.frame));
    }
}

/// The pane's browser closed.
pub(crate) fn forget(browser_id: i32) {
    WORLDS.lock().remove(&browser_id);
    NOTICED.lock().remove(&browser_id);
    SIGNIN_FRAME.lock().remove(&browser_id);
    FORMS.lock().remove(&browser_id);
    // What is queued still goes; then the pane's sender ends.
    SENDERS.lock().remove(&browser_id);
}

/// The origin of the frame whose world is `context` in `session`, if it is one a
/// sign-in can be saved for: `http(s)` with a host.
fn origin_of(browser_id: i32, session: Option<&str>, context: i64) -> Option<String> {
    let worlds = WORLDS.lock();
    let w = worlds.get(&browser_id)?.iter().rev().find(|w| w.context == context && w.session.as_deref() == session)?;
    savable(&w.origin).then(|| w.origin.clone())
}

/// The frame whose world is `context` in `session`.
fn frame_of(browser_id: i32, session: Option<&str>, context: i64) -> Option<String> {
    let worlds = WORLDS.lock();
    worlds.get(&browser_id)?.iter().rev().find(|w| w.context == context && w.session.as_deref() == session).map(|w| w.frame.clone())
}

/// Whether `browser_id`'s last sign-in, lately, was sent from `frame`.
fn signed_in_from(browser_id: i32, frame: &str) -> bool {
    SIGNIN_FRAME.lock().get(&browser_id).is_some_and(|(f, at)| f == frame && at.elapsed() < HELD)
}

/// Frame `frame` of `browser_id` has a new document. If the last sign-in was
/// sent from it, srv learns it went through: a sign-in in an iframe that
/// navigates only the iframe has no main-frame commit to say so, and the old
/// document's watch on the password field went with it.
pub(crate) fn on_frame_document(browser_id: i32, frame: &str) {
    if !signed_in_from(browser_id, frame) {
        return;
    }
    let Some((state, block_id)) = super::side_button_cdp::pane_of(browser_id) else { return };
    send(state, browser_id, "Settled", serde_json::json!({ "block_id": block_id }));
}

/// `http://` or `https://` with a host: not `about:blank`, `srcdoc`, a sandboxed
/// frame's `null`, `data:` or `file:`.
pub(crate) fn savable(origin: &str) -> bool {
    let rest = origin.strip_prefix("https://").or_else(|| origin.strip_prefix("http://"));
    rest.is_some_and(|host| !host.is_empty() && !host.contains('/'))
}

/// A value from the page that is the person's secret: zeroed when dropped.
struct Secret(Zeroizing<String>);

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d).map(|s| Secret(Zeroizing::new(s)))
    }
}

#[derive(Deserialize)]
struct Called {
    method: Option<String>,
    #[serde(rename = "sessionId")]
    session: Option<String>,
    params: Option<CalledParams>,
}

#[derive(Deserialize)]
struct CalledParams {
    name: String,
    payload: Secret,
    #[serde(rename = "executionContextId")]
    context: i64,
}

/// What the listener reports.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Notice {
    /// A sign-in was sent.
    Signin { username: Secret, password: Secret },
    /// A step that asks only for the user name was sent.
    Identifier { username: Secret },
    /// The password field of the last sign-in left the page.
    Left,
    /// The document shows a sign-in form.
    Form,
}

/// Take our binding's call out of the DevTools event stream: true when `message`
/// was it, and nothing else may read it. Cheap for every other message: a
/// substring test.
pub(crate) fn take(browser_id: i32, message: &[u8]) -> bool {
    if !message.windows(BINDING.len()).any(|w| w == BINDING.as_bytes()) {
        return false;
    }
    let Ok(called) = serde_json::from_slice::<Called>(message) else { return false };
    let Some(params) = called.params.filter(|_| called.method.as_deref() == Some("Runtime.bindingCalled")) else { return false };
    if params.name != BINDING {
        return false;
    }
    // Ours: from here on it is handled here or dropped, never handed on.
    let Some(origin) = origin_of(browser_id, called.session.as_deref(), params.context) else {
        tracing::debug!(browser_id, "[signin-notice] a sign-in in a frame with no savable origin: dropped");
        return true;
    };
    let Ok(notice) = serde_json::from_str::<Notice>(&params.payload.0) else { return true };
    if matches!(notice, Notice::Signin { .. }) {
        if let Some(frame) = frame_of(browser_id, called.session.as_deref(), params.context) {
            SIGNIN_FRAME.lock().insert(browser_id, (frame, Instant::now()));
        }
    }
    let Some((state, block_id)) = super::side_button_cdp::pane_of(browser_id) else { return true };
    let (method, args) = match notice {
        Notice::Signin { username, password } => {
            NOTICED.lock().insert(browser_id, Instant::now());
            // The request body is the password's last copy in the host.
            ("Offer", serde_json::json!({ "block_id": block_id, "origin": origin, "username": username.0.as_str(), "password": password.0.as_str() }))
        }
        Notice::Identifier { username } => ("Offer", serde_json::json!({ "block_id": block_id, "origin": origin, "username": username.0.as_str() })),
        Notice::Left => ("Settled", serde_json::json!({ "block_id": block_id })),
        Notice::Form => {
            FORMS.lock().entry(browser_id).or_default().insert(origin.to_string());
            ("Form", serde_json::json!({ "block_id": block_id, "origin": origin }))
        }
    };
    send(state, browser_id, method, args);
    true
}

/// Every (browser, origin) whose page showed a sign-in form, as told to srv.
fn forms() -> Vec<(i32, String)> {
    FORMS.lock().iter().flat_map(|(b, origins)| origins.iter().map(move |o| (*b, o.clone()))).collect()
}

/// Tell srv again of every sign-in form the pages show. Called once srv has
/// acknowledged the host's registration, which clears what srv knew: a page
/// reports its form once, so after srv restarts its key button would otherwise
/// stay gone until the page navigates.
pub(crate) fn resend_forms() {
    for (browser_id, origin) in forms() {
        let Some((state, block_id)) = super::side_button_cdp::pane_of(browser_id) else { continue };
        send(state, browser_id, "Form", serde_json::json!({ "block_id": block_id, "origin": origin }));
    }
}

/// The pane's main frame committed a document: if it handed srv a sign-in
/// lately, srv learns the sign-in went through (or, once the offer shows, that
/// the person moved on); if its page had a sign-in form, the pane's key button
/// goes until the new page shows one.
pub(crate) fn on_main_commit(browser_id: i32) {
    let recent = NOTICED.lock().get(&browser_id).is_some_and(|at| at.elapsed() < HELD * 2);
    let had_form = FORMS.lock().remove(&browser_id).is_some();
    if !recent && !had_form {
        return;
    }
    let Some((state, block_id)) = super::side_button_cdp::pane_of(browser_id) else { return };
    send(state, browser_id, "Navigated", serde_json::json!({ "block_id": block_id }));
}

/// Chromium's own password manager and autofill, which the Vault replaces: off in
/// every profile's prefs, Incognito included (§5.1). Panes are Alloy-style, so
/// none of them has a way to show today; this keeps it so. On the UI thread.
const MANAGER_PREFS: &[&str] =
    &["credentials_enable_service", "profile.password_manager_enabled", "autofill.profile_enabled", "autofill.credit_card_enabled"];

pub(crate) fn password_manager_off(browser: &cef::Browser) {
    use cef::*;
    let Some(context) = browser.host().and_then(|host| host.request_context()) else { return };
    for pref in MANAGER_PREFS {
        let name = CefString::from(*pref);
        // A pref this Chromium doesn't have, or one already off, is left alone.
        match ImplPreferenceManager::preference(&context, Some(&name)) {
            Some(current) if current.bool() != 0 => {}
            _ => continue,
        }
        let Some(mut value) = value_create() else { continue };
        value.set_bool(0);
        // An owned empty string, not `CefString::default()`: see autocomplete.rs.
        let mut error = CefString::from("");
        if ImplPreferenceManager::set_preference(&context, Some(&name), Some(&mut value), Some(&mut error)) == 0 {
            tracing::warn!(pref, error = %error, "[signin-notice] couldn't turn off Chromium's password manager pref");
        }
    }
}

fn send(state: Arc<AppState>, browser_id: i32, method: &'static str, args: serde_json::Value) {
    let Some(rt) = super::auth::runtime() else { return };
    let mut senders = SENDERS.lock();
    let tx = senders.entry(browser_id).or_insert_with(|| {
        in_order(&rt, move |method, args| {
            let state = state.clone();
            async move {
                if let Err(e) = crate::credential_broker::call_host_only_service(&state, "credential", method, args, SRV_WAIT).await {
                    tracing::warn!(error = %e, "[signin-notice] couldn't hand srv a {method}");
                }
            }
        })
    });
    let _ = tx.send((method, args));
}

/// A sender whose calls run one at a time, each finished before the next starts.
fn in_order<F, Fut>(rt: &tokio::runtime::Handle, call: F) -> tokio::sync::mpsc::UnboundedSender<Call>
where
    F: Fn(&'static str, serde_json::Value) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Call>();
    rt.spawn(async move {
        while let Some((method, args)) = rx.recv().await {
            call(method, args).await;
        }
    });
    tx
}

#[cfg(test)]
#[path = "signin_notice_tests.rs"]
mod tests;
