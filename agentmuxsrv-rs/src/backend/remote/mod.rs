// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Remote connections: SSH, WSL, and generic shell client abstractions.
//! Port of Go's pkg/remote/, pkg/genconn/, pkg/wsl/, pkg/wslconn/.
//!
//! Architecture:
//! - `connparse`: URI parsing for connection strings (wsh://, ssh://, s3://)
//! - `sshclient`: SSH configuration types and option parsing
//! - `conncontroller`: Connection state machine (init→connecting→connected→disconnected)
//! - `genconn`: Generic ShellClient/ShellProcessController traits

#![allow(dead_code)]

pub mod conncontroller;
pub mod connparse;
pub mod genconn;
pub mod sshclient;


// ---- Connection type constants ----

/// Local connection (no remote).
pub const CONN_TYPE_LOCAL: &str = "";

/// SSH connection.
pub const CONN_TYPE_SSH: &str = "ssh";

/// WSL connection.
pub const CONN_TYPE_WSL: &str = "wsl";

/// Connection status constants.
pub const STATUS_INIT: &str = "init";
pub const STATUS_CONNECTING: &str = "connecting";
pub const STATUS_CONNECTED: &str = "connected";
pub const STATUS_DISCONNECTED: &str = "disconnected";
pub const STATUS_ERROR: &str = "error";

/// Local connection name (empty string in Go).
pub const LOCAL_CONN_NAME: &str = "";

/// Maximum proxy jump chain depth for SSH.
pub const SSH_PROXY_JUMP_MAX_DEPTH: i32 = 10;

/// WSH binary name.
pub const WSH_BINARY_NAME: &str = "wsh";

/// Remote domain socket path template.
pub const REMOTE_DOMAIN_SOCKET_DIR: &str = "/tmp";

/// Fixed WSL domain socket path.
pub const WSL_DOMAIN_SOCKET_PATH: &str = "/var/run/wsh.sock";
