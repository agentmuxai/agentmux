// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Connection URI parsing and resolution.
//! Port of Go's pkg/remote/connparse/connparse.go.

//!
//! Supports URIs like:
//! - `wsh://user@host/path`
//! - `ssh://host:port/path`
//! - `s3://bucket/key`
//! - `wavefile:///path`
//! - Shorthand: `host:/path` or just `/path`

use serde::{Deserialize, Serialize};

// ---- Connection scheme constants ----

/// Default shell connection scheme.
pub const SCHEME_WSH: &str = "wsh";

/// S3 file system scheme.
pub const SCHEME_S3: &str = "s3";

/// Wave internal file system scheme.
pub const SCHEME_WAVE: &str = "wavefile";

/// SSH scheme (for explicit SSH URIs).
pub const SCHEME_SSH: &str = "ssh";

/// WSL scheme.
pub const SCHEME_WSL: &str = "wsl";

// ---- Special host names ----

/// Use the current connection from RPC context.
pub const CONN_HOST_CURRENT: &str = "current";

/// Server-side connection.
pub const CONN_HOST_WAVE_SRV: &str = "agentmuxsrv";

// ---- Connection type ----

/// Parsed connection URI with scheme, host, and path components.
///
/// Examples:
/// - `wsh://user@host/path` → scheme="wsh", host="user@host", path="/path"
/// - `s3://bucket/key` → scheme="s3", host="bucket", path="/key"
/// - `/local/path` → scheme="", host="", path="/local/path"
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    /// URI scheme (e.g., "wsh", "ssh", "s3", "wavefile").
    pub scheme: String,
    /// Host component (may include user@ prefix).
    pub host: String,
    /// Path component (always starts with / when present).
    pub path: String,
}

impl Connection {
    /// Create a new Connection with the given components.
    pub fn new(scheme: &str, host: &str, path: &str) -> Self {
        Self {
            scheme: scheme.to_string(),
            host: host.to_string(),
            path: path.to_string(),
        }
    }

    /// Create an empty connection (local, no path).
    pub fn empty() -> Self {
        Self {
            scheme: String::new(),
            host: String::new(),
            path: String::new(),
        }
    }

    /// Get scheme parts split by `:`.
    /// For example, "ssh:cmd" → ["ssh", "cmd"].
    pub fn get_scheme_parts(&self) -> Vec<&str> {
        if self.scheme.is_empty() {
            return vec![];
        }
        self.scheme.split(':').collect()
    }

    /// Get the connection type (last component after final `:` in scheme).
    /// For "ssh:cmd" returns "cmd", for "wsh" returns "wsh".
    pub fn get_type(&self) -> &str {
        if let Some(pos) = self.scheme.rfind(':') {
            &self.scheme[pos + 1..]
        } else {
            &self.scheme
        }
    }

    /// Combine host and path with `/` separator.
    pub fn get_path_with_host(&self) -> String {
        if self.host.is_empty() {
            self.path.clone()
        } else if self.path.is_empty() {
            self.host.clone()
        } else {
            format!("{}/{}", self.host, self.path.trim_start_matches('/'))
        }
    }

    /// Reconstruct the full URI: `scheme://host/path`.
    pub fn get_full_uri(&self) -> String {
        if self.scheme.is_empty() {
            return self.get_path_with_host();
        }
        if self.host.is_empty() && self.path.is_empty() {
            return format!("{}://", self.scheme);
        }
        if self.host.is_empty() {
            return format!("{}://{}", self.scheme, self.path);
        }
        if self.path.is_empty() {
            return format!("{}://{}", self.scheme, self.host);
        }
        format!(
            "{}://{}/{}",
            self.scheme,
            self.host,
            self.path.trim_start_matches('/')
        )
    }

    /// Get scheme and host: `scheme://host`.
    pub fn get_scheme_and_host(&self) -> String {
        if self.scheme.is_empty() {
            return self.host.clone();
        }
        format!("{}://{}", self.scheme, self.host)
    }

    /// Check if this is a local connection.
    pub fn is_local(&self) -> bool {
        self.host.is_empty() || self.host == super::LOCAL_CONN_NAME
    }

    /// Check if this connection targets the Wave server.
    pub fn is_wave_srv(&self) -> bool {
        self.host == CONN_HOST_WAVE_SRV
    }

    /// Check if this uses the "current" connection placeholder.
    pub fn is_current(&self) -> bool {
        self.host == CONN_HOST_CURRENT
    }
}

/// Parse a connection URI string into its components.
///
/// Supported formats:
/// - Full URI: `scheme://host/path`
/// - Host shorthand: `host:/path`
/// - Local path: `/path/to/file`
/// - Windows drive: `C:\path` (detected but not common in remote context)
/// - WSL: `wsl://distro/path`
///
/// # Examples
///
/// ```
/// use backend_test::backend::remote::connparse::parse_uri;
///
/// let conn = parse_uri("wsh://user@host/path/to/file");
/// assert_eq!(conn.scheme, "wsh");
/// assert_eq!(conn.host, "user@host");
/// assert_eq!(conn.path, "/path/to/file");
///
/// let conn = parse_uri("/local/path");
/// assert_eq!(conn.scheme, "");
/// assert_eq!(conn.host, "");
/// assert_eq!(conn.path, "/local/path");
/// ```
pub fn parse_uri(uri: &str) -> Connection {
    let uri = uri.trim();

    if uri.is_empty() {
        return Connection::empty();
    }

    // Check for scheme:// prefix
    if let Some(rest) = try_strip_scheme(uri) {
        let (scheme, after_scheme) = rest;
        // Split on first '/' after the host
        if let Some(slash_pos) = after_scheme.find('/') {
            let host = &after_scheme[..slash_pos];
            let path = &after_scheme[slash_pos..];
            return Connection::new(scheme, host, path);
        } else {
            // No path, just scheme://host
            return Connection::new(scheme, after_scheme, "");
        }
    }

    // Check for host:/path shorthand (not a Windows drive letter)
    if let Some(colon_pos) = uri.find(':') {
        let before_colon = &uri[..colon_pos];
        let after_colon = &uri[colon_pos + 1..];

        // Windows drive letter check: single letter followed by colon
        let is_windows_drive = before_colon.len() == 1 && before_colon.chars().next().is_some_and(|c| c.is_ascii_alphabetic());

        if !is_windows_drive && !before_colon.is_empty() && after_colon.starts_with('/') {
            // This is host:/path
            return Connection::new("", before_colon, after_colon);
        }
    }

    // Plain path (local)
    Connection::new("", "", uri)
}

/// Try to extract a scheme from a URI.
/// Returns Some((scheme, rest_after_://)) if found.
fn try_strip_scheme(uri: &str) -> Option<(&str, &str)> {
    // Look for "://" pattern
    let scheme_end = uri.find("://")?;
    let scheme = &uri[..scheme_end];

    // Validate scheme: must be non-empty and contain only valid chars
    if scheme.is_empty() {
        return None;
    }
    for c in scheme.chars() {
        if !c.is_ascii_alphanumeric() && c != '+' && c != '-' && c != '.' && c != ':' {
            return None;
        }
    }

    let rest = &uri[scheme_end + 3..]; // skip "://"
    Some((scheme, rest))
}

/// Format a connection name from user, host, and port.
/// Returns normalized form like "user@host:port".
/// Omits user@ if empty, omits :port if empty or "22".
pub fn format_conn_name(user: &str, host: &str, port: &str) -> String {
    let mut result = String::new();

    if !user.is_empty() {
        result.push_str(user);
        result.push('@');
    }

    result.push_str(host);

    if !port.is_empty() && port != "22" {
        result.push(':');
        result.push_str(port);
    }

    result
}

/// Determine the connection type from a connection name.
/// Returns CONN_TYPE_WSL for "wsl://*", CONN_TYPE_SSH for others with a host.
pub fn get_conn_type(conn_name: &str) -> &'static str {
    if conn_name.is_empty() {
        return super::CONN_TYPE_LOCAL;
    }
    if conn_name.starts_with("wsl://") {
        return super::CONN_TYPE_WSL;
    }
    super::CONN_TYPE_SSH
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_uri_full() {
        let conn = parse_uri("wsh://user@host/path/to/file");
        assert_eq!(conn.scheme, "wsh");
        assert_eq!(conn.host, "user@host");
        assert_eq!(conn.path, "/path/to/file");
    }

    #[test]
    fn test_parse_uri_no_path() {
        let conn = parse_uri("ssh://myhost");
        assert_eq!(conn.scheme, "ssh");
        assert_eq!(conn.host, "myhost");
        assert_eq!(conn.path, "");
    }

    #[test]
    fn test_parse_uri_s3() {
        let conn = parse_uri("s3://mybucket/some/key");
        assert_eq!(conn.scheme, "s3");
        assert_eq!(conn.host, "mybucket");
        assert_eq!(conn.path, "/some/key");
    }

    #[test]
    fn test_parse_uri_wsl() {
        let conn = parse_uri("wsl://Ubuntu/home/user");
        assert_eq!(conn.scheme, "wsl");
        assert_eq!(conn.host, "Ubuntu");
        assert_eq!(conn.path, "/home/user");
    }

    #[test]
    fn test_parse_uri_host_shorthand() {
        let conn = parse_uri("myhost:/path/to/file");
        assert_eq!(conn.scheme, "");
        assert_eq!(conn.host, "myhost");
        assert_eq!(conn.path, "/path/to/file");
    }

    #[test]
    fn test_parse_uri_local_path() {
        let conn = parse_uri("/home/user/file.txt");
        assert_eq!(conn.scheme, "");
        assert_eq!(conn.host, "");
        assert_eq!(conn.path, "/home/user/file.txt");
    }

    #[test]
    fn test_parse_uri_empty() {
        let conn = parse_uri("");
        assert_eq!(conn, Connection::empty());
    }

    #[test]
    fn test_parse_uri_relative_path() {
        let conn = parse_uri("file.txt");
        assert_eq!(conn.scheme, "");
        assert_eq!(conn.host, "");
        assert_eq!(conn.path, "file.txt");
    }

    #[test]
    fn test_connection_get_full_uri() {
        let conn = Connection::new("wsh", "user@host", "/path");
        assert_eq!(conn.get_full_uri(), "wsh://user@host/path");
    }

    #[test]
    fn test_connection_get_full_uri_no_scheme() {
        let conn = Connection::new("", "host", "/path");
        assert_eq!(conn.get_full_uri(), "host/path");
    }

    #[test]
    fn test_connection_get_full_uri_no_host() {
        let conn = Connection::new("wavefile", "", "/local/path");
        assert_eq!(conn.get_full_uri(), "wavefile:///local/path");
    }

    #[test]
    fn test_connection_get_scheme_parts() {
        let conn = Connection::new("ssh:cmd", "host", "/path");
        assert_eq!(conn.get_scheme_parts(), vec!["ssh", "cmd"]);
    }

    #[test]
    fn test_connection_get_scheme_parts_simple() {
        let conn = Connection::new("wsh", "host", "");
        assert_eq!(conn.get_scheme_parts(), vec!["wsh"]);
    }

    #[test]
    fn test_connection_get_type() {
        let conn = Connection::new("ssh:cmd", "", "");
        assert_eq!(conn.get_type(), "cmd");

        let conn = Connection::new("wsh", "", "");
        assert_eq!(conn.get_type(), "wsh");
    }

    #[test]
    fn test_connection_get_scheme_and_host() {
        let conn = Connection::new("wsh", "user@host", "/path");
        assert_eq!(conn.get_scheme_and_host(), "wsh://user@host");
    }

    #[test]
    fn test_connection_is_local() {
        assert!(Connection::new("", "", "/path").is_local());
        assert!(!Connection::new("ssh", "host", "").is_local());
    }

    #[test]
    fn test_connection_is_wave_srv() {
        assert!(Connection::new("wsh", "agentmuxsrv", "").is_wave_srv());
        assert!(!Connection::new("wsh", "myhost", "").is_wave_srv());
    }

    #[test]
    fn test_connection_is_current() {
        assert!(Connection::new("wsh", "current", "").is_current());
        assert!(!Connection::new("wsh", "myhost", "").is_current());
    }

    #[test]
    fn test_connection_get_path_with_host() {
        let conn = Connection::new("", "host", "/path/to/file");
        assert_eq!(conn.get_path_with_host(), "host/path/to/file");

        let conn = Connection::new("", "", "/local/path");
        assert_eq!(conn.get_path_with_host(), "/local/path");
    }

    #[test]
    fn test_format_conn_name() {
        assert_eq!(format_conn_name("user", "host", "22"), "user@host");
        assert_eq!(format_conn_name("user", "host", "2222"), "user@host:2222");
        assert_eq!(format_conn_name("", "host", ""), "host");
        assert_eq!(format_conn_name("user", "host", ""), "user@host");
    }

    #[test]
    fn test_get_conn_type() {
        assert_eq!(get_conn_type(""), super::super::CONN_TYPE_LOCAL);
        assert_eq!(get_conn_type("wsl://Ubuntu"), super::super::CONN_TYPE_WSL);
        assert_eq!(get_conn_type("user@host"), super::super::CONN_TYPE_SSH);
        assert_eq!(get_conn_type("myhost"), super::super::CONN_TYPE_SSH);
    }

    #[test]
    fn test_parse_uri_wavefile() {
        let conn = parse_uri("wavefile:///block/file.txt");
        assert_eq!(conn.scheme, "wavefile");
        assert_eq!(conn.host, "");
        assert_eq!(conn.path, "/block/file.txt");
    }

    #[test]
    fn test_connection_serde_roundtrip() {
        let conn = Connection::new("wsh", "user@host", "/path");
        let json = serde_json::to_string(&conn).unwrap();
        let parsed: Connection = serde_json::from_str(&json).unwrap();
        assert_eq!(conn, parsed);
    }
}
