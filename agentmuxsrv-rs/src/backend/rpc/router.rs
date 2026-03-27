// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! RPC message router: acts like a network switch, routing messages between
//! named endpoints (routes). Port of Go's pkg/wshutil/wshrouter.go.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Mutex;

use tokio::sync::mpsc;

use super::super::rpc_types::{
    CommandMessageData, RpcMessage, COMMAND_MESSAGE, COMMAND_ROUTE_ANNOUNCE,
    COMMAND_ROUTE_UNANNOUNCE,
};

// ---- Route constants ----

pub const DEFAULT_ROUTE: &str = "wavesrv";
pub const UPSTREAM_ROUTE: &str = "upstream";
pub const SYS_ROUTE: &str = "sys";
pub const TAURI_ROUTE: &str = "electron";

pub const ROUTE_PREFIX_CONN: &str = "conn:";
pub const ROUTE_PREFIX_CONTROLLER: &str = "controller:";
pub const ROUTE_PREFIX_PROC: &str = "proc:";
pub const ROUTE_PREFIX_TAB: &str = "tab:";
pub const ROUTE_PREFIX_FE_BLOCK: &str = "feblock:";

// ---- Route ID helpers (match Go) ----

pub fn make_connection_route_id(conn_id: &str) -> String {
    format!("conn:{}", conn_id)
}

pub fn make_controller_route_id(block_id: &str) -> String {
    format!("controller:{}", block_id)
}

pub fn make_proc_route_id(proc_id: &str) -> String {
    format!("proc:{}", proc_id)
}

pub fn make_tab_route_id(tab_id: &str) -> String {
    format!("tab:{}", tab_id)
}

pub fn make_fe_block_route_id(block_id: &str) -> String {
    format!("feblock:{}", block_id)
}

// ---- Route info tracking ----

#[derive(Debug, Clone)]
struct RouteInfo {
    source_route_id: String,
    dest_route_id: String,
}

struct MsgAndRoute {
    msg_bytes: Vec<u8>,
    from_route_id: String,
}

// ---- AbstractRpcClient trait ----

/// Trait for RPC message transport endpoints.
/// Matches Go's `AbstractRpcClient` interface.
pub trait RpcClient: Send + Sync {
    fn send_rpc_message(&self, msg: &[u8]);
}

// ---- Channel-based RPC client ----

/// An RPC client backed by a tokio mpsc channel.
/// Messages sent via `send_rpc_message` are forwarded to the receiver.
pub struct ChannelRpcClient {
    tx: mpsc::UnboundedSender<Vec<u8>>,
}

impl ChannelRpcClient {
    pub fn new() -> (Self, mpsc::UnboundedReceiver<Vec<u8>>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self { tx }, rx)
    }
}

impl RpcClient for ChannelRpcClient {
    fn send_rpc_message(&self, msg: &[u8]) {
        let _ = self.tx.send(msg.to_vec());
    }
}

// ---- Router inner state ----

struct RouterInner {
    route_map: HashMap<String, Box<dyn RpcClient>>,
    announced_routes: HashMap<String, String>, // remote route -> local route
    rpc_map: HashMap<String, RouteInfo>,       // rpcid -> route info
    simple_request_map: HashMap<String, tokio::sync::oneshot::Sender<RpcMessage>>,
}

/// RPC message router. Acts like a network switch, routing messages between
/// multiple named endpoints based on route IDs.
///
/// Port of Go's `WshRouter` from pkg/wshutil/wshrouter.go.
pub struct WshRouter {
    inner: Mutex<RouterInner>,
    input_tx: mpsc::UnboundedSender<MsgAndRoute>,
}

impl WshRouter {
    /// Create a new router and spawn the background message loop.
    pub fn new() -> std::sync::Arc<Self> {
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        let router = std::sync::Arc::new(Self {
            inner: Mutex::new(RouterInner {
                route_map: HashMap::new(),
                announced_routes: HashMap::new(),
                rpc_map: HashMap::new(),
                simple_request_map: HashMap::new(),
            }),
            input_tx,
        });
        let router_clone = router.clone();
        tokio::spawn(async move {
            router_clone.run_server(input_rx).await;
        });
        router
    }

    /// Inject a message into the router from a given route.
    pub fn inject_message(&self, msg_bytes: Vec<u8>, from_route_id: &str) {
        let _ = self.input_tx.send(MsgAndRoute {
            msg_bytes,
            from_route_id: from_route_id.to_string(),
        });
    }

    /// Register a route with the router.
    pub fn register_route(&self, route_id: &str, client: Box<dyn RpcClient>) {
        if route_id == SYS_ROUTE || route_id == UPSTREAM_ROUTE {
            tracing::error!("WshRouter cannot register {} route", route_id);
            return;
        }
        tracing::info!("[router] registering wsh route {:?}", route_id);
        let mut inner = self.inner.lock().unwrap();
        if inner.route_map.contains_key(route_id) {
            tracing::warn!("[router] route {:?} already exists (replacing)", route_id);
        }
        inner.route_map.insert(route_id.to_string(), client);
    }

    /// Unregister a route from the router.
    pub fn unregister_route(&self, route_id: &str) {
        tracing::info!("[router] unregistering wsh route {:?}", route_id);
        let mut inner = self.inner.lock().unwrap();
        inner.route_map.remove(route_id);
        inner
            .announced_routes
            .retain(|_, local_id| local_id != route_id);
    }

    /// Check if a route is registered.
    pub fn has_route(&self, route_id: &str) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.route_map.contains_key(route_id)
    }

    /// Get all registered route IDs.
    pub fn route_ids(&self) -> Vec<String> {
        let inner = self.inner.lock().unwrap();
        inner.route_map.keys().cloned().collect()
    }

    /// Wait for a route to be registered (with timeout).
    pub async fn wait_for_register(
        &self,
        route_id: &str,
        timeout: std::time::Duration,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            {
                let inner = self.inner.lock().unwrap();
                if inner.route_map.contains_key(route_id)
                    || inner.announced_routes.contains_key(route_id)
                {
                    return true;
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
    }

    /// Run a simple request through the router and wait for a single response.
    pub async fn run_simple_raw_command(
        &self,
        msg: RpcMessage,
        from_route_id: &str,
        timeout: std::time::Duration,
    ) -> Result<Option<RpcMessage>, String> {
        if msg.command.is_empty() {
            return Err("no command".to_string());
        }
        let msg_bytes =
            serde_json::to_vec(&msg).map_err(|e| format!("marshal error: {}", e))?;
        let rx = if !msg.reqid.is_empty() {
            Some(self.register_simple_request(&msg.reqid))
        } else {
            None
        };
        self.inject_message(msg_bytes, from_route_id);
        match rx {
            None => Ok(None),
            Some(rx) => {
                tokio::select! {
                    resp = rx => {
                        match resp {
                            Ok(resp) => {
                                if !resp.error.is_empty() {
                                    Err(resp.error)
                                } else {
                                    Ok(Some(resp))
                                }
                            }
                            Err(_) => Err("request cancelled".to_string()),
                        }
                    }
                    _ = tokio::time::sleep(timeout) => {
                        self.clear_simple_request(&msg.reqid);
                        Err("timeout".to_string())
                    }
                }
            }
        }
    }

    // ---- Internal methods ----

    fn register_route_info(
        &self,
        rpc_id: &str,
        source_route_id: &str,
        dest_route_id: &str,
    ) {
        if rpc_id.is_empty() {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        inner.rpc_map.insert(
            rpc_id.to_string(),
            RouteInfo {
                source_route_id: source_route_id.to_string(),
                dest_route_id: dest_route_id.to_string(),
            },
        );
    }

    fn unregister_route_info(&self, rpc_id: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.rpc_map.remove(rpc_id);
    }

    fn get_route_info(&self, rpc_id: &str) -> Option<RouteInfo> {
        let inner = self.inner.lock().unwrap();
        inner.rpc_map.get(rpc_id).cloned()
    }

    fn send_routed_message(&self, msg_bytes: &[u8], route_id: &str) -> bool {
        let inner = self.inner.lock().unwrap();
        if let Some(rpc) = inner.route_map.get(route_id) {
            rpc.send_rpc_message(msg_bytes);
            return true;
        }
        // Try announced routes
        if let Some(local_route) = inner.announced_routes.get(route_id) {
            if let Some(rpc) = inner.route_map.get(local_route.as_str()) {
                rpc.send_rpc_message(msg_bytes);
                return true;
            }
        }
        false
    }

    fn handle_no_route(&self, msg: &RpcMessage) {
        let err_msg = if msg.route.is_empty() {
            "no default route".to_string()
        } else {
            format!("no route for {:?}", msg.route)
        };
        if msg.reqid.is_empty() {
            // No response needed, but send message back to source
            if msg.command == COMMAND_MESSAGE {
                return; // prevent infinite loops
            }
            let resp = RpcMessage {
                command: COMMAND_MESSAGE.to_string(),
                route: msg.source.clone(),
                data: serde_json::to_value(CommandMessageData {
                    oref: Default::default(),
                    message: err_msg,
                })
                .ok(),
                ..Default::default()
            };
            if let Ok(resp_bytes) = serde_json::to_vec(&resp) {
                let _ = self.input_tx.send(MsgAndRoute {
                    msg_bytes: resp_bytes,
                    from_route_id: SYS_ROUTE.to_string(),
                });
            }
            return;
        }
        // Send error response back to source
        let response = RpcMessage {
            resid: msg.reqid.clone(),
            error: err_msg,
            ..Default::default()
        };
        if let Ok(resp_bytes) = serde_json::to_vec(&response) {
            self.send_routed_message(&resp_bytes, &msg.source);
        }
    }

    fn handle_announce_message(&self, msg: &RpcMessage, from_route_id: &str) {
        if msg.source == from_route_id {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        inner
            .announced_routes
            .insert(msg.source.clone(), from_route_id.to_string());
    }

    fn handle_unannounce_message(&self, msg: &RpcMessage) {
        let mut inner = self.inner.lock().unwrap();
        inner.announced_routes.remove(&msg.source);
    }

    fn register_simple_request(
        &self,
        req_id: &str,
    ) -> tokio::sync::oneshot::Receiver<RpcMessage> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut inner = self.inner.lock().unwrap();
        inner
            .simple_request_map
            .insert(req_id.to_string(), tx);
        rx
    }

    fn try_simple_response(&self, msg: &RpcMessage) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if let Some(tx) = inner.simple_request_map.remove(&msg.resid) {
            let _ = tx.send(msg.clone());
            return true;
        }
        false
    }

    fn clear_simple_request(&self, req_id: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.simple_request_map.remove(req_id);
    }

    async fn run_server(&self, mut input_rx: mpsc::UnboundedReceiver<MsgAndRoute>) {
        while let Some(input) = input_rx.recv().await {
            let msg: RpcMessage = match serde_json::from_slice(&input.msg_bytes) {
                Ok(m) => m,
                Err(e) => {
                    tracing::error!("error unmarshalling message: {}", e);
                    continue;
                }
            };

            // Handle route announce/unannounce
            if msg.command == COMMAND_ROUTE_ANNOUNCE {
                self.handle_announce_message(&msg, &input.from_route_id);
                continue;
            }
            if msg.command == COMMAND_ROUTE_UNANNOUNCE {
                self.handle_unannounce_message(&msg);
                continue;
            }

            // New command — route to destination
            if !msg.command.is_empty() {
                let route_id = if msg.route.is_empty() {
                    DEFAULT_ROUTE.to_string()
                } else {
                    msg.route.clone()
                };
                let ok = self.send_routed_message(&input.msg_bytes, &route_id);
                if !ok {
                    self.handle_no_route(&msg);
                    continue;
                }
                self.register_route_info(&msg.reqid, &msg.source, &route_id);
                continue;
            }

            // Follow-up request (has reqid, routed to dest)
            if !msg.reqid.is_empty() {
                if let Some(info) = self.get_route_info(&msg.reqid) {
                    self.send_routed_message(&input.msg_bytes, &info.dest_route_id);
                }
                continue;
            }

            // Response (has resid, routed back to source)
            if !msg.resid.is_empty() {
                if self.try_simple_response(&msg) {
                    continue;
                }
                if let Some(info) = self.get_route_info(&msg.resid) {
                    self.send_routed_message(&input.msg_bytes, &info.source_route_id);
                    if !msg.cont {
                        self.unregister_route_info(&msg.resid);
                    }
                }
                continue;
            }

            // Bad message (no command, reqid, or resid) — drop it
        }
    }
}

impl Default for WshRouter {
    fn default() -> Self {
        // Note: default() creates an unstarted router (no tokio runtime).
        // Use WshRouter::new() to get a properly started router.
        let (input_tx, _) = mpsc::unbounded_channel();
        Self {
            inner: Mutex::new(RouterInner {
                route_map: HashMap::new(),
                announced_routes: HashMap::new(),
                rpc_map: HashMap::new(),
                simple_request_map: HashMap::new(),
            }),
            input_tx,
        }
    }
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_route_id_helpers() {
        assert_eq!(make_connection_route_id("myconn"), "conn:myconn");
        assert_eq!(make_controller_route_id("blk-1"), "controller:blk-1");
        assert_eq!(make_proc_route_id("p-1"), "proc:p-1");
        assert_eq!(make_tab_route_id("tab-1"), "tab:tab-1");
        assert_eq!(make_fe_block_route_id("blk-2"), "feblock:blk-2");
    }

    /// Helper: a test client that collects messages
    struct CollectorClient {
        messages: std::sync::Mutex<Vec<Vec<u8>>>,
    }

    impl CollectorClient {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                messages: std::sync::Mutex::new(Vec::new()),
            })
        }

        fn received_messages(&self) -> Vec<RpcMessage> {
            let msgs = self.messages.lock().unwrap();
            msgs.iter()
                .filter_map(|b| serde_json::from_slice(b).ok())
                .collect()
        }
    }

    impl RpcClient for Arc<CollectorClient> {
        fn send_rpc_message(&self, msg: &[u8]) {
            self.messages.lock().unwrap().push(msg.to_vec());
        }
    }

    #[tokio::test]
    async fn test_register_unregister_route() {
        let router = WshRouter::new();
        let client = CollectorClient::new();
        router.register_route("test-route", Box::new(client.clone()));
        assert!(router.has_route("test-route"));
        assert!(!router.has_route("other-route"));

        router.unregister_route("test-route");
        assert!(!router.has_route("test-route"));
    }

    #[tokio::test]
    async fn test_route_command_to_destination() {
        let router = WshRouter::new();
        let server = CollectorClient::new();
        router.register_route(DEFAULT_ROUTE, Box::new(server.clone()));

        let msg = RpcMessage {
            command: "getmeta".to_string(),
            reqid: "req-1".to_string(),
            source: "tab:t1".to_string(),
            ..Default::default()
        };
        let msg_bytes = serde_json::to_vec(&msg).unwrap();
        router.inject_message(msg_bytes, "tab:t1");

        // Give the async server loop time to process
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let received = server.received_messages();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].command, "getmeta");
    }

    #[tokio::test]
    async fn test_route_response_back_to_source() {
        let router = WshRouter::new();

        let server = CollectorClient::new();
        let tab_client = CollectorClient::new();

        router.register_route(DEFAULT_ROUTE, Box::new(server.clone()));
        router.register_route("tab:t1", Box::new(tab_client.clone()));

        // Step 1: Send command from tab to server
        let cmd = RpcMessage {
            command: "getmeta".to_string(),
            reqid: "req-1".to_string(),
            source: "tab:t1".to_string(),
            ..Default::default()
        };
        router.inject_message(serde_json::to_vec(&cmd).unwrap(), "tab:t1");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Step 2: Server sends response
        let resp = RpcMessage {
            resid: "req-1".to_string(),
            data: Some(serde_json::json!({"view": "term"})),
            ..Default::default()
        };
        router.inject_message(serde_json::to_vec(&resp).unwrap(), DEFAULT_ROUTE);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Tab should have received the response
        let tab_msgs = tab_client.received_messages();
        assert_eq!(tab_msgs.len(), 1);
        assert_eq!(tab_msgs[0].resid, "req-1");
    }

    #[tokio::test]
    async fn test_streaming_response_cont_flag() {
        let router = WshRouter::new();
        let server = CollectorClient::new();
        let tab = CollectorClient::new();

        router.register_route(DEFAULT_ROUTE, Box::new(server.clone()));
        router.register_route("tab:t1", Box::new(tab.clone()));

        // Command
        let cmd = RpcMessage {
            command: "filereadstream".to_string(),
            reqid: "req-stream".to_string(),
            source: "tab:t1".to_string(),
            ..Default::default()
        };
        router.inject_message(serde_json::to_vec(&cmd).unwrap(), "tab:t1");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Streaming response 1 (cont=true)
        let resp1 = RpcMessage {
            resid: "req-stream".to_string(),
            cont: true,
            data: Some(serde_json::json!({"chunk": 1})),
            ..Default::default()
        };
        router.inject_message(serde_json::to_vec(&resp1).unwrap(), DEFAULT_ROUTE);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Streaming response 2 (cont=false, final)
        let resp2 = RpcMessage {
            resid: "req-stream".to_string(),
            cont: false,
            data: Some(serde_json::json!({"chunk": 2})),
            ..Default::default()
        };
        router.inject_message(serde_json::to_vec(&resp2).unwrap(), DEFAULT_ROUTE);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let msgs = tab.received_messages();
        assert_eq!(msgs.len(), 2);
        assert!(msgs[0].cont);
        assert!(!msgs[1].cont);
    }

    #[tokio::test]
    async fn test_no_route_returns_error() {
        let router = WshRouter::new();
        let tab = CollectorClient::new();
        router.register_route("tab:t1", Box::new(tab.clone()));

        // Send command to nonexistent route
        let cmd = RpcMessage {
            command: "getmeta".to_string(),
            reqid: "req-err".to_string(),
            source: "tab:t1".to_string(),
            route: "nonexistent".to_string(),
            ..Default::default()
        };
        router.inject_message(serde_json::to_vec(&cmd).unwrap(), "tab:t1");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let msgs = tab.received_messages();
        assert_eq!(msgs.len(), 1);
        assert!(!msgs[0].error.is_empty());
        assert!(msgs[0].error.contains("no route"));
    }

    #[tokio::test]
    async fn test_announced_routes() {
        let router = WshRouter::new();
        let proxy = CollectorClient::new();
        router.register_route("proxy-1", Box::new(proxy.clone()));

        // Announce that "conn:myhost" is reachable via "proxy-1"
        let announce = RpcMessage {
            command: COMMAND_ROUTE_ANNOUNCE.to_string(),
            source: "conn:myhost".to_string(),
            ..Default::default()
        };
        router.inject_message(serde_json::to_vec(&announce).unwrap(), "proxy-1");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Send a command to "conn:myhost" — should be routed via proxy-1
        let cmd = RpcMessage {
            command: "test".to_string(),
            route: "conn:myhost".to_string(),
            source: "sys".to_string(),
            ..Default::default()
        };
        router.inject_message(serde_json::to_vec(&cmd).unwrap(), "sys");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let msgs = proxy.received_messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].command, "test");
    }

    #[tokio::test]
    async fn test_simple_raw_command() {
        let router = WshRouter::new();
        let server = CollectorClient::new();
        router.register_route(DEFAULT_ROUTE, Box::new(server.clone()));

        // Spawn a task to respond
        let router_clone = router.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            let resp = RpcMessage {
                resid: "simple-req".to_string(),
                data: Some(serde_json::json!({"ok": true})),
                ..Default::default()
            };
            router_clone.inject_message(
                serde_json::to_vec(&resp).unwrap(),
                DEFAULT_ROUTE,
            );
        });

        let cmd = RpcMessage {
            command: "test".to_string(),
            reqid: "simple-req".to_string(),
            ..Default::default()
        };
        let result = router
            .run_simple_raw_command(
                cmd,
                "sys",
                std::time::Duration::from_secs(1),
            )
            .await;
        assert!(result.is_ok());
        let resp = result.unwrap().unwrap();
        assert_eq!(resp.data, Some(serde_json::json!({"ok": true})));
    }

    #[tokio::test]
    async fn test_wait_for_register() {
        let router = WshRouter::new();

        // Spawn delayed registration
        let router_clone = router.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let client = CollectorClient::new();
            router_clone.register_route("delayed-route", Box::new(client));
        });

        let found = router
            .wait_for_register("delayed-route", std::time::Duration::from_secs(1))
            .await;
        assert!(found);

        // Non-existent route with short timeout
        let not_found = router
            .wait_for_register("never-route", std::time::Duration::from_millis(50))
            .await;
        assert!(!not_found);
    }
}
