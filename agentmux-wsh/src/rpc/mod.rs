// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! WebSocket RPC client for communicating with agentmuxsrv-rs.
//!
//! Reads the backend's endpoints JSON file to discover the WebSocket address
//! and auth key, then establishes a connection for sending RPC commands.

use std::collections::HashMap;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, oneshot};
use tokio_tungstenite::tungstenite::Message;

// ---- Endpoints discovery ----

#[derive(Deserialize)]
struct Endpoints {
    auth_key: String,
    ws_endpoint: String,
    #[allow(dead_code)]
    version: String,
}

/// Find and read the wave-endpoints.json file.
fn read_endpoints() -> Result<Endpoints, String> {
    // Use compile-time version to match Tauri's versioned identifier.
    // On Windows: %APPDATA%/ai.agentmux.app.v0-31-60/instances/v0.31.60/wave-endpoints.json
    // On macOS:   ~/Library/Application Support/ai.agentmux.app.v0-31-60/instances/v0.31.60/...
    // On Linux:   ~/.config/ai.agentmux.app.v0-31-60/instances/v0.31.60/...

    let config_dir = if cfg!(windows) {
        std::env::var("APPDATA")
            .map(std::path::PathBuf::from)
            .map_err(|_| "APPDATA not set".to_string())?
    } else {
        dirs::config_dir().ok_or("cannot determine config directory")?
    };

    let version = env!("CARGO_PKG_VERSION");
    let version_dashed = version.replace('.', "-");
    let app_dir = format!("ai.agentmux.app.v{}", version_dashed);
    let instance_dir = format!("v{}", version);

    let endpoints_path = config_dir
        .join(&app_dir)
        .join("instances")
        .join(&instance_dir)
        .join("wave-endpoints.json");

    let contents = std::fs::read_to_string(&endpoints_path).map_err(|e| {
        format!(
            "cannot read {}: {} (is AgentMux running?)",
            endpoints_path.display(),
            e
        )
    })?;

    serde_json::from_str(&contents).map_err(|e| format!("invalid endpoints JSON: {}", e))
}

// ---- RPC message types ----

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RpcMessage {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    command: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    reqid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    resid: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    timeout: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    error: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    cont: bool,
}

fn is_zero(v: &i64) -> bool {
    *v == 0
}

/// Wrapper for incoming WebSocket events from the server.
#[derive(Deserialize)]
struct WSEvent {
    #[serde(default)]
    eventtype: String,
    #[serde(default)]
    data: Option<serde_json::Value>,
}

// ---- RPC Client ----

type PendingMap = HashMap<String, oneshot::Sender<Result<serde_json::Value, String>>>;

struct ClientInner {
    writer: futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
}

/// WebSocket RPC client for agentmuxsrv-rs.
pub struct RpcClient {
    inner: Arc<Mutex<ClientInner>>,
    pending: Arc<Mutex<PendingMap>>,
}

impl RpcClient {
    /// Connect to the running backend by reading endpoints.json.
    pub async fn connect() -> Result<Self, String> {
        let endpoints = read_endpoints()?;
        let url = format!(
            "ws://{}/ws?authkey={}",
            endpoints.ws_endpoint, endpoints.auth_key
        );

        let (ws_stream, _response) = tokio_tungstenite::connect_async(&url)
            .await
            .map_err(|e| format!("WebSocket connect failed: {}", e))?;

        let (writer, reader) = ws_stream.split();
        let pending: Arc<Mutex<PendingMap>> = Arc::new(Mutex::new(HashMap::new()));

        let client = RpcClient {
            inner: Arc::new(Mutex::new(ClientInner { writer })),
            pending: pending.clone(),
        };

        // Spawn reader task to route responses to pending requests
        tokio::spawn(Self::reader_loop(reader, pending));

        Ok(client)
    }

    /// Send an RPC command and wait for the response.
    pub async fn call(
        &self,
        command: &str,
        data: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let req_id = uuid::Uuid::new_v4().to_string();

        let msg = RpcMessage {
            command: command.to_string(),
            reqid: req_id.clone(),
            resid: String::new(),
            timeout: 5000,
            error: String::new(),
            data: Some(data),
            cont: false,
        };

        // Register pending response channel
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(req_id.clone(), tx);
        }

        // Send the message wrapped as wscommand: "rpc"
        let envelope = serde_json::json!({
            "wscommand": "rpc",
            "message": msg,
        });
        let text = serde_json::to_string(&envelope)
            .map_err(|e| format!("serialize error: {}", e))?;

        {
            let mut inner = self.inner.lock().await;
            inner
                .writer
                .send(Message::Text(text.into()))
                .await
                .map_err(|e| format!("WebSocket send error: {}", e))?;
        }

        // Wait for response with timeout
        let result = tokio::time::timeout(std::time::Duration::from_secs(10), rx)
            .await
            .map_err(|_| format!("RPC timeout: {}", command))?
            .map_err(|_| "response channel closed".to_string())?;

        result
    }

    async fn reader_loop(
        mut reader: futures_util::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
        >,
        pending: Arc<Mutex<PendingMap>>,
    ) {
        while let Some(msg) = reader.next().await {
            let text = match msg {
                Ok(Message::Text(t)) => t.to_string(),
                Ok(Message::Close(_)) => break,
                Ok(_) => continue,
                Err(_) => break,
            };

            // Parse the incoming event
            let event: WSEvent = match serde_json::from_str(&text) {
                Ok(e) => e,
                Err(_) => continue,
            };

            // We only care about RPC responses
            if event.eventtype != "rpc" {
                continue;
            }

            let rpc_msg: RpcMessage = match event.data {
                Some(data) => match serde_json::from_value(data) {
                    Ok(m) => m,
                    Err(_) => continue,
                },
                None => continue,
            };

            // Route response to the pending request.
            // Server responses use `resid` containing the original `reqid`.
            let response_id = if !rpc_msg.resid.is_empty() {
                &rpc_msg.resid
            } else if !rpc_msg.reqid.is_empty() {
                &rpc_msg.reqid
            } else {
                continue;
            };

            let mut pending = pending.lock().await;
            if let Some(tx) = pending.remove(response_id) {
                if !rpc_msg.error.is_empty() {
                    let _ = tx.send(Err(rpc_msg.error));
                } else {
                    let _ = tx.send(Ok(rpc_msg.data.unwrap_or(serde_json::Value::Null)));
                }
            }
        }
    }
}
