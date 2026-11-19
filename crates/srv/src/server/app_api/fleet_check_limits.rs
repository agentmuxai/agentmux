// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `fleet.check-limits`: look again, now, at agents marked out of quota (task #2648). The Swarm's
//! "Check now" calls it. Open to an agent as well as the window: it sends nothing in anyone's name, it
//! starts a small turn on each limited agent (`limit_probe::check_limited`).

use std::sync::Arc;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::*;

use super::AppState;

pub(super) fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_typed(COMMAND_FLEET_CHECK_LIMITS, move |cmd: CommandFleetCheckLimitsData, _ctx| {
        let state = state.clone();
        async move { Ok(check_limits_impl(&state, cmd.targets).await) }
    });
}

/// Starts a check on each of `targets` (block ids) that is out of quota; `succeeded` where one started or
/// runs, `failed` with why not.
pub(crate) async fn check_limits_impl(state: &AppState, targets: Vec<String>) -> FleetActionResult {
    let mstore = state.mstore.clone();
    let answers = tokio::task::spawn_blocking(move || crate::server::limit_probe::check_limited(&mstore, &targets)).await.unwrap_or_default();
    let mut result = FleetActionResult::default();
    for (id, answer) in answers {
        match answer {
            Ok(()) => result.succeeded.push(id),
            Err(error) => result.failed.push(FleetActionFailure { id, error }),
        }
    }
    result
}
