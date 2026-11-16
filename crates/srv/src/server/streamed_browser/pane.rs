// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One streamed pane: a page (a Chromium target) behind one WebSocket. Frames go out as JPEG
//! and are acknowledged only once handed to the socket; input comes in and goes to the page.
//! docs/specs/SPEC_STREAMED_BROWSER_PANE_DEMO_2026_11_15.md §4 to §6.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::ws::{Message, WebSocket};
use base64::Engine as _;
use serde_json::{json, Value};

use super::cdp::{Cdp, Event};
use super::chromium::Lease;
use super::nav;
use super::protocol::{self, ClientMsg};

/// Kind byte of a binary message: a JPEG frame.
const FRAME: u8 = 1;
/// The quality of the screencast (report section 6: SSIM 0.998 at 141 kB for a full frame).
const JPEG_QUALITY: u32 = 70;

pub struct Start {
    pub url: String,
    pub w: u32,
    pub h: u32,
}

/// A page and its session, closed when dropped by `close`.
struct Page {
    cdp: Arc<Cdp>,
    target: String,
    session: String,
    loading: bool,
    /// When the last resize was asked for, to say how long the new frame took.
    resized_at: Option<Instant>,
}

fn screencast(w: u32, h: u32) -> Value {
    json!({ "format": "jpeg", "quality": JPEG_QUALITY, "maxWidth": w, "maxHeight": h, "everyNthFrame": 1 })
}

fn viewport(w: u32, h: u32) -> Value {
    json!({ "width": w, "height": h, "deviceScaleFactor": 1, "mobile": false })
}

/// The page made for a pane: a target, its session and its events. Closed again if it cannot be made ready.
async fn open(cdp: &Arc<Cdp>, start: &Start) -> Result<(Page, tokio::sync::mpsc::UnboundedReceiver<Event>), String> {
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
    let events = cdp.subscribe(&session);
    let page = Page { cdp: cdp.clone(), target, session, loading: false, resized_at: None };
    match prepare(&page, start).await {
        Ok(()) => Ok((page, events)),
        Err(e) => {
            page.close().await;
            Err(e)
        }
    }
}

/// Switch the page on: events, the viewport, focus, the screencast and its first address.
async fn prepare(page: &Page, start: &Start) -> Result<(), String> {
    let (cdp, s) = (&page.cdp, Some(page.session.as_str()));
    cdp.call("Page.enable", json!({}), s).await?;
    cdp.call("Emulation.setDeviceMetricsOverride", viewport(start.w, start.h), s).await?;
    // The page counts as focused, so its carets and :focus styles show.
    cdp.call("Emulation.setFocusEmulationEnabled", json!({ "enabled": true }), s).await?;
    // A page's file chooser is intercepted and never answered: nothing on the server's disks is offered to a page.
    cdp.call("Page.setInterceptFileChooserDialog", json!({ "enabled": true }), s).await?;
    cdp.call("Page.startScreencast", screencast(start.w, start.h), s).await?;
    if start.url != "about:blank" {
        nav::go(cdp, &page.session, protocol::Go::Url, &start.url).await?;
    }
    Ok(())
}

impl Page {
    async fn close(self) {
        self.cdp.unsubscribe(&self.session);
        let _ = self.cdp.call("Target.closeTarget", json!({ "targetId": self.target }), None).await;
    }

    /// Resize the page and make the browser send a frame of the new size even when the page does not
    /// change: stopping and starting the screencast does that.
    async fn resize(&mut self, w: u32, h: u32) -> Result<(), String> {
        let s = Some(self.session.as_str());
        self.cdp.call("Emulation.setDeviceMetricsOverride", viewport(w, h), s).await?;
        self.cdp.call("Page.stopScreencast", json!({}), s).await?;
        self.cdp.call("Page.startScreencast", screencast(w, h), s).await?;
        self.resized_at = Some(Instant::now());
        Ok(())
    }

    fn input(&self, msg: &ClientMsg) -> bool {
        match protocol::input_call(msg) {
            Some((method, params)) => {
                self.cdp.send(method, params, Some(&self.session));
                true
            }
            None => false,
        }
    }
}

async fn say(socket: &mut WebSocket, value: Value) -> bool {
    socket.send(Message::Text(value.to_string().into())).await.is_ok()
}

async fn nav_update(page: &Page, socket: &mut WebSocket) -> bool {
    let h = nav::history(&page.cdp, &page.session).await;
    say(socket, nav::nav_message(&h, page.loading)).await
}

/// A screencast frame to the socket, then its acknowledgement.
async fn forward_frame(page: &mut Page, socket: &mut WebSocket, params: &Value) -> bool {
    let Some(data) = params.get("data").and_then(Value::as_str) else { return true };
    let Ok(jpeg) = base64::engine::general_purpose::STANDARD.decode(data) else { return true };
    let mut bytes = Vec::with_capacity(jpeg.len() + 1);
    bytes.push(FRAME);
    bytes.extend_from_slice(&jpeg);
    if socket.send(Message::Binary(bytes.into())).await.is_err() {
        return false;
    }
    if let Some(at) = page.resized_at.take() {
        tracing::debug!(ms = at.elapsed().as_millis() as u64, "streamed browser: first frame after a resize");
    }
    // Acknowledged only now, so a slow client slows the page's own capture instead of queueing frames.
    if let Some(id) = params.get("sessionId").and_then(Value::as_i64) {
        page.cdp.send("Page.screencastFrameAck", json!({ "sessionId": id }), Some(&page.session));
    }
    true
}

/// What a page event means for the pane. `false` ends the pane.
async fn on_event(page: &mut Page, socket: &mut WebSocket, event: Event) -> bool {
    match event.method.as_str() {
        "Page.screencastFrame" => forward_frame(page, socket, &event.params).await,
        "Page.frameStartedLoading" => {
            page.loading = true;
            nav_update(page, socket).await
        }
        "Page.frameStoppedLoading" => {
            page.loading = false;
            nav_update(page, socket).await
        }
        "Page.frameNavigated" if event.params.pointer("/frame/parentId").is_none() => nav_update(page, socket).await,
        // A dialog the page raised is dismissed: the demo has nowhere to show it.
        "Page.javascriptDialogOpening" => {
            page.cdp.send("Page.handleJavaScriptDialog", json!({ "accept": false }), Some(&page.session));
            true
        }
        "Inspector.targetCrashed" => {
            say(socket, json!({ "t": "error", "message": "the page crashed" })).await;
            false
        }
        _ => true,
    }
}

/// What a message from the pane means. `false` ends the pane.
async fn on_client(page: &mut Page, socket: &mut WebSocket, text: &str) -> bool {
    let msg = match protocol::parse(text) {
        Ok(m) => m,
        Err(e) => return say(socket, json!({ "t": "error", "message": e })).await,
    };
    if page.input(&msg) {
        return true;
    }
    match msg {
        ClientMsg::Resize { w, h } => match page.resize(w, h).await {
            Ok(()) => say(socket, json!({ "t": "size", "w": w, "h": h })).await,
            Err(e) => say(socket, json!({ "t": "error", "message": e })).await,
        },
        ClientMsg::Nav { go, url } => match nav::go(&page.cdp, &page.session, go, &url).await {
            Ok(()) => true,
            Err(e) => say(socket, json!({ "t": "error", "message": e })).await,
        },
        _ => true,
    }
}

/// Run a pane until its socket closes, its page ends, or the sign-in behind it does.
pub async fn run(mut socket: WebSocket, lease: Lease, start: Start, ended: impl std::future::Future<Output = ()>) {
    let (mut page, mut events) = match open(&lease.cdp, &start).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "streamed browser: a pane could not be opened");
            say(&mut socket, json!({ "t": "error", "message": e })).await;
            return;
        }
    };
    say(&mut socket, json!({ "t": "size", "w": start.w, "h": start.h })).await;
    tokio::pin!(ended);
    loop {
        let keep = tokio::select! {
            _ = &mut ended => false,
            event = events.recv() => match event {
                Some(e) => on_event(&mut page, &mut socket, e).await,
                None => false,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(t))) => on_client(&mut page, &mut socket, t.as_str()).await,
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => false,
                Some(Ok(_)) => true,
            },
        };
        if !keep {
            break;
        }
    }
    page.close().await;
    drop(lease);
}
