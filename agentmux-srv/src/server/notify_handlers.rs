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
#[ts(export, export_to = "../../frontend/types/rpc/")]
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
#[ts(export, export_to = "../../frontend/types/rpc/")]
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
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct NotifyFocusParams {
    pub window_focused: bool,
    #[ts(optional)]
    pub block_id: Option<String>,
}

#[derive(serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct NotifyAckParams {
    pub id: String,
    pub clicked: bool,
}

#[derive(serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct NotifyAckResult {
    /// False when no frontend window is connected — the caller (launcher)
    /// should open one; it will pick the block up via `notify.takeactivation`.
    pub has_window: bool,
}

#[derive(serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct NotifyTakeActivationResult {
    #[ts(optional)]
    pub block_id: Option<String>,
}

#[derive(serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct NotifyNoArgs {}

#[derive(serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct NotifyOk {
    pub ok: bool,
}

fn router_for(state: &AppState) -> Arc<router::Router> {
    router::init(state.broker.clone(), state.config_watcher.clone(), state.mstore.clone(), state.reactive_handler)
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
            r.focus(&conn, FocusReport { window_focused: p.window_focused, block_id: p.block_id.filter(|b| !b.is_empty()) });
            Ok(NotifyOk { ok: true })
        }
    });

    let rr = r.clone();
    engine.register_typed("notify.ack", move |p: NotifyAckParams, _ctx| {
        let r = rr.clone();
        async move { Ok(NotifyAckResult { has_window: r.ack(&p.id, p.clicked) }) }
    });

    let rr = r.clone();
    engine.register_typed("notify.test", move |_p: NotifyNoArgs, _ctx| {
        let r = rr.clone();
        async move {
            r.test();
            Ok(NotifyOk { ok: true })
        }
    });

    let rr = r;
    engine.register_typed("notify.takeactivation", move |_p: NotifyNoArgs, _ctx| {
        let r = rr.clone();
        async move { Ok(NotifyTakeActivationResult { block_id: r.take_pending_activation() }) }
    });
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
}
