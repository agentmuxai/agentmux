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
    update(broker, connection, |entry| apply(entry, status, error))
}

/// A pane's process on `connection` started (an SSH pane's `ssh`): one more
/// pane has it open, and it is connected, so the overlay never covers the
/// terminal while `ssh` asks there for a password.
pub fn pane_started(broker: Option<&Broker>, connection: &str) -> ConnStatus {
    update(broker, connection, |entry| {
        entry.activeconnnum += 1;
        apply(entry, state::CONNECTED, None);
    })
}

/// A pane's process on `connection` ended. A status is shared by every pane on
/// the connection, and the overlay covers each of them, so while another pane
/// still has it open it stays connected. The last one leaves it `error` with
/// `error` when it could not connect, and `disconnected` otherwise.
pub fn pane_ended(broker: Option<&Broker>, connection: &str, error: Option<&str>) -> ConnStatus {
    update(broker, connection, |entry| {
        entry.activeconnnum = (entry.activeconnnum - 1).max(0);
        if entry.activeconnnum == 0 {
            match error {
                Some(message) => apply(entry, state::ERROR, Some(message)),
                None => apply(entry, state::DISCONNECTED, None),
            }
        }
    })
}

fn apply(entry: &mut ConnStatus, status: &str, error: Option<&str>) {
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
}

fn update(
    broker: Option<&Broker>,
    connection: &str,
    change: impl FnOnce(&mut ConnStatus),
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
        change(entry);
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
    // An SSH connection that reached `connected` joins the Remotes pane's Recent
    // section (SPEC_REMOTES_PANE_2026_10_05.md §4.5); `connchange` above already
    // tells the pane to refresh. Not in unit tests, which would write the file
    // into whatever config dir the test process resolves.
    #[cfg(not(test))]
    if updated.connected {
        super::remotes::record_recent(connection);
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

    /// Two panes on one host: the first one ending, even with a failure, does
    /// not cover the second, still-open one; the last one says why.
    #[test]
    fn a_connection_stays_connected_while_any_pane_has_it_open() {
        let name = "test-status-two-panes";
        pane_started(None, name);
        let s = pane_started(None, name);
        assert_eq!((s.status.as_str(), s.activeconnnum), (state::CONNECTED, 2));
        let s = pane_ended(None, name, Some("could not connect"));
        assert_eq!((s.status.as_str(), s.activeconnnum), (state::CONNECTED, 1));
        let s = pane_ended(None, name, Some("could not connect"));
        assert_eq!(
            (s.status.as_str(), s.activeconnnum, s.error.as_str()),
            (state::ERROR, 0, "could not connect")
        );

        let name = "test-status-clean-exit";
        pane_started(None, name);
        let s = pane_ended(None, name, None);
        assert_eq!(
            (s.status.as_str(), s.activeconnnum),
            (state::DISCONNECTED, 0)
        );
        let s = pane_ended(None, name, None);
        assert_eq!(s.activeconnnum, 0, "never below zero");
    }
}
