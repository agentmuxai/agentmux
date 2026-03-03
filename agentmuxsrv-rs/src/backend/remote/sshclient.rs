// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! SSH client types and configuration parsing.
//! Port of Go's pkg/remote/sshclient.go.
//!
//! Provides SSH connection option types, configuration keyword merging,
//! and connection string parsing. Actual SSH transport is deferred until
//! russh is wired in; this module focuses on type definitions.

#![allow(dead_code)]

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---- SSH option types ----

/// SSH connection options parsed from a connection string.
/// Matches Go's `remote.SSHOpts` with identical JSON tags.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SSHOpts {
    /// SSH hostname or IP.
    #[serde(rename = "sshhost", default, skip_serializing_if = "String::is_empty")]
    pub ssh_host: String,

    /// SSH username (optional).
    #[serde(rename = "sshuser", default, skip_serializing_if = "String::is_empty")]
    pub ssh_user: String,

    /// SSH port (optional, defaults to "22").
    #[serde(
        rename = "sshport",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub ssh_port: String,
}

impl SSHOpts {
    /// Create SSHOpts from components.
    pub fn new(host: &str, user: &str, port: &str) -> Self {
        Self {
            ssh_host: host.to_string(),
            ssh_user: user.to_string(),
            ssh_port: port.to_string(),
        }
    }

    /// Get the effective port (default: "22").
    pub fn effective_port(&self) -> &str {
        if self.ssh_port.is_empty() {
            "22"
        } else {
            &self.ssh_port
        }
    }

    /// Format as a normalized connection name: "user@host:port".
    pub fn to_conn_name(&self) -> String {
        super::connparse::format_conn_name(&self.ssh_user, &self.ssh_host, &self.ssh_port)
    }
}

/// Parse a connection string like "user@host:port" into SSHOpts.
///
/// Accepted formats:
/// - `host`
/// - `host:port`
/// - `user@host`
/// - `user@host:port`
///
/// # Examples
///
/// ```
/// use backend_test::backend::remote::sshclient::parse_opts;
///
/// let opts = parse_opts("alice@example.com:2222").unwrap();
/// assert_eq!(opts.ssh_user, "alice");
/// assert_eq!(opts.ssh_host, "example.com");
/// assert_eq!(opts.ssh_port, "2222");
/// ```
pub fn parse_opts(input: &str) -> Result<SSHOpts, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("empty connection string".to_string());
    }

    let mut user = String::new();
    let host;
    let mut port = String::new();

    // Split user@rest
    let rest = if let Some(at_pos) = input.find('@') {
        user = input[..at_pos].to_string();
        &input[at_pos + 1..]
    } else {
        input
    };

    // Split host:port
    if let Some(colon_pos) = rest.rfind(':') {
        host = rest[..colon_pos].to_string();
        let port_str = &rest[colon_pos + 1..];
        // Validate port is numeric
        if !port_str.is_empty() {
            if port_str.chars().all(|c| c.is_ascii_digit()) {
                port = port_str.to_string();
            } else {
                return Err(format!("invalid port: {port_str}"));
            }
        }
    } else {
        host = rest.to_string();
    }

    if host.is_empty() {
        return Err("empty hostname".to_string());
    }

    // Validate hostname characters
    for c in host.chars() {
        if !c.is_ascii_alphanumeric() && c != '.' && c != '-' && c != '_' {
            return Err(format!("invalid character in hostname: {c}"));
        }
    }

    Ok(SSHOpts {
        ssh_host: host,
        ssh_user: user,
        ssh_port: port,
    })
}

// ---- SSH config keywords ----

/// SSH configuration keywords from SSH config + Wave config.
/// Port of Go's `wconfig.ConnKeywords`.
/// These are merged from multiple sources in priority order.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConnKeywords {
    /// SSH username.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,

    /// Resolved hostname (may differ from input pattern).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,

    /// SSH port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<String>,

    /// Identity files to try (multiple allowed).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identity_file: Vec<String>,

    /// Whether to run in batch mode (no interactive prompts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_mode: Option<bool>,

    /// Whether public key authentication is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pubkey_authentication: Option<bool>,

    /// Whether password authentication is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_authentication: Option<bool>,

    /// Whether keyboard-interactive authentication is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kbd_interactive_authentication: Option<bool>,

    /// Preferred authentication methods (comma-separated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_authentications: Option<String>,

    /// Whether to add decrypted keys to the SSH agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add_keys_to_agent: Option<bool>,

    /// Whether to only use identity files (no agent keys).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identities_only: Option<bool>,

    /// Custom identity agent socket path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_agent: Option<String>,

    /// Proxy jump hosts (comma-separated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_jump: Option<String>,

    /// User-specific known hosts file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_known_hosts_file: Option<String>,

    /// Global known hosts file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global_known_hosts_file: Option<String>,
}

impl ConnKeywords {
    /// Merge another set of keywords into this one.
    /// Values in `other` only override if the field in `self` is None/empty.
    /// Identity files are always appended (not replaced).
    pub fn merge_from(&mut self, other: &ConnKeywords) {
        if self.user.is_none() {
            self.user = other.user.clone();
        }
        if self.hostname.is_none() {
            self.hostname = other.hostname.clone();
        }
        if self.port.is_none() {
            self.port = other.port.clone();
        }
        // Identity files: append from other sources
        for f in &other.identity_file {
            if !self.identity_file.contains(f) {
                self.identity_file.push(f.clone());
            }
        }
        if self.batch_mode.is_none() {
            self.batch_mode = other.batch_mode;
        }
        if self.pubkey_authentication.is_none() {
            self.pubkey_authentication = other.pubkey_authentication;
        }
        if self.password_authentication.is_none() {
            self.password_authentication = other.password_authentication;
        }
        if self.kbd_interactive_authentication.is_none() {
            self.kbd_interactive_authentication = other.kbd_interactive_authentication;
        }
        if self.preferred_authentications.is_none() {
            self.preferred_authentications = other.preferred_authentications.clone();
        }
        if self.add_keys_to_agent.is_none() {
            self.add_keys_to_agent = other.add_keys_to_agent;
        }
        if self.identities_only.is_none() {
            self.identities_only = other.identities_only;
        }
        if self.identity_agent.is_none() {
            self.identity_agent = other.identity_agent.clone();
        }
        if self.proxy_jump.is_none() {
            self.proxy_jump = other.proxy_jump.clone();
        }
        if self.user_known_hosts_file.is_none() {
            self.user_known_hosts_file = other.user_known_hosts_file.clone();
        }
        if self.global_known_hosts_file.is_none() {
            self.global_known_hosts_file = other.global_known_hosts_file.clone();
        }
    }

    /// Get the list of proxy jump hosts (split on comma).
    pub fn proxy_jump_hosts(&self) -> Vec<String> {
        match &self.proxy_jump {
            Some(jumps) if !jumps.is_empty() => {
                jumps.split(',').map(|s| s.trim().to_string()).collect()
            }
            _ => vec![],
        }
    }
}

// ---- Connection flags ----

/// Flags passed from the UI/CLI when initiating a connection.
/// Port of Go's `wshrpc.ConnKeywords` used as connection flags.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConnFlags {
    /// Override SSH user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_user: Option<String>,

    /// Override SSH port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_port: Option<String>,

    /// Override SSH identity file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_identity: Option<String>,

    /// Override SSH password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_password: Option<String>,

    /// Whether to ignore SSH config file.
    #[serde(default)]
    pub ignore_ssh_config: bool,

    /// Whether to skip WSH installation.
    #[serde(default)]
    pub skip_wsh_install: bool,

    /// Custom environment variables.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
}

impl ConnFlags {
    /// Convert connection flags to ConnKeywords for merging.
    pub fn to_keywords(&self) -> ConnKeywords {
        let identity_file = self
            .ssh_identity
            .as_ref()
            .map(|id| vec![id.clone()])
            .unwrap_or_default();
        ConnKeywords {
            user: self.ssh_user.clone(),
            port: self.ssh_port.clone(),
            identity_file,
            ..Default::default()
        }
    }
}

// ---- Connection debug info ----

/// Debug info for SSH connection errors.
/// Port of Go's `ConnectionDebugInfo`.
#[derive(Debug, Clone, Default)]
pub struct ConnectionDebugInfo {
    /// Current SSH connection we're jumping through (if proxy).
    pub current_host: String,
    /// Target SSH options.
    pub next_opts: SSHOpts,
    /// Proxy jump depth counter.
    pub jump_num: i32,
}

/// Connection error with debug context.
#[derive(Debug)]
pub struct ConnectionError {
    pub debug_info: ConnectionDebugInfo,
    pub error: String,
}

impl std::fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.debug_info.jump_num > 0 {
            write!(
                f,
                "SSH connection error (jump #{} via {}): {}",
                self.debug_info.jump_num, self.debug_info.current_host, self.error
            )
        } else {
            write!(f, "SSH connection error: {}", self.error)
        }
    }
}

impl std::error::Error for ConnectionError {}

// ---- Platform detection ----

/// Normalize a remote platform architecture string.
/// Maps `uname -m` output to our canonical names.
pub fn normalize_arch(arch: &str) -> &str {
    match arch {
        "x86_64" | "amd64" => "x64",
        "aarch64" | "arm64" => "arm64",
        other => other,
    }
}

/// Normalize a remote platform OS string.
/// Maps `uname -s` output to our canonical names.
pub fn normalize_os(os: &str) -> &str {
    match os.to_lowercase().as_str() {
        "linux" => "linux",
        "darwin" => "darwin",
        "freebsd" => "freebsd",
        _ => os,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_opts_full() {
        let opts = parse_opts("alice@example.com:2222").unwrap();
        assert_eq!(opts.ssh_user, "alice");
        assert_eq!(opts.ssh_host, "example.com");
        assert_eq!(opts.ssh_port, "2222");
    }

    #[test]
    fn test_parse_opts_user_host() {
        let opts = parse_opts("bob@myserver").unwrap();
        assert_eq!(opts.ssh_user, "bob");
        assert_eq!(opts.ssh_host, "myserver");
        assert_eq!(opts.ssh_port, "");
    }

    #[test]
    fn test_parse_opts_host_only() {
        let opts = parse_opts("myserver").unwrap();
        assert_eq!(opts.ssh_user, "");
        assert_eq!(opts.ssh_host, "myserver");
        assert_eq!(opts.ssh_port, "");
    }

    #[test]
    fn test_parse_opts_host_port() {
        let opts = parse_opts("myserver:22").unwrap();
        assert_eq!(opts.ssh_user, "");
        assert_eq!(opts.ssh_host, "myserver");
        assert_eq!(opts.ssh_port, "22");
    }

    #[test]
    fn test_parse_opts_empty() {
        assert!(parse_opts("").is_err());
    }

    #[test]
    fn test_parse_opts_invalid_port() {
        assert!(parse_opts("host:abc").is_err());
    }

    #[test]
    fn test_ssh_opts_effective_port() {
        let opts = SSHOpts::default();
        assert_eq!(opts.effective_port(), "22");

        let opts = SSHOpts::new("host", "", "2222");
        assert_eq!(opts.effective_port(), "2222");
    }

    #[test]
    fn test_ssh_opts_to_conn_name() {
        let opts = SSHOpts::new("host.com", "alice", "");
        assert_eq!(opts.to_conn_name(), "alice@host.com");

        let opts = SSHOpts::new("host.com", "alice", "2222");
        assert_eq!(opts.to_conn_name(), "alice@host.com:2222");

        let opts = SSHOpts::new("host.com", "", "");
        assert_eq!(opts.to_conn_name(), "host.com");
    }

    #[test]
    fn test_ssh_opts_serde_roundtrip() {
        let opts = SSHOpts::new("example.com", "alice", "2222");
        let json = serde_json::to_string(&opts).unwrap();
        assert!(json.contains("\"sshhost\":\"example.com\""));
        assert!(json.contains("\"sshuser\":\"alice\""));
        assert!(json.contains("\"sshport\":\"2222\""));
        let parsed: SSHOpts = serde_json::from_str(&json).unwrap();
        assert_eq!(opts, parsed);
    }

    #[test]
    fn test_ssh_opts_empty_fields_omitted() {
        let opts = SSHOpts::new("host.com", "", "");
        let json = serde_json::to_string(&opts).unwrap();
        assert!(!json.contains("sshuser"));
        assert!(!json.contains("sshport"));
    }

    #[test]
    fn test_conn_keywords_merge() {
        let mut kw1 = ConnKeywords {
            user: Some("alice".to_string()),
            hostname: None,
            identity_file: vec!["~/.ssh/id_rsa".to_string()],
            ..Default::default()
        };
        let kw2 = ConnKeywords {
            user: Some("bob".to_string()), // should NOT override
            hostname: Some("example.com".to_string()), // should fill
            identity_file: vec![
                "~/.ssh/id_ed25519".to_string(),
                "~/.ssh/id_rsa".to_string(), // duplicate, should not add
            ],
            port: Some("2222".to_string()),
            ..Default::default()
        };

        kw1.merge_from(&kw2);

        assert_eq!(kw1.user, Some("alice".to_string())); // preserved
        assert_eq!(kw1.hostname, Some("example.com".to_string())); // filled
        assert_eq!(kw1.port, Some("2222".to_string())); // filled
        assert_eq!(kw1.identity_file.len(), 2); // deduped
        assert!(kw1.identity_file.contains(&"~/.ssh/id_rsa".to_string()));
        assert!(kw1.identity_file.contains(&"~/.ssh/id_ed25519".to_string()));
    }

    #[test]
    fn test_conn_keywords_proxy_jump_hosts() {
        let kw = ConnKeywords {
            proxy_jump: Some("jump1, jump2, jump3".to_string()),
            ..Default::default()
        };
        let hosts = kw.proxy_jump_hosts();
        assert_eq!(hosts, vec!["jump1", "jump2", "jump3"]);
    }

    #[test]
    fn test_conn_keywords_no_proxy() {
        let kw = ConnKeywords::default();
        assert!(kw.proxy_jump_hosts().is_empty());
    }

    #[test]
    fn test_conn_flags_to_keywords() {
        let flags = ConnFlags {
            ssh_user: Some("alice".to_string()),
            ssh_port: Some("2222".to_string()),
            ssh_identity: Some("~/.ssh/custom_key".to_string()),
            ..Default::default()
        };
        let kw = flags.to_keywords();
        assert_eq!(kw.user, Some("alice".to_string()));
        assert_eq!(kw.port, Some("2222".to_string()));
        assert_eq!(kw.identity_file, vec!["~/.ssh/custom_key".to_string()]);
    }

    #[test]
    fn test_connection_error_display() {
        let err = ConnectionError {
            debug_info: ConnectionDebugInfo {
                current_host: "jump-host".to_string(),
                next_opts: SSHOpts::new("target", "alice", ""),
                jump_num: 2,
            },
            error: "connection refused".to_string(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("jump #2"));
        assert!(msg.contains("jump-host"));
        assert!(msg.contains("connection refused"));
    }

    #[test]
    fn test_connection_error_no_jump() {
        let err = ConnectionError {
            debug_info: ConnectionDebugInfo::default(),
            error: "timeout".to_string(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("timeout"));
        assert!(!msg.contains("jump"));
    }

    #[test]
    fn test_normalize_arch() {
        assert_eq!(normalize_arch("x86_64"), "x64");
        assert_eq!(normalize_arch("amd64"), "x64");
        assert_eq!(normalize_arch("aarch64"), "arm64");
        assert_eq!(normalize_arch("arm64"), "arm64");
        assert_eq!(normalize_arch("riscv64"), "riscv64");
    }

    #[test]
    fn test_normalize_os() {
        assert_eq!(normalize_os("Linux"), "linux");
        assert_eq!(normalize_os("Darwin"), "darwin");
        assert_eq!(normalize_os("FreeBSD"), "freebsd");
    }
}
