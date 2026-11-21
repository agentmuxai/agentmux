// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for the status bar's GPU panel (`gpu.sample`, `backend::gpu`;
//! SPEC_STATUS_BAR_GPU_PANEL_2026_11_15.md).

use super::tower::TowerProcessKind;
use serde::{Deserialize, Serialize};

/// What kind of graphics an adapter is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum GpuAdapterKind {
    /// The CPU's own graphics, which use the machine's memory.
    Integrated,
    /// A card with memory of its own.
    Discrete,
    /// The OS's software renderer: no GPU is doing the work.
    Software,
}

/// One adapter. An absent figure is one the OS wouldn't give, never zero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct GpuAdapter {
    /// Its id on this machine; what a process's figures are for.
    pub luid: String,
    pub name: String,
    pub kind: GpuAdapterKind,
    /// How busy its busiest engine is, percent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub use_pct: Option<f64>,
    /// Memory on the card, used and installed, bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub dedicated_used: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub dedicated_total: Option<u64>,
    /// The machine's memory it uses, and how much of it the OS lets it have, bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub shared_used: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub shared_total: Option<u64>,
}

/// One process that uses the GPU.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct GpuProcess {
    pub pid: u32,
    /// The image name, and nothing else of it: no window title, path or command line.
    pub name: String,
    /// One of AgentMux's own: what it is, as Tower says ("GPU", "Renderer (window Main)", "Server").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub label: Option<String>,
    /// Tower's kind of it, so the row has the same icon and colour there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub kind: Option<TowerProcessKind>,
    /// Percent of one adapter's busiest use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub use_pct: Option<f64>,
    /// Bytes, on the card and in the machine's memory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub dedicated: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub shared: Option<u64>,
}

/// The processes that are not listed: the rest of those using the GPU, added up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct GpuOthers {
    pub processes: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub use_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub dedicated: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub shared: Option<u64>,
}

/// What the sample is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum GpuSampleState {
    /// Read.
    Ok,
    /// The counters need two reads to mean anything: ask again.
    Warming,
    /// This computer has no GPU counters to read (none installed, or the OS won't give them).
    Unavailable,
    /// AgentMux doesn't read GPU figures on this operating system yet.
    Unsupported,
}

/// `gpu.sample`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct GpuSampleResult {
    #[ts(type = "number")]
    pub ts_ms: u64,
    /// `windows`, `macos` or `linux`.
    pub os: String,
    pub state: GpuSampleState,
    pub adapters: Vec<GpuAdapter>,
    /// AgentMux's own processes, then the biggest others, by use and then memory.
    pub processes: Vec<GpuProcess>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub others: Option<GpuOthers>,
    /// Whether this operating system says which process uses the GPU. Where it doesn't (macOS) the panel says so.
    pub per_process: bool,
    /// How often to ask again while the panel is open: longer than 2 s when a sample was slow.
    pub interval_ms: u32,
    /// How long the sample took.
    pub sample_ms: u32,
    /// The interval was widened because a sample went over its budget.
    pub widened: bool,
}

/// `gpu.sample`'s request: nothing yet.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct GpuSampleReq {}
