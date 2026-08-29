// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Miscellaneous RPC payloads: authentication, id resolution, file data,
//! events, route/wait, block/wave/workspace info, connection status,
//! notifications, rpc opts/context, vars, time series, remote info, and the
//! tool-store command shapes.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::backend::oref::ORef;
use crate::backend::obj::{Block, Workspace};

use super::{is_zero_i64, is_zero_usize};

/// Matches Go's `CommandAuthenticateRtnData`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandAuthenticateRtnData {
    pub routeid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub authtoken: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub initscripttext: String,
}

/// Matches Go's `CommandAuthenticateTokenData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandAuthenticateTokenData {
    pub token: String,
}

/// Matches Go's `CommandDisposeData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandDisposeData {
    pub routeid: String,
}

/// Matches Go's `CommandResolveIdsData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResolveIdsData {
    #[serde(default)]
    pub blockid: String,
    pub ids: Vec<String>,
}

/// Matches Go's `CommandResolveIdsRtnData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResolveIdsRtnData {
    pub resolvedids: HashMap<String, ORef>,
}

/// Matches Go's `FileDataAt`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDataAt {
    pub offset: i64,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub size: usize,
}

/// Matches Go's `FileData`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FileData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<FileInfo>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub data64: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entries: Option<Vec<FileInfo>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<FileDataAt>,
}

/// Matches Go's `FileInfo`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FileInfo {
    pub path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub dir: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub notfound: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opts: Option<FileOpts>,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub size: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<HashMap<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub modtime: i64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub isdir: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mimetype: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub readonly: bool,
}

/// Matches Go's `FileOpts`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FileOpts {
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub maxsize: i64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub circular: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ijson: bool,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub ijsonbudget: usize,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncate: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub append: bool,
}

/// Matches Go's `CommandEventReadHistoryData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandEventReadHistoryData {
    pub event: String,
    pub scope: String,
    #[serde(default)]
    pub maxitems: usize,
}

/// Matches Go's `CommandWaitForRouteData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandWaitForRouteData {
    pub routeid: String,
    #[serde(default)]
    pub waitms: i64,
}

/// Matches Go's `BlockInfoData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockInfoData {
    pub blockid: String,
    pub tabid: String,
    pub workspaceid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<Block>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<FileInfo>>,
}

/// Matches Go's `WaveInfoData`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WaveInfoData {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub clientid: String,
    #[serde(default)]
    pub buildtime: String,
    #[serde(default)]
    pub configdir: String,
    #[serde(default)]
    pub datadir: String,
}

/// Matches Go's `WorkspaceInfoData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInfoData {
    pub windowid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspacedata: Option<Workspace>,
}

/// Matches Go's `ConnStatus`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConnStatus {
    pub status: String,
    #[serde(default)]
    pub connection: String,
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub hasconnected: bool,
    #[serde(default)]
    pub activeconnnum: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}

/// Matches Go's `WaveNotificationOptions`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WaveNotificationOptions {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub silent: bool,
}

/// Matches Go's `RpcOpts`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RpcOpts {
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub timeout: i64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub noresponse: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub route: String,
}

/// Matches Go's `RpcContext`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RpcContext {
    #[serde(default, skip_serializing_if = "String::is_empty", rename = "ctype")]
    pub client_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub blockid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tabid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub conn: String,
    /// Slug of the authenticated agent from `bus:register`. Empty for non-agent
    /// connections (plain WebSocket, CEF UI). App API handlers use this for S1
    /// enforcement — reject if empty (unauthenticated), reject if ≠ request agent_id.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub agent_id: String,
}

/// Matches Go's `CommandVarData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandVarData {
    pub key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub val: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub remove: bool,
    #[serde(default)]
    pub zoneid: String,
    #[serde(default)]
    pub filename: String,
}

/// Matches Go's `CommandVarResponseData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandVarResponseData {
    pub key: String,
    #[serde(default)]
    pub val: String,
    #[serde(default)]
    pub exists: bool,
}

/// Matches Go's `TimeSeriesData`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSeriesData {
    pub ts: i64,
    pub values: HashMap<String, f64>,
    /// Backend uptime in seconds, from a MONOTONIC clock (`sysinfo::uptime_secs`)
    /// — NOT `ts` minus some earlier wall-clock stamp. Only the top-level
    /// sysinfo tick carries it; per-block stats and the CPU-stream RPCs leave
    /// it `None`, hence `skip_serializing_if` so their payload shape is
    /// byte-for-byte unchanged.
    ///
    /// Exists because the frontend previously derived uptime by subtracting
    /// the host's wall-clock `backend_started_at` from this struct's own
    /// wall-clock `ts`, which goes permanently negative after any backwards
    /// clock step. See `frontend/app/statusbar/backend-uptime.ts`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uptime_secs: Option<u64>,
}

/// Matches Go's `RemoteInfo`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RemoteInfo {
    #[serde(default)]
    pub clientarch: String,
    #[serde(default)]
    pub clientos: String,
    #[serde(default)]
    pub clientversion: String,
    #[serde(default)]
    pub shell: String,
}

// ---- Tool store command data types ----

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandInstallToolData {
    pub tool_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetToolStatusResult {
    pub tools: Vec<crate::backend::tool_store::ToolStatusEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallToolResult {
    pub installed: Vec<String>,
    pub failed: Vec<InstallFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallFailure {
    pub id: String,
    pub error: String,
}
