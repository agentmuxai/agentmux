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
        ("block", "SendCommand") => WebReturnType::success_empty(),

        // Periodic terminal-state snapshot (SPEC_TERMINAL_SCROLLBACK_
        // PERSISTENCE_2026_07_23.md §2.2) — `TermWrap.processAndCacheData()`
        // (frontend/app/view/term/termwrap.ts) fires this every ~5s of active
        // output via `fireAndForget`, so this stays best-effort: log and
        // return success either way rather than surfacing storage errors to
        // a caller that doesn't check the result. Persisted as the
        // `cache:term:full` blockfile (`TermCacheFileName` on the frontend),
        // read back by `loadInitialTerminalData()` on reconnect alongside the
        // raw `"term"` delta since `ptyOffset` (Part A of the same spec).
        ("block", "SaveTerminalState") => {
            let block_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let term_state: String = match service::get_arg(args, 1) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            // stateType is always "full" today (no incremental-snapshot mode
            // exists); accepted but not branched on.
            let _state_type: String = service::get_arg(args, 2).unwrap_or_default();
            let pty_offset: i64 = service::get_arg(args, 3).unwrap_or(0);
            let term_size: serde_json::Value =
                service::get_arg(args, 4).unwrap_or(serde_json::Value::Null);

            const CACHE_FILE: &str = "cache:term:full";
            let fs = &state.filestore;

            let mut meta = std::collections::HashMap::new();
            meta.insert("ptyoffset".to_string(), serde_json::json!(pty_offset));
            meta.insert("termsize".to_string(), term_size);

            let exists = matches!(fs.stat(&block_id, CACHE_FILE), Ok(Some(_)));
            if !exists {
                if let Err(e) = fs.make_file(
                    &block_id,
                    CACHE_FILE,
                    meta.clone(),
                    crate::backend::storage::filestore::FileOpts::default(),
                ) {
                    tracing::warn!(block_id = %block_id, error = %e, "SaveTerminalState: make_file failed");
                    return WebReturnType::success_empty();
                }
            }
            if let Err(e) = fs.write_file(&block_id, CACHE_FILE, term_state.as_bytes()) {
                tracing::warn!(block_id = %block_id, error = %e, "SaveTerminalState: write_file failed");
            }
            if let Err(e) = fs.write_meta(&block_id, CACHE_FILE, meta, false) {
                tracing::warn!(block_id = %block_id, error = %e, "SaveTerminalState: write_meta failed");
            }
            WebReturnType::success_empty()
        }

        // ---- ShellService (Swarm-pane long-running-process rows,
        // SPEC_SWARM_LONG_RUNNING_PROCESS_ROWS_2026_07_20) ----
        ("shell", "ListActive") => {
            let shells = state.shell_sessions.list_active();
            WebReturnType::success(serde_json::to_value(&shells).unwrap_or_default())
        }

        // ---- CronService (Swarm-pane Cron bucket, Phase 2 of the same
        // spec) — unfiltered like `shell.ListActive`/`subagent.ListActive`
        // above: every job whose `created_by` resolves to a live agent
        // block, frontend groups by block_id in buildTree(). A job whose
        // creator has no live registration (agent pane closed, or created
        // via a raw HTTP call with no matching reactive registration) is
        // silently omitted — nothing to attach it to in a per-agent tree. ----
        ("cron", "ListActive") => {
            let jobs = match &state.shared_store {
                Some(s) => s.cron_list().unwrap_or_default(),
                None => Vec::new(),
            };
            let summaries: Vec<_> = jobs
                .iter()
                .filter_map(|job| {
                    state
                        .reactive_handler
                        .get_agent(&job.created_by)
                        .map(|agent| super::super::cron::to_summary(job, agent.block_id))
                })
                .collect();
            WebReturnType::success(serde_json::to_value(&summaries).unwrap_or_default())
        }

        // ---- SubagentService ----
        ("subagent", "ListActive") => {
            let subagents = state.subagent_watcher.list_active();
            WebReturnType::success(serde_json::to_value(&subagents).unwrap_or_default())
        }
        ("subagent", "ListDispatches") => {
            let dispatches = state.subagent_watcher.list_dispatches();
            WebReturnType::success(serde_json::to_value(&dispatches).unwrap_or_default())
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
        ("subagent", "GetInfo") => {
            let agent_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let info = state.subagent_watcher.get_info(&agent_id);
            WebReturnType::success(serde_json::to_value(&info).unwrap_or(serde_json::Value::Null))
        }
        ("subagent", "GenerateName") => {
            let agent_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let result = super::super::app_api::session::generate_subagent_name(
                &state.wstore,
                &state.subagent_watcher,
                &agent_id,
            )
            .await;
            let (display_name, tokens) = match result {
                Some((name, tokens)) => (Some(name), tokens),
                None => (None, None),
            };
            WebReturnType::success(serde_json::json!({ "displayName": display_name, "tokens": tokens }))
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
        ("history", "ListForAgent") => {
            let agent_id: String = match service::get_arg(args, 0) {
                Ok(v) => v,
                Err(e) => return WebReturnType::error(e),
            };
            let offset: usize = service::get_arg(args, 1).unwrap_or(0);
            let limit: usize = service::get_arg(args, 2).unwrap_or(50);
            let sort_by: String = service::get_arg(args, 3).unwrap_or_else(|_| "modified_at".to_string());
            let sort_dir: String = service::get_arg(args, 4).unwrap_or_else(|_| "desc".to_string());
            let result = state.history_service.list_for_agent(
                &state.id_store,
                &agent_id,
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
            match super::super::app_api::agent_define_core(state.wstore.clone(), state.id_store.clone(), state.broker.clone(), data).await {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::tests::test_state;

    fn save_terminal_state_call(block_id: &str, state_str: &str, pty_offset: i64) -> WebCallType {
        WebCallType {
            service: "block".to_string(),
            method: "SaveTerminalState".to_string(),
            uicontext: None,
            args: vec![
                serde_json::json!(block_id),
                serde_json::json!(state_str),
                serde_json::json!("full"),
                serde_json::json!(pty_offset),
                serde_json::json!({ "rows": 24, "cols": 80 }),
            ],
        }
    }

    #[tokio::test]
    async fn save_terminal_state_persists_content_and_meta() {
        // SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23.md §2.2 — the
        // previously-stubbed RPC must actually write the "cache:term:full"
        // blockfile the frontend's `loadInitialTerminalData()` reads back.
        let state = test_state();
        let call = save_terminal_state_call("block-1", "serialized-xterm-state", 42);

        let result = handle_misc_service(&state, &call).await;
        assert!(result.success, "expected success, got {:?}", result.error);

        let content = state
            .filestore
            .read_file("block-1", "cache:term:full")
            .expect("read_file ok")
            .expect("cache file should exist");
        assert_eq!(content, b"serialized-xterm-state");

        let file = state
            .filestore
            .stat("block-1", "cache:term:full")
            .expect("stat ok")
            .expect("file should exist");
        assert_eq!(file.meta["ptyoffset"], serde_json::json!(42));
        assert_eq!(file.meta["termsize"], serde_json::json!({ "rows": 24, "cols": 80 }));
    }

    #[tokio::test]
    async fn save_terminal_state_overwrites_on_second_snapshot() {
        // Periodic snapshots replace the previous one wholesale, they don't
        // accumulate — matches the frontend's "one current snapshot" model.
        let state = test_state();

        let first = save_terminal_state_call("block-1", "first-snapshot", 10);
        assert!(handle_misc_service(&state, &first).await.success);

        let second = save_terminal_state_call("block-1", "second-snapshot-longer", 99);
        assert!(handle_misc_service(&state, &second).await.success);

        let content = state
            .filestore
            .read_file("block-1", "cache:term:full")
            .unwrap()
            .unwrap();
        assert_eq!(content, b"second-snapshot-longer");

        let file = state.filestore.stat("block-1", "cache:term:full").unwrap().unwrap();
        assert_eq!(file.meta["ptyoffset"], serde_json::json!(99));
    }

    #[tokio::test]
    async fn save_terminal_state_missing_block_id_errors() {
        let state = test_state();
        let call = WebCallType {
            service: "block".to_string(),
            method: "SaveTerminalState".to_string(),
            uicontext: None,
            args: vec![],
        };
        let result = handle_misc_service(&state, &call).await;
        assert!(!result.success);
        assert!(result.error.is_some());
    }
}
