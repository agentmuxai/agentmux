// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Catch-all service handler for the smaller service namespaces
//! (userinput / block / subagent / history / agent).

use crate::backend::service::{self, WebCallType, WebReturnType};

use super::super::AppState;

pub(super) async fn handle_misc_service(state: &AppState, call: &WebCallType) -> WebReturnType {
    let _store = &state.wstore;
    let args = &call.args;
    match (call.service.as_str(), call.method.as_str()) {
        // ---- UserInputService ----
        ("userinput", "SendUserInputResponse") => {
            // Accept but drop — user input routing not yet wired
            WebReturnType::success_empty()
        }

        // ---- BlockService ----
        ("block", "GetControllerStatus") => {
            let block_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            match crate::backend::blockcontroller::get_block_controller_status(&block_id) {
                Some(status) => WebReturnType::success(
                    serde_json::to_value(&status).unwrap_or(serde_json::Value::Null),
                ),
                None => {
                    let default_status = crate::backend::blockcontroller::BlockControllerRuntimeStatus {
                        blockid: block_id,
                        ..Default::default()
                    };
                    WebReturnType::success(
                        serde_json::to_value(&default_status).unwrap_or(serde_json::Value::Null),
                    )
                }
            }
        }
        ("block", "SendCommand") | ("block", "SaveTerminalState") => {
            WebReturnType::success_empty()
        }

        // ---- SubagentService ----
        ("subagent", "ListActive") => {
            let subagents = state.subagent_watcher.list_active();
            WebReturnType::success(serde_json::to_value(&subagents).unwrap_or_default())
        }
        ("subagent", "ListWorkflows") => {
            let workflows = state.subagent_watcher.list_workflows();
            WebReturnType::success(serde_json::to_value(&workflows).unwrap_or_default())
        }
        ("subagent", "GetHistory") => {
            let agent_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let limit: usize = service::get_arg(args, 1).unwrap_or(100);
            let history = state.subagent_watcher.get_history(&agent_id, limit);
            WebReturnType::success(serde_json::to_value(&history).unwrap_or_default())
        }
        // ---- HistoryService ----
        ("history", "List") => {
            let provider: Option<String> = service::get_optional_arg(args, 0).unwrap_or(None);
            let project: Option<String> = service::get_optional_arg(args, 1).unwrap_or(None);
            let offset: usize = service::get_arg(args, 2).unwrap_or(0);
            let limit: usize = service::get_arg(args, 3).unwrap_or(50);
            let sort_by: String = service::get_arg(args, 4).unwrap_or_else(|_| "modified_at".to_string());
            let sort_dir: String = service::get_arg(args, 5).unwrap_or_else(|_| "desc".to_string());
            let result = state.history_service.list(
                provider.as_deref(),
                project.as_deref(),
                offset,
                limit,
                &sort_by,
                &sort_dir,
            );
            WebReturnType::success(result)
        }
        ("history", "Get") => {
            let session_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let result = state.history_service.get(&session_id);
            WebReturnType::success(result)
        }
        ("history", "Refresh") => {
            let result = state.history_service.refresh();
            WebReturnType::success(result)
        }
        ("history", "Delete") => {
            let session_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let result = state.history_service.delete(&session_id);
            WebReturnType::success(result)
        }
        ("history", "Clear") => {
            let provider: Option<String> = service::get_optional_arg(args, 0).unwrap_or(None);
            let project: Option<String> = service::get_optional_arg(args, 1).unwrap_or(None);
            let result = state
                .history_service
                .clear(provider.as_deref(), project.as_deref());
            WebReturnType::success(result)
        }

        ("subagent", "WatchAgent") => {
            let agent_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let config_dir: String = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            // Optional block_id (arg 2) — stamps emitted subagent events with the
            // owning pane so the frontend can filter. Defaults to "" for callers
            // that don't supply it (events then match no pane, which is correct
            // for this manual/legacy entry point).
            let block_id: String = service::get_optional_arg(args, 2)
                .unwrap_or(None)
                .unwrap_or_default();
            state.subagent_watcher.watch_agent(&agent_id, &block_id, std::path::PathBuf::from(config_dir));
            WebReturnType::success_empty()
        }

        // ---- App API (also reachable via WebSocket RPC in app_api.rs) ----
        ("agent", "define") => {
            let data: crate::backend::rpc_types::CommandAgentDefineData =
                match service::get_arg(args, 0) {
                    Ok(v) => v,
                    Err(e) => return WebReturnType::error(e),
                };
            match super::super::app_api::agent_define_core(state.wstore.clone(), state.broker.clone(), data).await {
                Ok(result) => WebReturnType::success(serde_json::to_value(&result).unwrap_or_default()),
                Err(e) => WebReturnType::error(e),
            }
        }

        _ => WebReturnType::error(format!(
            "unknown service method: {}.{}",
            call.service, call.method
        )),
    }
}
