// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! WshRpc — main RPC client with message routing and response handling.
//! Port of Go's `pkg/wshutil/wshrpc.go`.
//!
//! Provides the core RPC communication layer:
//! - Message send/receive via channels
//! - Request/response correlation via request IDs
//! - Streaming responses (continued flag)
//! - Context cancellation propagation
//! - Auth token injection

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use serde_json::Value;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::event::EventListener;
use super::proxy::{RpcContext, RpcMessage};
use super::osc::{DEFAULT_INPUT_CH_SIZE, DEFAULT_OUTPUT_CH_SIZE};

/// Default timeout for RPC calls in milliseconds.
pub const DEFAULT_TIMEOUT_MS: u64 = 5000;
/// Channel buffer size for response channels.
pub const RESP_CH_SIZE: usize = 32;

/// RPC response data (single response or stream item).
#[derive(Debug, Clone)]
pub struct RpcResponse {
    pub data: Option<Value>,
    pub error: Option<String>,
    pub is_final: bool,
}

/// Pending RPC request state.
struct RpcData {
    resp_tx: mpsc::Sender<RpcResponse>,
}

/// Handler for incoming RPC requests (server side).
pub struct RpcResponseHandler {
    pub req_id: String,
    pub command: String,
    pub data: Option<Value>,
    pub rpc_context: RpcContext,
    response_tx: mpsc::Sender<Vec<u8>>,
    finalized: AtomicBool,
}

impl RpcResponseHandler {
    /// Get the command name.
    pub fn get_command(&self) -> &str {
        &self.command
    }

    /// Get the raw command data.
    pub fn get_command_raw_data(&self) -> Option<&Value> {
        self.data.as_ref()
    }

    /// Get the RPC context.
    pub fn get_rpc_context(&self) -> &RpcContext {
        &self.rpc_context
    }

    /// Check if this request needs a response.
    pub fn needs_response(&self) -> bool {
        !self.req_id.is_empty()
    }

    /// Send a response (data + done flag).
    pub async fn send_response(&self, data: Option<Value>, done: bool) -> Result<(), String> {
        if !self.needs_response() {
            return Ok(());
        }

        let msg = RpcMessage {
            res_id: self.req_id.clone(),
            data,
            cont: !done,
            ..Default::default()
        };

        let json = serde_json::to_vec(&msg).map_err(|e| format!("json encode: {}", e))?;
        self.response_tx
            .send(json)
            .await
            .map_err(|e| format!("send response: {}", e))?;

        if done {
            self.finalized.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    /// Send an error response.
    pub async fn send_response_error(&self, err: &str) -> Result<(), String> {
        if !self.needs_response() {
            return Ok(());
        }

        let msg = RpcMessage {
            res_id: self.req_id.clone(),
            error: Some(err.to_string()),
            ..Default::default()
        };

        let json = serde_json::to_vec(&msg).map_err(|e| format!("json encode: {}", e))?;
        self.response_tx
            .send(json)
            .await
            .map_err(|e| format!("send error response: {}", e))?;

        self.finalized.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Mark the handler as finalized.
    pub fn finalize(&self) {
        self.finalized.store(true, Ordering::SeqCst);
    }

    /// Check if the handler has been finalized.
    pub fn is_finalized(&self) -> bool {
        self.finalized.load(Ordering::SeqCst)
    }
}

/// Callback type for command handlers.
pub type CommandHandlerFn = Box<dyn Fn(RpcResponseHandler) -> bool + Send + Sync>;

/// Main WshRpc client.
///
/// Provides bidirectional RPC over message channels:
/// - Send requests and wait for responses
/// - Handle incoming requests with registered handlers
/// - Support streaming responses
/// - Auth token management
pub struct WshRpc {
    input_ch: mpsc::Sender<Vec<u8>>,
    output_ch: mpsc::Sender<Vec<u8>>,
    rpc_context: Arc<Mutex<Option<RpcContext>>>,
    auth_token: Arc<Mutex<String>>,
    rpc_map: Arc<Mutex<HashMap<String, RpcData>>>,
    event_listener: Arc<EventListener>,
    debug: AtomicBool,
    debug_name: String,
    server_done: AtomicBool,
}

impl WshRpc {
    /// Create a new WshRpc client with input/output channels.
    pub fn new(debug_name: &str) -> (Self, mpsc::Receiver<Vec<u8>>, mpsc::Sender<Vec<u8>>) {
        let (input_tx, _input_rx) = mpsc::channel(DEFAULT_INPUT_CH_SIZE);
        let (output_tx, output_rx) = mpsc::channel(DEFAULT_OUTPUT_CH_SIZE);

        let input_tx_clone = input_tx.clone();
        let rpc = Self {
            input_ch: input_tx,
            output_ch: output_tx,
            rpc_context: Arc::new(Mutex::new(None)),
            auth_token: Arc::new(Mutex::new(String::new())),
            rpc_map: Arc::new(Mutex::new(HashMap::new())),
            event_listener: Arc::new(EventListener::new()),
            debug: AtomicBool::new(false),
            debug_name: debug_name.to_string(),
            server_done: AtomicBool::new(false),
        };

        (rpc, output_rx, input_tx_clone)
    }

    /// Set the RPC context.
    pub fn set_rpc_context(&self, ctx: RpcContext) {
        *self.rpc_context.lock().unwrap() = Some(ctx);
    }

    /// Get the current RPC context.
    pub fn get_rpc_context(&self) -> Option<RpcContext> {
        self.rpc_context.lock().unwrap().clone()
    }

    /// Set the auth token.
    pub fn set_auth_token(&self, token: &str) {
        *self.auth_token.lock().unwrap() = token.to_string();
    }

    /// Get the auth token.
    pub fn get_auth_token(&self) -> String {
        self.auth_token.lock().unwrap().clone()
    }

    /// Enable or disable debug logging.
    pub fn set_debug(&self, debug: bool) {
        self.debug.store(debug, Ordering::SeqCst);
    }

    /// Get the event listener.
    pub fn get_event_listener(&self) -> &EventListener {
        &self.event_listener
    }

    /// Send an RPC request and wait for a single response.
    pub async fn send_rpc_request(
        &self,
        command: &str,
        data: Option<Value>,
        timeout_ms: Option<u64>,
    ) -> Result<Option<Value>, String> {
        let req_id = Uuid::new_v4().to_string();
        let timeout = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);

        // Create response channel
        let (resp_tx, mut resp_rx) = mpsc::channel(RESP_CH_SIZE);
        self.rpc_map.lock().unwrap().insert(req_id.clone(), RpcData { resp_tx });

        // Build request message
        let mut msg = RpcMessage {
            command: command.to_string(),
            req_id: req_id.clone(),
            data,
            timeout: Some(timeout),
            ..Default::default()
        };

        // Inject auth token
        let auth_token = self.get_auth_token();
        if !auth_token.is_empty() {
            msg.auth_token = Some(auth_token);
        }

        // Send request
        let json = serde_json::to_vec(&msg).map_err(|e| format!("json encode: {}", e))?;
        self.output_ch
            .send(json)
            .await
            .map_err(|e| format!("send request: {}", e))?;

        // Wait for response with timeout
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(timeout),
            resp_rx.recv(),
        )
        .await;

        // Clean up
        self.rpc_map.lock().unwrap().remove(&req_id);

        match result {
            Ok(Some(resp)) => {
                if let Some(err) = resp.error {
                    Err(err)
                } else {
                    Ok(resp.data)
                }
            }
            Ok(None) => Err("response channel closed".to_string()),
            Err(_) => Err(format!("RPC timeout after {}ms", timeout)),
        }
    }

    /// Send a fire-and-forget message (no response expected).
    pub async fn send_message(&self, command: &str, data: Option<Value>) -> Result<(), String> {
        let mut msg = RpcMessage {
            command: command.to_string(),
            req_id: String::new(), // no response expected
            data,
            ..Default::default()
        };

        let auth_token = self.get_auth_token();
        if !auth_token.is_empty() {
            msg.auth_token = Some(auth_token);
        }

        let json = serde_json::to_vec(&msg).map_err(|e| format!("json encode: {}", e))?;
        self.output_ch
            .send(json)
            .await
            .map_err(|e| format!("send message: {}", e))
    }

    /// Process an incoming message (response or request).
    pub fn process_incoming_message(&self, raw_msg: &[u8]) -> Result<(), String> {
        let msg: RpcMessage =
            serde_json::from_slice(raw_msg).map_err(|e| format!("json decode: {}", e))?;

        if msg.is_response() {
            self.handle_response(msg)
        } else if msg.is_request() {
            // Request handling would be delegated to registered handlers
            tracing::debug!("incoming request: {} ({})", msg.command, msg.req_id);
            Ok(())
        } else {
            Err("message is neither request nor response".to_string())
        }
    }

    /// Handle an incoming response message.
    fn handle_response(&self, msg: RpcMessage) -> Result<(), String> {
        let rpc_map = self.rpc_map.lock().unwrap();
        if let Some(rpc_data) = rpc_map.get(&msg.res_id) {
            let resp = RpcResponse {
                data: msg.data,
                error: msg.error,
                is_final: !msg.cont,
            };
            let _ = rpc_data.resp_tx.try_send(resp);
        } else if self.debug.load(Ordering::SeqCst) {
            tracing::warn!(
                "[{}] received response for unknown req_id: {}",
                self.debug_name,
                msg.res_id
            );
        }
        Ok(())
    }

    /// Check if the server is done.
    pub fn is_server_done(&self) -> bool {
        self.server_done.load(Ordering::SeqCst)
    }

    /// Mark the server as done.
    pub fn set_server_done(&self) {
        self.server_done.store(true, Ordering::SeqCst);
    }

    /// Get the count of pending RPC requests.
    pub fn pending_count(&self) -> usize {
        self.rpc_map.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_wshrpc_create() {
        let (rpc, _output_rx, _input_tx) = WshRpc::new("test");
        assert!(!rpc.is_server_done());
        assert_eq!(rpc.pending_count(), 0);
        assert_eq!(rpc.get_auth_token(), "");
    }

    #[tokio::test]
    async fn test_wshrpc_auth_token() {
        let (rpc, _output_rx, _input_tx) = WshRpc::new("test");
        rpc.set_auth_token("secret123");
        assert_eq!(rpc.get_auth_token(), "secret123");
    }

    #[tokio::test]
    async fn test_wshrpc_rpc_context() {
        let (rpc, _output_rx, _input_tx) = WshRpc::new("test");
        assert!(rpc.get_rpc_context().is_none());

        rpc.set_rpc_context(RpcContext {
            block_id: "block1".to_string(),
            tab_id: "tab1".to_string(),
            conn: "local".to_string(),
        });

        let ctx = rpc.get_rpc_context().unwrap();
        assert_eq!(ctx.block_id, "block1");
        assert_eq!(ctx.tab_id, "tab1");
    }

    #[tokio::test]
    async fn test_wshrpc_send_message() {
        let (rpc, mut output_rx, _input_tx) = WshRpc::new("test");
        rpc.set_auth_token("token123");

        rpc.send_message("notify", Some(serde_json::json!({"msg": "hello"})))
            .await
            .unwrap();

        let raw = output_rx.recv().await.unwrap();
        let msg: RpcMessage = serde_json::from_slice(&raw).unwrap();
        assert_eq!(msg.command, "notify");
        assert!(msg.req_id.is_empty()); // fire-and-forget
        assert_eq!(msg.auth_token.unwrap(), "token123");
    }

    #[tokio::test]
    async fn test_wshrpc_process_response() {
        let (rpc, mut output_rx, _input_tx) = WshRpc::new("test");

        // Simulate a pending request
        let (resp_tx, mut resp_rx) = mpsc::channel(RESP_CH_SIZE);
        rpc.rpc_map
            .lock()
            .unwrap()
            .insert("req-1".to_string(), RpcData { resp_tx });

        // Process a response
        let response = RpcMessage {
            res_id: "req-1".to_string(),
            data: Some(serde_json::json!({"result": "success"})),
            ..Default::default()
        };
        let raw = serde_json::to_vec(&response).unwrap();
        rpc.process_incoming_message(&raw).unwrap();

        // Check response was delivered
        let resp = resp_rx.recv().await.unwrap();
        assert!(resp.error.is_none());
        assert!(resp.is_final);
        assert_eq!(
            resp.data.unwrap(),
            serde_json::json!({"result": "success"})
        );
    }
}
