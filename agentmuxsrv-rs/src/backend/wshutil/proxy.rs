// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! RPC proxy types for forwarding messages between connections.
//! Port of Go's `pkg/wshutil/wshproxy.go` and `wshmultiproxy.go`.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use super::osc::{DEFAULT_INPUT_CH_SIZE, DEFAULT_OUTPUT_CH_SIZE};

/// RPC context passed with each message.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RpcContext {
    #[serde(default, skip_serializing_if = "String::is_empty", rename = "blockid")]
    pub block_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty", rename = "tabid")]
    pub tab_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty", rename = "conn")]
    pub conn: String,
}

/// RPC message format for JSON-RPC communication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcMessage {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
    #[serde(default, skip_serializing_if = "String::is_empty", rename = "reqid")]
    pub req_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty", rename = "resid")]
    pub res_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub cont: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "route")]
    pub route: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "source")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "authtoken")]
    pub auth_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "timeout")]
    pub timeout: Option<u64>,
}

impl RpcMessage {
    /// Check if this is a request (has command and reqid).
    pub fn is_request(&self) -> bool {
        !self.command.is_empty() && !self.req_id.is_empty()
    }

    /// Check if this is a response (has resid).
    pub fn is_response(&self) -> bool {
        !self.res_id.is_empty()
    }

    /// Check if this is an error response.
    pub fn is_error(&self) -> bool {
        self.error.is_some()
    }

    /// Check if this is the final response (not continued).
    pub fn is_final(&self) -> bool {
        !self.cont
    }
}

/// Single-connection RPC proxy.
/// Forwards messages between a local connection and a remote endpoint.
pub struct WshRpcProxy {
    rpc_context: Arc<Mutex<Option<RpcContext>>>,
    auth_token: Arc<Mutex<String>>,
    pub to_remote: mpsc::Sender<Vec<u8>>,
    pub from_remote: mpsc::Receiver<Vec<u8>>,
    to_remote_rx: Option<mpsc::Receiver<Vec<u8>>>,
    from_remote_tx: mpsc::Sender<Vec<u8>>,
}

impl WshRpcProxy {
    pub fn new() -> Self {
        let (to_remote_tx, to_remote_rx) = mpsc::channel(DEFAULT_INPUT_CH_SIZE);
        let (from_remote_tx, from_remote_rx) = mpsc::channel(DEFAULT_OUTPUT_CH_SIZE);
        Self {
            rpc_context: Arc::new(Mutex::new(None)),
            auth_token: Arc::new(Mutex::new(String::new())),
            to_remote: to_remote_tx,
            from_remote: from_remote_rx,
            to_remote_rx: Some(to_remote_rx),
            from_remote_tx,
        }
    }

    pub fn set_rpc_context(&self, ctx: RpcContext) {
        *self.rpc_context.lock().unwrap() = Some(ctx);
    }

    pub fn get_rpc_context(&self) -> Option<RpcContext> {
        self.rpc_context.lock().unwrap().clone()
    }

    pub fn set_auth_token(&self, token: &str) {
        *self.auth_token.lock().unwrap() = token.to_string();
    }

    pub fn get_auth_token(&self) -> String {
        self.auth_token.lock().unwrap().clone()
    }

    /// Take the receiver end of to_remote channel (for proxy loop).
    pub fn take_to_remote_rx(&mut self) -> Option<mpsc::Receiver<Vec<u8>>> {
        self.to_remote_rx.take()
    }

    /// Get a clone of the from_remote sender (for injecting messages).
    pub fn from_remote_sender(&self) -> mpsc::Sender<Vec<u8>> {
        self.from_remote_tx.clone()
    }

    /// Send a message to the remote endpoint.
    pub async fn send_to_remote(&self, msg: Vec<u8>) -> Result<(), String> {
        self.to_remote
            .send(msg)
            .await
            .map_err(|e| format!("failed to send to remote: {}", e))
    }

    /// Inject an RPC message (encode as JSON and send to remote).
    pub async fn send_rpc_message(&self, msg: &RpcMessage) -> Result<(), String> {
        let json = serde_json::to_vec(msg).map_err(|e| format!("json encode: {}", e))?;
        self.send_to_remote(json).await
    }

    /// Send an error response for a given request.
    pub async fn send_response_error(&self, req_id: &str, err_msg: &str) -> Result<(), String> {
        if req_id.is_empty() {
            return Ok(());
        }
        let msg = RpcMessage {
            res_id: req_id.to_string(),
            error: Some(err_msg.to_string()),
            ..Default::default()
        };
        self.send_rpc_message(&msg).await
    }
}

impl Default for RpcMessage {
    fn default() -> Self {
        Self {
            command: String::new(),
            req_id: String::new(),
            res_id: String::new(),
            data: None,
            error: None,
            cont: false,
            cancel: None,
            route: None,
            source: None,
            auth_token: None,
            timeout: None,
        }
    }
}

/// Multi-connection broadcast proxy.
/// Sends messages to multiple remote connections simultaneously.
pub struct WshMultiProxy {
    proxies: Arc<Mutex<HashMap<String, mpsc::Sender<Vec<u8>>>>>,
}

impl WshMultiProxy {
    pub fn new() -> Self {
        Self {
            proxies: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Add a named proxy connection.
    pub fn add_proxy(&self, name: &str, sender: mpsc::Sender<Vec<u8>>) {
        self.proxies.lock().unwrap().insert(name.to_string(), sender);
    }

    /// Remove a named proxy connection.
    pub fn remove_proxy(&self, name: &str) {
        self.proxies.lock().unwrap().remove(name);
    }

    /// Broadcast a message to all connected proxies.
    pub async fn broadcast(&self, msg: Vec<u8>) {
        let senders: Vec<mpsc::Sender<Vec<u8>>> = {
            let proxies = self.proxies.lock().unwrap();
            proxies.values().cloned().collect()
        };

        for sender in senders {
            let msg_clone = msg.clone();
            let _ = sender.send(msg_clone).await;
        }
    }

    /// Broadcast an RPC message to all connected proxies.
    pub async fn broadcast_rpc_message(&self, msg: &RpcMessage) -> Result<(), String> {
        let json = serde_json::to_vec(msg).map_err(|e| format!("json encode: {}", e))?;
        self.broadcast(json).await;
        Ok(())
    }

    /// Get the count of connected proxies.
    pub fn proxy_count(&self) -> usize {
        self.proxies.lock().unwrap().len()
    }

    /// Get names of all connected proxies.
    pub fn proxy_names(&self) -> Vec<String> {
        self.proxies.lock().unwrap().keys().cloned().collect()
    }
}

impl Default for WshMultiProxy {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rpc_message_request() {
        let msg = RpcMessage {
            command: "test".to_string(),
            req_id: "abc123".to_string(),
            ..Default::default()
        };
        assert!(msg.is_request());
        assert!(!msg.is_response());
        assert!(!msg.is_error());
        assert!(msg.is_final());
    }

    #[test]
    fn test_rpc_message_response() {
        let msg = RpcMessage {
            res_id: "abc123".to_string(),
            data: Some(serde_json::json!({"result": "ok"})),
            ..Default::default()
        };
        assert!(!msg.is_request());
        assert!(msg.is_response());
        assert!(!msg.is_error());
    }

    #[test]
    fn test_rpc_message_error() {
        let msg = RpcMessage {
            res_id: "abc123".to_string(),
            error: Some("something failed".to_string()),
            ..Default::default()
        };
        assert!(msg.is_error());
    }

    #[test]
    fn test_rpc_message_serde() {
        let msg = RpcMessage {
            command: "getblock".to_string(),
            req_id: "req-1".to_string(),
            data: Some(serde_json::json!({"id": "block-1"})),
            route: Some("conn:local".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: RpcMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.command, "getblock");
        assert_eq!(parsed.req_id, "req-1");
        assert_eq!(parsed.route.unwrap(), "conn:local");
    }

    #[tokio::test]
    async fn test_multi_proxy_broadcast() {
        let multi = WshMultiProxy::new();
        let (tx1, mut rx1) = mpsc::channel(10);
        let (tx2, mut rx2) = mpsc::channel(10);

        multi.add_proxy("conn1", tx1);
        multi.add_proxy("conn2", tx2);
        assert_eq!(multi.proxy_count(), 2);

        multi.broadcast(b"hello".to_vec()).await;

        let msg1 = rx1.recv().await.unwrap();
        let msg2 = rx2.recv().await.unwrap();
        assert_eq!(msg1, b"hello");
        assert_eq!(msg2, b"hello");
    }

    #[test]
    fn test_multi_proxy_add_remove() {
        let multi = WshMultiProxy::new();
        let (tx, _rx) = mpsc::channel(10);

        multi.add_proxy("conn1", tx);
        assert_eq!(multi.proxy_count(), 1);

        multi.remove_proxy("conn1");
        assert_eq!(multi.proxy_count(), 0);
    }
}
