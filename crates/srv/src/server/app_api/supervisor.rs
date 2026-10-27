// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `supervisor.set-mode`: the Swarm's Supervisor control, for one row or a
//! fleet selection. WS-RPC only, like `fleet.broadcast`: it is the user's
//! control in Swarm, and no MCP tool sets it, so an agent can't put itself (or
//! another agent) in Drive. docs/specs/SPEC_SWARM_SUPERVISOR_2026_10_16.md §3.1.

use std::sync::Arc;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::*;

use super::AppState;

pub fn register(engine: &Arc<WshRpcEngine>, _state: &AppState) {
    engine.register_typed(COMMAND_SUPERVISOR_SET_MODE, move |cmd: CommandSupervisorSetModeData, ctx| async move {
        // Whoever asks is recorded; the mode itself is the Swarm's to set, and
        // refused for Steady/Drive while the Supervisor is switched off.
        let caller = if ctx.agent_id.is_empty() { "the app".to_string() } else { format!("agent {}", ctx.agent_id) };
        tracing::info!(
            caller = %caller,
            mode = cmd.mode.as_str(),
            targets = cmd.targets.len(),
            "supervisor.set-mode"
        );
        Ok(set_mode(cmd).await)
    });
}

async fn set_mode(cmd: CommandSupervisorSetModeData) -> FleetActionResult {
    let mut result = FleetActionResult::default();
    let Some(supervisor) = crate::backend::supervisor::watcher::instance() else {
        result.failed = cmd
            .targets
            .into_iter()
            .map(|id| FleetActionFailure { id, error: "the Supervisor is not running".to_string() })
            .collect();
        return result;
    };
    for block_id in cmd.targets {
        match supervisor.set_mode(&block_id, cmd.mode).await {
            Ok(()) => result.succeeded.push(block_id),
            Err(error) => result.failed.push(FleetActionFailure { id: block_id, error }),
        }
    }
    result
}
