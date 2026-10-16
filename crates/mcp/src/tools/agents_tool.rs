// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The agent and messaging tools that the Agents and Message tools replace in
//! the next slice (docs/specs/SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md
//! §7, S4). Until then they pass through here unchanged: their handler files
//! (fleet.rs, panes.rs) also hold tools that Stop and Pane replaced, so those
//! files are no longer routed to directly.

use super::*;

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "SendMessage" | "DiscoverAgents" | "FleetList" | "FleetBroadcast" | "SupervisorNudge" => {
            fleet::call(name, arguments, cx).await
        }
        "OpenAgent" => panes::call(name, arguments, cx).await,
        _ => Err(not_in_family()),
    }
}
