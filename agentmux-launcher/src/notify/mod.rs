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
#[cfg(target_os = "linux")]
mod linux;

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
    /// What the agent is working on — a small line under the body
    /// (`SPEC_OS_NOTIFICATIONS_RICH_CONTENT_AND_CLICK_TO_PANE_2026_09_25.md`
    /// §1.3). Absent from an older srv, and when the agent has none.
    #[serde(default)]
    pub summary: Option<String>,
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
    /// Set `app:startatlogin` (the tray's check item; `start_at_login`).
    SetStartAtLogin(bool),
}

/// A platform notification surface.
pub trait Presenter: Send + Sync {
    fn show(&self, n: &Notification);
    fn retract(&self, tag: &str);
    fn clear_all(&self);
}

/// No platform backend (macOS today — needs a signed bundle and a Mac to
/// verify, spec §6.2): log and drop.
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
}

/// The launcher's reducer state — where a clicked toast's window is looked up
/// (backend window id → label → HWND).
pub type LauncherState = Arc<tokio::sync::Mutex<crate::state::State>>;

static HANDLE: OnceLock<Handle> = OnceLock::new();

/// The user-action channel exists from first use, not from `start`: the tray
/// (and its Pause/Resume/Open menu) comes up before srv does, and a pick made
/// in that window must queue until the presenter connects rather than vanish
/// (ReAgent P2 on #3654). `start` takes the receiver.
type ActionChannel = (mpsc::UnboundedSender<UserAction>, std::sync::Mutex<Option<mpsc::UnboundedReceiver<UserAction>>>);
static ACTIONS: OnceLock<ActionChannel> = OnceLock::new();

fn actions() -> &'static ActionChannel {
    ACTIONS.get_or_init(|| {
        let (tx, rx) = mpsc::unbounded_channel();
        (tx, std::sync::Mutex::new(Some(rx)))
    })
}

fn make_presenter(actions: mpsc::UnboundedSender<UserAction>, data_dir: &std::path::Path) -> Arc<dyn Presenter> {
    #[cfg(target_os = "windows")]
    {
        match windows::WindowsPresenter::spawn(actions, data_dir) {
            Ok(p) => return Arc::new(p),
            Err(e) => crate::logging::log(&format!("notify: Windows toast backend unavailable: {e}")),
        }
    }
    #[cfg(target_os = "linux")]
    {
        let _ = data_dir;
        return Arc::new(linux::LinuxPresenter::spawn(actions));
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    let _ = (actions, data_dir);
    #[allow(unreachable_code)]
    Arc::new(NullPresenter)
}

/// Start the presenter. Idempotent: a second call only updates the endpoint.
pub fn start(
    ws_endpoint: &str,
    auth_key: &str,
    data_dir: std::path::PathBuf,
    dir_hash: String,
    state: LauncherState,
) {
    let ep = Endpoint { ws: ws_endpoint.to_string(), auth_key: auth_key.to_string() };
    if let Some(h) = HANDLE.get() {
        let _ = h.endpoint.send(Some(ep));
        return;
    }
    let act_tx = actions().0.clone();
    let Some(act_rx) = actions().1.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        return;
    };
    let presenter = make_presenter(act_tx, &data_dir);
    // A fresh launcher owns nothing on screen yet — anything left over from a
    // previous (crashed) run points at a dead instance.
    presenter.clear_all();
    let (ep_tx, ep_rx) = watch::channel(Some(ep));
    if HANDLE.set(Handle { endpoint: ep_tx, presenter: presenter.clone() }).is_err() {
        return;
    }
    tokio::spawn(run(ep_rx, act_rx, presenter, data_dir, dir_hash, state));
}

/// srv was respawned: reconnect to its new endpoint. The toasts on screen
/// name ids the new srv never issued — clear them rather than leave clicks
/// that can't find their pane (rich-content spec §3.3 F).
pub fn update_endpoint(ws_endpoint: &str, auth_key: &str) {
    if let Some(h) = HANDLE.get() {
        h.presenter.clear_all();
        let _ = h.endpoint.send(Some(Endpoint { ws: ws_endpoint.to_string(), auth_key: auth_key.to_string() }));
    }
}

/// The tray's notification menu (spec §4.2): open an attention item exactly
/// as its toast click would, or pause/resume. Non-blocking; callable from the
/// tray thread.
pub fn tray_request(action: crate::tray::notify_menu::NotifyMenuAction) {
    use crate::tray::notify_menu::{resolve_pause, NotifyMenuAction};
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
    // Queues even before `start` — delivered once the presenter connects.
    let _ = actions().0.send(ua);
}

/// Write `app:startatlogin` through srv, like any settings change, so the
/// Settings page and the OS login entry follow it (`start_at_login`). Queues
/// until the session is up.
pub fn request_start_at_login(on: bool) {
    let _ = actions().0.send(UserAction::SetStartAtLogin(on));
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
    /// A full-config broadcast: the value of `app:startatlogin` (`None` = unset).
    StartAtLogin(Option<bool>),
    Response { reqid: String, ack: AckResponse },
    Other,
}

/// srv's `notify.ack` answer (`NotifyAckResult`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct AckResponse {
    #[serde(default)]
    pub has_window: Option<bool>,
    #[serde(default)]
    pub window_id: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
}

/// What a click makes the launcher do (rich-content spec §3.3 C, D, F).
#[derive(Debug, PartialEq, Eq)]
pub enum ClickPlan {
    /// Raise this open window (backend window id); its frontend selects the
    /// pane from `notification:activate`.
    Raise(String),
    /// The pane's workspace is open in no window: open one on it.
    OpenWorkspace(String),
    /// No window at all (background mode, or the pane is gone): open one.
    OpenWindow,
    Nothing,
}

/// Pure: srv decided the target; this only maps its answer to an action.
pub fn click_plan(ack: &AckResponse) -> ClickPlan {
    if let Some(w) = ack.window_id.as_ref().filter(|w| !w.is_empty()) {
        return ClickPlan::Raise(w.clone());
    }
    if let Some(ws) = ack.workspace_id.as_ref().filter(|w| !w.is_empty()) {
        return ClickPlan::OpenWorkspace(ws.clone());
    }
    if ack.has_window == Some(false) {
        return ClickPlan::OpenWindow;
    }
    ClickPlan::Nothing
}

/// The host window showing backend window `window_id`: `(label, hwnd, iconic)`.
pub fn window_for(state: &crate::state::State, window_id: &str) -> Option<(String, Option<u64>, bool)> {
    let label = state.backend_window_ids.iter().find(|(_, w)| w.as_str() == window_id).map(|(l, _)| l.clone())?;
    let mirror = state.windows.get(&label);
    Some((label, mirror.and_then(|m| m.hwnd), mirror.is_some_and(|m| m.iconic)))
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
            ack: data.get("data").and_then(|d| serde_json::from_value(d.clone()).ok()).unwrap_or_default(),
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
        Some("config") => ev
            .get("data")
            .and_then(|d| d.get("fullconfig"))
            .and_then(|c| c.get("settings"))
            .map(|s| Frame::StartAtLogin(s.get(crate::start_at_login::SETTINGS_KEY).and_then(|v| v.as_bool())))
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
    state: LauncherState,
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
        match session(&ep, &mut ep_rx, &mut act_rx, &presenter, &data_dir, &dir_hash, &state).await {
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
    state: &LauncherState,
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
                    UserAction::SetStartAtLogin(on) => {
                        let msg = rpc("setconfig", Some(&uuid::Uuid::new_v4().to_string()), serde_json::json!({ crate::start_at_login::SETTINGS_KEY: on }));
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
                    Frame::StartAtLogin(v) => crate::start_at_login::observe(v),
                    Frame::Response { reqid, ack } => {
                        if pending_clicks.remove(&reqid) {
                            let plan = click_plan(&ack);
                            crate::logging::log(&format!("notify: click → {plan:?}"));
                            let window = match &plan {
                                ClickPlan::Raise(w) => window_for(&*state.lock().await, w),
                                _ => None,
                            };
                            let (dd, dh) = (data_dir.to_path_buf(), dir_hash.to_string());
                            tokio::task::spawn_blocking(move || carry_out_click(plan, window, &dd, &dh));
                        }
                    }
                    Frame::Other => {}
                }
            }
        }
    }
}

/// Raise first, in this process: the toast click made the launcher the
/// foreground-eligible process, and that right is what a raise needs —
/// waiting for the pane selection to go srv → renderer → host first can
/// lose it (rich-content spec §3.3 C, F4). Every branch logs.
fn carry_out_click(
    plan: ClickPlan,
    window: Option<(String, Option<u64>, bool)>,
    data_dir: &std::path::Path,
    dir_hash: &str,
) {
    use crate::second_instance::{forward_host_cmd_with_args, forward_open_new_window, ForwardError};
    let log_err = |what: &str, r: Result<(), ForwardError>| {
        if let Err(ForwardError::Transient(e) | ForwardError::Fatal(e)) = r {
            crate::logging::log(&format!("notify: {what} for click failed: {e}"));
        }
    };
    match plan {
        ClickPlan::Raise(window_id) => {
            let Some((label, hwnd, iconic)) = window else {
                // The window reported itself to srv but the launcher hasn't
                // mapped it (yet): let the host raise it by its frontend's own
                // request instead — the frontend calls focus_window itself.
                crate::logging::log(&format!("notify: click window {window_id} not in the launcher's mirror"));
                return;
            };
            if hwnd.is_some_and(|h| raise_hwnd(h, iconic)) {
                return;
            }
            crate::logging::log(&format!("notify: raising {label} directly failed; asking the host"));
            log_err("focus_window", forward_host_cmd_with_args(data_dir, dir_hash, "focus_window", serde_json::json!({ "label": label })));
            if let Some(h) = hwnd {
                flash_hwnd(h);
            }
        }
        ClickPlan::OpenWorkspace(workspace_id) => log_err(
            "open_new_window {workspace_id}",
            forward_host_cmd_with_args(data_dir, dir_hash, "open_new_window", serde_json::json!({ "workspace_id": workspace_id })),
        ),
        ClickPlan::OpenWindow => log_err("open_new_window", forward_open_new_window(data_dir, dir_hash)),
        ClickPlan::Nothing => {}
    }
}

/// Restore if minimized, then bring to the foreground. False when Windows
/// refused the foreground change.
#[cfg(target_os = "windows")]
fn raise_hwnd(hwnd: u64, iconic: bool) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE};
    let h = hwnd as usize as windows_sys::Win32::Foundation::HWND;
    unsafe {
        if iconic || IsIconic(h) != 0 {
            ShowWindow(h, SW_RESTORE);
        }
        SetForegroundWindow(h) != 0
    }
}

#[cfg(not(target_os = "windows"))]
fn raise_hwnd(_hwnd: u64, _iconic: bool) -> bool {
    false
}

/// Windows' own fallback when the foreground change is refused: flash the
/// taskbar button so the user at least sees where to go.
#[cfg(target_os = "windows")]
fn flash_hwnd(hwnd: u64) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{FlashWindowEx, FLASHWINFO, FLASHW_ALL, FLASHW_TIMERNOFG};
    let info = FLASHWINFO {
        cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
        hwnd: hwnd as usize as windows_sys::Win32::Foundation::HWND,
        dwFlags: FLASHW_ALL | FLASHW_TIMERNOFG,
        uCount: 3,
        dwTimeout: 0,
    };
    unsafe {
        FlashWindowEx(&info);
    }
}

#[cfg(not(target_os = "windows"))]
fn flash_hwnd(_hwnd: u64) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_plan_follows_srvs_answer() {
        let ack = |window: Option<&str>, ws: Option<&str>, has: Option<bool>| AckResponse {
            has_window: has,
            window_id: window.map(Into::into),
            workspace_id: ws.map(Into::into),
        };
        assert_eq!(click_plan(&ack(Some("w1"), None, Some(true))), ClickPlan::Raise("w1".into()));
        assert_eq!(click_plan(&ack(None, Some("ws1"), Some(false))), ClickPlan::OpenWorkspace("ws1".into()));
        assert_eq!(click_plan(&ack(None, None, Some(false))), ClickPlan::OpenWindow);
        // An older srv: only has_window.
        assert_eq!(click_plan(&ack(None, None, Some(true))), ClickPlan::Nothing);
        assert_eq!(click_plan(&ack(Some(""), Some(""), None)), ClickPlan::Nothing);
    }

    #[test]
    fn window_for_maps_backend_id_to_label_hwnd_and_minimized() {
        let mut s = crate::state::State::default();
        s.backend_window_ids.insert("main".into(), "wid-1".into());
        assert_eq!(window_for(&s, "wid-1"), Some(("main".into(), None, false)), "label known before its HWND");
        assert_eq!(window_for(&s, "wid-2"), None);
    }

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
        assert_eq!(
            parse_frame(resp),
            Frame::Response { reqid: "q1".into(), ack: AckResponse { has_window: Some(false), ..Default::default() } }
        );
        let resp = r#"{"eventtype":"rpc","data":{"resid":"q2","data":{"has_window":true,"known":true,"window_id":"w1"}}}"#;
        assert_eq!(
            parse_frame(resp),
            Frame::Response {
                reqid: "q2".into(),
                ack: AckResponse { has_window: Some(true), window_id: Some("w1".into()), workspace_id: None }
            }
        );
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
        let with = |settings: &str| {
            format!(r#"{{"eventtype":"rpc","data":{{"command":"eventrecv","data":{{"event":"config","data":{{"fullconfig":{{"settings":{settings}}}}}}}}}}}"#)
        };
        assert_eq!(parse_frame(&with(r#"{"app:startatlogin":true}"#)), Frame::StartAtLogin(Some(true)));
        assert_eq!(parse_frame(&with(r#"{"app:startatlogin":false}"#)), Frame::StartAtLogin(Some(false)));
        assert_eq!(parse_frame(&with(r#"{"term:fontsize":12}"#)), Frame::StartAtLogin(None));
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
        let state: LauncherState = Default::default();
        tokio::spawn(run(ep_rx, act_rx, presenter, dir, "h".into(), state));

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
