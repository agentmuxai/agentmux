// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for `getbuildslot`: who holds this machine's build slot
//! (`agentmux_common::build_slot`, docs/specs/SPEC_BUILD_SLOT_ENFORCEMENT_2026_10_20.md §6).

use serde::{Deserialize, Serialize};

/// Data for `COMMAND_GET_BUILD_SLOT`: nothing to ask. The answer is the holder, or null when the
/// slot is free (or its holder died).
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandGetBuildSlotData {}

/// The agent holding the build slot, as the Swarm shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct BuildSlotView {
    /// The holder's `AGENTMUX_AGENT_ID`, as the lock recorded it.
    pub agent: String,
    /// The block of the local agent that holds it, when one matches (by slug or name). Absent
    /// when the holder is not an agent of this AgentMux (a person's shell, another channel).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub block_id: Option<String>,
    /// When it took the slot, epoch milliseconds.
    #[ts(type = "number")]
    pub since_ms: u64,
    /// Why: the shim's reason (`cargo test`), else the command line.
    pub reason: String,
    /// The command line, cut to 120 characters. A background subagent's build holds the slot
    /// under its leader's id, and this says what is really running.
    pub command: String,
}
