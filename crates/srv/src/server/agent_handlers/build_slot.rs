// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `getbuildslot`: who holds this machine's build slot, for the Swarm row note
//! (docs/specs/SPEC_BUILD_SLOT_ENFORCEMENT_2026_10_20.md §6). Reads the lock's owner file; the
//! block it names is the local agent that matches the holder (`work_facts::build_slot_of`).

use std::sync::Arc;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{BuildSlotView, CommandGetBuildSlotData, COMMAND_GET_BUILD_SLOT};
use crate::backend::work_facts::build_slot_of;

use super::super::AppState;
use super::identity_env::block_agent_slug;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_typed(COMMAND_GET_BUILD_SLOT, move |_req: CommandGetBuildSlotData, _ctx| {
        let state = state.clone();
        async move {
            let regs = state.reactive_handler.list_agents();
            let store = state.mstore.clone();
            // File reads and store lookups, so off the async workers.
            let view = tokio::task::spawn_blocking(move || {
                let holder = agentmux_common::build_slot::holder()?;
                let block_id = regs
                    .iter()
                    .find(|reg| {
                        build_slot_of(Some(&holder), &reg.agent_id, || block_agent_slug(&store, &reg.block_id)).is_some()
                    })
                    .map(|reg| reg.block_id.clone());
                Some(BuildSlotView { agent: holder.agent, block_id, since_ms: holder.since_ms, reason: holder.reason, command: holder.command })
            })
            .await
            .map_err(|e| format!("getbuildslot: {e}"))?;
            Ok(view)
        }
    });
}
