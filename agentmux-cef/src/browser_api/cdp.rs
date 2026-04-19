// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Minimal Chrome DevTools Protocol (CDP) WebSocket client.
//!
//! One session = one WebSocket to `ws://127.0.0.1:<debug_port>/devtools/page/<target>`.
//! Per-request construction in Phase 1 (see PLAN §Phase 5 for pooling).
//!
//! CDP protocol: every client request is `{id, method, params}`; every
//! server reply with matching `id` is `{id, result}` or `{id, error}`.
//! Events (no id) are ignored here — we don't subscribe to any in
//! Phase 1 since every method we use is request/response.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async,
    tungstenite::protocol::Message,
    MaybeTlsStream, WebSocketStream,
};

pub type CdpError = String;

pub struct CdpSession {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    next_id: u64,
}

impl CdpSession {
    /// Open a CDP session against `ws://127.0.0.1:<port>/devtools/page/<target>`.
    pub async fn connect(ws_url: &str) -> Result<Self, CdpError> {
        let (ws, _resp) = connect_async(ws_url)
            .await
            .map_err(|e| format!("CDP connect {ws_url}: {e}"))?;
        Ok(Self { ws, next_id: 1 })
    }

    /// Send `{id, method, params}` and wait for the reply with the
    /// same id. Events (no id) and unrelated replies are silently
    /// discarded — v1 doesn't multiplex.
    pub async fn call(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<Value, CdpError> {
        let id = self.next_id;
        self.next_id += 1;

        let req = json!({ "id": id, "method": method, "params": params });
        let msg = Message::Text(req.to_string().into());
        self.ws
            .send(msg)
            .await
            .map_err(|e| format!("CDP send {method}: {e}"))?;

        // Wait up to 10s for the matching reply. CDP replies are
        // usually <100 ms on localhost; the generous bound is a
        // safety net for pages stalled on JS execution.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(format!("CDP timeout waiting for reply to {method}"));
            }
            let next = tokio::time::timeout(remaining, self.ws.next()).await;
            let frame = match next {
                Ok(Some(Ok(f))) => f,
                Ok(Some(Err(e))) => return Err(format!("CDP ws error: {e}")),
                Ok(None) => return Err("CDP ws closed".to_string()),
                Err(_) => return Err(format!("CDP timeout waiting for reply to {method}")),
            };
            let text = match frame {
                Message::Text(t) => t,
                Message::Binary(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
                Message::Close(_) => return Err("CDP ws closed by peer".to_string()),
            };

            let msg: Value = serde_json::from_str(&text)
                .map_err(|e| format!("CDP parse: {e}"))?;
            // Events have no top-level "id". Skip them.
            let Some(msg_id) = msg.get("id").and_then(|v| v.as_u64()) else {
                continue;
            };
            if msg_id != id {
                // Interleaved reply to a previous call — unexpected in
                // v1 since we don't pipeline, but skip it to be safe.
                continue;
            }
            if let Some(err) = msg.get("error") {
                return Err(format!(
                    "CDP {method} error: {}",
                    err.get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown"),
                ));
            }
            return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    pub async fn close(mut self) {
        let _ = self.ws.close(None).await;
    }
}
