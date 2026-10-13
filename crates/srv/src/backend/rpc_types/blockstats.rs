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

/// The machine a remote pane (SSH, WSL) is connected to.
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

/// One set of processes' figures: a pane's shell and everything it started,
/// on this computer or on the machine a remote pane is connected to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct BlockStatsFigures {
    pub processes: u32,
    /// The shell first, then the rest, most numerous first.
    pub names: Vec<BlockStatsName>,
    /// When the shell started, unix ms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub started_at_ms: Option<u64>,
    /// CPU, percent of one core. Remote only: a local pane's comes with the
    /// badge's own `blockstats` event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cpu: Option<f64>,
    /// Resident memory, bytes, shared pages included. Remote only, as `cpu`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub mem_resident: Option<u64>,
    /// Private memory, bytes, as `memory_metric` says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub mem_private: Option<u64>,
    /// Committed private bytes, in RAM or not (Windows only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub mem_commit: Option<u64>,
    /// What `mem_private` measures on that machine ("private working set", …).
    pub memory_metric: String,
}

/// A remote machine's totals, over all its processes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct BlockStatsHost {
    pub cpu_count: u32,
    /// Percent of one core; absent on the first sample.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cpu: Option<f64>,
    /// Private memory, bytes, as `memory_metric` says.
    #[ts(type = "number")]
    pub mem: u64,
    pub processes: u32,
    /// Of `processes`, those the machine wouldn't measure (other users' on
    /// macOS): `cpu` and `mem` are then only the rest's, not the host's.
    pub unmeasured: u32,
    pub memory_metric: String,
}

/// What a remote pane's machine says about the pane.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct BlockStatsRemote {
    /// How the pane's processes were found there: `session` (a durable
    /// pane's shell), `marker` (its id in their environment), `none` (the
    /// host can't tell them apart: `figures` is absent, `host` present), or
    /// `unavailable` (the machine couldn't be asked: see `reason`).
    pub found: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub figures: Option<BlockStatsFigures>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub host: Option<BlockStatsHost>,
    /// The highest `figures.mem_resident` seen for this shell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub peak: Option<u64>,
    /// Why the machine couldn't be asked (`unavailable`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
}

/// `blockstats.detail`: what a terminal's badge and its tooltip show beyond
/// the badge's own `blockstats` event. A local pane: its processes here, read
/// while the tooltip is open (one process snapshot costs too much to take
/// for every pane every second). A remote pane: its processes on the other
/// machine, asked of AgentMux's helper there, which the badge polls while
/// the pane is visible.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct BlockStatsDetail {
    /// A local pane's processes on this computer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub local: Option<BlockStatsFigures>,
    /// Present for a remote pane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub connection: Option<BlockStatsConnection>,
    /// A remote pane's processes on the other machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub remote: Option<BlockStatsRemote>,
}
