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
//! Limits, as recorded in the spec's P3 (each one tried on Linux, X11):
//! - a frame in another process (a cross-site iframe) is its own CDP target,
//!   which this page-target session can't create a world in: a press over one
//!   acts as the page's own (Chromium navigates back, no event);
//! - the listener is added once a document has committed, a few milliseconds
//!   after it starts, so a page listener registered in that window runs first:
//!   the press is still claimed (Chromium doesn't navigate and the window gets
//!   the event) but the page sees it too;
//! - when the DevTools agent detaches, its binding goes with it; the route is
//!   set up again (the listener already in the page stays, and only claims
//!   the buttons until the binding is back).

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
/// The listener; see `side_button.js`.
const SCRIPT: &str = include_str!("side_button.js");

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
        pane.inner.lock().await.ready = false;
        ensure(&pane).await
    });
}

/// Make sure the session is set up, then add the listener to every frame of
/// the page that doesn't have it for its current document.
async fn ensure(pane: &Pane) {
    let mut inner = pane.inner.lock().await;
    let mut cdp = CdpSession::attach(&pane.state, &pane.label);
    if !inner.ready {
        let setup = [
            ("Runtime.enable", json!({})),
            ("Runtime.addBinding", json!({ "name": BINDING, "executionContextName": WORLD })),
        ];
        for (method, params) in setup {
            if let Err(e) = cdp.call(method, params).await {
                tracing::warn!(label = %pane.label, error = %e, "[side-button] couldn't set up the pane ({method})");
                return;
            }
        }
        inner.ready = true;
    }
    let Ok(tree) = cdp.call("Page.getFrameTree", json!({})).await else { return };
    let frames = frames_of(&tree);
    inner.injected.retain(|k| frames.contains(k));
    for key in frames {
        // Marked first: a frame this session can't reach (another process) would fail again each time.
        if !inner.injected.insert(key.clone()) {
            continue;
        }
        let world = cdp.call("Page.createIsolatedWorld", json!({ "frameId": key.0, "worldName": WORLD })).await;
        let Some(context) = world.ok().and_then(|w| w.get("executionContextId").and_then(Value::as_i64)) else { continue };
        let params = json!({ "contextId": context, "expression": SCRIPT, "returnByValue": true });
        match cdp.call("Runtime.evaluate", params).await {
            Ok(_) => tracing::debug!(label = %pane.label, frame = %key.0, "[side-button] listener added"),
            Err(e) => tracing::warn!(label = %pane.label, error = %e, "[side-button] couldn't add the listener to a frame"),
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
/// binding's call, ignore the rest. Cheap for the common case (a substring
/// test) because every CDP event of every pane passes through here.
pub fn note_event(browser_id: i32, message: &[u8]) {
    const MARK: &[u8] = b"Runtime.bindingCalled";
    if !message.windows(MARK.len()).any(|w| w == MARK) {
        return;
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
    fn the_listener_script_claims_the_side_buttons_and_reports_through_the_binding() {
        assert!(SCRIPT.contains(&format!("window.{BINDING}(")));
        for needle in ["e.button !== 3 && e.button !== 4", "preventDefault", "stopImmediatePropagation", "pointerup", "mouseup"] {
            assert!(SCRIPT.contains(needle), "{needle}");
        }
    }
}
