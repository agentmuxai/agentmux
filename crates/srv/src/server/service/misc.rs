// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Catch-all service handler for the smaller service namespaces
//! (userinput / block / subagent / history / agent).

use crate::backend::service::{self, WebCallType, WebReturnType};

use super::super::AppState;

/// A handler's result: the reply, or the message of the first argument that failed to decode.
type Reply = Result<WebReturnType, String>;

pub(super) async fn handle_misc_service(state: &AppState, call: &WebCallType) -> WebReturnType {
    let args = call.args.as_slice();
    let reply = match (call.service.as_str(), call.method.as_str()) {
        ("userinput", "SendUserInputResponse") => Ok(send_user_input_response()),

        ("block", "GetControllerStatus") => get_controller_status(args),
        ("block", "SendCommand") => Ok(WebReturnType::success_empty()),
        ("block", "SaveTerminalState") => save_terminal_state(state, args),

        ("shell", "ListActive") => Ok(list_active_shells(state)),
        ("cron", "ListActive") => Ok(list_active_cron_jobs(state)),

        ("subagent", "ListActive") => Ok(json_reply(&state.subagent_watcher.list_active())),
        ("subagent", "ListDispatches") => Ok(json_reply(&state.subagent_watcher.list_dispatches())),
        ("subagent", "GetHistory") => subagent_history(state, args),
        ("subagent", "GetInfo") => subagent_info(state, args),
        ("subagent", "GenerateName") => generate_subagent_name(state, args).await,
        ("subagent", "ResolveUnnamedBacklog") => Ok(resolve_unnamed_backlog(state)),
        ("subagent", "WatchAgent") => watch_agent(state, args),

        ("history", "List") => history_list(state, args),
        ("history", "ListForAgent") => history_list_for_agent(state, args),
        ("history", "Get") => history_get(state, args),
        ("history", "Refresh") => Ok(WebReturnType::success(state.history_service.refresh())),
        ("history", "Delete") => history_delete(state, args),
        ("history", "Clear") => history_clear(state, args),

        // MuxAPI (also reachable via WebSocket RPC in app_api/).
        ("agent", "define") => agent_define(state, args).await,

        _ => Err(format!("unknown service method: {}.{}", call.service, call.method)),
    };
    reply.unwrap_or_else(WebReturnType::error)
}

/// A success reply carrying `value` as JSON (`null` data if it cannot be encoded).
fn json_reply<T: serde::Serialize>(value: &T) -> WebReturnType {
    WebReturnType::success(serde_json::to_value(value).unwrap_or_default())
}

/// Accepted and dropped: user input routing is not wired yet.
fn send_user_input_response() -> WebReturnType {
    WebReturnType::success_empty()
}

fn get_controller_status(args: &[serde_json::Value]) -> Reply {
    let block_id: String = service::get_arg(args, 0)?;
    let status = crate::backend::blockcontroller::get_block_controller_status(&block_id).unwrap_or_else(|| {
        crate::backend::blockcontroller::BlockControllerRuntimeStatus {
            blockid: block_id,
            ..Default::default()
        }
    });
    Ok(json_reply(&status))
}

/// Periodic terminal-state snapshot (SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23.md §2.2):
/// `TermWrap.processAndCacheData()` (frontend/app/view/term/termwrap.ts) fires this every ~5s of
/// active output via `fireAndForget`, so it stays best-effort: log and return success either way
/// rather than surfacing storage errors to a caller that doesn't check the result. Persisted as the
/// `cache:term:full` blockfile (`TermCacheFileName` on the frontend), read back by
/// `loadInitialTerminalData()` on reconnect alongside the raw `"term"` delta since `ptyOffset`.
fn save_terminal_state(state: &AppState, args: &[serde_json::Value]) -> Reply {
    let block_id: String = service::get_arg(args, 0)?;
    let term_state: String = service::get_arg(args, 1)?;
    // stateType is always "full" today (no incremental-snapshot mode exists); accepted but not
    // branched on.
    let _state_type: String = service::get_arg(args, 2).unwrap_or_default();
    let pty_offset: i64 = service::get_arg(args, 3).unwrap_or(0);
    let term_size: serde_json::Value = service::get_arg(args, 4).unwrap_or(serde_json::Value::Null);

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
            return Ok(WebReturnType::success_empty());
        }
    }
    if let Err(e) = fs.write_file(&block_id, CACHE_FILE, term_state.as_bytes()) {
        tracing::warn!(block_id = %block_id, error = %e, "SaveTerminalState: write_file failed");
    }
    if let Err(e) = fs.write_meta(&block_id, CACHE_FILE, meta, false) {
        tracing::warn!(block_id = %block_id, error = %e, "SaveTerminalState: write_meta failed");
    }
    Ok(WebReturnType::success_empty())
}

/// Swarm-pane long-running-process rows (SPEC_SWARM_LONG_RUNNING_PROCESS_ROWS_2026_07_20).
fn list_active_shells(state: &AppState) -> WebReturnType {
    json_reply(&state.shell_sessions.list_active())
}

/// Swarm-pane Cron bucket (Phase 2 of the same spec). Unfiltered like `shell.ListActive` and
/// `subagent.ListActive`: every job whose `created_by` resolves to a live agent block; the frontend
/// groups by block_id in buildTree(). A job whose creator has no live registration (agent pane
/// closed, or created via a raw HTTP call with no matching reactive registration) is silently
/// omitted: there is nothing to attach it to in a per-agent tree.
fn list_active_cron_jobs(state: &AppState) -> WebReturnType {
    let jobs = super::super::cron::jobs_store(state).and_then(|s| s.cron_list().ok()).unwrap_or_default();
    let summaries: Vec<_> = jobs
        .iter()
        .filter_map(|job| {
            state
                .reactive_handler
                .get_agent(&job.created_by)
                .map(|agent| super::super::cron::to_summary(job, agent.block_id))
        })
        .collect();
    json_reply(&summaries)
}

fn subagent_history(state: &AppState, args: &[serde_json::Value]) -> Reply {
    let agent_id: String = service::get_arg(args, 0)?;
    let limit: usize = service::get_arg(args, 1).unwrap_or(100);
    Ok(json_reply(&state.subagent_watcher.get_history(&agent_id, limit)))
}

fn subagent_info(state: &AppState, args: &[serde_json::Value]) -> Reply {
    let agent_id: String = service::get_arg(args, 0)?;
    let info = state.subagent_watcher.get_info(&agent_id);
    Ok(WebReturnType::success(serde_json::to_value(&info).unwrap_or(serde_json::Value::Null)))
}

async fn generate_subagent_name(state: &AppState, args: &[serde_json::Value]) -> Reply {
    let agent_id: String = service::get_arg(args, 0)?;
    let result = crate::ambient::tasks::generate_subagent_name(
        &state.mstore,
        &state.subagent_watcher,
        &agent_id,
        crate::ambient::limits::Class::Interactive,
    )
    .await;
    let (display_name, tokens) = match result {
        Some(generated) => (generated.text, generated.tokens),
        None => (None, None),
    };
    Ok(WebReturnType::success(serde_json::json!({ "displayName": display_name, "tokens": tokens })))
}

/// A bounded, rate-limited burst that resolves names for whatever is currently unnamed in the
/// backfilled/historical backlog (see `SubagentWatcher::resolve_unnamed_backlog`). Only called from
/// `SwarmViewModel`'s constructor (a human opening the Swarm pane); fire-and-forget, the same
/// pattern as eager naming's own `tokio::spawn`.
fn resolve_unnamed_backlog(state: &AppState) -> WebReturnType {
    tokio::spawn(state.subagent_watcher.clone().resolve_unnamed_backlog());
    WebReturnType::success_empty()
}

fn watch_agent(state: &AppState, args: &[serde_json::Value]) -> Reply {
    let agent_id: String = service::get_arg(args, 0)?;
    let config_dir: String = service::get_arg(args, 1)?;
    // Optional block_id (arg 2) stamps emitted subagent events with the owning pane so the
    // frontend can filter. It defaults to "" for callers that don't supply it (events then match
    // no pane, which is correct for this manual/legacy entry point).
    let block_id: String = service::get_optional_arg(args, 2).unwrap_or(None).unwrap_or_default();
    state.subagent_watcher.watch_agent(&agent_id, &block_id, std::path::PathBuf::from(config_dir));
    Ok(WebReturnType::success_empty())
}

/// The `sort_by` and `sort_dir` arguments shared by the history listings, from `idx` and `idx + 1`.
fn history_sort(args: &[serde_json::Value], idx: usize) -> (String, String) {
    let sort_by = service::get_arg(args, idx).unwrap_or_else(|_| "modified_at".to_string());
    let sort_dir = service::get_arg(args, idx + 1).unwrap_or_else(|_| "desc".to_string());
    (sort_by, sort_dir)
}

fn history_list(state: &AppState, args: &[serde_json::Value]) -> Reply {
    let provider: Option<String> = service::get_optional_arg(args, 0).unwrap_or(None);
    let project: Option<String> = service::get_optional_arg(args, 1).unwrap_or(None);
    let offset: usize = service::get_arg(args, 2).unwrap_or(0);
    let limit: usize = service::get_arg(args, 3).unwrap_or(50);
    let (sort_by, sort_dir) = history_sort(args, 4);
    let result = state.history_service.list(provider.as_deref(), project.as_deref(), offset, limit, &sort_by, &sort_dir);
    Ok(WebReturnType::success(result))
}

fn history_list_for_agent(state: &AppState, args: &[serde_json::Value]) -> Reply {
    let agent_id: String = service::get_arg(args, 0)?;
    let offset: usize = service::get_arg(args, 1).unwrap_or(0);
    let limit: usize = service::get_arg(args, 2).unwrap_or(50);
    let (sort_by, sort_dir) = history_sort(args, 3);
    // The identity store, not the id store: agent-identity links live in the permanently-global
    // identity store (SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md, #2632).
    let result = state.history_service.list_for_agent(&state.identity_store, &agent_id, offset, limit, &sort_by, &sort_dir);
    Ok(WebReturnType::success(result))
}

fn history_get(state: &AppState, args: &[serde_json::Value]) -> Reply {
    let session_id: String = service::get_arg(args, 0)?;
    Ok(WebReturnType::success(state.history_service.get(&session_id)))
}

fn history_delete(state: &AppState, args: &[serde_json::Value]) -> Reply {
    let session_id: String = service::get_arg(args, 0)?;
    Ok(WebReturnType::success(state.history_service.delete(&session_id)))
}

fn history_clear(state: &AppState, args: &[serde_json::Value]) -> Reply {
    let provider: Option<String> = service::get_optional_arg(args, 0).unwrap_or(None);
    let project: Option<String> = service::get_optional_arg(args, 1).unwrap_or(None);
    Ok(WebReturnType::success(state.history_service.clear(provider.as_deref(), project.as_deref())))
}

async fn agent_define(state: &AppState, args: &[serde_json::Value]) -> Reply {
    let data: crate::backend::rpc_types::CommandAgentDefineData = service::get_arg(args, 0)?;
    let result = super::super::app_api::agent_define_core(state.mstore.clone(), state.broker.clone(), data).await?;
    Ok(json_reply(&result))
}

#[cfg(test)]
#[path = "misc_tests.rs"]
mod tests;
