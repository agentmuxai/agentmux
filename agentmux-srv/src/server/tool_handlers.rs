// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! HTTP/WebSocket RPC handlers for the tool store.
//! Registers `gettoolstatus` and `installtool` commands with the WshRpcEngine.

use std::sync::Arc;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    CommandGetToolStatusData, CommandInstallToolData, GetToolStatusResult, InstallFailure,
    InstallToolResult,
    COMMAND_GET_TOOL_STATUS, COMMAND_INSTALL_TOOL,
};
use crate::backend::tool_store;

use super::AppState;

pub fn register_tool_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let http_client = state.http_client.clone();

    // gettoolstatus → return current install status of all catalog tools
    engine.register_typed(
        COMMAND_GET_TOOL_STATUS,
        move |_req: CommandGetToolStatusData, _ctx| async move {
            Ok(GetToolStatusResult { tools: tool_store::get_tool_statuses() })
        },
    );

    // installtool → download + verify + install requested tools
    engine.register_typed(
        COMMAND_INSTALL_TOOL,
        move |cmd: CommandInstallToolData, _ctx| {
            let client = http_client.clone();
            async move {

                let mut installed = Vec::new();
                let mut failed = Vec::new();

                for id in &cmd.tool_ids {
                    match tool_store::install_tool(id, &client).await {
                        Ok(path) => {
                            tracing::info!(tool = %id, path = %path, "tool installed");
                            installed.push(id.clone());
                        }
                        Err(e) => {
                            tracing::warn!(tool = %id, error = %e, "tool install failed");
                            failed.push(InstallFailure {
                                id: id.clone(),
                                error: e,
                            });
                        }
                    }
                }

                Ok(InstallToolResult { installed, failed })
            }
        },
    );
}
