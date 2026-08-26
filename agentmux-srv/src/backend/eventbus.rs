// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Event bus: WebSocket event dispatching to connected clients.
//! Port of Go's pkg/eventbus/eventbus.go.


use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::wps::{WaveEvent, WpsClient, EVENT_SYS_INFO, EVENT_BLOCK_STATS, EVENT_BLOCK_FILE};

// ---- Event type constants ----

pub const WS_EVENT_RPC: &str = "rpc";

/// One WS frame carrying an ARRAY of `WaveObjUpdate`s from a single atomic
/// backend transition. Mirrored in `frontend/app/store/wps-events.ts`
/// (`WpsEvent.WaveObjBatchedUpdates`) — the frontend applies the whole array
/// in one Solid `batch()` flush. See `broadcast_wave_obj_updates` below.
pub const WS_EVENT_WAVE_OBJ_BATCHED_UPDATES: &str = "waveobj:batchedupdates";

/// Egress priority lane for a server→client event.
///
/// `Background` is reserved for droppable perf telemetry (sysinfo + per-block
/// stats) that must never delay interactive terminal I/O; everything else is
/// `Priority`. The WebSocket egress loop drains the priority lane before the
/// background lane (see `server/websocket.rs` and
/// `docs/specs/SPEC_TERMINAL_INPUT_PRIORITY_OVER_SYSINFO_2026_06_16.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    Priority,
    Background,
}

/// Egress channel capacities. Bounded (not unbounded) so a stalled/hidden-
/// renderer WS consumer can't grow sidecar commit without limit — a healthy
/// connection drains far below these depths; hitting the cap is itself the
/// signal that the consumer isn't keeping up. See
/// docs/specs/SPEC_MEMORY_COMMIT_ATTRIBUTION_CORRECTION_2026_07_02.md §B.2.
///
/// Priority carries terminal echo + interactive events, which must not be
/// silently dropped in normal operation — sized generously so legitimate
/// bursts (e.g. `cat` of a large file) never hit the cap.
const PRIORITY_LANE_CAPACITY: usize = 8192;
/// Background carries droppable perf telemetry (sysinfo ~1/s, per-block
/// stats) already coalesced-to-latest by the receive loop — a healthy
/// consumer never queues more than a couple of entries.
const BACKGROUND_LANE_CAPACITY: usize = 256;

/// The pair of receivers handed to a WebSocket connection on registration.
/// Terminal echo + interactive events arrive on `priority`; perf telemetry on
/// `background`.
pub struct WsReceivers {
    pub priority: tokio::sync::mpsc::Receiver<serde_json::Value>,
    pub background: tokio::sync::mpsc::Receiver<serde_json::Value>,
}

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
    /// Interactive lane: terminal echo, RPC-routed wave events, obj updates.
    priority: tokio::sync::mpsc::Sender<serde_json::Value>,
    /// Background lane: droppable perf telemetry (sysinfo, blockstats).
    background: tokio::sync::mpsc::Sender<serde_json::Value>,
    #[allow(dead_code)]
    tab_id: String,
}

impl WindowWatchData {
    fn sender(&self, lane: Lane) -> &tokio::sync::mpsc::Sender<serde_json::Value> {
        match lane {
            Lane::Priority => &self.priority,
            Lane::Background => &self.background,
        }
    }
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
    /// Returns the priority + background receiver pair for the connection.
    pub fn register_ws(&self, conn_id: &str, tab_id: &str) -> WsReceivers {
        let (priority_tx, priority_rx) = tokio::sync::mpsc::channel(PRIORITY_LANE_CAPACITY);
        let (background_tx, background_rx) = tokio::sync::mpsc::channel(BACKGROUND_LANE_CAPACITY);
        let mut watches = self.watches.lock().unwrap();
        watches.insert(
            conn_id.to_string(),
            WindowWatchData {
                priority: priority_tx,
                background: background_tx,
                tab_id: tab_id.to_string(),
            },
        );
        WsReceivers {
            priority: priority_rx,
            background: background_rx,
        }
    }

    /// Unregister a WebSocket connection.
    pub fn unregister_ws(&self, conn_id: &str) {
        let mut watches = self.watches.lock().unwrap();
        watches.remove(conn_id);
    }

    /// Check if any connections exist for a given window/tab ID.
    #[allow(dead_code)]
    pub fn has_connections_for(&self, tab_id: &str) -> bool {
        let watches = self.watches.lock().unwrap();
        watches.values().any(|w| w.tab_id == tab_id)
    }

    /// Wait for a connection to appear for the given tab_id (with timeout).
    #[allow(dead_code)]
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

    /// Send an event to a single connection by conn_id on a specific lane.
    /// No-op if not found.
    pub fn send_to_conn_lane(&self, conn_id: &str, event: &WSEventType, lane: Lane) {
        let data = match serde_json::to_value(event) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("cannot marshal event: {}", e);
                return;
            }
        };
        let watches = self.watches.lock().unwrap();
        if let Some(watch) = watches.get(conn_id) {
            Self::try_send_lane(conn_id, watch.sender(lane), data);
        }
    }

    /// Broadcast an event to all connected WebSocket clients, on the priority lane.
    pub fn broadcast_event(&self, event: &WSEventType) {
        self.broadcast_event_lane(event, Lane::Priority);
    }

    /// Broadcast a set of related `WaveObjUpdate`s from ONE atomic backend
    /// transition as ONE `waveobj:batchedupdates` WS frame, so the frontend
    /// applies them in a single reactive flush (`updateWaveObjects`'s
    /// `batch()`), preserving the array's order.
    ///
    /// Why one frame instead of N `waveobj:update` frames: each frame is
    /// dispatched (and painted) independently by the renderer, so a
    /// `[delete tab, update workspace]` pair sent as two frames blanks the
    /// still-mounted tab a full paint before the workspace update unmounts
    /// it — the same flash class
    /// `SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25.md` §6 fixed at the
    /// RPC-response layer, riding in unbatched through the WS push path
    /// instead (§7). Single-update callers (blockcontroller, setmeta, the
    /// wave-obj bridge) keep emitting plain `waveobj:update` — there is
    /// nothing to batch there.
    pub fn broadcast_wave_obj_updates(&self, updates: &[super::obj::WaveObjUpdate]) {
        if updates.is_empty() {
            return;
        }
        let data = match serde_json::to_value(updates) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("cannot marshal waveobj batched updates: {}", e);
                return;
            }
        };
        self.broadcast_event(&WSEventType {
            eventtype: WS_EVENT_WAVE_OBJ_BATCHED_UPDATES.to_string(),
            oref: String::new(),
            data: Some(data),
        });
    }

    /// Broadcast an event to all connected WebSocket clients on a specific lane.
    pub fn broadcast_event_lane(&self, event: &WSEventType, lane: Lane) {
        let data = match serde_json::to_value(event) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("cannot marshal event: {}", e);
                return;
            }
        };
        let watches = self.watches.lock().unwrap();
        for (conn_id, watch) in watches.iter() {
            Self::try_send_lane(conn_id, watch.sender(lane), data.clone());
        }
    }

    /// Non-blocking send with distinct logging for a genuinely-full lane
    /// (stalled/hidden-renderer consumer — see `PRIORITY_LANE_CAPACITY` /
    /// `BACKGROUND_LANE_CAPACITY`) vs. a closed one (connection already
    /// gone). Never blocks the caller — callers hold the `watches` lock.
    fn try_send_lane(
        conn_id: &str,
        sender: &tokio::sync::mpsc::Sender<serde_json::Value>,
        data: serde_json::Value,
    ) {
        match sender.try_send(data) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                tracing::warn!(
                    "ws egress lane full for conn {} — consumer stalled, dropping event",
                    conn_id
                );
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                tracing::warn!("failed to send event to conn {}", conn_id);
            }
        }
    }

    /// Send an event to connections matching a specific tab_id.
    #[allow(dead_code)]
    pub fn send_to_tab(&self, tab_id: &str, event: &WSEventType) {
        let data = match serde_json::to_value(event) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("cannot marshal event: {}", e);
                return;
            }
        };
        let watches = self.watches.lock().unwrap();
        for (conn_id, watch) in watches.iter() {
            if watch.tab_id == tab_id {
                Self::try_send_lane(conn_id, watch.sender(Lane::Priority), data.clone());
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
    fn send_event(&self, route_id: &str, event: WaveEvent) {
        // Perf telemetry is droppable and must never delay interactive terminal
        // I/O, so route sysinfo + per-block stats to the background lane and
        // everything else to the priority lane. This is the only place the raw
        // WaveEvent type is visible before it's wrapped as an opaque RPC
        // envelope. See SPEC_TERMINAL_INPUT_PRIORITY_OVER_SYSINFO_2026_06_16.
        let lane = match event.event.as_str() {
            EVENT_SYS_INFO | EVENT_BLOCK_STATS => Lane::Background,
            _ => Lane::Priority,
        };
        // Wrap as RPC eventrecv message (format expected by frontend)
        let ws_event = WSEventType {
            eventtype: WS_EVENT_RPC.to_string(),
            oref: String::new(),
            data: Some(serde_json::json!({
                "command": "eventrecv",
                "data": event
            })),
        };
        // Route to the specific connection that subscribed. Broadcast is used
        // only for legacy callers that pass "ws-main" (none remain after the
        // per-conn-id fix), so this always takes the targeted path in practice.
        if route_id == "ws-main" {
            self.event_bus.broadcast_event_lane(&ws_event, lane);
        } else {
            self.event_bus.send_to_conn_lane(route_id, &ws_event, lane);
        }
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

        // Default broadcast lands on the priority lane.
        assert!(rx1.priority.try_recv().is_ok());
        assert!(rx2.priority.try_recv().is_ok());
        assert!(rx1.background.try_recv().is_err());
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

        assert!(rx1.priority.try_recv().is_ok());
        assert!(rx2.priority.try_recv().is_err()); // tab-2 should not receive
    }

    /// A multi-object atomic transition must go out as ONE
    /// `waveobj:batchedupdates` frame (array payload, order preserved), not
    /// N frames — N frames repaint the renderer in N unbatched steps (the
    /// SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25.md §7 flash).
    #[test]
    fn test_broadcast_wave_obj_updates_single_batched_frame() {
        use crate::backend::obj::WaveObjUpdate;

        let bus = EventBus::new();
        let mut rx = bus.register_ws("conn-1", "tab-1");

        let updates = vec![
            WaveObjUpdate {
                updatetype: "update".into(),
                otype: "workspace".into(),
                oid: "ws-1".into(),
                obj: Some(serde_json::json!({"oid": "ws-1"})),
            },
            WaveObjUpdate {
                updatetype: "delete".into(),
                otype: "tab".into(),
                oid: "tab-9".into(),
                obj: None,
            },
        ];
        bus.broadcast_wave_obj_updates(&updates);

        let msg = rx.priority.try_recv().expect("one frame expected");
        assert_eq!(
            msg.get("eventtype").and_then(|v| v.as_str()),
            Some(WS_EVENT_WAVE_OBJ_BATCHED_UPDATES),
        );
        let arr = msg.get("data").and_then(|d| d.as_array()).expect("array payload");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0].get("otype").and_then(|v| v.as_str()), Some("workspace"));
        assert_eq!(arr[1].get("updatetype").and_then(|v| v.as_str()), Some("delete"));
        // Exactly one frame — nothing else queued.
        assert!(rx.priority.try_recv().is_err());

        // Empty slice → no frame at all.
        bus.broadcast_wave_obj_updates(&[]);
        assert!(rx.priority.try_recv().is_err());
    }

    #[test]
    fn test_lane_separation() {
        // A background-lane send must not land on the priority lane, and vice
        // versa — this is the core of "terminal typing has complete priority
        // over perf monitoring".
        let bus = EventBus::new();
        let mut rx = bus.register_ws("conn-1", "tab-1");

        let event = WSEventType {
            eventtype: WS_EVENT_RPC.to_string(),
            oref: String::new(),
            data: None,
        };
        bus.send_to_conn_lane("conn-1", &event, Lane::Background);
        assert!(rx.priority.try_recv().is_err()); // nothing on priority
        assert!(rx.background.try_recv().is_ok()); // telemetry on background

        bus.send_to_conn_lane("conn-1", &event, Lane::Priority);
        assert!(rx.background.try_recv().is_err()); // nothing on background
        assert!(rx.priority.try_recv().is_ok()); // interactive on priority
    }

    #[test]
    fn test_stalled_consumer_drops_instead_of_growing_unbounded() {
        // A consumer that never drains must not let the channel grow past its
        // bound — this is the memory-safety property B.2 exists for. Fill the
        // background lane (small capacity) past its cap without ever calling
        // recv(), then drain and count: the received count must be bounded
        // by the channel capacity, proving excess sends were dropped
        // (try_send Full) rather than queued without limit.
        let bus = EventBus::new();
        let mut rx = bus.register_ws("conn-1", "tab-1"); // never drained during the flood
        let event = WSEventType {
            eventtype: WS_EVENT_RPC.to_string(),
            oref: String::new(),
            data: None,
        };
        let sent = BACKGROUND_LANE_CAPACITY * 4;
        for _ in 0..sent {
            bus.send_to_conn_lane("conn-1", &event, Lane::Background);
        }
        let mut received = 0;
        while rx.background.try_recv().is_ok() {
            received += 1;
        }
        assert!(
            received <= BACKGROUND_LANE_CAPACITY,
            "received {received} events from a lane capped at {BACKGROUND_LANE_CAPACITY} — sends were not bounded"
        );
        assert!(received < sent, "no events were dropped despite flooding well past capacity");
    }

    #[test]
    fn test_bridge_routes_telemetry_to_background() {
        // EventBusBridge must demote sysinfo + blockstats to the background lane
        // and keep everything else (e.g. terminal blockfile output) on priority.
        let bus = Arc::new(EventBus::new());
        let mut rx = bus.register_ws("conn-1", "tab-1");
        let bridge = EventBusBridge::new(bus.clone());

        let telemetry = |event: &str| WaveEvent {
            event: event.to_string(),
            scopes: vec![],
            sender: String::new(),
            persist: 0,
            data: None,
        };

        bridge.send_event("conn-1", telemetry(EVENT_SYS_INFO));
        bridge.send_event("conn-1", telemetry(EVENT_BLOCK_STATS));
        assert!(rx.priority.try_recv().is_err()); // no telemetry on priority
        assert!(rx.background.try_recv().is_ok()); // sysinfo
        assert!(rx.background.try_recv().is_ok()); // blockstats

        // Terminal output (blockfile) stays on the interactive priority lane.
        bridge.send_event("conn-1", telemetry(EVENT_BLOCK_FILE));
        assert!(rx.priority.try_recv().is_ok());
        assert!(rx.background.try_recv().is_err());
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
