// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! RPC surface of the notification Router —
//! `docs/specs/SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md` §3.3.
//!
//! - `notify.emit`   renderer → router: a pane event the renderer is the
//!                   authority for (turn outcome, waiting-for-input).
//! - `notify.focus`  renderer → router: this window's focus state.
//! - `notify.ack`    presenter → router: user clicked / dismissed a toast.
//! - `notify.test`   Settings "Send test notification".
//! - `notify.takeactivation` new window → router: a click that arrived while
//!                   no window was open.
//! - `block.reveal`  any renderer → router: reveal a block that lives in
//!                   ANOTHER window. Same target resolution as a toast click,
//!                   so it lives with the Router, which knows which windows
//!                   are open (`SPEC_REVEAL_BLOCK_ONE_PATH_2026_09_27.md` §4.3).
//!
//! `conn_id` is captured per connection so focus reports are keyed by window
//! and dropped on disconnect (see `websocket.rs`'s disconnect path).

use std::sync::Arc;

use crate::backend::notify::policy::{Family, FocusReport, NotifyKind};
use crate::backend::notify::router;
use crate::backend::rpc::engine::WshRpcEngine;

use super::AppState;

/// Pane-level events the renderer reports. `question` accompanies
/// `input_waiting` only; the router redacts it (§9.2).
#[derive(serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum NotifyPaneEvent {
    /// Ignored since srv's controller status became the source of truth for
    /// turns (the renderer's reducer can report turn-ended mid-turn). Kept so
    /// an older frontend's calls still deserialize.
    TurnStarted,
    /// Ignored — see `TurnStarted`.
    TurnCompleted,
    /// Ignored — see `TurnStarted`. Real failures arrive as `agentfailure`.
    TurnErrored,
    /// The user stopped/interrupted the turn in the pane.
    TurnStopped,
    InputWaiting,
    InputResolved,
}

#[derive(serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct NotifyEmitParams {
    pub block_id: String,
    pub event: NotifyPaneEvent,
    #[ts(optional)]
    pub question: Option<String>,
    /// How many questions the pending call asks; > 1 adds " (+N more)".
    #[ts(optional)]
    pub question_count: Option<u32>,
}

#[derive(serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct NotifyFocusParams {
    pub window_focused: bool,
    #[ts(optional)]
    pub block_id: Option<String>,
    /// This window's own `Window` oid, so srv knows which windows are open.
    #[ts(optional)]
    pub window_id: Option<String>,
}

#[derive(serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct NotifyAckParams {
    pub id: String,
    pub clicked: bool,
}

#[derive(serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct NotifyAckResult {
    /// False when the click's window isn't open — an older launcher opens a
    /// window; the new one picks the pane up via `notify.takeactivation`.
    pub has_window: bool,
    /// The acked id named a live notification.
    #[serde(default)]
    pub known: bool,
    /// Raise this window (a connected frontend's `Window` oid).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub window_id: Option<String>,
    /// No window shows the pane: open one on this workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub workspace_id: Option<String>,
}

#[derive(serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct NotifyTakeActivationParams {
    /// The asking window's workspace; only a click for that workspace is
    /// handed over.
    #[serde(default)]
    #[ts(optional)]
    pub workspace_id: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct NotifyTakeActivationResult {
    #[ts(optional)]
    pub block_id: Option<String>,
    #[ts(optional)]
    pub tab_id: Option<String>,
}

#[derive(serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct NotifyNoArgs {}

#[derive(serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct NotifyOk {
    pub ok: bool,
}

#[derive(serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct BlockRevealParams {
    pub block_id: String,
}

#[derive(serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct BlockRevealResult {
    /// The block exists and its workspace is open in a window, which was
    /// told to reveal it (`block:reveal`).
    pub found: bool,
    /// That window (a connected frontend's `Window` oid) — the caller raises it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub window_id: Option<String>,
}

fn router_for(state: &AppState) -> Arc<router::Router> {
    router::init(state.broker.clone(), state.config_watcher.clone(), state.mstore.clone(), state.reactive_handler)
}

/// `block.reveal`: resolve the block's tab and open window like a toast
/// click, make the block its pane's visible tab in the STORED layout (so a
/// window that loads the tab fresh already shows it), then tell only that
/// window's renderer to reveal it (`block:reveal`). A workspace open in no
/// window is out of scope: `found: false`, nothing changed.
pub(crate) async fn reveal_block(
    state: &AppState,
    r: &Arc<router::Router>,
    block_id: String,
) -> Result<BlockRevealResult, String> {
    if block_id.is_empty() {
        return Err("block.reveal: block_id required".to_string());
    }
    let rr = r.clone();
    let target = tokio::task::spawn_blocking(move || rr.reveal_target(&block_id))
        .await
        .map_err(|e| format!("block.reveal: {e}"))?;
    let Some(target) = target else {
        return Ok(BlockRevealResult { found: false, window_id: None });
    };
    activate_in_stored_layout(state, &target.tab_id, &target.block_id).await;
    r.publish_block_reveal(&target);
    Ok(BlockRevealResult { found: true, window_id: Some(target.window_id) })
}

/// `LayoutStackActivate` through the reducer, then mstore — the same
/// dispatch → apply → publish shape as `pane.moveTab`. Best-effort: the
/// renderer switches the pane itself on `block:reveal`, so a reducer that
/// can't apply it (no tree for the tab yet) only costs the head start.
async fn activate_in_stored_layout(state: &AppState, tab_id: &str, block_id: &str) {
    let events = crate::server::service::dispatch_to_reducer(
        state,
        agentmux_common::ipc::Command::LayoutStackActivate {
            tab_id: tab_id.to_string(),
            block_id: block_id.to_string(),
            correlation_id: String::new(),
        },
    )
    .await;
    if let Some(msg) = events.iter().find_map(|e| match e {
        agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    }) {
        tracing::debug!(block_id, "block.reveal: stored layout not updated: {msg}");
        return;
    }
    for ev in &events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore) {
            tracing::warn!(block_id, "block.reveal: mstore apply failed: {e}");
        }
    }
    crate::server::service::publish_events(state, &events);
}

pub fn register_notify_handlers(engine: &Arc<WshRpcEngine>, state: &AppState, conn_id: String) {
    let r = router_for(state);

    let rr = r.clone();
    engine.register_typed("notify.emit", move |p: NotifyEmitParams, _ctx| {
        let r = rr.clone();
        async move {
            if p.block_id.is_empty() {
                return Err("notify.emit: block_id required".to_string());
            }
            // Everything goes through the Router's single ordered queue, so a
            // resolve can never overtake the emit it cancels — whichever
            // source (renderer or srv) sent either one (Codex P2 on #3662).
            // Emits resolve the agent name off the async workers there too.
            match p.event {
                // Turn start/finish come from srv's controller status (see
                // router `TurnStatus`), not from the renderer.
                NotifyPaneEvent::TurnStarted | NotifyPaneEvent::TurnCompleted | NotifyPaneEvent::TurnErrored => {}
                NotifyPaneEvent::TurnStopped => r.turn_stopped_nonblocking(&p.block_id),
                NotifyPaneEvent::InputResolved => r.resolve_nonblocking(&p.block_id, Family::Input),
                NotifyPaneEvent::InputWaiting => {
                    r.input_waiting_nonblocking(&p.block_id, p.question, p.question_count.unwrap_or(0) as usize)
                }
            }
            Ok(NotifyOk { ok: true })
        }
    });

    let rr = r.clone();
    let conn = conn_id.clone();
    engine.register_typed("notify.focus", move |p: NotifyFocusParams, _ctx| {
        let r = rr.clone();
        let conn = conn.clone();
        async move {
            r.focus(
                &conn,
                FocusReport {
                    window_focused: p.window_focused,
                    block_id: p.block_id.filter(|b| !b.is_empty()),
                    window_id: p.window_id.filter(|w| !w.is_empty()),
                },
            );
            Ok(NotifyOk { ok: true })
        }
    });

    let rr = r.clone();
    engine.register_typed("notify.ack", move |p: NotifyAckParams, _ctx| {
        let r = rr.clone();
        async move {
            // Resolving the click's pane reads the store — off the async workers.
            let o = tokio::task::spawn_blocking(move || r.ack(&p.id, p.clicked))
                .await
                .map_err(|e| format!("notify.ack: {e}"))?;
            Ok(NotifyAckResult { has_window: o.has_window, known: o.known, window_id: o.window_id, workspace_id: o.workspace_id })
        }
    });

    let rr = r.clone();
    engine.register_typed("notify.test", move |_p: NotifyNoArgs, _ctx| {
        let r = rr.clone();
        async move {
            r.test();
            Ok(NotifyOk { ok: true })
        }
    });

    let rr = r.clone();
    engine.register_typed("notify.takeactivation", move |p: NotifyTakeActivationParams, _ctx| {
        let r = rr.clone();
        async move {
            let taken = r.take_pending_activation(p.workspace_id.as_deref().filter(|w| !w.is_empty()));
            Ok(NotifyTakeActivationResult {
                block_id: taken.as_ref().map(|t| t.block_id.clone()),
                tab_id: taken.map(|t| t.tab_id),
            })
        }
    });

    let rr = r;
    let st = state.clone();
    engine.register_typed("block.reveal", move |p: BlockRevealParams, _ctx| {
        let r = rr.clone();
        let st = st.clone();
        async move { reveal_block(&st, &r, p.block_id).await }
    });
}

#[cfg(test)]
mod block_reveal_tests {
    use std::sync::{Arc, Mutex};

    use agentmux_common::ipc::{Command, Event};

    use crate::backend::mps::MuxEvent;
    use crate::backend::notify::policy::FocusReport;
    use crate::backend::notify::router::{self, BlockRevealEvent, EVENT_BLOCK_REVEAL};
    use crate::backend::obj::Window;
    use crate::server::AppState;

    async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
        let evs = crate::server::service::dispatch_to_reducer(state, cmd).await;
        for ev in &evs {
            crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
        }
        evs
    }

    struct Fixture {
        state: AppState,
        router: Arc<router::Router>,
        tab_id: String,
        /// Pane tabs, `a` showing, `b` in the background.
        a: String,
        b: String,
        window_id: String,
        reveals: Arc<Mutex<Vec<serde_json::Value>>>,
    }

    /// A workspace with one tab holding a two-tab pane [a, b] (a showing), a
    /// `Window` object on that workspace, and a recorder for `block:reveal`.
    /// `open`: a connected frontend reports being that window.
    async fn fixture(open: bool) -> Fixture {
        let state = crate::server::tests::test_state();
        let ws_id = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() })
            .await
            .iter()
            .find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            })
            .unwrap();
        let tab_id = dispatch_apply(&state, Command::CreateTab { workspace_id: ws_id.clone(), name: "t".into() })
            .await
            .iter()
            .find_map(|e| match e {
                Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            })
            .unwrap();
        let mut ids = Vec::new();
        for _ in 0..2 {
            let id = dispatch_apply(&state, Command::CreateBlock { tab_id: tab_id.clone(), meta: serde_json::Value::Null })
                .await
                .iter()
                .find_map(|e| match e {
                    Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
                    _ => None,
                })
                .unwrap();
            ids.push(id);
        }
        let (a, b) = (ids[0].clone(), ids[1].clone());
        dispatch_apply(
            &state,
            Command::LayoutSetTree {
                tab_id: tab_id.clone(),
                new_tree: Some(agentmux_common::LayoutNode {
                    id: "pane".into(),
                    data: Some(agentmux_common::LayoutNodeData {
                        block_id: a.clone(),
                        block_stack: vec![a.clone(), b.clone()],
                        active_block_id: a.clone(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                correlation_id: String::new(),
                slices: None,
            },
        )
        .await;
        // A window showing that workspace (the seeded one shows another).
        let mut window = state.mstore.get_all::<Window>().unwrap().into_iter().next().expect("seeded window");
        window.oid = uuid::Uuid::new_v4().to_string();
        window.workspaceid = ws_id;
        state.mstore.insert(&mut window).unwrap();

        let reveals = Arc::new(Mutex::new(Vec::new()));
        let rec = reveals.clone();
        state.broker.add_observer(Arc::new(move |ev: &MuxEvent| {
            if ev.event == EVENT_BLOCK_REVEAL {
                rec.lock().unwrap().push(ev.data.clone().unwrap_or_default());
            }
        }));
        let r = super::router_for(&state);
        if open {
            r.focus("conn-1", FocusReport { window_focused: false, block_id: None, window_id: Some(window.oid.clone()) });
        }
        Fixture { state, router: r, tab_id, a, b, window_id: window.oid, reveals }
    }

    #[tokio::test]
    async fn reveals_a_background_pane_tab_in_the_window_showing_it() {
        let f = fixture(true).await;
        let res = super::reveal_block(&f.state, &f.router, f.b.clone()).await.unwrap();
        assert!(res.found);
        assert_eq!(res.window_id.as_deref(), Some(f.window_id.as_str()));

        // The stored tree shows b — in the reducer and in mstore.
        let tree = f.state.srv_state.lock().await.tabs[&f.tab_id].rootnode.clone().unwrap();
        let data = tree.data.as_ref().unwrap();
        assert_eq!((data.block_id.as_str(), data.active_block_id.as_str()), (f.b.as_str(), f.b.as_str()));
        let tab = f.state.mstore.must_get::<crate::backend::obj::Tab>(&f.tab_id).unwrap();
        let layout = f.state.mstore.must_get::<crate::backend::obj::LayoutState>(&tab.layoutstate).unwrap();
        assert_eq!(layout.rootnode.as_ref(), Some(&tree), "db_layout has b active");

        // Only that window is told, with the tab it should switch to.
        let reveals = f.reveals.lock().unwrap().clone();
        assert_eq!(reveals.len(), 1, "{reveals:?}");
        let ev: BlockRevealEvent = serde_json::from_value(reveals[0].clone()).unwrap();
        assert_eq!(ev, BlockRevealEvent { block_id: f.b.clone(), tab_id: f.tab_id.clone(), window_id: f.window_id.clone() });
    }

    #[tokio::test]
    async fn unknown_block_is_not_found_and_publishes_nothing() {
        let f = fixture(true).await;
        let res = super::reveal_block(&f.state, &f.router, "no-such-block".into()).await.unwrap();
        assert!(!res.found);
        assert_eq!(res.window_id, None);
        assert!(f.reveals.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn workspace_open_in_no_window_is_not_found_and_changes_nothing() {
        let f = fixture(false).await;
        let res = super::reveal_block(&f.state, &f.router, f.b.clone()).await.unwrap();
        assert!(!res.found);
        assert!(f.reveals.lock().unwrap().is_empty());
        let tree = f.state.srv_state.lock().await.tabs[&f.tab_id].rootnode.clone().unwrap();
        assert_eq!(tree.data.as_ref().unwrap().active_block_id, f.a, "stored tree untouched");
    }

    #[tokio::test]
    async fn empty_block_id_is_an_error() {
        let f = fixture(true).await;
        assert!(super::reveal_block(&f.state, &f.router, String::new()).await.is_err());
    }
}

#[cfg(test)]
mod live_tests {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{ClientRequestBuilder, Message};

    async fn next_event<S>(rx: &mut S, want: &str) -> serde_json::Value
    where
        S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
    {
        loop {
            let msg = tokio::time::timeout(std::time::Duration::from_secs(5), rx.next())
                .await
                .unwrap_or_else(|_| panic!("timed out waiting for {want}"))
                .unwrap()
                .unwrap();
            let Ok(text) = msg.to_text() else { continue };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { continue };
            if v["data"]["command"] == "eventrecv" && v["data"]["data"]["event"] == want {
                return v["data"]["data"]["data"].clone();
            }
        }
    }

    /// The launcher presenter's exact wire shape, against the REAL srv router:
    /// `X-AuthKey` header auth on /ws, `eventsub`, then a Router-published
    /// `notification` (via notify.test) and its `notification:retract` after
    /// a dismiss ack.
    #[tokio::test]
    async fn header_authed_client_receives_router_events_over_ws() {
        let state = crate::server::tests::test_state();
        // test_state() leaves the broker without its fan-out client; wire it
        // exactly as bootstrap.rs does so published events reach /ws clients.
        state
            .broker
            .set_client(Box::new(crate::backend::eventbus::EventBusBridge::new(state.event_bus.clone())));
        let auth = state.auth_key.clone();
        let app = crate::server::build_router(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>())
                .await
                .unwrap();
        });

        let uri: tokio_tungstenite::tungstenite::http::Uri = format!("ws://{addr}/ws").parse().unwrap();
        let req = ClientRequestBuilder::new(uri).with_header("X-AuthKey", auth);
        let (ws, _) = tokio_tungstenite::connect_async(req).await.expect("header auth accepted on /ws");
        let (mut tx, mut rx) = ws.split();

        let send = |cmd: &str, reqid: &str, data: serde_json::Value| {
            serde_json::json!({"wscommand":"rpc","message":{"command":cmd,"reqid":reqid,"data":data}}).to_string()
        };
        for (i, ev) in ["notification", "notification:retract"].iter().enumerate() {
            tx.send(Message::Text(send("eventsub", &format!("s{i}"), serde_json::json!({"event":ev,"scopes":[],"allscopes":true})).into()))
                .await
                .unwrap();
        }
        tx.send(Message::Text(send("notify.test", "t1", serde_json::json!({})).into())).await.unwrap();

        let n = next_event(&mut rx, "notification").await;
        assert_eq!(n["kind"], "test");
        assert_eq!(n["title"], "AgentMux notifications are working");
        let id = n["id"].as_str().unwrap().to_string();
        let tag = n["tag"].as_str().unwrap().to_string();

        tx.send(Message::Text(send("notify.ack", "a1", serde_json::json!({"id": id, "clicked": false})).into()))
            .await
            .unwrap();
        let r = next_event(&mut rx, "notification:retract").await;
        assert_eq!(r["tag"], tag);
    }

    /// Rich-content spec §3.3 A/B: a click on a pane's toast resolves the
    /// pane's tab and open window server-side, names that window in
    /// `notification:activate`, and tells the launcher which window to raise.
    #[tokio::test]
    async fn clicking_a_pane_toast_names_its_open_window_and_tab() {
        use crate::backend::obj::{Block, Window};
        let state = crate::server::tests::test_state();
        state
            .broker
            .set_client(Box::new(crate::backend::eventbus::EventBusBridge::new(state.event_bus.clone())));
        let (broker, store) = (state.broker.clone(), state.mstore.clone());
        crate::backend::wcore::ensure_initial_data(&store).unwrap();
        let block = store.get_all::<Block>().unwrap().into_iter().next().expect("seeded block");
        let window = store.get_all::<Window>().unwrap().into_iter().next().expect("seeded window");
        let auth = state.auth_key.clone();
        let app = crate::server::build_router(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>())
                .await
                .unwrap();
        });
        let uri: tokio_tungstenite::tungstenite::http::Uri = format!("ws://{addr}/ws").parse().unwrap();
        let (ws, _) = tokio_tungstenite::connect_async(ClientRequestBuilder::new(uri).with_header("X-AuthKey", auth))
            .await
            .unwrap();
        let (mut tx, mut rx) = ws.split();
        let send = |cmd: &str, reqid: &str, data: serde_json::Value| {
            serde_json::json!({"wscommand":"rpc","message":{"command":cmd,"reqid":reqid,"data":data}}).to_string()
        };
        for (i, ev) in ["notification", "notification:activate"].iter().enumerate() {
            tx.send(Message::Text(send("eventsub", &format!("s{i}"), serde_json::json!({"event":ev,"scopes":[],"allscopes":true})).into()))
                .await
                .unwrap();
        }
        // This connection is the seeded window, open but not focused.
        tx.send(Message::Text(send("notify.focus", "f1", serde_json::json!({"window_focused": false, "window_id": window.oid})).into()))
            .await
            .unwrap();
        let r = crate::backend::notify::router::get(&broker).expect("router created by the connection");
        let b = block.oid.clone();
        tokio::task::spawn_blocking(move || {
            r.emit_fixed(crate::backend::notify::policy::NotifyKind::MessageNeedsReview, &b, Some("x".into()))
        })
        .await
        .unwrap();
        let n = next_event(&mut rx, "notification").await;
        let id = n["id"].as_str().unwrap().to_string();

        tx.send(Message::Text(send("notify.ack", "a1", serde_json::json!({"id": id, "clicked": true})).into()))
            .await
            .unwrap();
        let (mut activate, mut response) = (None, None);
        while activate.is_none() || response.is_none() {
            let msg = tokio::time::timeout(std::time::Duration::from_secs(5), rx.next()).await.expect("ack answered").unwrap().unwrap();
            let Ok(v) = serde_json::from_str::<serde_json::Value>(msg.to_text().unwrap_or("")) else { continue };
            if v["data"]["resid"] == "a1" {
                response = Some(v["data"]["data"].clone());
            } else if v["data"]["command"] == "eventrecv" && v["data"]["data"]["event"] == "notification:activate" {
                activate = Some(v["data"]["data"]["data"].clone());
            }
        }
        let (a, resp) = (activate.unwrap(), response.unwrap());
        let tab_id = block.parentoref.strip_prefix("tab:").unwrap();
        assert_eq!(a["block_id"], block.oid.as_str());
        assert_eq!(a["tab_id"], tab_id);
        assert_eq!(a["window_id"], window.oid.as_str());
        assert_eq!(a["workspace_id"], window.workspaceid.as_str());
        assert_eq!(resp["window_id"], window.oid.as_str(), "the launcher raises that window: {resp}");
        assert_eq!(resp["known"], true);
        assert_eq!(resp["has_window"], true);
    }
}
