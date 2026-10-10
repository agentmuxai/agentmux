// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for Tower, the read-only task manager pane (`tower.*`,
//! `backend::tower_sampler`; SPEC_TOWER_TASK_MANAGER_PANE_2026_10_08.md).

use serde::{Deserialize, Serialize};

/// What a task is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum TowerTaskKind {
    /// An agent pane: its CLI and everything it started.
    Agent,
    /// A terminal pane: its shell and everything started in it.
    Terminal,
    /// AgentMux itself: launcher, backend, the window host and its helpers.
    Agentmux,
}

/// A process's part in its task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum TowerProcessRole {
    /// The pane's own process: the agent CLI, or the terminal's shell.
    Main,
    /// Started for the agent rather than by it: its MCP servers, conhost.
    Support,
    /// Started by the agent's tools or typed in the terminal.
    Started,
}

/// One process. CPU is a fraction of one core since the previous sample
/// (1.0 = one core fully busy); the pane divides by `cpu_count` for "percent
/// of the machine". Absent values are ones the OS wouldn't give without
/// elevation, never zeros.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct TowerProcess {
    /// `pid:start`, unique across PID reuse.
    pub id: String,
    pub pid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ppid: Option<u32>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub started_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cpu: Option<f64>,
    /// Private memory, bytes (`TowerSnapshot::memory_metric`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub mem: Option<u64>,
    /// Working set / resident set, shared pages included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub mem_resident: Option<u64>,
    /// Committed private bytes (Windows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub mem_commit: Option<u64>,
    /// In a task's list: its part in the task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub role: Option<TowerProcessRole>,
    /// In the host list: the task it belongs to (`TowerTask::id`), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub task: Option<String>,
    /// One of AgentMux's own processes: what it is ("GPU", "Renderer",
    /// "Network service", "Server", "Launcher", …). Absent for any other
    /// process, and for one of AgentMux's the backend can't place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub detail: Option<String>,
}

/// A pane and every process it started, or AgentMux itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct TowerTask {
    /// The block id, or `agentmux`.
    pub id: String,
    pub kind: TowerTaskKind,
    /// The agent's name, or what the pane runs.
    pub label: String,
    /// How its processes were found: `high` (a Job Object or cgroup nothing
    /// escapes), `best_effort` (a process scan; a detached child may be
    /// missed) or `tree` (descendants of the pane's process).
    pub tracking: String,
    /// Fraction of one core. Where `cpu_account` is set it includes members
    /// that exited since the previous sample.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cpu: Option<f64>,
    pub cpu_account: bool,
    /// Summed private memory of the processes the OS measured, bytes.
    #[ts(type = "number")]
    pub mem: u64,
    pub processes: Vec<TowerProcess>,
}

/// Every process on the machine this user may see.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct TowerHost {
    pub processes: Vec<TowerProcess>,
    /// Processes the OS listed without CPU or memory: other users' on macOS,
    /// which only root may measure.
    pub unmeasured: u32,
    /// All processes' CPU, fraction of one core. Absent until there is a
    /// previous sample to measure against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cpu: Option<f64>,
    /// All processes' private memory, bytes.
    #[ts(type = "number")]
    pub mem: u64,
    /// Every process the machine listed. On another machine `processes` holds
    /// only the busiest and the largest of those matching the filter.
    pub total: u32,
    /// Those matching the request's filter (`total` without one).
    pub matched: u32,
}

/// `tower.sample`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct TowerSnapshot {
    #[ts(type = "number")]
    pub ts_ms: u64,
    pub hostname: String,
    /// `windows`, `macos` or `linux`.
    pub os: String,
    /// Logical CPUs on the machine.
    pub cpu_count: u32,
    /// What `mem` measures here ("private working set", …).
    pub memory_metric: String,
    /// How often to ask again while the pane is visible.
    pub interval_ms: u32,
    /// Another machine's (`TowerSampleReq::connection`): no tasks, no
    /// command lines.
    pub remote: bool,
    pub tasks: Vec<TowerTask>,
    /// Present when the request asked for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub host: Option<TowerHost>,
}

/// `tower.sample`'s request.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct TowerSampleReq {
    /// Also list every process on the machine (the Host view).
    #[serde(default)]
    #[ts(optional)]
    pub host: Option<bool>,
    /// Another machine: an SSH connection or `wsl://<distro>`, as a pane's
    /// `connection` meta names it, or a paired AgentMux computer
    /// (`peer:<id>`). Absent or `local`: this computer.
    #[serde(default)]
    #[ts(optional)]
    pub connection: Option<String>,
    /// On another machine, only processes whose name or PID holds every word.
    #[serde(default)]
    #[ts(optional)]
    pub filter: Option<String>,
    /// The asking pane, where ssh's prompts (a password, a host key) go.
    #[serde(default)]
    #[ts(optional)]
    pub block_id: Option<String>,
}



/// Another AgentMux computer this one is paired with (`tower.peers`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct TowerPeerInfo {
    /// What `TowerSampleReq::connection` takes: `peer:<id>`.
    pub connection: String,
    pub hostname: String,
    /// Where it was last reached, `address:port`.
    pub address: String,
    #[ts(type = "number")]
    pub paired_ms: i64,
}

/// `tower.peers`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct TowerPeersResult {
    pub peers: Vec<TowerPeerInfo>,
}

/// `tower.pair`: the other computer's pairing link.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct TowerPairReq {
    pub link: String,
}

/// `tower.forget`: a `TowerPeerInfo::connection`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct TowerForgetReq {
    pub connection: String,
}
