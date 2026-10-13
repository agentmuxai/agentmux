// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for a terminal's stats badge tooltip (`blockstats.detail`,
//! `backend::blockstats_detail`).

use serde::{Deserialize, Serialize};

/// `blockstats.detail`'s request.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct BlockStatsDetailReq {
    pub block_id: String,
}

/// One executable among a pane's processes, and how many of it run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct BlockStatsName {
    pub name: String,
    pub count: u32,
}

/// The machine a remote pane (SSH, WSL) is connected to. The badge's figures
/// are still this computer's: the `ssh` or `wsl.exe` that carries the
/// connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct BlockStatsConnection {
    /// The pane's `connection`, as written: `user@host`, an ssh alias, or
    /// `wsl://<distro>`.
    pub name: String,
    /// `ssh` or `wsl`.
    pub kind: String,
    /// SSH: the host name `ssh` connects to. WSL: the distribution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub host: Option<String>,
    /// Its IP address, when it could be found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub address: Option<String>,
}

/// `blockstats.detail`: what the badge's tooltip shows beyond the badge's own
/// CPU and memory, for the same processes (the pane's shell and everything
/// it started). Read on demand, while the tooltip is open: one process
/// snapshot costs too much to take for every pane every second.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct BlockStatsDetail {
    pub processes: u32,
    /// The shell first, then the rest, most numerous first.
    pub names: Vec<BlockStatsName>,
    /// When the shell started, unix ms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub started_at_ms: Option<u64>,
    /// Private memory, bytes, as `memory_metric` says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub mem_private: Option<u64>,
    /// Committed private bytes, in RAM or not (Windows only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub mem_commit: Option<u64>,
    /// What `mem_private` measures on this OS ("private working set", …).
    pub memory_metric: String,
    /// Present for a remote pane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub connection: Option<BlockStatsConnection>,
}
