// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Event bus: WebSocket event dispatching to connected clients.
//! Port of Go's pkg/eventbus/eventbus.go.


use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::wps::{WaveEvent, WpsClient};

// ---- Event type constants ----

pub const WS_EVENT_TAURI_NEW_WINDOW: &str = "electron:newwindow";
pub const WS_EVENT_TAURI_CLOSE_WINDOW: &str = "electron:closewindow";
pub const WS_EVENT_TAURI_UPDATE_ACTIVE_TAB: &str = "electron:updateactivetab";
pub const WS_EVENT_RPC: &str = "rpc";

// ---- Types ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WSEventType {
    pub eventtype: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub oref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

struct WindowWatchData {
    sender: tokio::sync::mpsc::UnboundedSender<serde_json::Value>,
    tab_id: String,
}

/// Global event bus for dispatching WebSocket events to connected clients.
pub struct EventBus {
    watches: Mutex<HashMap<String, WindowWatchData>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            watches: Mutex::new(HashMap::new()),
        }
    }

    /// Register a WebSocket connection for receiving events.
    /// Returns a receiver channel for the connection.
    pub fn register_ws(
        &self,
        conn_id: &str,
        tab_id: &str,
    ) -> tokio::sync::mpsc::UnboundedReceiver<serde_json::Value> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut watches = self.watches.lock().unwrap();
        watches.insert(
            conn_id.to_string(),
            WindowWatchData {
                sender: tx,
                tab_id: tab_id.to_string(),
            },
        );
        rx
    }

    /// Unregister a WebSocket connection.
    pub fn unregister_ws(&self, conn_id: &str) {
        let mut watches = self.watches.lock().unwrap();
        watches.remove(conn_id);
    }

    /// Check if any connections exist for a given window/tab ID.
    pub fn has_connections_for(&self, tab_id: &str) -> bool {
        let watches = self.watches.lock().unwrap();
        watches.values().any(|w| w.tab_id == tab_id)
    }

    /// Wait for a connection to appear for the given tab_id (with timeout).
    pub async fn wait_for_connection(
        &self,
        tab_id: &str,
        timeout: std::time::Duration,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.has_connections_for(tab_id) {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    /// Send an event to all connected WebSocket clients.
    pub fn broadcast_event(&self, event: &WSEventType) {
        let data = match serde_json::to_value(event) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("cannot marshal event: {}", e);
                return;
            }
        };
        let watches = self.watches.lock().unwrap();
        for (conn_id, watch) in watches.iter() {
            if watch.sender.send(data.clone()).is_err() {
                tracing::warn!("failed to send event to conn {}", conn_id);
            }
        }
    }

    /// Send an event to connections matching a specific tab_id.
    pub fn send_to_tab(&self, tab_id: &str, event: &WSEventType) {
        let data = match serde_json::to_value(event) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("cannot marshal event: {}", e);
                return;
            }
        };
        let watches = self.watches.lock().unwrap();
        for watch in watches.values() {
            if watch.tab_id == tab_id {
                let _ = watch.sender.send(data.clone());
            }
        }
    }

    /// Get the number of active connections.
    pub fn connection_count(&self) -> usize {
        self.watches.lock().unwrap().len()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// Bridge from WPS Broker to EventBus.
/// Wraps WaveEvents as RPC eventrecv messages and broadcasts them to all WS clients.
pub struct EventBusBridge {
    event_bus: Arc<EventBus>,
}

impl EventBusBridge {
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self { event_bus }
    }
}

impl WpsClient for EventBusBridge {
    fn send_event(&self, _route_id: &str, event: WaveEvent) {
        // Wrap as RPC eventrecv message (format expected by frontend)
        let ws_event = WSEventType {
            eventtype: WS_EVENT_RPC.to_string(),
            oref: String::new(),
            data: Some(serde_json::json!({
                "command": "eventrecv",
                "data": event
            })),
        };
        self.event_bus.broadcast_event(&ws_event);
    }
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_unregister() {
        let bus = EventBus::new();
        let _rx = bus.register_ws("conn-1", "tab-1");
        assert_eq!(bus.connection_count(), 1);
        assert!(bus.has_connections_for("tab-1"));
        assert!(!bus.has_connections_for("tab-2"));

        bus.unregister_ws("conn-1");
        assert_eq!(bus.connection_count(), 0);
        assert!(!bus.has_connections_for("tab-1"));
    }

    #[test]
    fn test_broadcast_event() {
        let bus = EventBus::new();
        let mut rx1 = bus.register_ws("conn-1", "tab-1");
        let mut rx2 = bus.register_ws("conn-2", "tab-2");

        let event = WSEventType {
            eventtype: WS_EVENT_RPC.to_string(),
            oref: String::new(),
            data: Some(serde_json::json!({"test": true})),
        };
        bus.broadcast_event(&event);

        assert!(rx1.try_recv().is_ok());
        assert!(rx2.try_recv().is_ok());
    }

    #[test]
    fn test_send_to_tab() {
        let bus = EventBus::new();
        let mut rx1 = bus.register_ws("conn-1", "tab-1");
        let mut rx2 = bus.register_ws("conn-2", "tab-2");

        let event = WSEventType {
            eventtype: WS_EVENT_RPC.to_string(),
            oref: String::new(),
            data: None,
        };
        bus.send_to_tab("tab-1", &event);

        assert!(rx1.try_recv().is_ok());
        assert!(rx2.try_recv().is_err()); // tab-2 should not receive
    }

    #[test]
    fn test_ws_event_serialization() {
        let event = WSEventType {
            eventtype: "test".to_string(),
            oref: String::new(),
            data: Some(serde_json::json!(42)),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"eventtype\":\"test\""));
        // Empty oref should be omitted
        assert!(!json.contains("\"oref\""));
    }
}
