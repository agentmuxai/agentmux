// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The mouse side buttons over a browser pane on Linux and macOS.
//!
//! A pane's page gets the buttons as DOM events (`button` 3 / 4) and Chromium
//! also navigates the page on them unless they are cancelled, so a plain press
//! went to the page's history instead of the window's selection history
//! (SPEC_MOUSE_SIDE_BUTTONS_PANE_AND_TAB_NAV_2026_10_19.md §4.3, §5). Windows
//! takes the press in the pane's window procedure (`hwnd.rs`); there is no
//! such hook here, so each document of a pane gets a small listener instead
//! (`side_button.js`), added through the in-process CDP client:
//!
//! - once per DevTools session: `Runtime.enable` and `Runtime.addBinding`
//!   scoped to the isolated world `WORLD`, so only that world can call it and
//!   the page can't;
//! - for every document, when the host sees it commit (`on_frame_commit`):
//!   `Page.getFrameTree`, then in each frame not yet done, a fresh isolated
//!   world named `WORLD` (`Page.createIsolatedWorld`) and the listener
//!   evaluated in it. The same way `browser_api/act.rs` works in a page.
//!
//! The listener claims the buttons (Chromium doesn't navigate the page, and a
//! page listener registered after it doesn't see them) and reports the release
//! with `Runtime.bindingCalled`, which the CDP observer hands to [`note_event`].
//! From there it is the Windows route: a plain press sends the window a
//! `side-button` event (source `pane`, with the block id); Ctrl calls the
//! pane's own `go_back` / `go_forward`.
//!
//! Not `Page.addScriptToEvaluateOnNewDocument`: its scripts only run while the
//! Page domain is enabled (`Page.disable` removes them), and an enabled Page
//! domain takes over the page's JavaScript dialogs from the host's own handler
//! (`client::jsdialog`). Enabling `Runtime` has no such effect.
//!
//! An iframe from another site runs in its own process and is its own CDP
//! target, which the page's session can't create a world in. `ensure` therefore
//! also turns on auto-attach (`CdpSession::attach_frames`, the way the forms
//! snapshot reaches such frames), and in each attached frame's session adds the
//! world and the listener in each of its frames. A new frame attaching
//! (`Target.attachedToTarget`, seen by [`note_event`]) runs `ensure` again,
//! because the host's commit callback can come before the attach.
//!
//! An iframe's session does not get `Runtime.enable` (it would stream every
//! console call and context of the frame to the host): the binding is added
//! after the world exists (`executionContextName` reaches a context that is
//! already there), and `Runtime.bindingCalled` still arrives. Added before the
//! world exists, with `Runtime` not enabled, it does not reach it.
//!
//! Limits, as recorded in the spec's P3 (each one tried on Linux, X11):
//! - a page with more than [`MAX_FRAMES`] frames gets the listener in its own
//!   process's frames only: a press over a cross-site iframe acts as the page's
//!   own (Chromium navigates back, no event). The cap is on the injection, not
//!   on auto-attach, which a page that starts with one frame has on before its
//!   iframes exist (measured on 15 cross-site iframes that log to the console:
//!   about 1% of a core more than without the route, memory the same);
//! - the listener is added once a document has committed, a few milliseconds
//!   after it starts, so a page listener registered in that window runs first:
//!   the press is still claimed (Chromium doesn't navigate and the window gets
//!   the event) but the page sees it too;
//! - when the DevTools agent detaches, its binding goes with it; the route is
//!   set up again (the listener already in the page stays, and only claims
//!   the buttons until the binding is back).
//!
//! The same world carries the sign-in listener (`signin_notice.js`, Vault spec
//! SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §5.1) with its
//! own binding, on every platform: on Windows the world holds only that one,
//! since the window procedure takes the side buttons there. Windows panes also
//! never get `Runtime.enable`, which anti-bot pages probe for: as in an iframe's
//! session, the binding is added to each world once it exists.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use cef::*;
use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::browser_api::cdp::CdpSession;
use crate::state::AppState;

/// The isolated world the listener runs in and the binding lives in.
const WORLD: &str = "agentmux-side-buttons";
/// The function the listener calls; added only to `WORLD`.
const BINDING: &str = "agentmuxSideButton";
/// A page with more frames than this (the main one and its iframes, counted in
/// the page's own frame tree) gets no listener in the frames in other processes
/// (a world, a binding and a script each, for every document of every frame).
/// Its other frames still get it.
const MAX_FRAMES: usize = 9;
/// The listener; see `side_button.js`.
const SCRIPT: &str = include_str!("side_button.js");

/// The bindings the world gets: the sign-in listener's everywhere, the side
/// buttons' where the window procedure doesn't take them.
fn bindings() -> &'static [&'static str] {
    if cfg!(target_os = "windows") {
        &[super::signin_notice::BINDING]
    } else {
        &[BINDING, super::signin_notice::BINDING]
    }
}

/// Whether the page's own session goes without `Runtime.enable` and binds each
/// world instead, as an iframe's session does: on Windows, which needs no side
/// button listener, so a pane shows anti-bot pages no sign of automation.
const BIND_EACH_WORLD: bool = cfg!(target_os = "windows");

/// The listeners evaluated in each world, as one expression.
static SCRIPTS: LazyLock<String> = LazyLock::new(|| {
    if cfg!(target_os = "windows") {
        super::signin_notice::SCRIPT.to_string()
    } else {
        format!("{SCRIPT};\n{}", super::signin_notice::SCRIPT)
    }
});

struct Pane {
    state: Arc<AppState>,
    label: String,
    block_id: String,
    /// One setup or injection at a time per pane, so a document's injection
    /// waits for the binding.
    inner: tokio::sync::Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// `Runtime.enable` and the binding are in place on the current DevTools session.
    ready: bool,
    /// `(frame id, loader id)` of the documents the listener was already added to.
    injected: HashSet<(String, String)>,
}

type Key = (String, String);

/// The panes with the route, by browser id.
static PANES: LazyLock<Mutex<HashMap<i32, Arc<Pane>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Start the route for `browser`'s pane, once (a pane's `on_after_created` can
/// fire again after a renderer swap).
pub fn install(state: &Arc<AppState>, browser: &Browser) {
    let Some((label, block_id)) = super::callbacks::resolve_pane_label_and_block(state, browser) else {
        tracing::warn!("[side-button] a browser pane without a label; its side buttons keep the page's default");
        return;
    };
    let pane = Arc::new(Pane { state: state.clone(), label, block_id, inner: Default::default() });
    if register_new(&mut PANES.lock(), browser.identifier(), pane.clone()) {
        spawn_ensure(pane);
    }
}

/// Put `value` under `id` unless something is already there, and say whether it
/// was put. A repeat `install` must keep the pane already registered, with what
/// it knows is done, not replace it.
fn register_new<T>(map: &mut HashMap<i32, Arc<T>>, id: i32, value: Arc<T>) -> bool {
    match map.entry(id) {
        Entry::Occupied(_) => false,
        Entry::Vacant(slot) => {
            slot.insert(value);
            true
        }
    }
}

/// A document of the pane committed (the main frame or a frame in it): add the
/// listener to it.
pub fn on_frame_commit(browser: &Browser) {
    let pane = PANES.lock().get(&browser.identifier()).cloned();
    if let Some(pane) = pane {
        spawn_ensure(pane);
    }
}

/// The pane's browser closed.
pub fn forget(browser_id: i32) {
    PANES.lock().remove(&browser_id);
    super::signin_notice::forget(browser_id);
}

/// The host's state and the block id of `browser_id`'s pane, if it has the route.
pub(super) fn pane_of(browser_id: i32) -> Option<(Arc<AppState>, String)> {
    PANES.lock().get(&browser_id).map(|p| (p.state.clone(), p.block_id.clone()))
}

/// The DevTools agent detached: the binding is gone, so add it again (the next
/// CDP call attaches a new session).
pub fn reinstall(browser_id: i32) {
    let pane = PANES.lock().get(&browser_id).cloned();
    if let Some(pane) = pane {
        spawn_ensure_after_reset(pane);
    }
}

fn spawn_ensure(pane: Arc<Pane>) {
    let Some(rt) = super::auth::runtime() else { return };
    rt.spawn(async move { ensure(&pane).await });
}

fn spawn_ensure_after_reset(pane: Arc<Pane>) {
    let Some(rt) = super::auth::runtime() else { return };
    rt.spawn(async move {
        pane.inner.lock().await.after_detach();
        ensure(&pane).await
    });
}

impl Inner {
    /// The DevTools agent detached, taking every binding with it. Where each
    /// world is bound when it is made (Windows), the page's documents get a new
    /// world, bound, since the old ones can't be reached to bind again; elsewhere
    /// `bind` adds the binding back to every world.
    fn after_detach(&mut self) {
        self.ready = false;
        if BIND_EACH_WORLD {
            self.injected.clear();
        }
    }
}

/// Make sure the session is set up, then add the listener to every frame of
/// the page that doesn't have it for its current document, in the page's own
/// process and in the processes of its cross-site iframes. Nothing is sent to a
/// frozen page (any call thaws it, `cdp::CdpSession::call`): it runs again when
/// the page thaws ([`on_thaw`]).
async fn ensure(pane: &Pane) {
    let mut guard = pane.inner.lock().await;
    let inner = &mut *guard;
    let mut cdp = CdpSession::attach(&pane.state, &pane.label);
    match cdp.browser_id() {
        Some(id) if !crate::browser_api::cdp::is_frozen(id) => {}
        Some(_) => {
            tracing::debug!(label = %pane.label, "[side-button] the page is frozen: the listener waits until it runs again");
            return;
        }
        None => return,
    }
    if !inner.ready {
        if !bind(&pane.label, &mut cdp).await {
            return;
        }
        inner.ready = true;
    }
    let Ok(tree) = cdp.call("Page.getFrameTree", json!({})).await else { return };
    // Before the page's own frames: the ones this finds are reached through
    // their own sessions, not the page's.
    let targets = if frames_of(&tree).len() > MAX_FRAMES {
        tracing::debug!(label = %pane.label, "[side-button] many frames: no listener in the ones in other processes");
        Vec::new()
    } else {
        cdp.attach_frames().await
    };
    let remote: HashSet<String> = targets.iter().map(|t| t.target_id.clone()).collect();
    let mut live = HashSet::new();
    let browser_id = cdp.browser_id().unwrap_or_default();
    let page = Frames { browser_id, session: None, skip: &remote, bind_world: BIND_EACH_WORLD };
    inject_frames(&pane.label, &mut cdp, &tree, &mut inner.injected, &mut live, page).await;
    for target in &targets {
        let mut on = cdp.in_session(Some(&target.session_id));
        let Ok(tree) = on.call("Page.getFrameTree", json!({})).await else { continue };
        let frame = Frames { browser_id, session: Some(&target.session_id), skip: &HashSet::new(), bind_world: true };
        inject_frames(&pane.label, &mut on, &tree, &mut inner.injected, &mut live, frame).await;
    }
    inner.injected.retain(|k| live.contains(k));
    super::signin_notice::keep_frames(browser_id, &live.iter().map(|k| k.0.clone()).collect());
}

/// `Runtime.enable` and the binding on the page's session. False when it
/// couldn't be set up. Nothing on Windows: there each world gets the binding.
async fn bind(label: &str, cdp: &mut CdpSession) -> bool {
    if BIND_EACH_WORLD {
        return true;
    }
    let setup = std::iter::once(("Runtime.enable", json!({})))
        .chain(bindings().iter().map(|name| ("Runtime.addBinding", json!({ "name": name, "executionContextName": WORLD }))));
    for (method, params) in setup {
        if let Err(e) = cdp.call(method, params).await {
            tracing::warn!(label = %label, error = %e, "[side-button] couldn't set up the pane ({method})");
            return false;
        }
    }
    true
}

/// Which frames of one session [`inject_frames`] covers.
struct Frames<'a> {
    browser_id: i32,
    /// The frame's own session, for a cross-site iframe; `None` for the page's.
    session: Option<&'a str>,
    /// Frames reached through another session.
    skip: &'a HashSet<String>,
    /// In a frame's own session `Runtime` isn't enabled, so the bindings are
    /// added to each world once it exists.
    bind_world: bool,
}

/// Add the listeners to each frame in `tree` (`cdp`'s session's) that isn't in
/// `injected` for its current document, except the ids in `on.skip`. Every
/// frame seen goes into `live`. Each world made is noted with its document's
/// origin, which is what a sign-in in it is saved for.
async fn inject_frames(label: &str, cdp: &mut CdpSession, tree: &Value, injected: &mut HashSet<Key>, live: &mut HashSet<Key>, on: Frames<'_>) {
    let Frames { browser_id, session, skip, bind_world } = on;
    for key in frames_of(tree) {
        if skip.contains(&key.0) {
            continue;
        }
        live.insert(key.clone());
        // Marked first: a frame this session can't reach would fail again each time.
        if !injected.insert(key.clone()) {
            continue;
        }
        let world = cdp.call("Page.createIsolatedWorld", json!({ "frameId": key.0, "worldName": WORLD })).await;
        let Some(context) = world.ok().and_then(|w| w.get("executionContextId").and_then(Value::as_i64)) else { continue };
        if bind_world {
            for name in bindings() {
                let binding = json!({ "name": name, "executionContextName": WORLD });
                if let Err(e) = cdp.call("Runtime.addBinding", binding).await {
                    tracing::warn!(label = %label, error = %e, "[side-button] couldn't add a binding to a frame");
                }
            }
        }
        // The origin of the document the world is in, from the world itself: the
        // frame tree read before could be a document the frame has since left.
        let origin = cdp.call("Runtime.evaluate", json!({ "contextId": context, "expression": "location.origin", "returnByValue": true })).await;
        let origin = origin.ok().and_then(|v| v.pointer("/result/value").and_then(Value::as_str).map(str::to_string)).unwrap_or_default();
        super::signin_notice::note_world(browser_id, session, &key.0, context, &origin);
        let params = json!({ "contextId": context, "expression": SCRIPTS.as_str(), "returnByValue": true });
        match cdp.call("Runtime.evaluate", params).await {
            Ok(_) => tracing::debug!(label = %label, frame = %key.0, "[side-button] listener added"),
            Err(e) => tracing::warn!(label = %label, error = %e, "[side-button] couldn't add the listener to a frame"),
        }
    }
}

/// `(frame id, loader id)` of every frame in a `Page.getFrameTree` result: the
/// loader id changes with each document, so it tells a frame's new document
/// from one already done.
fn frames_of(tree: &Value) -> Vec<(String, String)> {
    fn walk(node: &Value, out: &mut Vec<(String, String)>) {
        let frame = node.get("frame");
        let text = |k: &str| frame.and_then(|f| f.get(k)).and_then(Value::as_str).map(str::to_string);
        if let Some(id) = text("id") {
            out.push((id, text("loaderId").unwrap_or_default()));
        }
        for child in node.get("childFrames").and_then(Value::as_array).into_iter().flatten() {
            walk(child, out);
        }
    }
    let mut out = Vec::new();
    if let Some(root) = tree.get("frameTree") {
        walk(root, &mut out);
    }
    out
}

/// A raw DevTools message the observer for `browser_id` saw: act on our
/// binding's call, ignore the rest, except a frame attaching, which means a
/// cross-site iframe to add the listener to. Cheap for the common case (a
/// substring test) because every CDP event of every pane passes through here.
pub fn note_event(browser_id: i32, message: &[u8]) {
    const BINDING_CALLED: &[u8] = b"Runtime.bindingCalled";
    const ATTACHED: &[u8] = b"Target.attachedToTarget";
    // One pass: both marks start with a different capital.
    let mark = (0..message.len()).find_map(|i| match message[i] {
        b'R' if message[i..].starts_with(BINDING_CALLED) => Some(BINDING_CALLED),
        b'T' if message[i..].starts_with(ATTACHED) => Some(ATTACHED),
        _ => None,
    });
    match mark {
        Some(ATTACHED) => {
            if is_event(message, "Target.attachedToTarget") {
                on_frame_attached(browser_id);
            }
            return;
        }
        Some(_) => {}
        None => return,
    }
    let Ok(msg) = serde_json::from_slice::<Value>(message) else { return };
    if msg.get("method").and_then(Value::as_str) != Some("Runtime.bindingCalled") {
        return;
    }
    let params = msg.get("params");
    if params.and_then(|p| p.get("name")).and_then(Value::as_str) != Some(BINDING) {
        return;
    }
    let Some(payload) = params.and_then(|p| p.get("payload")).and_then(Value::as_str) else { return };
    let Some(press) = parse_press(payload) else { return };
    let Some(pane) = PANES.lock().get(&browser_id).cloned() else { return };
    act(&pane, press);
}

/// Whether `message` is the DevTools event `method`.
fn is_event(message: &[u8], method: &str) -> bool {
    serde_json::from_slice::<Value>(message).ok().and_then(|m| m.get("method").and_then(Value::as_str).map(|m| m == method)) == Some(true)
}

/// A frame in its own process attached to the pane's page: it has no listener
/// yet.
fn on_frame_attached(browser_id: i32) {
    on_thaw(browser_id);
}

/// The pane's page runs again (or something changed that `ensure` skipped while
/// it was frozen): add what is missing.
pub fn on_thaw(browser_id: i32) {
    let pane = PANES.lock().get(&browser_id).cloned();
    if let Some(pane) = pane {
        spawn_ensure(pane);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dir {
    Back,
    Forward,
}

impl Dir {
    fn as_str(self) -> &'static str {
        match self {
            Dir::Back => "back",
            Dir::Forward => "forward",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Press {
    dir: Dir,
    ctrl: bool,
    blocked: bool,
}

fn parse_press(payload: &str) -> Option<Press> {
    let v: Value = serde_json::from_str(payload).ok()?;
    let dir = match v.get("button")?.as_str()? {
        "back" => Dir::Back,
        "forward" => Dir::Forward,
        _ => return None,
    };
    let flag = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
    Some(Press { dir, ctrl: flag("ctrl"), blocked: flag("blocked") })
}

/// What a press means, as on Windows (`hwnd.rs`): Ctrl is the page's own
/// history; a plain press goes to the window's frontend, which moves the
/// selection history (or, with the setting off, the page's history).
fn act(pane: &Pane, press: Press) {
    let state = &pane.state;
    if press.ctrl {
        match press.dir {
            Dir::Back => state.browser_panes.go_back(&pane.block_id, state),
            Dir::Forward => state.browser_panes.go_forward(&pane.block_id, state),
        }
    } else if let Some(window) = state.browser_pane_window_label(&pane.block_id) {
        crate::events::emit_side_button(state, &window, press.dir.as_str(), false, press.blocked, "pane", Some(&pane.block_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn after_a_detach_a_pane_that_binds_each_world_makes_new_ones() {
        let mut inner = Inner { ready: true, injected: [("frame".to_string(), "loader".to_string())].into() };
        inner.after_detach();
        assert!(!inner.ready);
        assert_eq!(inner.injected.is_empty(), BIND_EACH_WORLD, "the old worlds lost their binding");
    }

    #[test]
    fn a_press_is_read_from_the_listeners_payload() {
        assert_eq!(
            parse_press(r#"{"button":"back","ctrl":false,"blocked":false}"#),
            Some(Press { dir: Dir::Back, ctrl: false, blocked: false })
        );
        assert_eq!(
            parse_press(r#"{"button":"forward","ctrl":true,"blocked":true}"#),
            Some(Press { dir: Dir::Forward, ctrl: true, blocked: true })
        );
        assert_eq!(parse_press(r#"{"button":"forward"}"#), Some(Press { dir: Dir::Forward, ctrl: false, blocked: false }));
    }

    #[test]
    fn anything_else_is_not_a_press() {
        for bad in ["", "back", "{}", r#"{"button":"left"}"#, r#"{"button":3}"#] {
            assert_eq!(parse_press(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn only_our_bindings_call_is_taken_from_the_event_stream() {
        // Not our binding, other events and garbage return before touching any state.
        note_event(1, br#"{"method":"Runtime.bindingCalled","params":{"name":"other","payload":"{}"}}"#);
        note_event(1, br#"{"method":"Page.loadEventFired","params":{}}"#);
        note_event(1, b"Runtime.bindingCalled but not json");
        // Ours, but no pane is registered for that browser: nothing to act on.
        note_event(
            987_654,
            br#"{"method":"Runtime.bindingCalled","params":{"name":"agentmuxSideButton","payload":"{\"button\":\"back\"}"}}"#,
        );
    }

    #[test]
    fn a_repeat_registration_keeps_the_first() {
        let mut map: HashMap<i32, Arc<&str>> = HashMap::new();
        assert!(register_new(&mut map, 7, Arc::new("first")));
        assert!(!register_new(&mut map, 7, Arc::new("second")));
        assert_eq!(*map[&7], "first");
        assert!(register_new(&mut map, 8, Arc::new("other browser")));
    }

    #[test]
    fn the_frames_of_a_tree_are_listed_with_their_documents() {
        let tree = json!({ "frameTree": {
            "frame": { "id": "main", "loaderId": "L1" },
            "childFrames": [
                { "frame": { "id": "a", "loaderId": "L2" }, "childFrames": [{ "frame": { "id": "a1", "loaderId": "L3" } }] },
                { "frame": { "id": "b" } }
            ]
        }});
        assert_eq!(
            frames_of(&tree),
            [("main", "L1"), ("a", "L2"), ("a1", "L3"), ("b", "")].map(|(f, l)| (f.to_string(), l.to_string()))
        );
        assert!(frames_of(&json!({})).is_empty());
    }

    #[test]
    fn a_frame_attaching_is_told_from_text_that_only_mentions_it() {
        let attached = br#"{"method":"Target.attachedToTarget","params":{"sessionId":"S","targetInfo":{"targetId":"T","type":"iframe"}}}"#;
        assert!(is_event(attached, "Target.attachedToTarget"));
        let mention = br#"{"method":"Runtime.consoleAPICalled","params":{"args":[{"value":"Target.attachedToTarget"}]}}"#;
        assert!(!is_event(mention, "Target.attachedToTarget"));
        assert!(!is_event(b"Target.attachedToTarget but not json", "Target.attachedToTarget"));
        // Neither of these has a pane registered for the browser: nothing to start.
        note_event(987_655, attached);
        note_event(987_655, mention);
    }

    #[test]
    fn either_mark_is_found_wherever_it_sits_in_a_message() {
        let pad = br#"{"sessionId":"S","params":{"x":"TRUE Runtime Target.attach"}}"#;
        // Not our events: nothing registered for these browsers, and neither may panic on the near-misses above.
        note_event(987_656, pad);
        note_event(987_656, b"R");
        note_event(987_656, b"Runtime.bindingCalle");
        note_event(987_656, b"Target.attachedToTarge");
        note_event(987_656, br#"{"method":"Target.attachedToTarget"}"#);
        note_event(987_656, br#"{"method":"Runtime.bindingCalled","params":{"name":"agentmuxSideButton","payload":"{\"button\":\"back\"}"}}"#);
    }

    #[test]
    fn the_world_gets_the_sign_in_listener_everywhere_and_the_side_buttons_off_windows() {
        assert!(bindings().contains(&super::super::signin_notice::BINDING));
        assert!(SCRIPTS.contains("window.agentmuxSignIn("));
        assert_eq!(bindings().contains(&BINDING), !cfg!(target_os = "windows"));
        assert_eq!(SCRIPTS.contains(&format!("window.{BINDING}(")), !cfg!(target_os = "windows"));
        assert_eq!(BIND_EACH_WORLD, cfg!(target_os = "windows"), "a Windows pane never gets Runtime.enable");
    }

    #[test]
    fn the_listener_script_claims_the_side_buttons_and_reports_through_the_binding() {
        assert!(SCRIPT.contains(&format!("window.{BINDING}(")));
        for needle in ["e.button !== 3 && e.button !== 4", "preventDefault", "stopImmediatePropagation", "pointerup", "mouseup"] {
            assert!(SCRIPT.contains(needle), "{needle}");
        }
    }
}
