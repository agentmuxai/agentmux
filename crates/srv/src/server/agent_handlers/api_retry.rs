// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agentapiretry`: the API retry a pane's CLI is waiting on, for a pane that
//! opens mid-retry (`backend::agent_api_retry`).

use std::sync::Arc;

use crate::backend::agent_api_retry::{current_api_retry, CommandAgentApiRetryData};
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::COMMAND_AGENT_API_RETRY;

pub(super) fn register(engine: &Arc<WshRpcEngine>) {
    engine.register_typed(COMMAND_AGENT_API_RETRY, |cmd: CommandAgentApiRetryData, _ctx| async move {
        Ok::<_, String>(current_api_retry(&cmd.blockid))
    });
}
