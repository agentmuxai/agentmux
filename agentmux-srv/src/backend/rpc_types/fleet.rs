// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for the fleet-control commands (`fleet.broadcast`,
//! `fleet.bulk-stop`, `fleet.group.*`) — see
//! docs/specs/SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md.

use serde::{Deserialize, Serialize};

/// One outcome bucket per bulk action, mirroring `ImportAgentDefinitionsResult`'s
/// partial-success shape — the spec's §3 finding that silent partial failure
/// is the single most commonly-cited fleet-ops pitfall means this must never
/// collapse to a single bool/count.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FleetActionResult {
    pub succeeded: Vec<String>,
    pub failed: Vec<FleetActionFailure>,
    /// True only when a staged run's `max_fail_percentage` was crossed and
    /// remaining batches were skipped — lets the frontend show "aborted
    /// early" distinctly from "ran to completion, some failed."
    #[serde(default)]
    pub aborted_early: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetActionFailure {
    pub id: String,
    pub error: String,
}

/// Input for fleet.broadcast. `targets` are block ids (Swarm's native
/// per-agent selection unit) — resolved server-side to each block's
/// registered agent name before delivery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandFleetBroadcastData {
    pub targets: Vec<String>,
    pub message: String,
}

/// Caps blast radius on a bulk-stop: targets are stopped in batches of
/// `batch_size`; if the failure rate within a completed batch exceeds
/// `max_fail_percentage`, remaining batches are skipped. A simplified,
/// fixed-batch-size take on Ansible's `serial` + `max_fail_percentage`
/// (spec §3/§5.3) — not the full canary-then-widen ladder, which is more
/// generality than a first version needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagePlanInput {
    pub batch_size: usize,
    pub max_fail_percentage: u8,
}

/// Input for fleet.bulk-stop. `targets` are block ids, same as broadcast.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandFleetBulkStopData {
    pub targets: Vec<String>,
    #[serde(default)]
    pub signal: Option<String>,
    #[serde(default)]
    pub staged: Option<StagePlanInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetGroup {
    pub id: String,
    pub name: String,
    pub member_ids: Vec<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandFleetGroupCreateData {
    pub name: String,
    #[serde(default)]
    pub member_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetGroupListResult {
    pub groups: Vec<FleetGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandFleetGroupUpdateData {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub member_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandFleetGroupDeleteData {
    pub id: String,
}
