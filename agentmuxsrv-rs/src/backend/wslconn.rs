// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! WSL connection types and status constants.
//! Port of Go's `pkg/wslconn/wslconn.go` — wire types only.

#![allow(dead_code)]
//!
//! The full connection lifecycle (Connect, Reconnect, etc.) involves system-level
//! operations that will be wired in later. This module provides the type definitions
//! for serialization and status tracking.

use serde::{Deserialize, Serialize};

// ---- Status Constants ----

/// Connection status: initializing.
pub const STATUS_INIT: &str = "init";
/// Connection status: connecting.
pub const STATUS_CONNECTING: &str = "connecting";
/// Connection status: connected.
pub const STATUS_CONNECTED: &str = "connected";
/// Connection status: disconnected.
pub const STATUS_DISCONNECTED: &str = "disconnected";
/// Connection status: error.
pub const STATUS_ERROR: &str = "error";

/// Default timeout for WSL connections (60 seconds).
pub const DEFAULT_CONNECTION_TIMEOUT_MS: u64 = 60_000;

/// Template for connserver command invocation.
pub const CONN_SERVER_CMD_TEMPLATE: &str = concat!(
    "%s version 2> /dev/null || (echo -n \"not-installed \"; uname -sm);\n",
    "exec %s connserver --router"
);

// ---- Wire Types ----

/// Connection status, sent over RPC and to the frontend.
/// Matches Go's `wshrpc.ConnStatus`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ConnStatus {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub connected: bool,
    #[serde(default, rename = "wshenabled")]
    pub wsh_enabled: bool,
    #[serde(default)]
    pub connection: String,
    #[serde(default, rename = "hasconnected")]
    pub has_connected: bool,
    #[serde(default, rename = "activeconnnum")]
    pub active_conn_num: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(default, skip_serializing_if = "String::is_empty", rename = "wsherror")]
    pub wsh_error: String,
    #[serde(default, skip_serializing_if = "String::is_empty", rename = "nowshreason")]
    pub no_wsh_reason: String,
    #[serde(default, skip_serializing_if = "String::is_empty", rename = "wshversion")]
    pub wsh_version: String,
}

/// Options for WSH installation.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WshInstallOpts {
    #[serde(default)]
    pub force: bool,
    #[serde(default, rename = "nouserprompt")]
    pub no_user_prompt: bool,
}

/// Result of checking WSH availability on a connection.
#[derive(Debug, Clone, Default)]
pub struct WshCheckResult {
    /// Whether WSH is enabled.
    pub wsh_enabled: bool,
    /// The client version string.
    pub client_version: String,
    /// Reason WSH is not available (if not enabled).
    pub no_wsh_reason: String,
    /// Error from WSH check (if any).
    pub wsh_error: Option<String>,
}

/// WSL distribution name wrapper.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WslName {
    pub distro: String,
}

impl WslName {
    pub fn new(distro: impl Into<String>) -> Self {
        Self { distro: distro.into() }
    }

    /// Get the full connection name (wsl://distro).
    pub fn connection_name(&self) -> String {
        format!("wsl://{}", self.distro)
    }
}

/// Remote info for WSH update operations.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RemoteInfo {
    #[serde(default, rename = "clientos")]
    pub client_os: String,
    #[serde(default, rename = "clientarch")]
    pub client_arch: String,
    #[serde(default, rename = "clientversion")]
    pub client_version: String,
}

/// Input fields for deriving a ConnStatus.
pub struct ConnStateFields<'a> {
    pub status: &'a str,
    pub wsh_enabled: bool,
    pub connection_name: &'a str,
    pub last_connect_time: i64,
    pub active_conn_num: i32,
    pub error: &'a str,
    pub wsh_error: &'a str,
    pub no_wsh_reason: &'a str,
    pub wsh_version: &'a str,
}

/// Derive a ConnStatus from connection state fields.
pub fn derive_conn_status(fields: &ConnStateFields<'_>) -> ConnStatus {
    ConnStatus {
        status: fields.status.to_string(),
        connected: fields.status == STATUS_CONNECTED,
        wsh_enabled: fields.wsh_enabled,
        connection: fields.connection_name.to_string(),
        has_connected: fields.last_connect_time > 0,
        active_conn_num: fields.active_conn_num,
        error: fields.error.to_string(),
        wsh_error: fields.wsh_error.to_string(),
        no_wsh_reason: fields.no_wsh_reason.to_string(),
        wsh_version: fields.wsh_version.to_string(),
    }
}

// ---- WSL Distro Detection (Windows only) ----

#[cfg(windows)]
use std::process::Command;

/// Get list of registered WSL distributions on Windows.
/// Returns vector of distro names.
#[cfg(windows)]
pub fn registered_distros() -> Result<Vec<String>, String> {
    let output = Command::new("wsl.exe")
        .args(&["--list", "--quiet"])
        .output()
        .map_err(|e| format!("failed to execute wsl.exe: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "wsl.exe exited with code {}",
            output.status.code().unwrap_or(-1)
        ));
    }

    // WSL outputs UTF-16 on Windows, decode it
    let raw_output = if output.stdout.len() >= 2 && output.stdout[0] == 0xFF && output.stdout[1] == 0xFE {
        // UTF-16 LE with BOM
        let utf16_data: Vec<u16> = output.stdout[2..]
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        String::from_utf16(&utf16_data).map_err(|e| format!("invalid UTF-16: {}", e))?
    } else {
        // Fallback to UTF-8
        String::from_utf8(output.stdout).map_err(|e| format!("invalid UTF-8: {}", e))?
    };

    let distros: Vec<String> = raw_output
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect();

    Ok(distros)
}

/// Get list of registered WSL distributions (non-Windows stub).
#[cfg(not(windows))]
pub fn registered_distros() -> Result<Vec<String>, String> {
    Err("WSL is only available on Windows".to_string())
}

/// Get the default WSL distribution on Windows.
/// Returns the distro name marked as default, or first distro if none marked.
#[cfg(windows)]
pub fn default_distro() -> Result<String, String> {
    let output = Command::new("wsl.exe")
        .args(&["--list", "--verbose"])
        .output()
        .map_err(|e| format!("failed to execute wsl.exe: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "wsl.exe exited with code {}",
            output.status.code().unwrap_or(-1)
        ));
    }

    // Decode UTF-16
    let raw_output = if output.stdout.len() >= 2 && output.stdout[0] == 0xFF && output.stdout[1] == 0xFE {
        let utf16_data: Vec<u16> = output.stdout[2..]
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        String::from_utf16(&utf16_data).map_err(|e| format!("invalid UTF-16: {}", e))?
    } else {
        String::from_utf8(output.stdout).map_err(|e| format!("invalid UTF-8: {}", e))?
    };

    // Parse output looking for "* <distro>" (default marker) or first distro
    let mut first_distro: Option<String> = None;
    for line in raw_output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("NAME") {
            continue;
        }

        // Check if this is the default (marked with *)
        if line.starts_with('*') {
            // Extract distro name (format: "* Name  State  Version")
            let parts: Vec<&str> = line[1..].split_whitespace().collect();
            if !parts.is_empty() {
                return Ok(parts[0].to_string());
            }
        }

        // Track first distro as fallback
        if first_distro.is_none() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if !parts.is_empty() {
                first_distro = Some(parts[0].to_string());
            }
        }
    }

    first_distro.ok_or_else(|| "no WSL distributions found".to_string())
}

/// Get the default WSL distribution (non-Windows stub).
#[cfg(not(windows))]
pub fn default_distro() -> Result<String, String> {
    Err("WSL is only available on Windows".to_string())
}

/// Validate that a WSL distribution exists and return its name.
/// Returns the distro name if found, error otherwise.
#[cfg(windows)]
pub fn get_distro(distro_name: &str) -> Result<String, String> {
    let distros = registered_distros()?;

    for distro in distros {
        if distro.eq_ignore_ascii_case(distro_name) {
            return Ok(distro);
        }
    }

    Err(format!("WSL distro '{}' not found", distro_name))
}

/// Validate that a WSL distribution exists (non-Windows stub).
#[cfg(not(windows))]
pub fn get_distro(_distro_name: &str) -> Result<String, String> {
    Err("WSL is only available on Windows".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_constants() {
        assert_eq!(STATUS_INIT, "init");
        assert_eq!(STATUS_CONNECTING, "connecting");
        assert_eq!(STATUS_CONNECTED, "connected");
        assert_eq!(STATUS_DISCONNECTED, "disconnected");
        assert_eq!(STATUS_ERROR, "error");
    }

    #[test]
    fn test_wsl_name() {
        let name = WslName::new("Ubuntu");
        assert_eq!(name.distro, "Ubuntu");
        assert_eq!(name.connection_name(), "wsl://Ubuntu");
    }

    #[test]
    fn test_derive_conn_status_connected() {
        let status = derive_conn_status(&ConnStateFields {
            status: STATUS_CONNECTED, wsh_enabled: true, connection_name: "wsl://Ubuntu",
            last_connect_time: 1700000000, active_conn_num: 1,
            error: "", wsh_error: "", no_wsh_reason: "", wsh_version: "0.20.0",
        });
        assert!(status.connected);
        assert!(status.wsh_enabled);
        assert!(status.has_connected);
        assert_eq!(status.active_conn_num, 1);
        assert_eq!(status.wsh_version, "0.20.0");
    }

    #[test]
    fn test_derive_conn_status_error() {
        let status = derive_conn_status(&ConnStateFields {
            status: STATUS_ERROR, wsh_enabled: false, connection_name: "wsl://Ubuntu",
            last_connect_time: 0, active_conn_num: 0,
            error: "connection failed", wsh_error: "", no_wsh_reason: "wsh not installed", wsh_version: "",
        });
        assert!(!status.connected);
        assert!(!status.has_connected);
        assert_eq!(status.error, "connection failed");
        assert_eq!(status.no_wsh_reason, "wsh not installed");
    }

    #[test]
    fn test_derive_conn_status_init() {
        let status = derive_conn_status(&ConnStateFields {
            status: STATUS_INIT, wsh_enabled: false, connection_name: "wsl://Debian",
            last_connect_time: 0, active_conn_num: 0,
            error: "", wsh_error: "", no_wsh_reason: "", wsh_version: "",
        });
        assert!(!status.connected);
        assert!(!status.has_connected);
        assert_eq!(status.status, "init");
    }

    #[test]
    fn test_conn_status_serde_roundtrip() {
        let status = ConnStatus {
            status: STATUS_CONNECTED.into(),
            connected: true,
            wsh_enabled: true,
            connection: "wsl://Ubuntu".into(),
            has_connected: true,
            active_conn_num: 3,
            error: String::new(),
            wsh_error: String::new(),
            no_wsh_reason: String::new(),
            wsh_version: "0.20.0".into(),
        };
        let json = serde_json::to_string(&status).unwrap();
        let parsed: ConnStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(status, parsed);
    }

    #[test]
    fn test_conn_status_json_field_names() {
        let status = ConnStatus {
            status: "connected".into(),
            connected: true,
            wsh_enabled: true,
            connection: "wsl://test".into(),
            has_connected: true,
            active_conn_num: 1,
            error: String::new(),
            wsh_error: "some error".into(),
            no_wsh_reason: String::new(),
            wsh_version: "1.0".into(),
        };
        let json = serde_json::to_string(&status).unwrap();
        // Verify renamed fields
        assert!(json.contains("\"wshenabled\""));
        assert!(json.contains("\"hasconnected\""));
        assert!(json.contains("\"activeconnnum\""));
        assert!(json.contains("\"wsherror\""));
        assert!(json.contains("\"wshversion\""));
    }

    #[test]
    fn test_wsh_install_opts_default() {
        let opts = WshInstallOpts::default();
        assert!(!opts.force);
        assert!(!opts.no_user_prompt);
    }

    #[test]
    fn test_wsh_install_opts_serde() {
        let opts = WshInstallOpts { force: true, no_user_prompt: true };
        let json = serde_json::to_string(&opts).unwrap();
        assert!(json.contains("\"nouserprompt\""));
        let parsed: WshInstallOpts = serde_json::from_str(&json).unwrap();
        assert_eq!(opts, parsed);
    }

    #[test]
    fn test_wsh_check_result_enabled() {
        let result = WshCheckResult {
            wsh_enabled: true,
            client_version: "0.20.0".into(),
            no_wsh_reason: String::new(),
            wsh_error: None,
        };
        assert!(result.wsh_enabled);
        assert_eq!(result.client_version, "0.20.0");
    }

    #[test]
    fn test_wsh_check_result_disabled() {
        let result = WshCheckResult {
            wsh_enabled: false,
            client_version: String::new(),
            no_wsh_reason: "user declined".into(),
            wsh_error: None,
        };
        assert!(!result.wsh_enabled);
        assert_eq!(result.no_wsh_reason, "user declined");
    }

    #[test]
    fn test_remote_info_serde() {
        let info = RemoteInfo {
            client_os: "linux".into(),
            client_arch: "x64".into(),
            client_version: "0.20.0".into(),
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"clientos\""));
        assert!(json.contains("\"clientarch\""));
        assert!(json.contains("\"clientversion\""));
        let parsed: RemoteInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, parsed);
    }

    #[test]
    fn test_wsl_name_serde() {
        let name = WslName::new("Ubuntu-22.04");
        let json = serde_json::to_string(&name).unwrap();
        let parsed: WslName = serde_json::from_str(&json).unwrap();
        assert_eq!(name, parsed);
    }

    // WSL function tests (manual testing required on Windows)
    #[test]
    #[cfg(not(windows))]
    fn test_registered_distros_non_windows() {
        let result = registered_distros();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("only available on Windows"));
    }

    #[test]
    #[cfg(not(windows))]
    fn test_default_distro_non_windows() {
        let result = default_distro();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("only available on Windows"));
    }

    #[test]
    #[cfg(not(windows))]
    fn test_get_distro_non_windows() {
        let result = get_distro("Ubuntu");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("only available on Windows"));
    }

    // Note: Windows tests require manual verification as they depend on system WSL installation
    // Run manually: cargo test --features test-wsl-integration
}
