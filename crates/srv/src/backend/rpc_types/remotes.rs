// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Remotes pane's commands and the records they return
//! (docs/specs/SPEC_REMOTES_PANE_2026_10_05.md §4.6). Built by
//! `backend::remote::remotes`.

use serde::{Deserialize, Serialize};

/// `remoteslist`: no arguments.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandRemotesListData {}

/// One remote machine, as the Remotes pane shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct RemoteRecord {
    /// The connection name, as a pane's `connection` meta takes it.
    pub name: String,
    /// `ssh` or `wsl`.
    pub kind: String,
    /// Where AgentMux knows it from: `ssh_config`, `settings`, `recent`, `wsl`.
    pub sources: Vec<String>,
    pub status: RemoteStatus,
    /// From the host's `uname -sm`, once AgentMux has seen it.
    pub platform: Option<RemotePlatform>,
    pub helper: RemoteHelper,
    /// Durable sessions on the host when last listed; `null` if never listed.
    pub sessions: Option<u32>,
    /// Agents the user has always allowed on this host.
    pub agents: Vec<String>,
    /// Its `connections.<name>` settings, as stored.
    #[ts(type = "Record<string, unknown>")]
    pub settings: serde_json::Value,
    /// When it last reached `connected` (recent connections only), Unix ms.
    #[ts(type = "number | null")]
    pub last_used_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct RemoteStatus {
    /// `connecting`, `connected`, `disconnected` or `error`.
    pub state: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct RemotePlatform {
    /// `linux`, `macos`, or the lowercased `uname -s` for anything else.
    pub os: String,
    /// `x86_64` or `arm64` for the common machines; otherwise as printed.
    pub arch: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct RemoteHelper {
    /// `installed`, `absent`, `never` (the user said never), `unsupported` (no
    /// build for the platform), or `none` (WSL has no helper).
    pub state: String,
    /// The AgentMux version whose helper last answered (when installed).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
}

/// `remotesetconfig`: change `connection`'s settings (`settings.json` →
/// `connections.<connection>`). A `null` value removes that key.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandRemoteSetConfigData {
    pub connection: String,
    #[ts(type = "Record<string, unknown>")]
    pub values: serde_json::Map<String, serde_json::Value>,
}

/// `remoteforget`: remove `connection` from the recent list.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandRemoteForgetData {
    pub connection: String,
}
