// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! OS notification **Presenter** — `docs/specs/SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md` §3.2, §6.
//!
//! The srv Router decides *whether* to notify and publishes `notification` /
//! `notification:retract` on its event bus. This module is the launcher's
//! subscriber: it keeps a WebSocket to srv (the launcher's first), renders
//! each notification with the platform backend, and reports the user's
//! click/dismiss back as `notify.ack`.
//!
//! Why the launcher (spec §3.2): it outlives host restarts, stays up with no
//! window in background mode, owns the tray, and — on macOS — is the bundle's
//! main executable. Click callbacks need a live process; this is the
//! longest-lived one.
//!
//! ## Lifecycle
//!
//! `start` once srv is up; `update_endpoint` whenever srv is respawned (new
//! port + auth key); `shutdown` before the launcher exits, which clears every
//! toast this app put in the notification center — a click on a toast from a
//! dead instance could do nothing useful (Phase 1 has no cold-start
//! activation, spec §6.1).

use std::sync::{Arc, OnceLock};

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::{ClientRequestBuilder, Message};

#[cfg(target_os = "windows")]
mod windows;

/// Mirrors srv's `OsNotification` (`agentmux-srv/src/backend/notify/policy.rs`).
/// Unknown fields are ignored so srv can grow the payload first.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Notification {
    pub id: String,
    pub kind: String,
    pub priority: String,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    pub tag: String,
}

impl Notification {
    pub fn is_attention(&self) -> bool {
        self.priority == "attention"
    }
}

/// What the user did — on a toast (reported by a backend) or in the tray's
/// notification menu (`tray_request`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserAction {
    Clicked(String),
    Dismissed(String),
    /// Set `notify:pause:until` (epoch ms; 0 = resume).
    SetPause(i64),
}

/// A platform notification surface.
pub trait Presenter: Send + Sync {
    fn show(&self, n: &Notification);
    fn retract(&self, tag: &str);
    fn clear_all(&self);
}

/// No platform backend yet (macOS / Linux are Phase 3): log and drop.
struct NullPresenter;
impl Presenter for NullPresenter {
    fn show(&self, n: &Notification) {
        crate::logging::log(&format!("notify: no OS backend on this platform; dropping '{}'", n.kind));
    }
    fn retract(&self, _tag: &str) {}
    fn clear_all(&self) {}
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Endpoint {
    ws: String,
    auth_key: String,
}

struct Handle {
    endpoint: watch::Sender<Option<Endpoint>>,
    presenter: Arc<dyn Presenter>,
    actions: mpsc::UnboundedSender<UserAction>,
}

static HANDLE: OnceLock<Handle> = OnceLock::new();

fn make_presenter(actions: mpsc::UnboundedSender<UserAction>, data_dir: &std::path::Path) -> Arc<dyn Presenter> {
    #[cfg(target_os = "windows")]
    {
        match windows::WindowsPresenter::spawn(actions, data_dir) {
            Ok(p) => return Arc::new(p),
            Err(e) => crate::logging::log(&format!("notify: Windows toast backend unavailable: {e}")),
        }
    }
    #[cfg(not(target_os = "windows"))]
    let _ = (actions, data_dir);
    Arc::new(NullPresenter)
}

/// Start the presenter. Idempotent: a second call only updates the endpoint.
pub fn start(ws_endpoint: &str, auth_key: &str, data_dir: std::path::PathBuf, dir_hash: String) {
    let ep = Endpoint { ws: ws_endpoint.to_string(), auth_key: auth_key.to_string() };
    if let Some(h) = HANDLE.get() {
        let _ = h.endpoint.send(Some(ep));
        return;
    }
    let (act_tx, act_rx) = mpsc::unbounded_channel();
    let presenter = make_presenter(act_tx.clone(), &data_dir);
    // A fresh launcher owns nothing on screen yet — anything left over from a
    // previous (crashed) run points at a dead instance.
    presenter.clear_all();
    let (ep_tx, ep_rx) = watch::channel(Some(ep));
    if HANDLE.set(Handle { endpoint: ep_tx, presenter: presenter.clone(), actions: act_tx }).is_err() {
        return;
    }
    tokio::spawn(run(ep_rx, act_rx, presenter, data_dir, dir_hash));
}

/// srv was respawned: reconnect to its new endpoint.
pub fn update_endpoint(ws_endpoint: &str, auth_key: &str) {
    if let Some(h) = HANDLE.get() {
        let _ = h.endpoint.send(Some(Endpoint { ws: ws_endpoint.to_string(), auth_key: auth_key.to_string() }));
    }
}

/// The tray's notification menu (spec §4.2): open an attention item exactly
/// as its toast click would, or pause/resume. Non-blocking; callable from the
/// tray thread.
pub fn tray_request(action: crate::tray::notify_menu::NotifyMenuAction) {
    use crate::tray::notify_menu::{resolve_pause, NotifyMenuAction};
    let Some(h) = HANDLE.get() else { return };
    let ua = match action {
        NotifyMenuAction::Open(id) => {
            // The menu pick made us foreground-eligible; hand that to the host
            // so it can raise its window.
            #[cfg(target_os = "windows")]
            unsafe {
                let _ = ::windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(
                    ::windows::Win32::UI::WindowsAndMessaging::ASFW_ANY,
                );
            }
            UserAction::Clicked(id)
        }
        NotifyMenuAction::Pause(choice) => UserAction::SetPause(resolve_pause(choice, chrono::Local::now())),
        NotifyMenuAction::Resume => UserAction::SetPause(0),
    };
    let _ = h.actions.send(ua);
}

/// Launcher is exiting: remove our toasts from the notification center.
pub fn shutdown() {
    if let Some(h) = HANDLE.get() {
        h.presenter.clear_all();
    }
}

/// `host:port` (srv's ESTART form) or a full URL → the `/ws` URL.
pub fn ws_url(endpoint: &str) -> String {
    let base = endpoint.trim_end_matches('/');
    let base = if base.starts_with("ws://") || base.starts_with("wss://") {
        base.to_string()
    } else {
        format!("ws://{base}")
    };
    format!("{base}/ws")
}

/// What an incoming srv frame means to the presenter.
#[derive(Debug, PartialEq, Eq)]
pub enum Frame {
    Show(Notification),
    Retract(String),
    State(crate::tray::notify_menu::NotifyTrayState),
    Response { reqid: String, has_window: Option<bool> },
    Other,
}

/// Parse one srv WS text frame. Events arrive as
/// `{"eventtype":"rpc","data":{"command":"eventrecv","data":<MuxEvent>}}`;
/// RPC responses as `{"eventtype":"rpc","data":{"resid":..,"data":..}}`.
pub fn parse_frame(text: &str) -> Frame {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return Frame::Other;
    };
    let Some(data) = v.get("data") else { return Frame::Other };
    if let Some(resid) = data.get("resid").and_then(|r| r.as_str()) {
        return Frame::Response {
            reqid: resid.to_string(),
            has_window: data.get("data").and_then(|d| d.get("has_window")).and_then(|b| b.as_bool()),
        };
    }
    if data.get("command").and_then(|c| c.as_str()) != Some("eventrecv") {
        return Frame::Other;
    }
    let Some(ev) = data.get("data") else { return Frame::Other };
    match ev.get("event").and_then(|e| e.as_str()) {
        Some("notification") => ev
            .get("data")
            .and_then(|d| serde_json::from_value::<Notification>(d.clone()).ok())
            .map(Frame::Show)
            .unwrap_or(Frame::Other),
        Some("notification:state") => ev
            .get("data")
            .and_then(|d| serde_json::from_value(d.clone()).ok())
            .map(Frame::State)
            .unwrap_or(Frame::Other),
        Some("notification:retract") => ev
            .get("data")
            .and_then(|d| d.get("tag"))
            .and_then(|t| t.as_str())
            .map(|t| Frame::Retract(t.to_string()))
            .unwrap_or(Frame::Other),
        _ => Frame::Other,
    }
}

fn rpc(command: &str, reqid: Option<&str>, data: serde_json::Value) -> String {
    let mut msg = serde_json::json!({ "command": command, "data": data });
    if let Some(r) = reqid {
        msg["reqid"] = serde_json::Value::String(r.to_string());
    }
    serde_json::json!({ "wscommand": "rpc", "message": msg }).to_string()
}

async fn run(
    mut ep_rx: watch::Receiver<Option<Endpoint>>,
    mut act_rx: mpsc::UnboundedReceiver<UserAction>,
    presenter: Arc<dyn Presenter>,
    data_dir: std::path::PathBuf,
    dir_hash: String,
) {
    let mut backoff_ms = 500u64;
    loop {
        let ep = ep_rx.borrow_and_update().clone();
        let Some(ep) = ep else {
            if ep_rx.changed().await.is_err() {
                return;
            }
            continue;
        };
        match session(&ep, &mut ep_rx, &mut act_rx, &presenter, &data_dir, &dir_hash).await {
            SessionEnd::EndpointChanged => {
                backoff_ms = 500;
                continue;
            }
            SessionEnd::Closed(reason) => {
                crate::logging::log(&format!("notify: srv connection ended ({reason}); retrying in {backoff_ms}ms"));
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)) => {}
            r = ep_rx.changed() => if r.is_err() { return; },
        }
        backoff_ms = (backoff_ms * 2).min(30_000);
    }
}

enum SessionEnd {
    EndpointChanged,
    Closed(String),
}

async fn session(
    ep: &Endpoint,
    ep_rx: &mut watch::Receiver<Option<Endpoint>>,
    act_rx: &mut mpsc::UnboundedReceiver<UserAction>,
    presenter: &Arc<dyn Presenter>,
    data_dir: &std::path::Path,
    dir_hash: &str,
) -> SessionEnd {
    let uri: tokio_tungstenite::tungstenite::http::Uri = match ws_url(&ep.ws).parse() {
        Ok(u) => u,
        Err(e) => return SessionEnd::Closed(format!("bad endpoint: {e}")),
    };
    // Header auth (`X-AuthKey`), as srv's auth_middleware accepts for /ws.
    // ClientRequestBuilder generates the handshake headers itself — see
    // cloud_subscriber.rs for why a hand-built Request fails the handshake.
    let request = ClientRequestBuilder::new(uri).with_header("X-AuthKey", ep.auth_key.clone());
    let (ws, _) = match tokio_tungstenite::connect_async(request).await {
        Ok(s) => s,
        Err(e) => return SessionEnd::Closed(format!("connect: {e}")),
    };
    let (mut write, mut read) = ws.split();
    for event in ["notification", "notification:retract", "notification:state"] {
        let sub = rpc("eventsub", None, serde_json::json!({ "event": event, "scopes": [], "allscopes": true }));
        if let Err(e) = write.send(Message::Text(sub.into())).await {
            return SessionEnd::Closed(format!("subscribe: {e}"));
        }
    }
    crate::logging::log("notify: subscribed to srv notifications");

    // reqid → was it a click (needs has_window handling)?
    let mut pending_clicks: std::collections::HashSet<String> = Default::default();

    loop {
        tokio::select! {
            changed = ep_rx.changed() => {
                if changed.is_err() || ep_rx.borrow().as_ref() != Some(ep) {
                    return SessionEnd::EndpointChanged;
                }
            }
            action = act_rx.recv() => {
                let Some(action) = action else { return SessionEnd::Closed("presenter gone".into()) };
                let (id, clicked) = match &action {
                    UserAction::Clicked(id) => (id.clone(), true),
                    UserAction::Dismissed(id) => (id.clone(), false),
                    UserAction::SetPause(until) => {
                        let msg = rpc("setconfig", Some(&uuid::Uuid::new_v4().to_string()), serde_json::json!({ "notify:pause:until": until }));
                        if let Err(e) = write.send(Message::Text(msg.into())).await {
                            return SessionEnd::Closed(format!("send setconfig: {e}"));
                        }
                        continue;
                    }
                };
                let reqid = uuid::Uuid::new_v4().to_string();
                if clicked {
                    pending_clicks.insert(reqid.clone());
                }
                let msg = rpc("notify.ack", Some(&reqid), serde_json::json!({ "id": id, "clicked": clicked }));
                if let Err(e) = write.send(Message::Text(msg.into())).await {
                    return SessionEnd::Closed(format!("send ack: {e}"));
                }
            }
            msg = read.next() => {
                let text = match msg {
                    Some(Ok(Message::Text(t))) => t.to_string(),
                    Some(Ok(Message::Close(_))) | None => return SessionEnd::Closed("closed by srv".into()),
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => return SessionEnd::Closed(format!("read: {e}")),
                };
                match parse_frame(&text) {
                    Frame::Show(n) => presenter.show(&n),
                    Frame::Retract(tag) => presenter.retract(&tag),
                    Frame::State(state) => crate::tray::notify_menu::set(state),
                    Frame::Response { reqid, has_window } => {
                        if pending_clicks.remove(&reqid) && has_window == Some(false) {
                            // Clicked with no window open (background mode):
                            // open one; its frontend picks the target up via
                            // notify.takeactivation.
                            let (dd, dh) = (data_dir.to_path_buf(), dir_hash.to_string());
                            tokio::task::spawn_blocking(move || {
                                use crate::second_instance::ForwardError;
                                match crate::second_instance::forward_open_new_window(&dd, &dh) {
                                    Ok(()) => {}
                                    Err(ForwardError::Transient(r)) | Err(ForwardError::Fatal(r)) => {
                                        crate::logging::log(&format!("notify: open_new_window for click failed: {r}"))
                                    }
                                }
                            });
                        }
                    }
                    Frame::Other => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_url_forms() {
        assert_eq!(ws_url("127.0.0.1:5000"), "ws://127.0.0.1:5000/ws");
        assert_eq!(ws_url("ws://127.0.0.1:5000/"), "ws://127.0.0.1:5000/ws");
    }

    #[test]
    fn parses_notification_event() {
        let f = r#"{"eventtype":"rpc","data":{"command":"eventrecv","data":{"event":"notification","data":{
            "id":"n1-1","kind":"input_waiting","priority":"attention","block_id":"b1","agent_name":"lark",
            "title":"lark needs your input","body":"Which branch?","tag":"abcdef0123456789","created_at_ms":1}}}}"#;
        match parse_frame(f) {
            Frame::Show(n) => {
                assert_eq!(n.id, "n1-1");
                assert!(n.is_attention());
                assert_eq!(n.body.as_deref(), Some("Which branch?"));
                assert_eq!(n.tag, "abcdef0123456789");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_retract_and_response_and_ignores_others() {
        let r = r#"{"eventtype":"rpc","data":{"command":"eventrecv","data":{"event":"notification:retract","data":{"id":"n1","tag":"t1"}}}}"#;
        assert_eq!(parse_frame(r), Frame::Retract("t1".into()));
        let resp = r#"{"eventtype":"rpc","data":{"resid":"q1","data":{"has_window":false}}}"#;
        assert_eq!(parse_frame(resp), Frame::Response { reqid: "q1".into(), has_window: Some(false) });
        let st = r#"{"eventtype":"rpc","data":{"command":"eventrecv","data":{"event":"notification:state","data":{"attention":[{"id":"n1","block_id":"b1","title":"lark needs your input"}],"paused_until_ms":5}}}}"#;
        match parse_frame(st) {
            Frame::State(s) => {
                assert_eq!(s.attention.len(), 1);
                assert_eq!(s.paused_until_ms, 5);
            }
            other => panic!("unexpected {other:?}"),
        }
        let cfg = r#"{"eventtype":"rpc","data":{"command":"eventrecv","data":{"event":"config","data":{}}}}"#;
        assert_eq!(parse_frame(cfg), Frame::Other);
        assert_eq!(parse_frame(r#"{"type":"ping","stime":1}"#), Frame::Other);
        assert_eq!(parse_frame("garbage"), Frame::Other);
    }

    /// End-to-end over a real socket: a fake srv checks the auth header and
    /// subscriptions, pushes a notification + retract, and receives the ack
    /// the presenter's click produces.
    #[tokio::test]
    async fn session_round_trip_against_fake_srv() {
        use std::sync::Mutex;
        use tokio::net::TcpListener;
        use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

        #[derive(Default)]
        struct Fake {
            calls: Mutex<Vec<String>>,
        }
        impl Presenter for Fake {
            fn show(&self, n: &Notification) {
                self.calls.lock().unwrap().push(format!("show:{}", n.id));
            }
            fn retract(&self, tag: &str) {
                self.calls.lock().unwrap().push(format!("retract:{tag}"));
            }
            fn clear_all(&self) {}
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (got_tx, mut got_rx) = mpsc::unbounded_channel::<serde_json::Value>();
        tokio::spawn(async move {
            let (sock, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_hdr_async(sock, |req: &Request, resp: Response| {
                assert_eq!(req.uri().path(), "/ws");
                assert_eq!(req.headers().get("X-AuthKey").unwrap(), "k1");
                Ok(resp)
            })
            .await
            .unwrap();
            // Three eventsubs first.
            for _ in 0..3 {
                let m = ws.next().await.unwrap().unwrap();
                got_tx.send(serde_json::from_str(m.to_text().unwrap()).unwrap()).unwrap();
            }
            let show = r#"{"eventtype":"rpc","data":{"command":"eventrecv","data":{"event":"notification","data":{"id":"n1","kind":"turn_completed","priority":"normal","title":"lark finished","body":null,"tag":"t1"}}}}"#;
            ws.send(Message::Text(show.into())).await.unwrap();
            let retract = r#"{"eventtype":"rpc","data":{"command":"eventrecv","data":{"event":"notification:retract","data":{"id":"n1","tag":"t1"}}}}"#;
            ws.send(Message::Text(retract.into())).await.unwrap();
            // The ack produced by the simulated click.
            let m = ws.next().await.unwrap().unwrap();
            got_tx.send(serde_json::from_str(m.to_text().unwrap()).unwrap()).unwrap();
        });

        let fake = Arc::new(Fake::default());
        let presenter: Arc<dyn Presenter> = fake.clone();
        let (_ep_tx, ep_rx) = watch::channel(Some(Endpoint { ws: addr.to_string(), auth_key: "k1".into() }));
        let (act_tx, act_rx) = mpsc::unbounded_channel();
        let dir = std::env::temp_dir();
        tokio::spawn(run(ep_rx, act_rx, presenter, dir, "h".into()));

        let sub1 = got_rx.recv().await.unwrap();
        let sub2 = got_rx.recv().await.unwrap();
        let sub3 = got_rx.recv().await.unwrap();
        assert_eq!(sub3["message"]["data"]["event"], "notification:state");
        assert_eq!(sub1["message"]["command"], "eventsub");
        assert_eq!(sub1["message"]["data"]["event"], "notification");
        assert_eq!(sub2["message"]["data"]["event"], "notification:retract");

        // Wait until show + retract reached the presenter, then click.
        for _ in 0..100 {
            if fake.calls.lock().unwrap().len() >= 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(*fake.calls.lock().unwrap(), vec!["show:n1".to_string(), "retract:t1".to_string()]);
        act_tx.send(UserAction::Clicked("n1".into())).unwrap();
        let ack = got_rx.recv().await.unwrap();
        assert_eq!(ack["message"]["command"], "notify.ack");
        assert_eq!(ack["message"]["data"]["id"], "n1");
        assert_eq!(ack["message"]["data"]["clicked"], true);
        assert!(ack["message"]["reqid"].is_string());
    }

    #[test]
    fn rpc_envelope_shape() {
        let v: serde_json::Value = serde_json::from_str(&rpc("notify.ack", Some("r1"), serde_json::json!({"id":"n1","clicked":true}))).unwrap();
        assert_eq!(v["wscommand"], "rpc");
        assert_eq!(v["message"]["command"], "notify.ack");
        assert_eq!(v["message"]["reqid"], "r1");
        assert_eq!(v["message"]["data"]["clicked"], true);
    }
}
