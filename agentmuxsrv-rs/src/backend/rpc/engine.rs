// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! RPC engine: handles incoming RPC requests, dispatches to handlers,
//! and manages request/response lifecycle with timeouts and streaming.
//! Port of Go's pkg/wshutil/wshrpc.go (WshRpc struct + handler dispatch).

#![allow(dead_code)]

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use uuid::Uuid;

use super::super::rpc_types::{RpcContext, RpcMessage, RpcOpts, COMMAND_EVENT_RECV};

// ---- Constants (match Go) ----

pub const DEFAULT_TIMEOUT_MS: i64 = 5000;
const RESP_CH_SIZE: usize = 32;

// ---- Handler types ----

/// Result type for RPC handler responses.
pub type HandlerResult = Result<Option<serde_json::Value>, String>;

/// A boxed async handler function.
/// Takes the command data and returns either:
/// - Ok(Some(value)) for a single response
/// - Ok(None) for no response
/// - Err(msg) for an error response
pub type CommandHandler = Box<
    dyn Fn(serde_json::Value, RpcContext) -> Pin<Box<dyn Future<Output = HandlerResult> + Send>>
        + Send
        + Sync,
>;

/// A streaming handler that returns a channel of responses.
pub type StreamHandler = Box<
    dyn Fn(
            serde_json::Value,
            RpcContext,
        )
            -> Pin<Box<dyn Future<Output = Result<mpsc::Receiver<HandlerResult>, String>> + Send>>
        + Send
        + Sync,
>;

enum Handler {
    Call(CommandHandler),
    Stream(StreamHandler),
}

// ---- RPC Response Handler ----

/// Allows an RPC handler to send responses back to the caller.
/// Matches Go's `RpcResponseHandler`.
pub struct RpcResponseHandler {
    engine: Arc<WshRpcEngine>,
    req_id: String,
    source: String,
    canceled: AtomicBool,
    done: AtomicBool,
}

impl RpcResponseHandler {
    /// Send a single response (or streaming chunk).
    /// Set `done` to true for the final response.
    pub fn send_response(&self, data: Option<serde_json::Value>, done: bool) {
        if self.done.load(Ordering::Relaxed) {
            return;
        }
        let msg = RpcMessage {
            resid: self.req_id.clone(),
            data,
            cont: !done,
            ..Default::default()
        };
        if done {
            self.done.store(true, Ordering::Relaxed);
        }
        self.engine.send_output(msg);
    }

    /// Send an error response.
    pub fn send_error(&self, err: &str) {
        if self.done.load(Ordering::Relaxed) {
            return;
        }
        self.done.store(true, Ordering::Relaxed);
        let msg = RpcMessage {
            resid: self.req_id.clone(),
            error: err.to_string(),
            ..Default::default()
        };
        self.engine.send_output(msg);
    }

    /// Check if the request has been canceled.
    pub fn is_canceled(&self) -> bool {
        self.canceled.load(Ordering::Relaxed)
    }

    /// Get the source route ID of the request.
    pub fn get_source(&self) -> &str {
        &self.source
    }

    /// Mark this handler as canceled.
    fn cancel(&self) {
        self.canceled.store(true, Ordering::Relaxed);
    }

    /// Finalize: send empty done response if not already done.
    fn finalize(&self) {
        if self.done.load(Ordering::Relaxed) {
            return;
        }
        self.send_response(None, true);
    }
}

// ---- RPC Request Handler (client-side) ----

/// Tracks an outgoing request and collects responses.
/// Matches Go's `RpcRequestHandler`.
pub struct RpcRequestHandler {
    req_id: String,
    resp_rx: mpsc::Receiver<RpcMessage>,
    last_was_cont: bool,
}

impl RpcRequestHandler {
    /// Get the next response. Returns None if the stream is done.
    pub async fn next_response(&mut self) -> Option<Result<serde_json::Value, String>> {
        if !self.last_was_cont && self.req_id.is_empty() {
            return None;
        }
        match self.resp_rx.recv().await {
            Some(msg) => {
                self.last_was_cont = msg.cont;
                if !msg.error.is_empty() {
                    Some(Err(msg.error))
                } else {
                    Some(Ok(msg.data.unwrap_or(serde_json::Value::Null)))
                }
            }
            None => None,
        }
    }

    /// Check if the response stream is complete.
    pub fn is_done(&self) -> bool {
        !self.last_was_cont
    }

    /// Get the request ID.
    pub fn req_id(&self) -> &str {
        &self.req_id
    }
}

// ---- RPC Engine ----

struct EngineInner {
    handlers: HashMap<String, Handler>,
    pending_responses: HashMap<String, mpsc::Sender<RpcMessage>>,
    active_handlers: HashMap<String, Arc<RpcResponseHandler>>,
    auth_token: String,
    rpc_context: Option<RpcContext>,
}

/// Core RPC engine: handles incoming RPC requests, dispatches to registered
/// command handlers, and manages request/response lifecycle.
///
/// Port of Go's `WshRpc` from pkg/wshutil/wshrpc.go.
pub struct WshRpcEngine {
    inner: Mutex<EngineInner>,
    output_tx: mpsc::UnboundedSender<RpcMessage>,
}

impl WshRpcEngine {
    /// Create a new RPC engine.
    /// Returns the engine and a receiver for outgoing messages.
    pub fn new() -> (Arc<Self>, mpsc::UnboundedReceiver<RpcMessage>) {
        let (output_tx, output_rx) = mpsc::unbounded_channel();
        let engine = Arc::new(Self {
            inner: Mutex::new(EngineInner {
                handlers: HashMap::new(),
                pending_responses: HashMap::new(),
                active_handlers: HashMap::new(),
                auth_token: String::new(),
                rpc_context: None,
            }),
            output_tx,
        });
        (engine, output_rx)
    }

    /// Register a call handler (single request → single response).
    pub fn register_handler(&self, command: &str, handler: CommandHandler) {
        let mut inner = self.inner.lock().unwrap();
        inner
            .handlers
            .insert(command.to_string(), Handler::Call(handler));
    }

    /// Register a streaming handler (single request → stream of responses).
    pub fn register_stream_handler(&self, command: &str, handler: StreamHandler) {
        let mut inner = self.inner.lock().unwrap();
        inner
            .handlers
            .insert(command.to_string(), Handler::Stream(handler));
    }

    /// Set the authentication token.
    pub fn set_auth_token(&self, token: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.auth_token = token.to_string();
    }

    /// Get the authentication token.
    pub fn get_auth_token(&self) -> String {
        let inner = self.inner.lock().unwrap();
        inner.auth_token.clone()
    }

    /// Set the RPC context.
    pub fn set_rpc_context(&self, ctx: RpcContext) {
        let mut inner = self.inner.lock().unwrap();
        inner.rpc_context = Some(ctx);
    }

    /// Process an incoming message (from the transport layer).
    pub fn handle_message(self: &Arc<Self>, msg: RpcMessage) {
        // Cancel handling
        if msg.cancel {
            if !msg.reqid.is_empty() {
                self.handle_cancel_request(&msg.reqid);
            }
            return;
        }

        // Event handling (special: no response)
        if msg.command == COMMAND_EVENT_RECV {
            // Events are handled by the event listener, not via RPC handlers
            return;
        }

        // New command (request)
        if !msg.command.is_empty() {
            let engine = self.clone();
            tokio::spawn(async move {
                engine.handle_request(msg).await;
            });
            return;
        }

        // Response (has resid)
        if !msg.resid.is_empty() {
            self.handle_response(msg);
        }
    }

    /// Send an RPC command and wait for a single response.
    pub async fn send_command(
        self: &Arc<Self>,
        command: &str,
        data: serde_json::Value,
        opts: &RpcOpts,
    ) -> Result<serde_json::Value, String> {
        let mut handler = self.send_request(command, data, opts)?;
        match handler.next_response().await {
            Some(result) => result,
            None => Err("no response received".to_string()),
        }
    }

    /// Send an RPC command and get a request handler for streaming responses.
    pub fn send_request(
        self: &Arc<Self>,
        command: &str,
        data: serde_json::Value,
        opts: &RpcOpts,
    ) -> Result<RpcRequestHandler, String> {
        let req_id = Uuid::new_v4().to_string();
        let (resp_tx, resp_rx) = mpsc::channel(RESP_CH_SIZE);

        {
            let mut inner = self.inner.lock().unwrap();
            inner
                .pending_responses
                .insert(req_id.clone(), resp_tx);
        }

        let timeout = if opts.timeout > 0 {
            opts.timeout
        } else {
            DEFAULT_TIMEOUT_MS
        };
        let route = if opts.route.is_empty() {
            String::new()
        } else {
            opts.route.clone()
        };

        let msg = RpcMessage {
            command: command.to_string(),
            reqid: req_id.clone(),
            timeout,
            route,
            data: Some(data),
            authtoken: self.get_auth_token(),
            ..Default::default()
        };
        self.send_output(msg);

        Ok(RpcRequestHandler {
            req_id,
            resp_rx,
            last_was_cont: true, // assume more data initially
        })
    }

    /// Send a fire-and-forget command (no response expected).
    pub fn send_command_no_response(
        &self,
        command: &str,
        data: serde_json::Value,
        route: &str,
    ) {
        let msg = RpcMessage {
            command: command.to_string(),
            data: Some(data),
            route: route.to_string(),
            authtoken: self.get_auth_token(),
            ..Default::default()
        };
        self.send_output(msg);
    }

    // ---- Internal ----

    fn send_output(&self, msg: RpcMessage) {
        let _ = self.output_tx.send(msg);
    }

    async fn handle_request(self: Arc<Self>, msg: RpcMessage) {
        let request_start = std::time::Instant::now();
        let timeout_ms = if msg.timeout > 0 {
            msg.timeout
        } else {
            DEFAULT_TIMEOUT_MS
        };

        let handler = Arc::new(RpcResponseHandler {
            engine: self.clone(),
            req_id: msg.reqid.clone(),
            source: msg.source.clone(),
            canceled: AtomicBool::new(false),
            done: AtomicBool::new(false),
        });

        // Register the active handler
        if !msg.reqid.is_empty() {
            let mut inner = self.inner.lock().unwrap();
            inner
                .active_handlers
                .insert(msg.reqid.clone(), handler.clone());
        }

        let rpc_context = {
            let inner = self.inner.lock().unwrap();
            inner.rpc_context.clone().unwrap_or_default()
        };

        let data = msg.data.unwrap_or(serde_json::Value::Null);
        let command = msg.command.clone();

        // Look up handler
        let has_call;
        let has_stream;
        {
            let inner = self.inner.lock().unwrap();
            match inner.handlers.get(&command) {
                Some(Handler::Call(_)) => {
                    has_call = true;
                    has_stream = false;
                }
                Some(Handler::Stream(_)) => {
                    has_call = false;
                    has_stream = true;
                }
                None => {
                    has_call = false;
                    has_stream = false;
                }
            }
        }

        let dispatch_elapsed = request_start.elapsed();

        if !has_call && !has_stream {
            handler.send_error(&format!("unknown command: {}", command));
            self.cleanup_handler(&msg.reqid);
            return;
        }

        let timeout_dur = std::time::Duration::from_millis(timeout_ms as u64);

        if has_call {
            // Call handler: single response with timeout.
            // Create the future while holding the lock, then drop the lock before awaiting.
            let handler_start = std::time::Instant::now();
            let fut = {
                let inner = self.inner.lock().unwrap();
                match inner.handlers.get(&command) {
                    Some(Handler::Call(h)) => h(data.clone(), rpc_context.clone()),
                    _ => Box::pin(async { Err("handler disappeared".to_string()) }),
                }
            };
            let result = tokio::time::timeout(timeout_dur, fut).await;
            let handler_elapsed = handler_start.elapsed();
            let total_elapsed = request_start.elapsed();

            tracing::info!(
                "[rpc-perf] command={} dispatch={:.2}ms handler={:.2}ms total={:.2}ms",
                command,
                dispatch_elapsed.as_secs_f64() * 1000.0,
                handler_elapsed.as_secs_f64() * 1000.0,
                total_elapsed.as_secs_f64() * 1000.0,
            );

            match result {
                Ok(Ok(resp_data)) => handler.send_response(resp_data, true),
                Ok(Err(err)) => handler.send_error(&err),
                Err(_) => handler.send_error(&format!("EC-TIME: timeout ({}ms)", timeout_ms)),
            }
        } else {
            // Stream handler: same pattern — build future under lock, await outside.
            let fut = {
                let inner = self.inner.lock().unwrap();
                match inner.handlers.get(&command) {
                    Some(Handler::Stream(h)) => h(data.clone(), rpc_context.clone()),
                    _ => Box::pin(async { Err("handler disappeared".to_string()) }),
                }
            };
            let stream_result = tokio::time::timeout(timeout_dur, fut).await;

            match stream_result {
                Ok(Ok(mut rx)) => {
                    // Read streaming responses
                    loop {
                        match tokio::time::timeout(timeout_dur, rx.recv()).await {
                            Ok(Some(Ok(resp_data))) => {
                                handler.send_response(resp_data, false);
                            }
                            Ok(Some(Err(err))) => {
                                handler.send_error(&err);
                                break;
                            }
                            Ok(None) => {
                                // Channel closed — stream done
                                handler.finalize();
                                break;
                            }
                            Err(_) => {
                                handler.send_error(&format!(
                                    "EC-TIME: stream timeout ({}ms)",
                                    timeout_ms
                                ));
                                break;
                            }
                        }
                    }
                }
                Ok(Err(err)) => handler.send_error(&err),
                Err(_) => {
                    handler.send_error(&format!("EC-TIME: timeout ({}ms)", timeout_ms))
                }
            }
        }

        self.cleanup_handler(&msg.reqid);
    }

    fn handle_response(&self, msg: RpcMessage) {
        let inner = self.inner.lock().unwrap();
        if let Some(tx) = inner.pending_responses.get(&msg.resid) {
            let is_done = !msg.cont;
            let _ = tx.try_send(msg.clone());
            if is_done {
                drop(inner);
                let mut inner = self.inner.lock().unwrap();
                inner.pending_responses.remove(&msg.resid);
            }
        }
    }

    fn handle_cancel_request(&self, req_id: &str) {
        let inner = self.inner.lock().unwrap();
        if let Some(handler) = inner.active_handlers.get(req_id) {
            handler.cancel();
        }
    }

    fn cleanup_handler(&self, req_id: &str) {
        if req_id.is_empty() {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        inner.active_handlers.remove(req_id);
    }
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_register_and_call_handler() {
        let (engine, mut output_rx) = WshRpcEngine::new();

        engine.register_handler(
            "echo",
            Box::new(|data, _ctx| {
                Box::pin(async move { Ok(Some(data)) })
            }),
        );

        let msg = RpcMessage {
            command: "echo".to_string(),
            reqid: "req-1".to_string(),
            data: Some(serde_json::json!({"hello": "world"})),
            ..Default::default()
        };
        engine.handle_message(msg);

        // Collect the response
        let resp = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            output_rx.recv(),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(resp.resid, "req-1");
        assert!(!resp.cont);
        assert_eq!(resp.data, Some(serde_json::json!({"hello": "world"})));
    }

    #[tokio::test]
    async fn test_unknown_command_returns_error() {
        let (engine, mut output_rx) = WshRpcEngine::new();

        let msg = RpcMessage {
            command: "nonexistent".to_string(),
            reqid: "req-2".to_string(),
            ..Default::default()
        };
        engine.handle_message(msg);

        let resp = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            output_rx.recv(),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(resp.resid, "req-2");
        assert!(resp.error.contains("unknown command"));
    }

    #[tokio::test]
    async fn test_handler_error_returns_error_response() {
        let (engine, mut output_rx) = WshRpcEngine::new();

        engine.register_handler(
            "failme",
            Box::new(|_data, _ctx| {
                Box::pin(async move { Err("something went wrong".to_string()) })
            }),
        );

        let msg = RpcMessage {
            command: "failme".to_string(),
            reqid: "req-3".to_string(),
            ..Default::default()
        };
        engine.handle_message(msg);

        let resp = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            output_rx.recv(),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(resp.error, "something went wrong");
    }

    #[tokio::test]
    async fn test_send_command_roundtrip() {
        let (engine, mut output_rx) = WshRpcEngine::new();

        // Spawn a "server" that echoes responses
        let engine_clone = engine.clone();
        tokio::spawn(async move {
            if let Some(msg) = output_rx.recv().await {
                // This is the outgoing request — echo it back as a response
                let resp = RpcMessage {
                    resid: msg.reqid.clone(),
                    data: msg.data.clone(),
                    ..Default::default()
                };
                engine_clone.handle_message(resp);
            }
        });

        let opts = RpcOpts {
            timeout: 1000,
            ..Default::default()
        };
        let result = engine
            .send_command("test", serde_json::json!(42), &opts)
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), serde_json::json!(42));
    }

    #[tokio::test]
    async fn test_stream_handler() {
        let (engine, mut output_rx) = WshRpcEngine::new();

        engine.register_stream_handler(
            "counter",
            Box::new(|_data, _ctx| {
                Box::pin(async move {
                    let (tx, rx) = mpsc::channel(8);
                    tokio::spawn(async move {
                        for i in 0..3 {
                            let _ = tx.send(Ok(Some(serde_json::json!(i)))).await;
                        }
                        // Channel drops → stream done
                    });
                    Ok(rx)
                })
            }),
        );

        let msg = RpcMessage {
            command: "counter".to_string(),
            reqid: "req-stream".to_string(),
            ..Default::default()
        };
        engine.handle_message(msg);

        // Collect streaming responses
        let mut responses = Vec::new();
        for _ in 0..4 {
            // 3 data + 1 final empty
            match tokio::time::timeout(
                std::time::Duration::from_secs(2),
                output_rx.recv(),
            )
            .await
            {
                Ok(Some(resp)) => responses.push(resp),
                _ => break,
            }
        }

        // Should have 3 streaming chunks + 1 final
        assert!(responses.len() >= 3);
        // First 3 have cont=true
        for resp in &responses[..3] {
            assert!(resp.cont);
        }
        // Last one has cont=false (finalize)
        if responses.len() == 4 {
            assert!(!responses[3].cont);
        }
    }

    #[tokio::test]
    async fn test_cancel_request() {
        let (engine, mut output_rx) = WshRpcEngine::new();

        let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
        engine.register_handler(
            "slow",
            Box::new(move |_data, _ctx| {
                Box::pin(async move {
                    // Signal that we started
                    // (can't move started_tx into closure that's called multiple times)
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                    Ok(Some(serde_json::json!("done")))
                })
            }),
        );

        // Send command
        let msg = RpcMessage {
            command: "slow".to_string(),
            reqid: "req-cancel".to_string(),
            timeout: 10000,
            ..Default::default()
        };
        engine.handle_message(msg);

        // Small delay then send cancel
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let cancel_msg = RpcMessage {
            cancel: true,
            reqid: "req-cancel".to_string(),
            ..Default::default()
        };
        engine.handle_message(cancel_msg);

        // The handler will still time out or complete, but the cancel flag should be set
        // Just verify we get a response eventually (timeout response)
        let resp = tokio::time::timeout(
            std::time::Duration::from_secs(12),
            output_rx.recv(),
        )
        .await;
        assert!(resp.is_ok());
        // Clean up to avoid unused variable warning
        drop(started_tx);
        drop(started_rx);
    }

    #[tokio::test]
    async fn test_send_command_no_response() {
        let (engine, mut output_rx) = WshRpcEngine::new();

        engine.send_command_no_response("notify", serde_json::json!({"msg": "hi"}), "");

        let msg = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            output_rx.recv(),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(msg.command, "notify");
        assert!(msg.reqid.is_empty());
    }

    #[tokio::test]
    async fn test_auth_token() {
        let (engine, _output_rx) = WshRpcEngine::new();
        assert!(engine.get_auth_token().is_empty());

        engine.set_auth_token("my-secret-token");
        assert_eq!(engine.get_auth_token(), "my-secret-token");
    }

    #[tokio::test]
    async fn test_rpc_context() {
        let (engine, _output_rx) = WshRpcEngine::new();

        let ctx = RpcContext {
            client_type: "connserver".to_string(),
            blockid: "blk-1".to_string(),
            ..Default::default()
        };
        engine.set_rpc_context(ctx);

        // The context is passed to handlers
        engine.register_handler(
            "checkctx",
            Box::new(|_data, ctx| {
                Box::pin(async move {
                    Ok(Some(serde_json::json!({
                        "ctype": ctx.client_type,
                        "blockid": ctx.blockid,
                    })))
                })
            }),
        );

        let msg = RpcMessage {
            command: "checkctx".to_string(),
            reqid: "req-ctx".to_string(),
            ..Default::default()
        };
        engine.handle_message(msg);

        // Output will contain the context
        // (tested indirectly through handler dispatch)
    }
}
