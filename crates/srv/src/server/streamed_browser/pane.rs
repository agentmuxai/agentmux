// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One streamed pane: its windows (a page of the server's Chromium each, `windows.rs`) behind one WebSocket. Frames of the shown
//! window go out as JPEG and are acknowledged only once handed to the socket; input comes in and goes to the shown window.
//! docs/specs/SPEC_STREAMED_BROWSER_PANE_DEMO_2026_11_15.md §4 to §6.
//!
//! The logic is in `Pane`, which answers each event or message with the things to send (`Out`), so it is tested with a stand-in
//! browser and no socket; `run` is the loop that connects it to a WebSocket.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::ws::{Message, WebSocket};
use base64::Engine as _;
use serde_json::{json, Value};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender, WeakUnboundedSender};

use super::cdp::{Cdp, Event};
use super::chromium::Lease;
use super::nav;
use super::popups::{self, TargetEvent};
use super::protocol::{self, ClientMsg, WindowOp};
use super::windows::{self, NoticeLimiter, WindowId, Windows, FIRST};

/// Kind byte of a binary message: a JPEG frame.
const FRAME: u8 = 1;
/// The quality of the screencast (report section 6: SSIM 0.998 at 141 kB for a full frame).
const JPEG_QUALITY: u32 = 70;

pub struct Start {
    pub url: String,
    pub w: u32,
    pub h: u32,
}

/// What a pane has to send, in order.
#[derive(Debug, PartialEq)]
pub(super) enum Out {
    Text(Value),
    /// A frame (kind byte first), and the screencast session to acknowledge once it is handed to the socket.
    Frame { bytes: Vec<u8>, ack: Option<(String, i64)> },
}

/// What the pane keeps of a window that the list of windows does not.
struct PageState {
    loading: bool,
    /// When the last resize was asked for, to say how long the new frame took.
    resized_at: Option<Instant>,
}

/// URL patterns no page may load (`Network.setBlockedURLs`): local files and the browser's own internal pages.
pub(super) const BLOCKED_URLS: &[&str] = &["file:*", "chrome:*", "chrome-untrusted:*", "chrome-extension:*", "devtools:*", "view-source:*"];

fn screencast(w: u32, h: u32) -> Value {
    json!({ "format": "jpeg", "quality": JPEG_QUALITY, "maxWidth": w, "maxHeight": h, "everyNthFrame": 1 })
}

fn viewport(w: u32, h: u32) -> Value {
    json!({ "width": w, "height": h, "deviceScaleFactor": 1, "mobile": false })
}

type Events = UnboundedReceiver<(WindowId, Event)>;

pub(super) struct Pane {
    cdp: Arc<Cdp>,
    windows: Windows,
    state: HashMap<WindowId, PageState>,
    size: (u32, u32),
    /// Where the events of a window's session go, tagged with the window. Weak: when the browser goes, every session's end drops the
    /// last strong sender and the pane's queue ends with it.
    events: WeakUnboundedSender<(WindowId, Event)>,
    /// Where the browser's news of this pane's windows is told (`popups`).
    targets: UnboundedSender<TargetEvent>,
    limiter: NoticeLimiter,
}

fn listen(cdp: &Cdp, session: &str, id: WindowId, tx: UnboundedSender<(WindowId, Event)>) {
    cdp.subscribe_with(session, move |event| {
        let _ = tx.send((id, event));
    });
}

impl Pane {
    /// The pane's first window: a page made for it, switched on and sent to its first address. Closed again if it cannot be made ready.
    pub(super) async fn open(cdp: &Arc<Cdp>, start: &Start) -> Result<(Pane, Events, UnboundedReceiver<TargetEvent>), String> {
        let made = cdp.call("Target.createTarget", json!({ "url": "about:blank" }), None).await?;
        let target = made.get("targetId").and_then(Value::as_str).ok_or("the browser made no page")?.to_string();
        let attached = match cdp.call("Target.attachToTarget", json!({ "targetId": target, "flatten": true }), None).await {
            Ok(a) => a,
            Err(e) => {
                let _ = cdp.call("Target.closeTarget", json!({ "targetId": target }), None).await;
                return Err(e);
            }
        };
        let session = attached.get("sessionId").and_then(Value::as_str).ok_or("the browser gave no session")?.to_string();
        let (tx, events) = mpsc::unbounded_channel();
        let weak = tx.downgrade();
        listen(cdp, &session, FIRST, tx);
        let (targets, told) = mpsc::unbounded_channel();
        popups::claim(&target, targets.clone());
        let mut state = HashMap::new();
        state.insert(FIRST, PageState { loading: false, resized_at: None });
        let pane = Pane { cdp: cdp.clone(), windows: Windows::starting(target, session), state, size: (start.w, start.h), events: weak, targets, limiter: NoticeLimiter::default() };
        match pane.prepare(FIRST, Some(&start.url)).await {
            Ok(()) => Ok((pane, events, told)),
            Err(e) => {
                pane.finish().await;
                Err(e)
            }
        }
    }

    fn session_of(&self, id: WindowId) -> Option<String> {
        self.windows.get(id).map(|w| w.session.clone())
    }

    fn active_session(&self) -> Option<String> {
        self.session_of(self.windows.active())
    }

    /// Switch a window on: events, the viewport, focus, the screencast and, for the first window, its first address.
    async fn prepare(&self, id: WindowId, url: Option<&str>) -> Result<(), String> {
        let session = self.session_of(id).ok_or("that window is gone")?;
        let (cdp, s) = (&self.cdp, Some(session.as_str()));
        cdp.call("Page.enable", json!({}), s).await?;
        cdp.call("Emulation.setDeviceMetricsOverride", viewport(self.size.0, self.size.1), s).await?;
        // The page counts as focused, so its carets and :focus styles show.
        cdp.call("Emulation.setFocusEmulationEnabled", json!({ "enabled": true }), s).await?;
        // A page's file chooser is intercepted and never answered: nothing on the server's disks is offered to a page.
        cdp.call("Page.setInterceptFileChooserDialog", json!({ "enabled": true }), s).await?;
        // No page loads, or navigates to, the server's own files or the browser's internal pages, in the top frame, in a
        // frame, or as a script, image or request of a page. (The address bar already takes only http and https.) A window a
        // page opens is switched on the same way as the first one.
        cdp.call("Network.enable", json!({ "maxTotalBufferSize": 0, "maxResourceBufferSize": 0 }), s).await?;
        cdp.call("Network.setBlockedURLs", json!({ "urls": BLOCKED_URLS }), s).await?;
        cdp.call("Page.startScreencast", screencast(self.size.0, self.size.1), s).await?;
        if let Some(url) = url.filter(|u| *u != "about:blank") {
            nav::go(cdp, &session, protocol::Go::Url, url).await?;
        }
        Ok(())
    }

    /// The pane has ended: every window is let go and closed.
    pub(super) async fn finish(self) {
        for w in self.windows.entries() {
            self.cdp.unsubscribe(&w.session);
            popups::release(&w.target);
        }
        for w in self.windows.entries() {
            let _ = self.cdp.call("Target.closeTarget", json!({ "targetId": w.target }), None).await;
        }
    }

    fn notice(&mut self, text: String, out: &mut Vec<Out>) {
        if self.limiter.allow(Instant::now()) {
            out.push(Out::Text(json!({ "t": "notice", "message": text })));
        }
    }

    /// Tell the pane which windows there are and where the shown one is.
    async fn announce(&mut self, out: &mut Vec<Out>) {
        out.push(Out::Text(self.windows.message()));
        let id = self.windows.active();
        self.nav_update(id, out, false).await;
    }

    /// A window's address or loading state changed: remember it, tell the pane the windows, and the shown window's bar.
    async fn nav_update(&mut self, id: WindowId, out: &mut Vec<Out>, windows_too: bool) {
        let Some(session) = self.session_of(id) else { return };
        let loading = self.state.get(&id).is_some_and(|s| s.loading);
        let message = nav::nav_message(&nav::history(&self.cdp, &session).await, loading);
        let text = |key: &str| message.get(key).and_then(Value::as_str).unwrap_or("").to_string();
        let (url, title) = (text("url"), text("title"));
        if self.windows.get(id).is_some_and(|w| w.url != url || w.title != title) {
            self.windows.set_location(id, url.clone(), title);
            if windows_too {
                out.push(Out::Text(self.windows.message()));
            }
            if let Some(line) = windows::provider_refusal(&url) {
                self.notice(line, out);
            }
        }
        if id == self.windows.active() {
            out.push(Out::Text(message));
        }
    }

    /// Show `id`: the screencast moves to it.
    async fn show(&mut self, id: WindowId, out: &mut Vec<Out>) {
        let before = self.windows.active();
        if !self.windows.focus(id) {
            return;
        }
        if before != id {
            if let Some(old) = self.session_of(before) {
                let _ = self.cdp.call("Page.stopScreencast", json!({}), Some(&old)).await;
            }
            if let Some(new) = self.session_of(id) {
                let _ = self.cdp.call("Page.startScreencast", screencast(self.size.0, self.size.1), Some(&new)).await;
            }
        }
        self.announce(out).await;
    }

    /// A window ended (or was closed): it is let go, and when it was the shown one the window that opened it is shown again.
    /// False when it was the pane's first, which ends the pane.
    async fn forget(&mut self, id: WindowId, out: &mut Vec<Out>) -> bool {
        if id == FIRST {
            return false;
        }
        let Some(gone) = self.windows.get(id).cloned() else { return true };
        let Some(removed) = self.windows.remove(id) else { return true };
        self.cdp.unsubscribe(&gone.session);
        popups::release(&gone.target);
        self.state.remove(&id);
        if removed.was_active {
            if let Some(next) = self.session_of(removed.now_active) {
                let _ = self.cdp.call("Page.startScreencast", screencast(self.size.0, self.size.1), Some(&next)).await;
            }
        }
        self.announce(out).await;
        true
    }

    /// The browser's news of this pane's windows. `false` ends the pane.
    pub(super) async fn on_target(&mut self, event: TargetEvent, out: &mut Vec<Out>) -> bool {
        match event {
            TargetEvent::Opened { target_id, opener_id, url } => {
                self.opened(&target_id, &opener_id, &url, out).await;
                true
            }
            TargetEvent::Gone { target_id } => match self.windows.by_target(&target_id) {
                Some(id) => self.forget(id, out).await,
                None => true,
            },
        }
    }

    async fn close_target(&self, target: &str) {
        let _ = self.cdp.call("Target.closeTarget", json!({ "targetId": target }), None).await;
    }

    /// A page of this pane opened a window: it is shown, unless the pane already shows as many as it may.
    async fn opened(&mut self, target: &str, opener_target: &str, url: &str, out: &mut Vec<Out>) {
        let opener = self.windows.by_target(opener_target);
        let opener_url = opener.and_then(|o| self.windows.get(o)).map(|w| w.url.clone()).unwrap_or_default();
        if opener.is_none() || !self.windows.has_room() {
            self.close_target(target).await;
            let line = if opener.is_none() { windows::window_not_shown(url) } else { windows::too_many_windows(&opener_url) };
            self.notice(line, out);
            return;
        }
        let attached = self.cdp.call("Target.attachToTarget", json!({ "targetId": target, "flatten": true }), None).await;
        let session = attached.ok().and_then(|a| a.get("sessionId").and_then(Value::as_str).map(str::to_string));
        let (Some(session), Some(tx)) = (session, self.events.upgrade()) else {
            self.close_target(target).await;
            self.notice(windows::window_not_shown(&opener_url), out);
            return;
        };
        let before = self.windows.active();
        let Some(id) = self.windows.add(target.to_string(), session.clone(), opener) else {
            self.close_target(target).await;
            return;
        };
        listen(&self.cdp, &session, id, tx);
        popups::claim(target, self.targets.clone());
        self.state.insert(id, PageState { loading: false, resized_at: None });
        if let Err(e) = self.prepare(id, None).await {
            tracing::warn!(error = %e, "streamed browser: a window a page opened could not be shown");
            self.windows.remove(id);
            self.cdp.unsubscribe(&session);
            popups::release(target);
            self.state.remove(&id);
            self.close_target(target).await;
            // The shown window was never changed by the one that failed: say so, and carry on with it.
            let _ = self.windows.focus(before);
            self.notice(windows::window_not_shown(&opener_url), out);
            return;
        }
        // The screencast moves to the new window: the one before it stops.
        if let Some(old) = self.session_of(before) {
            let _ = self.cdp.call("Page.stopScreencast", json!({}), Some(&old)).await;
        }
        self.announce(out).await;
    }

    /// What a window's event means for the pane. `false` ends the pane.
    pub(super) async fn on_event(&mut self, id: WindowId, event: Event, out: &mut Vec<Out>) -> bool {
        match event.method.as_str() {
            "Page.screencastFrame" => self.frame(id, &event.params, out),
            "Page.frameStartedLoading" => {
                self.state.entry(id).or_insert(PageState { loading: false, resized_at: None }).loading = true;
                self.nav_update(id, out, true).await;
            }
            "Page.frameStoppedLoading" => {
                self.state.entry(id).or_insert(PageState { loading: false, resized_at: None }).loading = false;
                self.nav_update(id, out, true).await;
            }
            "Page.frameNavigated" if event.params.pointer("/frame/parentId").is_none() => self.nav_update(id, out, true).await,
            // A dialog the page raised is dismissed (the pane has nowhere to show it), and the person is told which site raised it.
            "Page.javascriptDialogOpening" => {
                if let Some(session) = self.session_of(id) {
                    self.cdp.send("Page.handleJavaScriptDialog", json!({ "accept": false }), Some(&session));
                }
                let page_url = event.params.get("url").and_then(Value::as_str).map(str::to_string).or_else(|| self.windows.get(id).map(|w| w.url.clone())).unwrap_or_default();
                let kind = event.params.get("type").and_then(Value::as_str).unwrap_or("alert");
                let message = event.params.get("message").and_then(Value::as_str).unwrap_or("");
                self.notice(windows::dialog_dismissed(&page_url, kind, message), out);
            }
            "Inspector.targetCrashed" => {
                if id == FIRST {
                    out.push(Out::Text(json!({ "t": "error", "message": "the page crashed" })));
                    return false;
                }
                if let Some(target) = self.windows.get(id).map(|w| w.target.clone()) {
                    self.close_target(&target).await;
                }
                return self.forget(id, out).await;
            }
            _ => {}
        }
        true
    }

    /// A screencast frame: the shown window's goes to the pane; another window's is acknowledged and dropped.
    fn frame(&mut self, id: WindowId, params: &Value, out: &mut Vec<Out>) {
        let ack = params.get("sessionId").and_then(Value::as_i64);
        let session = self.session_of(id);
        let pair = session.clone().zip(ack);
        if id != self.windows.active() {
            if let Some((s, a)) = pair {
                self.cdp.send("Page.screencastFrameAck", json!({ "sessionId": a }), Some(&s));
            }
            return;
        }
        let Some(data) = params.get("data").and_then(Value::as_str) else { return };
        let Ok(jpeg) = base64::engine::general_purpose::STANDARD.decode(data) else { return };
        let mut bytes = Vec::with_capacity(jpeg.len() + 1);
        bytes.push(FRAME);
        bytes.extend_from_slice(&jpeg);
        if let Some(at) = self.state.get_mut(&id).and_then(|s| s.resized_at.take()) {
            tracing::debug!(ms = at.elapsed().as_millis() as u64, "streamed browser: first frame after a resize");
        }
        // Acknowledged only once handed to the socket, so a slow client slows the page's own capture instead of queueing frames.
        out.push(Out::Frame { bytes, ack: pair });
    }

    /// Resize every window and make the browser send a frame of the new size even when the page does not change: stopping and
    /// starting the screencast does that.
    async fn resize(&mut self, w: u32, h: u32) -> Result<(), String> {
        self.size = (w, h);
        let active = self.windows.active();
        let ids: Vec<WindowId> = self.windows.entries().iter().map(|e| e.id).collect();
        for id in ids {
            let Some(session) = self.session_of(id) else { continue };
            self.cdp.call("Emulation.setDeviceMetricsOverride", viewport(w, h), Some(&session)).await?;
            if id == active {
                self.cdp.call("Page.stopScreencast", json!({}), Some(&session)).await?;
                self.cdp.call("Page.startScreencast", screencast(w, h), Some(&session)).await?;
                if let Some(s) = self.state.get_mut(&id) {
                    s.resized_at = Some(Instant::now());
                }
            }
        }
        Ok(())
    }

    /// What a message from the pane means. `false` ends the pane.
    pub(super) async fn on_client(&mut self, text: &str, out: &mut Vec<Out>) -> bool {
        let msg = match protocol::parse(text) {
            Ok(m) => m,
            Err(e) => {
                out.push(Out::Text(json!({ "t": "error", "message": e })));
                return true;
            }
        };
        if let Some((method, params)) = protocol::input_call(&msg) {
            if let Some(session) = self.active_session() {
                self.cdp.send(method, params, Some(&session));
            }
            return true;
        }
        match msg {
            ClientMsg::Resize { w, h } => match self.resize(w, h).await {
                Ok(()) => out.push(Out::Text(json!({ "t": "size", "w": w, "h": h }))),
                Err(e) => out.push(Out::Text(json!({ "t": "error", "message": e }))),
            },
            ClientMsg::Nav { go, url } => {
                if let Some(session) = self.active_session() {
                    if let Err(e) = nav::go(&self.cdp, &session, go, &url).await {
                        out.push(Out::Text(json!({ "t": "error", "message": e })));
                    }
                }
            }
            ClientMsg::Window { op: WindowOp::Focus, id } => self.show(id, out).await,
            ClientMsg::Window { op: WindowOp::Close, id } => {
                // The first window is the pane itself: it is not closed from here.
                if id != FIRST {
                    if let Some(target) = self.windows.get(id).map(|w| w.target.clone()) {
                        self.close_target(&target).await;
                        self.forget(id, out).await;
                    }
                }
            }
            _ => {}
        }
        true
    }
}

async fn send_out(socket: &mut WebSocket, cdp: &Cdp, out: Vec<Out>) -> bool {
    for item in out {
        match item {
            Out::Text(v) => {
                if socket.send(Message::Text(v.to_string().into())).await.is_err() {
                    return false;
                }
            }
            Out::Frame { bytes, ack } => {
                if socket.send(Message::Binary(bytes.into())).await.is_err() {
                    return false;
                }
                if let Some((session, id)) = ack {
                    cdp.send("Page.screencastFrameAck", json!({ "sessionId": id }), Some(&session));
                }
            }
        }
    }
    true
}

/// Run a pane until its socket closes, its first page ends, or the sign-in behind it does.
pub async fn run(mut socket: WebSocket, lease: Lease, start: Start, ended: impl std::future::Future<Output = ()>) {
    let (mut pane, mut events, mut told) = match Pane::open(&lease.cdp, &start).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "streamed browser: a pane could not be opened");
            let _ = socket.send(Message::Text(json!({ "t": "error", "message": e }).to_string().into())).await;
            return;
        }
    };
    let _ = socket.send(Message::Text(json!({ "t": "size", "w": start.w, "h": start.h }).to_string().into())).await;
    tokio::pin!(ended);
    loop {
        let mut out = Vec::new();
        let keep = tokio::select! {
            _ = &mut ended => false,
            event = events.recv() => match event {
                Some((id, e)) => pane.on_event(id, e, &mut out).await,
                None => false,
            },
            news = told.recv() => match news {
                Some(n) => pane.on_target(n, &mut out).await,
                None => false,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(t))) => pane.on_client(t.as_str(), &mut out).await,
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => false,
                Some(Ok(_)) => true,
            },
        };
        if !send_out(&mut socket, &lease.cdp, out).await || !keep {
            break;
        }
    }
    pane.finish().await;
    drop(lease);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::streamed_browser::fake_browser::FakeBrowser;

    /// The registry of who owns which page is the process's: tests that use the same page names take turns.
    async fn serial() -> tokio::sync::MutexGuard<'static, ()> {
        static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        TURN.lock().await
    }

    fn start() -> Start {
        Start { url: "https://app.example/".into(), w: 800, h: 600 }
    }

    /// A pane over a stand-in browser, with `app.example` loaded in its first window.
    async fn pane() -> (Pane, Events, UnboundedReceiver<TargetEvent>, FakeBrowser) {
        let fake = FakeBrowser::start();
        fake.set_address("S-T1", "https://app.example/", "App");
        let (pane, events, told) = Pane::open(&fake.cdp, &start()).await.expect("the pane opens");
        (pane, events, told, fake)
    }

    /// What the browser-level watcher does with a new page: the pane that owns the opener is told, else the page is closed.
    fn browser_says(method: &str, params: Value) -> Option<String> {
        popups::route(method, &params)
    }

    fn created(target: &str, opener: &str) -> Value {
        json!({ "targetInfo": { "targetId": target, "type": "page", "openerId": opener, "url": "https://idp.example/auth" } })
    }

    fn texts(out: &[Out]) -> Vec<Value> {
        out.iter().filter_map(|o| if let Out::Text(v) = o { Some(v.clone()) } else { None }).collect()
    }

    fn of_kind(out: &[Out], kind: &str) -> Vec<Value> {
        texts(out).into_iter().filter(|v| v["t"] == kind).collect()
    }

    /// A pane with a pop-up open (window 1, shown).
    async fn with_popup() -> (Pane, Events, UnboundedReceiver<TargetEvent>, FakeBrowser, Vec<Out>) {
        let (mut pane, events, mut told, fake) = pane().await;
        fake.set_address("S-T2", "https://idp.example/auth", "Sign in");
        assert_eq!(browser_says("Target.targetCreated", created("T2", "T1")), None, "a page of the pane opened it: it is not closed");
        let news = told.recv().await.unwrap();
        let mut out = Vec::new();
        assert!(pane.on_target(news, &mut out).await);
        (pane, events, told, fake, out)
    }

    #[tokio::test]
    async fn the_first_window_is_switched_on_with_the_blocked_addresses_and_sent_to_its_address() {
        let _serial = serial().await;
        let (pane, _events, _told, fake) = pane().await;
        let calls = fake.calls();
        let on = |m: &str| calls.iter().any(|c| c.method == m && c.session.as_deref() == Some("S-T1"));
        for m in ["Page.enable", "Emulation.setDeviceMetricsOverride", "Network.enable", "Network.setBlockedURLs", "Page.startScreencast", "Page.navigate"] {
            assert!(on(m), "{m}");
        }
        let blocked = calls.iter().find(|c| c.method == "Network.setBlockedURLs").unwrap();
        assert_eq!(blocked.params["urls"], json!(BLOCKED_URLS));
        pane.finish().await;
    }

    #[tokio::test]
    async fn a_window_the_page_opens_is_shown_with_its_address_and_is_not_closed() {
        let _serial = serial().await;
        let (pane, _events, _told, fake, out) = with_popup().await;
        let calls = fake.calls();
        // It was attached and switched on exactly as the first one: the blocked addresses apply to it too.
        for m in ["Page.enable", "Network.enable", "Network.setBlockedURLs", "Page.startScreencast"] {
            assert!(calls.iter().any(|c| c.method == m && c.session.as_deref() == Some("S-T2")), "{m} on the pop-up");
        }
        let blocked = calls.iter().find(|c| c.method == "Network.setBlockedURLs" && c.session.as_deref() == Some("S-T2")).unwrap();
        assert_eq!(blocked.params["urls"], json!(BLOCKED_URLS));
        // The screencast moved to it, and nothing closed it.
        assert!(calls.iter().any(|c| c.method == "Page.stopScreencast" && c.session.as_deref() == Some("S-T1")));
        assert!(!calls.iter().any(|c| c.method == "Target.closeTarget" && c.params["targetId"] == "T2"));
        let windows = of_kind(&out, "windows");
        assert_eq!(windows.last().unwrap()["active"], 1);
        assert_eq!(windows.last().unwrap()["windows"].as_array().unwrap().len(), 2);
        assert_eq!(windows.last().unwrap()["windows"][1]["opener"], 0);
        // The bar shows the pop-up's own address, so the person sees which site asks for the password.
        let nav = of_kind(&out, "nav");
        assert_eq!(nav.last().unwrap()["url"], "https://idp.example/auth");
        pane.finish().await;
    }

    #[tokio::test]
    async fn input_and_addresses_go_to_the_shown_window() {
        let _serial = serial().await;
        let (mut pane, _events, _told, fake, _) = with_popup().await;
        let mut out = Vec::new();
        assert!(pane.on_client(r#"{"t":"mouse","type":"down","x":5,"y":6,"button":"left","buttons":1,"clicks":1}"#, &mut out).await);
        assert!(pane.on_client(r#"{"t":"nav","go":"url","url":"https://idp.example/next"}"#, &mut out).await);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let calls = fake.calls();
        let mouse = calls.iter().rev().find(|c| c.method == "Input.dispatchMouseEvent").unwrap();
        assert_eq!(mouse.session.as_deref(), Some("S-T2"));
        let go = calls.iter().rev().find(|c| c.method == "Page.navigate").unwrap();
        assert_eq!((go.session.as_deref(), go.params["url"].as_str()), (Some("S-T2"), Some("https://idp.example/next")));
        pane.finish().await;
    }

    #[tokio::test]
    async fn a_window_that_closes_itself_hands_the_pane_back_to_the_page_that_opened_it() {
        let _serial = serial().await;
        let (mut pane, _events, mut told, fake, _) = with_popup().await;
        assert_eq!(browser_says("Target.targetDestroyed", json!({ "targetId": "T2" })), None);
        let news = told.recv().await.unwrap();
        assert_eq!(news, TargetEvent::Gone { target_id: "T2".into() });
        let mut out = Vec::new();
        assert!(pane.on_target(news, &mut out).await);
        assert_eq!(of_kind(&out, "windows").last().unwrap()["active"], 0);
        assert_eq!(of_kind(&out, "windows").last().unwrap()["windows"].as_array().unwrap().len(), 1);
        assert_eq!(of_kind(&out, "nav").last().unwrap()["url"], "https://app.example/");
        let calls = fake.calls();
        assert!(calls.iter().rev().any(|c| c.method == "Page.startScreencast" && c.session.as_deref() == Some("S-T1")), "the page is streamed again");
        // A late event from the closed window finds nothing.
        assert!(pane.on_event(1, Event { method: "Page.frameStoppedLoading".into(), params: Value::Null }, &mut Vec::new()).await);
        pane.finish().await;
    }

    #[tokio::test]
    async fn the_first_window_going_ends_the_pane() {
        let _serial = serial().await;
        let (mut pane, _events, mut told, _fake) = pane().await;
        browser_says("Target.targetDestroyed", json!({ "targetId": "T1" }));
        let news = told.recv().await.unwrap();
        assert!(!pane.on_target(news, &mut Vec::new()).await);
        pane.finish().await;
    }

    #[tokio::test]
    async fn a_page_cannot_open_more_windows_than_a_pane_shows_and_the_person_is_told_once() {
        let _serial = serial().await;
        let (mut pane, _events, mut told, fake) = pane().await;
        for n in 2..=windows::MAX_WINDOWS as u32 {
            fake.set_address(&format!("S-T{n}"), "https://idp.example/auth", "x");
            browser_says("Target.targetCreated", created(&format!("T{n}"), "T1"));
            let news = told.recv().await.unwrap();
            assert!(pane.on_target(news, &mut Vec::new()).await);
        }
        let mut out = Vec::new();
        for extra in ["T90", "T91"] {
            browser_says("Target.targetCreated", created(extra, "T1"));
            let news = told.recv().await.unwrap();
            assert!(pane.on_target(news, &mut out).await);
            assert!(fake.calls().iter().any(|c| c.method == "Target.closeTarget" && c.params["targetId"] == extra), "{extra} is closed");
            assert!(!fake.calls().iter().any(|c| c.method == "Target.attachToTarget" && c.params["targetId"] == extra), "{extra} is never attached");
        }
        let notices = of_kind(&out, "notice");
        assert_eq!(notices.len(), 1, "two refusals in a moment are one line");
        let line = notices[0]["message"].as_str().unwrap();
        assert!(line.contains("app.example") || line.contains("a page"), "{line}");
        assert!(line.contains("another window"), "{line}");
        pane.finish().await;
    }

    #[tokio::test]
    async fn a_dialog_is_dismissed_and_the_person_is_told_which_site_raised_it() {
        let _serial = serial().await;
        let (mut pane, _events, _told, fake) = pane().await;
        let mut out = Vec::new();
        let event = Event { method: "Page.javascriptDialogOpening".into(), params: json!({ "url": "https://ads.example/x", "type": "confirm", "message": "Leave this page?" }) };
        assert!(pane.on_event(FIRST, event, &mut out).await);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let dismissal = fake.calls().into_iter().find(|c| c.method == "Page.handleJavaScriptDialog").unwrap();
        assert_eq!((dismissal.params["accept"].as_bool(), dismissal.session.as_deref()), (Some(false), Some("S-T1")));
        let line = of_kind(&out, "notice")[0]["message"].as_str().unwrap().to_string();
        assert_eq!(line, "ads.example opened a confirmation question (“Leave this page?”). It was dismissed.");
        pane.finish().await;
    }

    #[tokio::test]
    async fn a_page_that_is_googles_refusal_gets_a_line_that_says_so_and_nothing_more() {
        let _serial = serial().await;
        let (mut pane, _events, _told, fake) = pane().await;
        fake.set_address("S-T1", "https://accounts.google.com/v3/signin/rejected?dsh=x", "Sign in - Google Accounts");
        let mut out = Vec::new();
        let navigated = Event { method: "Page.frameNavigated".into(), params: json!({ "frame": { "id": "f" } }) };
        assert!(pane.on_event(FIRST, navigated, &mut out).await);
        let line = of_kind(&out, "notice")[0]["message"].as_str().unwrap().to_string();
        assert!(line.starts_with("Google refused to sign in with this browser"), "{line}");
        // An ordinary page says nothing.
        fake.set_address("S-T1", "https://example.com/", "Example");
        let mut quiet = Vec::new();
        pane.on_event(FIRST, Event { method: "Page.frameNavigated".into(), params: json!({ "frame": { "id": "f" } }) }, &mut quiet).await;
        assert!(of_kind(&quiet, "notice").is_empty());
        pane.finish().await;
    }

    #[tokio::test]
    async fn the_person_can_show_another_window_and_close_a_pop_up_but_never_the_first_window() {
        let _serial = serial().await;
        let (mut pane, _events, _told, fake, _) = with_popup().await;
        let mut out = Vec::new();
        assert!(pane.on_client(r#"{"t":"window","op":"focus","id":0}"#, &mut out).await);
        assert_eq!(of_kind(&out, "windows").last().unwrap()["active"], 0);
        assert_eq!(of_kind(&out, "nav").last().unwrap()["url"], "https://app.example/");
        // closing the first window is refused: nothing is closed
        let before = fake.calls().iter().filter(|c| c.method == "Target.closeTarget").count();
        assert!(pane.on_client(r#"{"t":"window","op":"close","id":0}"#, &mut out).await);
        assert_eq!(fake.calls().iter().filter(|c| c.method == "Target.closeTarget").count(), before);
        // a window that is not there changes nothing
        assert!(pane.on_client(r#"{"t":"window","op":"focus","id":42}"#, &mut out).await);
        assert_eq!(pane.windows.active(), 0);
        // closing the pop-up closes it in the browser and leaves one window
        let mut closing = Vec::new();
        assert!(pane.on_client(r#"{"t":"window","op":"close","id":1}"#, &mut closing).await);
        assert!(fake.calls().iter().any(|c| c.method == "Target.closeTarget" && c.params["targetId"] == "T2"));
        assert_eq!(of_kind(&closing, "windows").last().unwrap()["windows"].as_array().unwrap().len(), 1);
        pane.finish().await;
    }

    #[tokio::test]
    async fn only_the_shown_windows_frames_reach_the_pane_and_the_others_are_acknowledged() {
        let _serial = serial().await;
        let (mut pane, _events, _told, fake, _) = with_popup().await;
        let frame = |n: i64| Event { method: "Page.screencastFrame".into(), params: json!({ "data": "AQID", "sessionId": n }) };
        let mut out = Vec::new();
        assert!(pane.on_event(0, frame(7), &mut out).await);
        assert!(out.is_empty(), "the first window is not shown now");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(fake.calls().iter().any(|c| c.method == "Page.screencastFrameAck" && c.params["sessionId"] == 7 && c.session.as_deref() == Some("S-T1")));
        assert!(pane.on_event(1, frame(8), &mut out).await);
        assert_eq!(out, vec![Out::Frame { bytes: vec![1, 1, 2, 3], ack: Some(("S-T2".to_string(), 8)) }]);
        pane.finish().await;
    }

    #[tokio::test]
    async fn a_resize_reaches_every_window_and_the_pane_ending_closes_them_all() {
        let _serial = serial().await;
        let (mut pane, _events, _told, fake, _) = with_popup().await;
        let mut out = Vec::new();
        assert!(pane.on_client(r#"{"t":"resize","w":900,"h":700}"#, &mut out).await);
        let sized: Vec<_> = fake.calls().into_iter().filter(|c| c.method == "Emulation.setDeviceMetricsOverride" && c.params["width"] == 900).collect();
        assert_eq!(sized.len(), 2);
        pane.finish().await;
        let closed: Vec<_> = fake.calls().into_iter().filter(|c| c.method == "Target.closeTarget").map(|c| c.params["targetId"].as_str().unwrap().to_string()).collect();
        assert!(closed.contains(&"T1".to_string()) && closed.contains(&"T2".to_string()), "{closed:?}");
        // Nothing of this pane is claimed any more: a window opened by one of its pages is nobody's and is closed.
        assert_eq!(browser_says("Target.targetCreated", created("T77", "T2")).as_deref(), Some("T77"));
    }

    #[tokio::test]
    async fn a_window_the_browser_will_not_let_the_pane_attach_is_closed_and_the_person_told() {
        let _serial = serial().await;
        let (mut pane, _events, mut told, fake) = pane().await;
        fake.refuse("Target.attachToTarget", "T2");
        browser_says("Target.targetCreated", created("T2", "T1"));
        let news = told.recv().await.unwrap();
        let mut out = Vec::new();
        assert!(pane.on_target(news, &mut out).await);
        assert!(fake.calls().iter().any(|c| c.method == "Target.closeTarget" && c.params["targetId"] == "T2"));
        assert!(of_kind(&out, "notice")[0]["message"].as_str().unwrap().contains("could not be shown"));
        assert_eq!(pane.windows.len(), 1);
        pane.finish().await;
    }

    // ---- with a real Chromium (not run by default: `cargo test -p agentmux-srv real_chromium -- --ignored`) ----------------------------

    /// A page server on a loopback port of its own: `/app` has a button that opens a sign-in window; `/popup` is that window, which
    /// returns its result to the page that opened it and closes itself.
    async fn sign_in_site() -> u16 {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..n]).to_string();
                    let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let body = if path.starts_with("/popup") {
                        "<title>Sign in</title><p>the sign-in window</p><script>setTimeout(() => { opener.postMessage({ code: 'abc' }, '*'); window.close(); }, 600);</script>"
                    } else {
                        "<title>App</title><button id=b style='position:absolute;left:10px;top:10px;width:300px;height:80px' onclick='signIn()'>Sign in</button>\
                         <script>window.result = { messages: [], closed: false, opened: false };\
                         addEventListener('message', (e) => result.messages.push(e.data));\
                         function signIn() { const w = open('/popup', 'idp', 'width=500,height=600'); result.opened = !!w; const t = setInterval(() => { if (w && w.closed) { result.closed = true; clearInterval(t); } }, 100); }</script>"
                    };
                    let reply = format!("HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                    let _ = socket.write_all(reply.as_bytes()).await;
                });
            }
        });
        port
    }

    /// Feed the pane what the browser says for `ms`, collecting what it would send.
    async fn pump(pane: &mut Pane, events: &mut Events, told: &mut UnboundedReceiver<TargetEvent>, out: &mut Vec<Out>, ms: u64) {
        let end = tokio::time::Instant::now() + std::time::Duration::from_millis(ms);
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(end) => return,
                Some((id, e)) = events.recv() => { pane.on_event(id, e, out).await; }
                Some(n) = told.recv() => { pane.on_target(n, out).await; }
            }
        }
    }

    #[tokio::test]
    #[ignore = "needs a Chromium: AGENTMUX_STREAMED_BROWSER_BIN, or one on the PATH"]
    async fn real_chromium_a_sign_in_window_returns_its_result_to_the_page_and_the_pane_goes_back() {
        let _serial = serial().await;
        super::super::chromium::use_profile_root(std::env::temp_dir().join(format!("amx-pane-test-{}", std::process::id())));
        let port = sign_in_site().await;
        let lease = super::super::chromium::lease().await.expect("a browser for the pane");
        let (mut pane, mut events, mut told) = Pane::open(&lease.cdp, &Start { url: format!("http://127.0.0.1:{port}/app"), w: 800, h: 600 }).await.expect("the pane opens");
        let mut out = Vec::new();
        pump(&mut pane, &mut events, &mut told, &mut out, 2000).await;

        // A person's click on the button (a real input event, so the page may open a window).
        for kind in ["down", "up"] {
            let click = format!(r#"{{"t":"mouse","type":"{kind}","x":50,"y":40,"button":"left","buttons":1,"clicks":1}}"#);
            assert!(pane.on_client(&click, &mut out).await);
        }
        // The window opens and is shown, with its own address.
        let mut shown = false;
        for _ in 0..20 {
            pump(&mut pane, &mut events, &mut told, &mut out, 250).await;
            if pane.windows.len() == 2 && pane.windows.active() != FIRST {
                shown = true;
                break;
            }
        }
        assert!(shown, "the sign-in window was never shown: {:?}", of_kind(&out, "windows").last());
        let popup_nav = of_kind(&out, "nav").into_iter().rev().find(|n| n["url"].as_str().is_some_and(|u| u.contains("/popup")));
        assert!(popup_nav.is_some(), "the bar never showed the window's own address");

        // It returns its result and closes itself; the pane goes back to the page.
        let mut back = false;
        for _ in 0..40 {
            pump(&mut pane, &mut events, &mut told, &mut out, 250).await;
            if pane.windows.len() == 1 && pane.windows.active() == FIRST {
                back = true;
                break;
            }
        }
        assert!(back, "the pane did not go back to the page");
        let session = pane.windows.get(FIRST).unwrap().session.clone();
        let result = pane.cdp.call("Runtime.evaluate", json!({ "expression": "JSON.stringify(window.result)", "returnByValue": true }), Some(&session)).await.unwrap();
        let seen: Value = serde_json::from_str(result["result"]["value"].as_str().expect("the page's result")).unwrap();
        assert_eq!(seen["opened"], true, "{seen}");
        assert_eq!(seen["messages"], json!([{ "code": "abc" }]), "the window's result reached the page that opened it: {seen}");
        assert_eq!(seen["closed"], true, "the page sees the window closed: {seen}");

        pane.finish().await;
        drop(lease);
    }
}
