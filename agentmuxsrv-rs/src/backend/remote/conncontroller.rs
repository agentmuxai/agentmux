// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Connection controller: state machine for SSH and WSL connections.
//! Port of Go's pkg/remote/conncontroller/conncontroller.go and pkg/wslconn/.
//!
//! State machine:
//!   Init ─(connect)─> Connecting ─(success)─> Connected
//!                          │                       │
//!                        (error)              (disconnect)
//!                          │                       │
//!                          v                       v
//!                        Error              Disconnected
//!
//! Provides a global registry of connections keyed by connection name.
//! Actual SSH/WSL transport is deferred; this module implements the
//! lifecycle management, status tracking, and event emission.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{LazyLock, RwLock};

use serde::{Deserialize, Serialize};

use super::sshclient::SSHOpts;
use super::{STATUS_CONNECTED, STATUS_CONNECTING, STATUS_DISCONNECTED, STATUS_ERROR, STATUS_INIT};

// ---- Global connection registry ----

/// Global registry of active connections.
static CONN_REGISTRY: LazyLock<RwLock<HashMap<String, ConnState>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Counter for total SSH connections ever made (for telemetry).
static SSH_CONNECT_COUNTER: AtomicI64 = AtomicI64::new(0);

// ---- Connection status types ----

/// Connection status as reported to the UI.
/// Port of Go's `wshrpc.ConnStatus`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConnStatus {
    /// Current status string.
    pub status: String,

    /// Whether the connection is currently active.
    pub connected: bool,

    /// Connection name (user@host:port or wsl://distro).
    pub connection: String,

    /// Whether this connection has ever been connected.
    #[serde(rename = "hasconnected")]
    pub has_connected: bool,

    /// Connection counter for telemetry.
    #[serde(rename = "activeconnnum")]
    pub active_conn_num: i64,

    /// Error message (if any).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,

    /// Whether WSH is enabled on the remote.
    #[serde(rename = "wshenabled")]
    pub wsh_enabled: bool,

    /// WSH error (if WSH failed to start).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub wsh_error: String,

    /// Reason WSH is not available (if not enabled).
    #[serde(
        default,
        rename = "nowshreason",
        skip_serializing_if = "String::is_empty"
    )]
    pub no_wsh_reason: String,

    /// WSH version on the remote.
    #[serde(
        default,
        rename = "wshversion",
        skip_serializing_if = "String::is_empty"
    )]
    pub wsh_version: String,
}

/// Internal connection state held in the registry.
#[derive(Debug)]
pub struct ConnState {
    /// Connection name (unique key).
    pub conn_name: String,
    /// Connection type: "ssh" or "wsl".
    pub conn_type: String,
    /// Current status.
    pub status: String,
    /// SSH options (for SSH connections).
    pub ssh_opts: Option<SSHOpts>,
    /// WSL distro name (for WSL connections).
    pub wsl_distro: Option<String>,
    /// Whether WSH is enabled.
    pub wsh_enabled: AtomicBool,
    /// Error message.
    pub error: String,
    /// WSH error message.
    pub wsh_error: String,
    /// Reason WSH is unavailable.
    pub no_wsh_reason: String,
    /// WSH version string.
    pub wsh_version: String,
    /// Whether connection has ever been established.
    pub has_connected: AtomicBool,
    /// Unix millis of last connection time.
    pub last_connect_time: AtomicI64,
    /// Active connection number.
    pub active_conn_num: i64,
    /// Whether someone is waiting for this connection.
    pub has_waiter: AtomicBool,
    /// Domain socket path (for RPC over unix socket).
    pub domain_sock_name: String,
}

impl ConnState {
    /// Create a new SSH connection state.
    pub fn new_ssh(conn_name: String, opts: SSHOpts) -> Self {
        Self {
            conn_name,
            conn_type: super::CONN_TYPE_SSH.to_string(),
            status: STATUS_INIT.to_string(),
            ssh_opts: Some(opts),
            wsl_distro: None,
            wsh_enabled: AtomicBool::new(false),
            error: String::new(),
            wsh_error: String::new(),
            no_wsh_reason: String::new(),
            wsh_version: String::new(),
            has_connected: AtomicBool::new(false),
            last_connect_time: AtomicI64::new(0),
            active_conn_num: 0,
            has_waiter: AtomicBool::new(false),
            domain_sock_name: String::new(),
        }
    }

    /// Create a new WSL connection state.
    pub fn new_wsl(conn_name: String, distro: String) -> Self {
        Self {
            conn_name,
            conn_type: super::CONN_TYPE_WSL.to_string(),
            status: STATUS_INIT.to_string(),
            ssh_opts: None,
            wsl_distro: Some(distro),
            wsh_enabled: AtomicBool::new(false),
            error: String::new(),
            wsh_error: String::new(),
            no_wsh_reason: String::new(),
            wsh_version: String::new(),
            has_connected: AtomicBool::new(false),
            last_connect_time: AtomicI64::new(0),
            active_conn_num: 0,
            has_waiter: AtomicBool::new(false),
            domain_sock_name: super::WSL_DOMAIN_SOCKET_PATH.to_string(),
        }
    }

    /// Get current status as a ConnStatus for UI display.
    pub fn to_conn_status(&self) -> ConnStatus {
        ConnStatus {
            status: self.status.clone(),
            connected: self.status == STATUS_CONNECTED,
            connection: self.conn_name.clone(),
            has_connected: self.has_connected.load(Ordering::Relaxed),
            active_conn_num: self.active_conn_num,
            error: self.error.clone(),
            wsh_enabled: self.wsh_enabled.load(Ordering::Relaxed),
            wsh_error: self.wsh_error.clone(),
            no_wsh_reason: self.no_wsh_reason.clone(),
            wsh_version: self.wsh_version.clone(),
        }
    }

    /// Transition to connecting state.
    pub fn set_connecting(&mut self) {
        self.status = STATUS_CONNECTING.to_string();
        self.error.clear();
        self.wsh_error.clear();
        self.no_wsh_reason.clear();
    }

    /// Transition to connected state.
    pub fn set_connected(&mut self) {
        self.status = STATUS_CONNECTED.to_string();
        self.has_connected.store(true, Ordering::Relaxed);
        self.last_connect_time.store(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
            Ordering::Relaxed,
        );
        self.active_conn_num = SSH_CONNECT_COUNTER.fetch_add(1, Ordering::Relaxed) + 1;
    }

    /// Transition to disconnected state.
    pub fn set_disconnected(&mut self) {
        self.status = STATUS_DISCONNECTED.to_string();
        self.wsh_enabled.store(false, Ordering::Relaxed);
    }

    /// Transition to error state.
    pub fn set_error(&mut self, error: String) {
        self.status = STATUS_ERROR.to_string();
        self.error = error;
        self.wsh_enabled.store(false, Ordering::Relaxed);
    }

    /// Update WSH status after successful connection.
    pub fn set_wsh_enabled(&mut self, version: String) {
        self.wsh_enabled.store(true, Ordering::Relaxed);
        self.wsh_version = version;
        self.wsh_error.clear();
        self.no_wsh_reason.clear();
    }

    /// Update WSH status with an error.
    pub fn set_wsh_error(&mut self, error: String) {
        self.wsh_enabled.store(false, Ordering::Relaxed);
        self.wsh_error = error;
    }

    /// Update WSH status with a reason it's not available.
    pub fn set_no_wsh(&mut self, reason: String) {
        self.wsh_enabled.store(false, Ordering::Relaxed);
        self.no_wsh_reason = reason;
    }
}

// ---- Registry operations ----

/// Get or create a connection state in the registry.
/// Does NOT start the connection.
pub fn get_or_create_conn(conn_name: &str) -> ConnStatus {
    let registry = CONN_REGISTRY.read().unwrap();
    if let Some(state) = registry.get(conn_name) {
        return state.to_conn_status();
    }
    drop(registry);

    // Create a new connection state
    let mut registry = CONN_REGISTRY.write().unwrap();
    // Double-check after acquiring write lock
    if let Some(state) = registry.get(conn_name) {
        return state.to_conn_status();
    }

    let conn_type = super::connparse::get_conn_type(conn_name);
    let state = if conn_type == super::CONN_TYPE_WSL {
        // Extract distro from "wsl://distro" format
        let distro = conn_name
            .strip_prefix("wsl://")
            .unwrap_or("")
            .to_string();
        ConnState::new_wsl(conn_name.to_string(), distro)
    } else {
        // SSH connection
        let opts = super::sshclient::parse_opts(conn_name).unwrap_or_default();
        ConnState::new_ssh(conn_name.to_string(), opts)
    };
    let status = state.to_conn_status();
    registry.insert(conn_name.to_string(), state);
    status
}

/// Get the current status of a connection.
pub fn get_conn_status(conn_name: &str) -> Option<ConnStatus> {
    let registry = CONN_REGISTRY.read().unwrap();
    registry.get(conn_name).map(|s| s.to_conn_status())
}

/// Get all connection statuses.
pub fn get_all_conn_status() -> Vec<ConnStatus> {
    let registry = CONN_REGISTRY.read().unwrap();
    registry.values().map(|s| s.to_conn_status()).collect()
}

/// Remove a connection from the registry.
pub fn remove_conn(conn_name: &str) {
    let mut registry = CONN_REGISTRY.write().unwrap();
    registry.remove(conn_name);
}

/// Disconnect a connection (set status to disconnected).
pub fn disconnect(conn_name: &str) -> Result<(), String> {
    let mut registry = CONN_REGISTRY.write().unwrap();
    let state = registry
        .get_mut(conn_name)
        .ok_or_else(|| format!("connection not found: {conn_name}"))?;

    match state.status.as_str() {
        s if s == STATUS_CONNECTED || s == STATUS_CONNECTING => {
            state.set_disconnected();
            Ok(())
        }
        s if s == STATUS_DISCONNECTED => Ok(()), // already disconnected
        _ => Err(format!(
            "cannot disconnect: connection is in '{}' state",
            state.status
        )),
    }
}

/// Start connecting (transition from init to connecting).
/// Returns error if already connecting or connected.
pub fn start_connecting(conn_name: &str) -> Result<(), String> {
    let mut registry = CONN_REGISTRY.write().unwrap();
    let state = registry
        .get_mut(conn_name)
        .ok_or_else(|| format!("connection not found: {conn_name}"))?;

    match state.status.as_str() {
        s if s == STATUS_INIT || s == STATUS_DISCONNECTED || s == STATUS_ERROR => {
            state.set_connecting();
            Ok(())
        }
        s if s == STATUS_CONNECTED => Ok(()), // already connected
        _ => Err(format!(
            "cannot connect: connection is in '{}' state",
            state.status
        )),
    }
}

/// Mark a connection as connected.
pub fn mark_connected(conn_name: &str) -> Result<(), String> {
    let mut registry = CONN_REGISTRY.write().unwrap();
    let state = registry
        .get_mut(conn_name)
        .ok_or_else(|| format!("connection not found: {conn_name}"))?;

    if state.status != STATUS_CONNECTING {
        return Err(format!(
            "cannot mark connected: expected 'connecting', got '{}'",
            state.status
        ));
    }

    state.set_connected();
    Ok(())
}

/// Mark a connection as errored.
pub fn mark_error(conn_name: &str, error: String) -> Result<(), String> {
    let mut registry = CONN_REGISTRY.write().unwrap();
    let state = registry
        .get_mut(conn_name)
        .ok_or_else(|| format!("connection not found: {conn_name}"))?;

    state.set_error(error);
    Ok(())
}

/// Update WSH status for a connection.
pub fn update_wsh_status(
    conn_name: &str,
    version: Option<String>,
    error: Option<String>,
    no_wsh_reason: Option<String>,
) -> Result<(), String> {
    let mut registry = CONN_REGISTRY.write().unwrap();
    let state = registry
        .get_mut(conn_name)
        .ok_or_else(|| format!("connection not found: {conn_name}"))?;

    if let Some(v) = version {
        state.set_wsh_enabled(v);
    } else if let Some(e) = error {
        state.set_wsh_error(e);
    } else if let Some(r) = no_wsh_reason {
        state.set_no_wsh(r);
    }

    Ok(())
}

/// Get the total number of SSH connections ever made.
pub fn get_ssh_connect_count() -> i64 {
    SSH_CONNECT_COUNTER.load(Ordering::Relaxed)
}

/// Clear the global connection registry (for testing).
pub fn clear_registry() {
    let mut registry = CONN_REGISTRY.write().unwrap();
    registry.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    // Each test clears the registry to avoid interference
    fn setup() {
        clear_registry();
    }

    #[test]
    fn test_conn_state_new_ssh() {
        let opts = SSHOpts::new("host.com", "alice", "22");
        let state = ConnState::new_ssh("alice@host.com".to_string(), opts);
        assert_eq!(state.status, STATUS_INIT);
        assert_eq!(state.conn_type, "ssh");
        assert!(!state.wsh_enabled.load(Ordering::Relaxed));
        assert!(!state.has_connected.load(Ordering::Relaxed));
    }

    #[test]
    fn test_conn_state_new_wsl() {
        let state = ConnState::new_wsl("wsl://Ubuntu".to_string(), "Ubuntu".to_string());
        assert_eq!(state.status, STATUS_INIT);
        assert_eq!(state.conn_type, "wsl");
        assert_eq!(state.wsl_distro, Some("Ubuntu".to_string()));
        assert_eq!(state.domain_sock_name, super::super::WSL_DOMAIN_SOCKET_PATH);
    }

    #[test]
    fn test_conn_state_transitions() {
        let opts = SSHOpts::new("host", "", "");
        let mut state = ConnState::new_ssh("host".to_string(), opts);

        assert_eq!(state.status, STATUS_INIT);

        state.set_connecting();
        assert_eq!(state.status, STATUS_CONNECTING);

        state.set_connected();
        assert_eq!(state.status, STATUS_CONNECTED);
        assert!(state.has_connected.load(Ordering::Relaxed));
        assert!(state.last_connect_time.load(Ordering::Relaxed) > 0);

        state.set_disconnected();
        assert_eq!(state.status, STATUS_DISCONNECTED);
        assert!(!state.wsh_enabled.load(Ordering::Relaxed));
    }

    #[test]
    fn test_conn_state_error() {
        let opts = SSHOpts::new("host", "", "");
        let mut state = ConnState::new_ssh("host".to_string(), opts);
        state.set_error("connection refused".to_string());
        assert_eq!(state.status, STATUS_ERROR);
        assert_eq!(state.error, "connection refused");
    }

    #[test]
    fn test_conn_state_wsh_status() {
        let opts = SSHOpts::new("host", "", "");
        let mut state = ConnState::new_ssh("host".to_string(), opts);

        state.set_wsh_enabled("v0.10.4".to_string());
        assert!(state.wsh_enabled.load(Ordering::Relaxed));
        assert_eq!(state.wsh_version, "v0.10.4");

        state.set_wsh_error("binary not found".to_string());
        assert!(!state.wsh_enabled.load(Ordering::Relaxed));
        assert_eq!(state.wsh_error, "binary not found");
    }

    #[test]
    fn test_conn_status_serde() {
        let status = ConnStatus {
            status: STATUS_CONNECTED.to_string(),
            connected: true,
            connection: "alice@host".to_string(),
            has_connected: true,
            active_conn_num: 1,
            error: String::new(),
            wsh_enabled: true,
            wsh_error: String::new(),
            no_wsh_reason: String::new(),
            wsh_version: "v0.10.4".to_string(),
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"hasconnected\":true"));
        assert!(json.contains("\"wshenabled\":true"));
        assert!(json.contains("\"wshversion\":\"v0.10.4\""));
        // Empty fields should be omitted
        assert!(!json.contains("\"error\""));
        assert!(!json.contains("\"nowshreason\""));

        let parsed: ConnStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.connection, "alice@host");
        assert!(parsed.connected);
    }

    #[test]
    fn test_to_conn_status() {
        let opts = SSHOpts::new("host", "alice", "");
        let mut state = ConnState::new_ssh("alice@host".to_string(), opts);
        state.set_connecting();
        state.set_connected();
        state.set_wsh_enabled("v0.10.4".to_string());

        let status = state.to_conn_status();
        assert_eq!(status.status, STATUS_CONNECTED);
        assert!(status.connected);
        assert!(status.has_connected);
        assert!(status.wsh_enabled);
        assert_eq!(status.wsh_version, "v0.10.4");
    }

    #[test]
    fn test_get_or_create_conn_ssh() {
        setup();
        let status = get_or_create_conn("user@host.com");
        assert_eq!(status.status, STATUS_INIT);
        assert_eq!(status.connection, "user@host.com");
        assert!(!status.connected);
    }

    #[test]
    fn test_get_or_create_conn_wsl() {
        setup();
        let status = get_or_create_conn("wsl://Ubuntu");
        assert_eq!(status.status, STATUS_INIT);
        assert_eq!(status.connection, "wsl://Ubuntu");
    }

    #[test]
    fn test_get_or_create_conn_idempotent() {
        setup();
        let s1 = get_or_create_conn("host1");
        let s2 = get_or_create_conn("host1");
        assert_eq!(s1.connection, s2.connection);
    }

    #[test]
    fn test_get_conn_status_not_found() {
        setup();
        assert!(get_conn_status("nonexistent").is_none());
    }

    #[test]
    fn test_get_all_conn_status() {
        setup();
        get_or_create_conn("host1");
        get_or_create_conn("host2");
        let all = get_all_conn_status();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_disconnect() {
        setup();
        get_or_create_conn("host1");
        start_connecting("host1").unwrap();
        mark_connected("host1").unwrap();
        disconnect("host1").unwrap();
        let status = get_conn_status("host1").unwrap();
        assert_eq!(status.status, STATUS_DISCONNECTED);
    }

    #[test]
    fn test_disconnect_not_found() {
        setup();
        assert!(disconnect("nonexistent").is_err());
    }

    #[test]
    fn test_start_connecting() {
        setup();
        get_or_create_conn("host1");
        start_connecting("host1").unwrap();
        let status = get_conn_status("host1").unwrap();
        assert_eq!(status.status, STATUS_CONNECTING);
    }

    #[test]
    fn test_mark_connected() {
        setup();
        get_or_create_conn("host1");
        start_connecting("host1").unwrap();
        mark_connected("host1").unwrap();
        let status = get_conn_status("host1").unwrap();
        assert_eq!(status.status, STATUS_CONNECTED);
        assert!(status.connected);
        assert!(status.has_connected);
    }

    #[test]
    fn test_mark_error() {
        setup();
        get_or_create_conn("host1");
        start_connecting("host1").unwrap();
        mark_error("host1", "timeout".to_string()).unwrap();
        let status = get_conn_status("host1").unwrap();
        assert_eq!(status.status, STATUS_ERROR);
        assert_eq!(status.error, "timeout");
    }

    #[test]
    fn test_update_wsh_status() {
        setup();
        get_or_create_conn("host1");
        update_wsh_status("host1", Some("v0.10.4".to_string()), None, None).unwrap();
        let status = get_conn_status("host1").unwrap();
        assert!(status.wsh_enabled);
        assert_eq!(status.wsh_version, "v0.10.4");
    }

    #[test]
    fn test_remove_conn() {
        setup();
        get_or_create_conn("host1");
        assert!(get_conn_status("host1").is_some());
        remove_conn("host1");
        assert!(get_conn_status("host1").is_none());
    }

    #[test]
    fn test_reconnect_after_disconnect() {
        setup();
        get_or_create_conn("host1");
        start_connecting("host1").unwrap();
        mark_connected("host1").unwrap();
        disconnect("host1").unwrap();

        // Should be able to reconnect
        start_connecting("host1").unwrap();
        mark_connected("host1").unwrap();
        let status = get_conn_status("host1").unwrap();
        assert!(status.connected);
    }

    #[test]
    fn test_reconnect_after_error() {
        setup();
        get_or_create_conn("host1");
        start_connecting("host1").unwrap();
        mark_error("host1", "refused".to_string()).unwrap();

        // Should be able to reconnect after error
        start_connecting("host1").unwrap();
        let status = get_conn_status("host1").unwrap();
        assert_eq!(status.status, STATUS_CONNECTING);
    }
}
