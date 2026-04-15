// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! HTTP/WebSocket RPC handlers for the tool store.
//! Registers `gettoolstatus` and `installtool` commands with the WshRpcEngine.

use std::sync::Arc;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    CommandInstallToolData, GetToolStatusResult, InstallFailure, InstallToolResult,
    COMMAND_GET_TOOL_STATUS, COMMAND_INSTALL_TOOL,
};
use crate::backend::tool_store;

use super::AppState;

pub fn register_tool_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let http_client = state.http_client.clone();

    // gettoolstatus → return current install status of all catalog tools
    engine.register_handler(
        COMMAND_GET_TOOL_STATUS,
        Box::new(move |_data, _ctx| {
            Box::pin(async move {
                let tools = tool_store::get_tool_statuses();
                Ok(Some(
                    serde_json::to_value(GetToolStatusResult { tools })
                        .map_err(|e| format!("serialize: {e}"))?,
                ))
            })
        }),
    );

    // installtool → download + verify + install requested tools
    engine.register_handler(
        COMMAND_INSTALL_TOOL,
        Box::new(move |data, _ctx| {
            let client = http_client.clone();
            Box::pin(async move {
                let cmd: CommandInstallToolData = serde_json::from_value(data)
                    .map_err(|e| format!("installtool: {e}"))?;

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

                Ok(Some(
                    serde_json::to_value(InstallToolResult { installed, failed })
                        .map_err(|e| format!("serialize: {e}"))?,
                ))
            })
        }),
    );
}
