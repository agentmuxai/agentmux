// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Each connection's status, published as `connchange` events.
//!
//! The frontend already has the consumer, from the Wave code AgentMux grew out
//! of: `conn-status.ts` loads every status with `GetAllConnStatus` and then
//! follows `connchange` events, and `ConnStatusOverlay` shows a pane's
//! connection state with Reconnect and Disconnect actions. This is the producer
//! it never had. `local` is never listed: it is always connected.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use crate::backend::mps::{Broker, MuxEvent, EVENT_CONN_CHANGE};
use crate::backend::rpc_types::ConnStatus;

/// The status words the frontend's overlay understands.
pub mod state {
    pub const CONNECTING: &str = "connecting";
    pub const CONNECTED: &str = "connected";
    pub const DISCONNECTED: &str = "disconnected";
    pub const ERROR: &str = "error";
}

fn registry() -> &'static Mutex<BTreeMap<String, ConnStatus>> {
    static REG: OnceLock<Mutex<BTreeMap<String, ConnStatus>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Record a connection's new state and tell the frontend. `error` is shown in
/// the pane's overlay; it is cleared on any non-error state.
pub fn set(
    broker: Option<&Broker>,
    connection: &str,
    status: &str,
    error: Option<&str>,
) -> ConnStatus {
    let updated = {
        let mut map = registry().lock().unwrap_or_else(|e| e.into_inner());
        let entry = map
            .entry(connection.to_string())
            .or_insert_with(|| ConnStatus {
                status: state::DISCONNECTED.to_string(),
                connection: connection.to_string(),
                connected: false,
                hasconnected: false,
                activeconnnum: 0,
                error: String::new(),
            });
        entry.status = status.to_string();
        entry.connected = status == state::CONNECTED;
        if entry.connected {
            entry.hasconnected = true;
        }
        entry.error = if status == state::ERROR {
            error.unwrap_or_default().to_string()
        } else {
            String::new()
        };
        entry.clone()
    };
    if let Some(broker) = broker {
        broker.publish(MuxEvent {
            event: EVENT_CONN_CHANGE.to_string(),
            scopes: vec![],
            sender: String::new(),
            persist: 0,
            data: serde_json::to_value(&updated).ok(),
        });
    }
    updated
}

/// Every known connection's status, for `GetAllConnStatus`.
pub fn all() -> Vec<ConnStatus> {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_moves_and_remembers_it_once_connected() {
        let name = "test-status-host-a";
        let s = set(None, name, state::CONNECTING, None);
        assert_eq!((s.connected, s.hasconnected), (false, false));
        let s = set(None, name, state::CONNECTED, None);
        assert_eq!((s.connected, s.hasconnected), (true, true));
        let s = set(None, name, state::ERROR, Some("Connection refused"));
        assert_eq!(
            (s.connected, s.hasconnected, s.error.as_str()),
            (false, true, "Connection refused")
        );
        let s = set(None, name, state::CONNECTING, Some("ignored"));
        assert!(s.error.is_empty(), "an error is cleared by the next state");
        assert!(all().iter().any(|c| c.connection == name));
    }
}
