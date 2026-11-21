// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The connection loop against the real router over a real socket: what a client is sent on
//! connect, how each lane forwards, how a client's own messages are answered, and what a
//! disconnect cleans up.

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::{ClientRequestBuilder, Message};

use crate::backend::eventbus::{Lane, WSEventType};
use crate::server::AppState;

type Client = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Live {
    ws: Client,
    state: AppState,
}

async fn connect() -> Live {
    let state = crate::server::tests::test_state();
    let auth = state.auth_key.clone();
    let app = crate::server::build_router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>()).await.unwrap();
    });
    let uri: tokio_tungstenite::tungstenite::http::Uri = format!("ws://{addr}/ws").parse().unwrap();
    let (ws, _) = tokio_tungstenite::connect_async(ClientRequestBuilder::new(uri).with_header("X-AuthKey", auth)).await.unwrap();
    Live { ws, state }
}

impl Live {
    /// The next text frame as JSON, skipping anything else.
    async fn frame(&mut self) -> serde_json::Value {
        loop {
            let msg = tokio::time::timeout(std::time::Duration::from_secs(5), self.ws.next())
                .await
                .expect("timed out waiting for a frame")
                .expect("socket closed")
                .expect("socket error");
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(msg.to_text().unwrap_or("")) {
                return v;
            }
        }
    }

    /// Frames until one matches, so the connect-time frames and pings can come first.
    async fn until(&mut self, what: &str, pred: impl Fn(&serde_json::Value) -> bool) -> serde_json::Value {
        for _ in 0..40 {
            let f = self.frame().await;
            if pred(&f) {
                return f;
            }
        }
        panic!("no frame matched: {what}");
    }

    async fn send_json(&mut self, v: serde_json::Value) {
        self.ws.send(Message::Text(v.to_string().into())).await.unwrap();
    }
}

fn is_event(f: &serde_json::Value, name: &str) -> bool {
    f["data"]["command"] == "eventrecv" && f["data"]["data"]["event"] == name
}

#[tokio::test]
async fn a_new_connection_is_sent_the_config_then_the_srv_info() {
    let mut live = connect().await;
    let first = live.frame().await;
    let second = live.frame().await;
    assert!(is_event(&first, "config"), "{first}");
    assert!(is_event(&second, "srvinfo"), "{second}");
    assert_eq!(second["data"]["data"]["data"]["version"], live.state.version.as_str());
}

#[tokio::test]
async fn the_server_pings_with_a_timestamp() {
    let mut live = connect().await;
    let ping = live.until("a ping", |f| f["type"] == "ping").await;
    assert!(ping["stime"].as_i64().unwrap() > 0);
}

#[tokio::test]
async fn a_client_ping_is_answered_with_the_same_pong() {
    let mut live = connect().await;
    live.ws.send(Message::Ping(vec![7, 8, 9].into())).await.unwrap();
    loop {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), live.ws.next()).await.expect("pong").unwrap().unwrap();
        if let Message::Pong(data) = msg {
            assert_eq!(&data[..], &[7u8, 8, 9][..]);
            break;
        }
    }
}

#[tokio::test]
async fn an_rpc_command_is_answered_on_the_rpc_path_with_its_request_id() {
    let mut live = connect().await;
    let sub = serde_json::json!({"wscommand": "rpc", "message": {"command": "eventsub", "reqid": "r1", "data": {"event": "x", "scopes": [], "allscopes": true}}});
    live.send_json(sub).await;
    let reply = live.until("the eventsub reply", |f| f["eventtype"] == "rpc" && f["data"]["resid"] == "r1").await;
    assert_eq!(reply["eventtype"], "rpc");
}

#[tokio::test]
async fn a_raw_event_bus_event_reaches_the_client_wrapped_as_an_rpc_event() {
    let mut live = connect().await;
    live.until("the srv info", |f| is_event(f, "srvinfo")).await;
    let raw = WSEventType { eventtype: "waveobj:update".into(), oref: "block:b1".into(), data: Some(serde_json::json!({"updatetype": "update"})) };
    live.state.event_bus.broadcast_event(&raw);
    let f = live.until("the wrapped update", |f| is_event(f, "waveobj:update")).await;
    assert_eq!(f["eventtype"], "rpc");
    assert_eq!(f["data"]["data"]["scopes"], serde_json::json!(["block:b1"]));
    assert_eq!(f["data"]["data"]["data"]["updatetype"], "update");
}

#[tokio::test]
async fn an_event_that_is_already_an_rpc_frame_is_forwarded_as_it_is() {
    let mut live = connect().await;
    live.until("the srv info", |f| is_event(f, "srvinfo")).await;
    let ready = serde_json::json!({"eventtype": "rpc", "data": {"command": "eventrecv", "data": {"event": "already-wrapped", "scopes": ["block:b2"], "data": {"k": 1}}}});
    live.state.event_bus.broadcast_value_lane(ready.clone(), Lane::Priority);
    let f = live.until("the forwarded frame", |f| is_event(f, "already-wrapped")).await;
    assert_eq!(f, ready);
}

#[tokio::test]
async fn one_panes_events_keep_their_order_when_several_arrive_together() {
    let mut live = connect().await;
    live.until("the srv info", |f| is_event(f, "srvinfo")).await;
    for n in 0..5 {
        let ev = serde_json::json!({"eventtype": "rpc", "data": {"command": "eventrecv", "data": {"event": "blockfile", "scopes": ["block:order"], "data": {"n": n}}}});
        live.state.event_bus.broadcast_value_lane(ev, Lane::Priority);
    }
    let mut seen = Vec::new();
    while seen.len() < 5 {
        let f = live.until("a blockfile event", |f| is_event(f, "blockfile")).await;
        seen.push(f["data"]["data"]["data"]["n"].as_i64().unwrap());
    }
    assert_eq!(seen, vec![0, 1, 2, 3, 4]);
}

#[tokio::test]
async fn background_telemetry_that_piles_up_is_sent_as_its_latest_reading() {
    let mut live = connect().await;
    live.until("the srv info", |f| is_event(f, "srvinfo")).await;
    // Sent without yielding, so all three are queued before the connection task reads any.
    for n in 1..=3 {
        let ev = serde_json::json!({"eventtype": "rpc", "data": {"command": "eventrecv", "data": {"event": "sysinfo", "scopes": ["local"], "data": {"n": n}}}});
        live.state.event_bus.broadcast_value_lane(ev, Lane::Background);
    }
    let f = live.until("a sysinfo reading", |f| is_event(f, "sysinfo")).await;
    assert_eq!(f["data"]["data"]["data"]["n"], 3, "{f}");
}

#[tokio::test]
async fn a_closing_client_is_unregistered_from_the_event_bus() {
    let mut live = connect().await;
    live.until("the srv info", |f| is_event(f, "srvinfo")).await;
    assert!(live.state.event_bus.connection_count() >= 1);
    live.ws.close(None).await.unwrap();
    for _ in 0..50 {
        if live.state.event_bus.connection_count() == 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("the connection stayed registered after the client closed");
}

#[tokio::test]
async fn a_message_that_is_not_json_does_not_end_the_connection() {
    let mut live = connect().await;
    live.until("the srv info", |f| is_event(f, "srvinfo")).await;
    live.ws.send(Message::Text("not json at all".into())).await.unwrap();
    live.ws.send(Message::Ping(vec![1].into())).await.unwrap();
    loop {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), live.ws.next()).await.expect("still connected").unwrap().unwrap();
        if matches!(msg, Message::Pong(_)) {
            break;
        }
    }
}
