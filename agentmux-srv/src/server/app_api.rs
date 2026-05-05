//! App API — high-level commands for programmatic control of AgentMux.
//!
//! These commands orchestrate multiple low-level operations (CreateBlock, SetMeta,
//! ControllerResync) behind stable, intent-based interfaces. Callers express what
//! they want ("open an agent pane with AgentX"), not how to do it.

use std::sync::Arc;

use base64::Engine;
use serde_json::json;

use crate::backend::blockcontroller;
use crate::backend::obj::{self, Block, Tab, Workspace, MetaMapType};
use crate::backend::providers;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::*;
use crate::backend::session_archive;
use crate::backend::storage::wstore::WaveStore;

use super::AppState;
use crate::server::cli_handlers::resolve_cli_on_path;

/// Register all App API handlers on the RPC engine.
pub fn register_app_api_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_agent_open(engine, state);
    register_agent_send(engine, state);
    register_agent_stop(engine, state);
    register_agent_status(engine, state);
    register_agent_list(engine, state);
    register_agent_output(engine, state);
    register_agent_process_list(engine, state);
    register_agent_tracked_blocks(engine, state);
    register_agent_kill_process(engine, state);
    register_agent_kill_tree(engine, state);
    register_pane_open(engine, state);
    register_blockfile_line_count(engine, state);
    register_blockfile_read_range(engine, state);
    register_session_digest(engine, state);
    register_session_archive_handler(engine, state);
    register_session_restore_handler(engine, state);
    register_session_export_handler(engine, state);
}

// ---------------------------------------------------------------------------
// agent.process-list + agent.tracked-blocks
// ---------------------------------------------------------------------------

fn register_agent_process_list(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let process_tracker = state.process_tracker.clone();
    engine.register_handler(
        COMMAND_AGENT_PROCESS_LIST,
        Box::new(move |data, _ctx| {
            let process_tracker = process_tracker.clone();
            Box::pin(async move {
                let cmd: AgentProcessListCommand = serde_json::from_value(data)
                    .map_err(|e| format!("agent.process-list: {e}"))?;
                let members = process_tracker.list_block(&cmd.block_id);
                let confidence = match process_tracker.confidence_of(&cmd.block_id) {
                    crate::backend::process_tracker::TrackingConfidence::High => "high",
                    crate::backend::process_tracker::TrackingConfidence::BestEffort => "best_effort",
                    crate::backend::process_tracker::TrackingConfidence::None => "none",
                };
                let processes: Vec<AgentProcessInfo> = members
                    .into_iter()
                    .map(|m| AgentProcessInfo {
                        pid: m.pid,
                        command: m.command,
                        rss_bytes: m.rss_bytes,
                        started_at_ms: m.started_at_ms,
                    })
                    .collect();
                Ok(Some(serde_json::to_value(&AgentProcessListResult {
                    block_id: cmd.block_id,
                    confidence: confidence.to_string(),
                    processes,
                }).unwrap()))
            })
        }),
    );
}

fn register_agent_tracked_blocks(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let process_tracker = state.process_tracker.clone();
    engine.register_handler(
        COMMAND_AGENT_TRACKED_BLOCKS,
        Box::new(move |_data, _ctx| {
            let process_tracker = process_tracker.clone();
            Box::pin(async move {
                Ok(Some(serde_json::to_value(&AgentTrackedBlocksResult {
                    block_ids: process_tracker.list_all_blocks(),
                }).unwrap()))
            })
        }),
    );
}

fn register_agent_kill_process(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let process_tracker = state.process_tracker.clone();
    engine.register_handler(
        COMMAND_AGENT_KILL_PROCESS,
        Box::new(move |data, _ctx| {
            let process_tracker = process_tracker.clone();
            Box::pin(async move {
                let cmd: AgentKillProcessCommand = serde_json::from_value(data)
                    .map_err(|e| format!("agent.kill-process: {e}"))?;
                tracing::info!(
                    block_id = %cmd.block_id,
                    pid = cmd.pid,
                    "agent.kill-process"
                );
                let ok = process_tracker.kill_pid(&cmd.block_id, cmd.pid);
                Ok(Some(serde_json::to_value(&AgentKillResult { ok }).unwrap()))
            })
        }),
    );
}

fn register_agent_kill_tree(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let process_tracker = state.process_tracker.clone();
    engine.register_handler(
        COMMAND_AGENT_KILL_TREE,
        Box::new(move |data, _ctx| {
            let process_tracker = process_tracker.clone();
            Box::pin(async move {
                let cmd: AgentKillTreeCommand = serde_json::from_value(data)
                    .map_err(|e| format!("agent.kill-tree: {e}"))?;
                tracing::info!(block_id = %cmd.block_id, "agent.kill-tree");
                let ok = process_tracker.kill_tree(&cmd.block_id);
                Ok(Some(serde_json::to_value(&AgentKillResult { ok }).unwrap()))
            })
        }),
    );
}

// ---------------------------------------------------------------------------
// agent.open
// ---------------------------------------------------------------------------

fn register_agent_open(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    let event_bus = state.event_bus.clone();
    let filestore = state.filestore.clone();

    engine.register_handler(
        COMMAND_AGENT_OPEN,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            let event_bus = event_bus.clone();
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandAgentOpenData = serde_json::from_value(data)
                    .map_err(|e| format!("agent.open: {e}"))?;

                tracing::info!(agent_id = %cmd.agent_id, "agent.open");

                // 1. Load the Forge agent (by id or name)
                let agents = wstore.forge_list()
                    .map_err(|e| format!("agent.open: {e}"))?;
                let agent = agents.iter()
                    .find(|a| a.id == cmd.agent_id || a.name.eq_ignore_ascii_case(&cmd.agent_id))
                    .ok_or_else(|| format!("AGENT_NOT_FOUND: no forge agent with id '{}'", cmd.agent_id))?
                    .clone();

                // 2. Resolve provider
                let provider = providers::get_provider(&agent.provider)
                    .ok_or_else(|| format!("INVALID_PROVIDER: unknown provider '{}'", agent.provider))?;

                // 3. Determine tab
                let tab_id = resolve_tab_id(&wstore, cmd.tab_id.as_deref())?;

                // 4. Check for existing agent pane in this tab (idempotent)
                // Use resolved agent.id (not raw user input which could be a name)
                if let Some(existing) = find_agent_block(&wstore, &tab_id, &agent.id)? {
                    // Ensure the controller is registered (may be missing if block
                    // was created by the frontend without backend initialization)
                    if blockcontroller::get_controller(&existing.oid).is_none() {
                        let controller_type = provider.controller_type_str();
                        // Set essential metadata if missing
                        let mut meta_update = obj::MetaMapType::new();
                        meta_update.insert("controller".to_string(), json!(controller_type));
                        meta_update.insert("agentProvider".to_string(), json!(&agent.provider));
                        let _ = crate::server::service::update_object_meta(
                            &wstore, &format!("block:{}", existing.oid), &meta_update,
                        );
                        // Register controller
                        let block_for_resync = wstore.must_get::<Block>(&existing.oid)
                            .map_err(|e| format!("agent.open: reload block: {e}"))?;
                        let _ = blockcontroller::resync_controller(
                            &block_for_resync, &tab_id, None, true,
                            Some(broker.clone()), Some(event_bus.clone()), Some(wstore.clone()),
                            Some(filestore.clone()),
                        );
                    }
                    let status = blockcontroller::get_block_controller_status(&existing.oid)
                        .map(|s| s.shellprocstatus)
                        .unwrap_or_else(|| "init".to_string());
                    return Ok(Some(serde_json::to_value(&AgentOpenResult {
                        block_id: existing.oid,
                        tab_id,
                        agent_id: cmd.agent_id,
                        provider: agent.provider,
                        controller_type: provider.controller_type_str().to_string(),
                        status,
                        created: false,
                    }).unwrap()));
                }

                // 5. Resolve CLI path
                let version = env!("CARGO_PKG_VERSION");
                let home = std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .map_err(|_| "cannot determine home directory".to_string())?;
                let provider_dir = format!("{}/.agentmux/{}/cli/{}", home, version, provider.id);
                let npm_bin = if cfg!(windows) {
                    format!("{}/node_modules/.bin/{}.cmd", provider_dir, provider.cli_command)
                } else {
                    format!("{}/node_modules/.bin/{}", provider_dir, provider.cli_command)
                };
                let mut resolved_cli_path = npm_bin.clone();
                if !std::path::Path::new(&resolved_cli_path).exists() {
                    // Fallback: provider not installed via npm — try system PATH.
                    // This is used for Python-based CLIs like Kimi that are not
                    // distributed on npm.
                    if provider.npm_package.is_empty() {
                        if let Some(path) = resolve_cli_on_path(provider.cli_command).await {
                            resolved_cli_path = path;
                        }
                    }
                    if !std::path::Path::new(&resolved_cli_path).exists() {
                        return Err(format!(
                            "CLI_NOT_AVAILABLE: {} not installed at {}. Open an agent pane in the UI to trigger installation.",
                            provider.cli_command, npm_bin
                        ));
                    }
                }

                // 6. Build metadata
                let controller_type = provider.controller_type_str();
                let is_persistent = controller_type == "persistent";
                let cli_args: Vec<String> = if is_persistent {
                    provider.persistent_launch_args
                        .unwrap_or(provider.launch_args)
                        .iter().map(|s| s.to_string()).collect()
                } else {
                    provider.launch_args.iter().map(|s| s.to_string()).collect()
                };

                let agent_slug = agent.name.to_lowercase()
                    .chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
                    .collect::<String>();
                let work_dir = if agent.working_directory.is_empty() {
                    format!("~/.agentmux/agents/{}", agent_slug)
                } else {
                    agent.working_directory.clone()
                };

                // Build env vars
                let mut env_vars = serde_json::Map::new();
                for key in provider.unset_env {
                    env_vars.insert(key.to_string(), json!(""));
                }
                // Use AGENTMUX_CONFIG_HOME so portable installs stay self-contained.
                // Falls back to ~/.agentmux/config for non-portable installs.
                let config_home = std::env::var("AGENTMUX_CONFIG_HOME")
                    .unwrap_or_else(|_| format!("{}/.agentmux/config", home));
                // Auth dir
                let auth_dir = format!("{}/auth/{}", config_home, provider.auth_dir_name);
                let _ = std::fs::create_dir_all(&auth_dir);
                env_vars.insert(provider.auth_config_dir_env_var.to_string(), json!(auth_dir));
                for (k, v) in provider.auth_extra_env {
                    env_vars.insert(k.to_string(), json!(v));
                }
                // Agent identity
                env_vars.insert("GH_CONFIG_DIR".to_string(), json!(format!("{}/gh-{}", config_home, agent_slug)));
                env_vars.insert("AGENTMUX_AGENT_ID".to_string(), json!(&agent.name));
                // Exit delay only for subprocess
                if !is_persistent {
                    env_vars.insert("CLAUDE_CODE_EXIT_AFTER_STOP_DELAY".to_string(), json!("30000"));
                }

                let mut meta = MetaMapType::new();
                meta.insert("view".to_string(), json!("agent"));
                meta.insert("agentId".to_string(), json!(&agent.id));
                meta.insert("agentProvider".to_string(), json!(&agent.provider));
                meta.insert("agentName".to_string(), json!(&agent.name));
                meta.insert("agentIcon".to_string(), json!(if agent.icon.is_empty() { "sparkles" } else { &agent.icon }));
                meta.insert("agentMode".to_string(), json!(if agent.agent_type.is_empty() { "host" } else { &agent.agent_type }));
                // Derive output format from provider ID (matches frontend providers/index.ts)
                let output_format = match provider.id {
                    "claude" => "claude-stream-json",
                    "codex" => "codex-json",
                    "gemini" => "gemini-json",
                    "kimi" => "kimi-stream-json",
                    _ => "claude-stream-json",
                };
                meta.insert("agentOutputFormat".to_string(), json!(output_format));
                meta.insert("controller".to_string(), json!(controller_type));
                meta.insert("cmd".to_string(), json!(&resolved_cli_path));
                meta.insert("cmd:args".to_string(), json!(cli_args));
                meta.insert("cmd:cwd".to_string(), json!(&work_dir));
                meta.insert("cmd:env".to_string(), serde_json::Value::Object(env_vars));
                meta.insert("agent:resume_flag".to_string(), json!(provider.resume_flag.unwrap_or("")));
                meta.insert("agent:session_id_field".to_string(), json!(provider.session_id_field));

                // 7. Create block + insert into layout tree
                let block = crate::backend::wcore::create_block(&wstore, &tab_id, meta)
                    .map_err(|e| format!("agent.open: create_block: {e}"))?;
                let block_id = block.oid.clone();

                // Enqueue a layout insert action for the frontend to process.
                // The frontend's LayoutModel watches pendingbackendactions on the
                // LayoutState and applies them via treeReducer — same mechanism
                // used by cross-window drag-and-drop (dnd.rs).
                {
                    let tab: Tab = wstore.must_get(&tab_id)
                        .map_err(|e| format!("agent.open: reload tab: {e}"))?;
                    if let Ok(mut layout) = wstore.must_get::<obj::LayoutState>(&tab.layoutstate) {
                        let mut actions = layout.pendingbackendactions.take().unwrap_or_default();
                        actions.push(obj::LayoutActionData {
                            actiontype: "insert".to_string(),
                            actionid: uuid::Uuid::new_v4().to_string(),
                            blockid: block_id.clone(),
                            nodesize: None,
                            indexarr: None,
                            focused: true,
                            magnified: false,
                            ephemeral: false,
                            targetblockid: String::new(),
                            position: String::new(),
                        });
                        layout.pendingbackendactions = Some(actions);
                        let _ = wstore.update(&mut layout);
                    }
                }

                tracing::info!(
                    block_id = %block_id,
                    agent_id = %cmd.agent_id,
                    provider = %agent.provider,
                    controller_type = %controller_type,
                    "agent.open: block created + layout updated"
                );

                // 8. Write agent config files. No collision resolution
                //    in this path — the function creates the dir if
                //    missing and overwrites whatever's there. Same-
                //    name same-hour launches will share a workdir;
                //    proper allocation is tracked as a follow-up.
                write_agent_config_files(&wstore, &agent, &agent_slug, &work_dir)?;

                // 9. Register controller (resync)
                let block_for_resync = wstore.must_get::<Block>(&block_id)
                    .map_err(|e| format!("agent.open: reload block: {e}"))?;
                blockcontroller::resync_controller(
                    &block_for_resync,
                    &tab_id,
                    None,
                    true,
                    Some(broker.clone()),
                    Some(event_bus.clone()),
                    Some(wstore.clone()),
                    Some(filestore.clone()),
                )?;

                // 10. Broadcast block + tab + layout updates to frontend
                {
                    let mut updates = Vec::new();
                    if let Ok(updated_block) = wstore.must_get::<Block>(&block_id) {
                        updates.push(obj::WaveObjUpdate {
                            updatetype: "update".into(),
                            otype: "block".into(),
                            oid: block_id.clone(),
                            obj: Some(obj::wave_obj_to_value(&updated_block)),
                        });
                    }
                    if let Ok(updated_tab) = wstore.must_get::<Tab>(&tab_id) {
                        updates.push(obj::WaveObjUpdate {
                            updatetype: "update".into(),
                            otype: "tab".into(),
                            oid: tab_id.clone(),
                            obj: Some(obj::wave_obj_to_value(&updated_tab)),
                        });
                        if let Ok(updated_layout) = wstore.must_get::<obj::LayoutState>(&updated_tab.layoutstate) {
                            updates.push(obj::WaveObjUpdate {
                                updatetype: "update".into(),
                                otype: "layout".into(),
                                oid: updated_tab.layoutstate.clone(),
                                obj: Some(obj::wave_obj_to_value(&updated_layout)),
                            });
                        }
                    }
                    for update in &updates {
                        let oref = format!("{}:{}", update.otype, update.oid);
                        if let Ok(data) = serde_json::to_value(update) {
                            event_bus.broadcast_event(
                                &crate::backend::eventbus::WSEventType {
                                    eventtype: "waveobj:update".to_string(),
                                    oref,
                                    data: Some(data),
                                },
                            );
                        }
                    }
                }

                Ok(Some(serde_json::to_value(&AgentOpenResult {
                    block_id,
                    tab_id,
                    agent_id: cmd.agent_id,
                    provider: agent.provider,
                    controller_type: controller_type.to_string(),
                    status: "init".to_string(),
                    created: true,
                }).unwrap()))
            })
        }),
    );
}

// ---------------------------------------------------------------------------
// agent.send
// ---------------------------------------------------------------------------

fn register_agent_send(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();

    engine.register_handler(
        COMMAND_AGENT_SEND,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: CommandAgentSendData = serde_json::from_value(data)
                    .map_err(|e| format!("agent.send: {e}"))?;

                tracing::info!(block_id = %cmd.block_id, "agent.send");

                let ctrl = blockcontroller::get_controller(&cmd.block_id)
                    .ok_or_else(|| format!("NOT_RUNNING: no controller for block {}", cmd.block_id))?;

                // Re-read spawn config from block metadata (same pattern as agentinput)
                let block: Block = wstore
                    .get(&cmd.block_id)
                    .map_err(|e| format!("agent.send: {e}"))?
                    .ok_or_else(|| format!("BLOCK_NOT_FOUND: {}", cmd.block_id))?;

                let cli_command = obj::meta_get_string(&block.meta, "cmd", "claude");
                let cli_args: Vec<String> = match block.meta.get("cmd:args") {
                    Some(serde_json::Value::Array(arr)) => arr
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect(),
                    _ => vec![],
                };
                let working_dir = obj::meta_get_string(&block.meta, "cmd:cwd", "");
                let env_vars: std::collections::HashMap<String, String> = match block.meta.get("cmd:env") {
                    Some(serde_json::Value::Object(obj)) => obj
                        .iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect(),
                    _ => std::collections::HashMap::new(),
                };
                let session_id_field = obj::meta_get_string(
                    &block.meta, "agent:session_id_field", "session_id",
                );

                // Dispatch to persistent or subprocess controller
                let mut session_id = None;
                if let Some(persistent_ctrl) = ctrl
                    .as_any()
                    .downcast_ref::<blockcontroller::persistent::PersistentSubprocessController>()
                {
                    let config = blockcontroller::persistent::PersistentSpawnConfig {
                        cli_command,
                        cli_args,
                        working_dir,
                        env_vars,
                        session_id_field,
                    };
                    persistent_ctrl.send_message(cmd.message, config)?;
                    session_id = persistent_ctrl.session_id();
                } else if let Some(subprocess_ctrl) = ctrl
                    .as_any()
                    .downcast_ref::<blockcontroller::subprocess::SubprocessController>()
                {
                    let resume_flag = obj::meta_get_string(
                        &block.meta, "agent:resume_flag", "--resume",
                    );
                    let config = blockcontroller::subprocess::SubprocessSpawnConfig {
                        cli_command,
                        cli_args,
                        working_dir,
                        env_vars,
                        message: cmd.message,
                        resume_flag,
                        session_id_field,
                        message_id: None,
                    };
                    subprocess_ctrl.spawn_turn(config)?;
                } else {
                    return Err("NOT_RUNNING: controller type not supported".to_string());
                }

                Ok(Some(serde_json::to_value(&AgentSendResult {
                    block_id: cmd.block_id,
                    status: "running".to_string(),
                    session_id,
                }).unwrap()))
            })
        }),
    );
}

// ---------------------------------------------------------------------------
// agent.stop
// ---------------------------------------------------------------------------

fn register_agent_stop(engine: &Arc<WshRpcEngine>, _state: &AppState) {
    engine.register_handler(
        COMMAND_AGENT_STOP_API,
        Box::new(|data, _ctx| {
            Box::pin(async move {
                let cmd: CommandAgentStopApiData = serde_json::from_value(data)
                    .map_err(|e| format!("agent.stop: {e}"))?;

                tracing::info!(block_id = %cmd.block_id, signal = ?cmd.signal, "agent.stop");

                let ctrl = blockcontroller::get_controller(&cmd.block_id)
                    .ok_or_else(|| format!("NOT_RUNNING: no controller for block {}", cmd.block_id))?;

                let force = matches!(cmd.signal.as_deref(), Some("SIGKILL") | Some("SIGTERM"));
                ctrl.stop(!force, blockcontroller::STATUS_DONE)?;

                let exit_code = blockcontroller::get_block_controller_status(&cmd.block_id)
                    .map(|s| s.shellprocexitcode);

                Ok(Some(serde_json::to_value(&AgentStopResult {
                    block_id: cmd.block_id,
                    status: "done".to_string(),
                    exit_code,
                }).unwrap()))
            })
        }),
    );
}

// ---------------------------------------------------------------------------
// agent.status
// ---------------------------------------------------------------------------

fn register_agent_status(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();

    engine.register_handler(
        COMMAND_AGENT_STATUS,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: CommandAgentStatusData = serde_json::from_value(data)
                    .map_err(|e| format!("agent.status: {e}"))?;

                let block: Block = wstore
                    .get(&cmd.block_id)
                    .map_err(|e| format!("agent.status: {e}"))?
                    .ok_or_else(|| format!("BLOCK_NOT_FOUND: {}", cmd.block_id))?;

                let agent_id = obj::meta_get_string(&block.meta, "agentId", "");
                let provider = obj::meta_get_string(&block.meta, "agentProvider", "");
                let controller_type = obj::meta_get_string(&block.meta, "controller", "");

                let runtime = blockcontroller::get_block_controller_status(&cmd.block_id);
                let status = runtime.as_ref()
                    .map(|s| s.shellprocstatus.clone())
                    .unwrap_or_else(|| "init".to_string());
                let exit_code = runtime.as_ref()
                    .and_then(|s| if s.shellprocstatus == "done" { Some(s.shellprocexitcode) } else { None });
                let pid = None; // PID not currently exposed in status struct

                // Get session ID from block meta
                let session_id = block.meta.get("agent:sessionid")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                Ok(Some(serde_json::to_value(&AgentStatusResult {
                    block_id: cmd.block_id,
                    agent_id,
                    provider,
                    controller_type,
                    status,
                    session_id,
                    pid,
                    exit_code,
                }).unwrap()))
            })
        }),
    );
}

// ---------------------------------------------------------------------------
// agent.list
// ---------------------------------------------------------------------------

fn register_agent_list(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();

    engine.register_handler(
        COMMAND_AGENT_LIST,
        Box::new(move |_data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let tabs: Vec<Tab> = wstore.get_all::<Tab>()
                    .map_err(|e| format!("agent.list: {e}"))?;

                let mut agents = Vec::new();
                for tab in &tabs {
                    for block_id in &tab.blockids {
                        if let Ok(Some(block)) = wstore.get::<Block>(block_id) {
                            let agent_id = obj::meta_get_string(&block.meta, "agentId", "");
                            if agent_id.is_empty() {
                                continue;
                            }
                            let provider = obj::meta_get_string(&block.meta, "agentProvider", "");
                            let status = blockcontroller::get_block_controller_status(block_id)
                                .map(|s| s.shellprocstatus)
                                .unwrap_or_else(|| "init".to_string());
                            let session_id = block.meta.get("agent:sessionid")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());

                            agents.push(AgentListEntry {
                                block_id: block_id.clone(),
                                tab_id: tab.oid.clone(),
                                agent_id,
                                provider,
                                status,
                                session_id,
                            });
                        }
                    }
                }

                Ok(Some(serde_json::to_value(&AgentListResult { agents }).unwrap()))
            })
        }),
    );
}

// ---------------------------------------------------------------------------
// agent.output
// ---------------------------------------------------------------------------

fn register_agent_output(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let broker = state.broker.clone();

    engine.register_handler(
        COMMAND_AGENT_OUTPUT,
        Box::new(move |data, _ctx| {
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandAgentOutputData = serde_json::from_value(data)
                    .map_err(|e| format!("agent.output: {e}"))?;

                let scope = format!("block:{}", cmd.block_id);
                let max = cmd.max_lines.unwrap_or(1000);
                let after = cmd.after_line.unwrap_or(0);

                // Read persisted blockfile events from broker history
                let mut all_lines: Vec<String> = Vec::new();
                {
                    let events = broker.read_event_history(
                        crate::backend::wps::EVENT_BLOCK_FILE,
                        &scope,
                        max + after, // read enough to cover offset
                    );
                    for event in events {
                        if let Some(ref data) = event.data {
                            if let Some(data64) = data.get("data64").and_then(|v| v.as_str()) {
                                if let Ok(bytes) = base64::engine::general_purpose::STANDARD
                                    .decode(data64)
                                {
                                    let text = String::from_utf8_lossy(&bytes);
                                    for line in text.lines() {
                                        if !line.trim().is_empty() {
                                            all_lines.push(line.to_string());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                let total = all_lines.len();
                let lines: Vec<String> = all_lines.into_iter()
                    .skip(after)
                    .take(max)
                    .collect();
                let has_more = after + lines.len() < total;

                Ok(Some(serde_json::to_value(&AgentOutputResult {
                    block_id: cmd.block_id,
                    lines,
                    total_lines: total,
                    has_more,
                }).unwrap()))
            })
        }),
    );
}

// ---------------------------------------------------------------------------
// pane.open
// ---------------------------------------------------------------------------

fn register_pane_open(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let event_bus = state.event_bus.clone();

    engine.register_handler(
        COMMAND_PANE_OPEN,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let event_bus = event_bus.clone();
            Box::pin(async move {
                let cmd: CommandPaneOpenData = serde_json::from_value(data)
                    .map_err(|e| format!("pane.open: {e}"))?;

                tracing::info!(view = %cmd.view, "pane.open");

                // Build meta for the requested view, validating required args
                let meta = build_pane_meta(&cmd)?;

                // Resolve tab
                let tab_id = resolve_tab_id(&wstore, cmd.tab_id.as_deref())?;

                // Create block
                let block = crate::backend::wcore::create_block(&wstore, &tab_id, meta)
                    .map_err(|e| format!("pane.open: create_block: {e}"))?;
                let block_id = block.oid.clone();

                // Enqueue layout action — split if requested, else append
                let (actiontype, targetblockid, position) = resolve_placement(
                    cmd.split_direction.as_deref(),
                    cmd.split_reference_block_id.as_deref(),
                );
                let focused = cmd.focus.unwrap_or(true);

                {
                    let tab: Tab = wstore.must_get(&tab_id)
                        .map_err(|e| format!("pane.open: reload tab: {e}"))?;
                    if let Ok(mut layout) = wstore.must_get::<obj::LayoutState>(&tab.layoutstate) {
                        let mut actions = layout.pendingbackendactions.take().unwrap_or_default();
                        actions.push(obj::LayoutActionData {
                            actiontype,
                            actionid: uuid::Uuid::new_v4().to_string(),
                            blockid: block_id.clone(),
                            nodesize: None,
                            indexarr: None,
                            focused,
                            magnified: false,
                            ephemeral: false,
                            targetblockid,
                            position,
                        });
                        layout.pendingbackendactions = Some(actions);
                        let _ = wstore.update(&mut layout);
                    }
                }

                tracing::info!(
                    block_id = %block_id,
                    view = %cmd.view,
                    "pane.open: block created + layout updated"
                );

                // Broadcast block + tab + layout updates
                {
                    let mut updates = Vec::new();
                    if let Ok(updated_block) = wstore.must_get::<Block>(&block_id) {
                        updates.push(obj::WaveObjUpdate {
                            updatetype: "update".into(),
                            otype: "block".into(),
                            oid: block_id.clone(),
                            obj: Some(obj::wave_obj_to_value(&updated_block)),
                        });
                    }
                    if let Ok(updated_tab) = wstore.must_get::<Tab>(&tab_id) {
                        updates.push(obj::WaveObjUpdate {
                            updatetype: "update".into(),
                            otype: "tab".into(),
                            oid: tab_id.clone(),
                            obj: Some(obj::wave_obj_to_value(&updated_tab)),
                        });
                        if let Ok(updated_layout) = wstore.must_get::<obj::LayoutState>(&updated_tab.layoutstate) {
                            updates.push(obj::WaveObjUpdate {
                                updatetype: "update".into(),
                                otype: "layout".into(),
                                oid: updated_tab.layoutstate.clone(),
                                obj: Some(obj::wave_obj_to_value(&updated_layout)),
                            });
                        }
                    }
                    for update in &updates {
                        let oref = format!("{}:{}", update.otype, update.oid);
                        if let Ok(data) = serde_json::to_value(update) {
                            event_bus.broadcast_event(
                                &crate::backend::eventbus::WSEventType {
                                    eventtype: "waveobj:update".to_string(),
                                    oref,
                                    data: Some(data),
                                },
                            );
                        }
                    }
                }

                Ok(Some(serde_json::to_value(&PaneOpenResult {
                    block_id,
                    tab_id,
                    view: cmd.view,
                    created: true,
                }).unwrap()))
            })
        }),
    );
}

/// Build the metadata map for a pane.open request, validating required args.
fn build_pane_meta(cmd: &CommandPaneOpenData) -> Result<MetaMapType, String> {
    let mut meta = MetaMapType::new();

    match cmd.view.as_str() {
        "editor" => {
            let file = cmd.file.as_deref().filter(|s| !s.is_empty())
                .ok_or_else(|| "MISSING_ARG: view=editor requires 'file'".to_string())?;
            meta.insert("view".to_string(), json!("editor"));
            meta.insert("file".to_string(), json!(file));
        }
        "term" => {
            meta.insert("view".to_string(), json!("term"));
            meta.insert("controller".to_string(), json!("shell"));
            if let Some(cwd) = cmd.cwd.as_deref().filter(|s| !s.is_empty()) {
                meta.insert("cmd:cwd".to_string(), json!(cwd));
            }
        }
        "browser" => {
            let url = cmd.url.as_deref().filter(|s| !s.is_empty())
                .ok_or_else(|| "MISSING_ARG: view=browser requires 'url'".to_string())?;
            meta.insert("view".to_string(), json!("browser"));
            meta.insert("url".to_string(), json!(url));
        }
        "sysinfo" => {
            meta.insert("view".to_string(), json!("sysinfo"));
        }
        "help" => {
            meta.insert("view".to_string(), json!("help"));
        }
        other => {
            return Err(format!(
                "INVALID_VIEW: unsupported view '{other}' (expected editor/term/browser/sysinfo/help)"
            ));
        }
    }

    if let Some(title) = cmd.title.as_deref().filter(|s| !s.is_empty()) {
        meta.insert("frame:title".to_string(), json!(title));
    }

    Ok(meta)
}

/// Translate `split_direction` + `split_reference_block_id` into the backend
/// layout action triple. Returns `(actiontype, targetblockid, position)`.
/// Falls back to a plain `insert` if direction/reference are missing.
fn resolve_placement(
    direction: Option<&str>,
    reference: Option<&str>,
) -> (String, String, String) {
    let reference = match reference.filter(|s| !s.is_empty()) {
        Some(r) => r,
        None => return ("insert".to_string(), String::new(), String::new()),
    };

    let (actiontype, position) = match direction {
        Some("right") => (crate::backend::wcore::LAYOUT_ACTION_SPLIT_HORIZONTAL, "after"),
        Some("left") => (crate::backend::wcore::LAYOUT_ACTION_SPLIT_HORIZONTAL, "before"),
        Some("down") | Some("below") => (crate::backend::wcore::LAYOUT_ACTION_SPLIT_VERTICAL, "after"),
        Some("up") | Some("above") => (crate::backend::wcore::LAYOUT_ACTION_SPLIT_VERTICAL, "before"),
        _ => return ("insert".to_string(), String::new(), String::new()),
    };

    (actiontype.to_string(), reference.to_string(), position.to_string())
}

// ---------------------------------------------------------------------------
// blockfile:line_count
// ---------------------------------------------------------------------------

fn register_blockfile_line_count(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let broker = state.broker.clone();
    let wstore = state.wstore.clone();

    engine.register_handler(
        COMMAND_BLOCKFILE_LINE_COUNT,
        Box::new(move |data, _ctx| {
            let broker = broker.clone();
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: CommandBlockfileLineCountData = serde_json::from_value(data)
                    .map_err(|e| format!("blockfile:line_count: {e}"))?;

                tracing::info!(block_id = %cmd.block_id, filename = %cmd.filename, "blockfile:line_count");

                // Fast path: read session:line_count meta (O(1), maintained
                // by SessionStatsAccumulator). For "output" filename this is
                // the authoritative total — matches the unbounded counter
                // that SessionStats increments on every line. FileStore's
                // persisted line count will trail meta by up to the debounce
                // interval (1s), and reading the full file just to count
                // lines is O(file size) which defeats the point of a fast
                // line_count endpoint.
                if cmd.filename == "output" {
                    if let Ok(Some(block)) = wstore.get::<Block>(&cmd.block_id) {
                        if let Some(count) = block.meta.get("session:line_count").and_then(|v| v.as_u64()) {
                            return Ok(Some(serde_json::to_value(
                                &BlockfileLineCountResult { count },
                            ).unwrap()));
                        }
                    }
                }

                // Fallback: count from WPS event ring buffer (capped at MAX_PERSIST = 4096).
                let scope = format!("block:{}", cmd.block_id);
                let events = broker.read_event_history(
                    crate::backend::wps::EVENT_BLOCK_FILE,
                    &scope,
                    usize::MAX, // broker clamps to MAX_PERSIST internally
                );

                let mut count: u64 = 0;
                for event in events {
                    if let Some(ref event_data) = event.data {
                        let ev_filename = event_data.get("filename")
                            .and_then(|v| v.as_str()).unwrap_or("");
                        if ev_filename != cmd.filename {
                            continue;
                        }
                        if let Some(data64) = event_data.get("data64").and_then(|v| v.as_str()) {
                            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data64) {
                                let text = String::from_utf8_lossy(&bytes);
                                for line in text.lines() {
                                    if !line.trim().is_empty() {
                                        count += 1;
                                    }
                                }
                            }
                        }
                    }
                }

                Ok(Some(serde_json::to_value(&BlockfileLineCountResult { count }).unwrap()))
            })
        }),
    );
}

// ---------------------------------------------------------------------------
// blockfile:read_range
// ---------------------------------------------------------------------------

fn register_blockfile_read_range(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let broker = state.broker.clone();
    let filestore = state.filestore.clone();

    engine.register_handler(
        COMMAND_BLOCKFILE_READ_RANGE,
        Box::new(move |data, _ctx| {
            let broker = broker.clone();
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandBlockfileReadRangeData = serde_json::from_value(data)
                    .map_err(|e| format!("blockfile:read_range: {e}"))?;

                tracing::info!(block_id = %cmd.block_id, filename = %cmd.filename, offset = cmd.offset, limit = cmd.limit, "blockfile:read_range");

                let limit = cmd.limit.min(10_000) as usize;
                let offset = cmd.offset as usize;
                let end = offset.saturating_add(limit);

                // Phase 1.3: Prefer FileStore (persistent, no size cap) over the
                // WPS broker ring buffer (MAX_PERSIST = 4096 events).
                //
                // If FileStore has the file and it is non-empty, read from disk.
                // Otherwise fall back to ring buffer for backward compatibility.
                let filestore_lines = match filestore.stat(&cmd.block_id, &cmd.filename) {
                    Ok(Some(ref wf)) if wf.size > 0 => {
                        match filestore.read_file(&cmd.block_id, &cmd.filename) {
                            Ok(Some(bytes)) => {
                                let text = String::from_utf8_lossy(&bytes);
                                let lines: Vec<String> = text.lines()
                                    .filter(|l| !l.trim().is_empty())
                                    .map(|l| l.to_string())
                                    .collect();
                                Some(lines)
                            }
                            Ok(None) => None,
                            Err(e) => {
                                tracing::warn!(
                                    block_id = %cmd.block_id,
                                    filename = %cmd.filename,
                                    error = %e,
                                    "blockfile:read_range: filestore read failed, falling back to ring buffer"
                                );
                                None
                            }
                        }
                    }
                    Ok(_) => None, // file absent or empty → fall back
                    Err(e) => {
                        tracing::warn!(
                            block_id = %cmd.block_id,
                            error = %e,
                            "blockfile:read_range: filestore stat failed, falling back to ring buffer"
                        );
                        None
                    }
                };

                let all_lines = if let Some(lines) = filestore_lines {
                    lines
                } else {
                    // Fallback: reconstruct from WPS event ring buffer.
                    // The ring buffer holds at most MAX_PERSIST = 4096 events;
                    // older events are evicted. Offset 0 = oldest retained line.
                    let scope = format!("block:{}", cmd.block_id);
                    let events = broker.read_event_history(
                        crate::backend::wps::EVENT_BLOCK_FILE,
                        &scope,
                        usize::MAX, // broker clamps to MAX_PERSIST internally
                    );

                    let mut lines: Vec<String> = Vec::new();
                    for event in events {
                        let Some(ref event_data) = event.data else { continue };
                        let ev_filename = event_data.get("filename")
                            .and_then(|v| v.as_str()).unwrap_or("");
                        if ev_filename != cmd.filename {
                            continue;
                        }
                        let Some(data64) = event_data.get("data64").and_then(|v| v.as_str()) else { continue };
                        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data64) else { continue };
                        let text = String::from_utf8_lossy(&bytes);
                        for line in text.lines() {
                            if !line.trim().is_empty() {
                                lines.push(line.to_string());
                            }
                        }
                    }
                    lines
                };

                let total = all_lines.len() as u64;
                let clamped_offset = offset.min(all_lines.len());
                let clamped_end = end.min(all_lines.len());
                let lines: Vec<String> = if clamped_offset >= clamped_end {
                    Vec::new()
                } else {
                    all_lines[clamped_offset..clamped_end].to_vec()
                };

                Ok(Some(serde_json::to_value(&BlockfileReadRangeResult {
                    lines,
                    total,
                }).unwrap()))
            })
        }),
    );
}

// ---------------------------------------------------------------------------
// session:archive
// ---------------------------------------------------------------------------

fn register_session_archive_handler(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let filestore = state.filestore.clone();

    engine.register_handler(
        COMMAND_SESSION_ARCHIVE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandSessionArchiveData = serde_json::from_value(data)
                    .map_err(|e| format!("session:archive: {e}"))?;

                tracing::info!(block_id = %cmd.block_id, "session:archive");

                let archive_dir = session_archive::default_archive_dir()
                    .ok_or_else(|| "cannot determine home directory".to_string())?;

                let (archived_bytes, archived_at) = session_archive::archive_session_output(
                    &wstore,
                    &filestore,
                    &cmd.block_id,
                    &archive_dir,
                )?;

                Ok(Some(serde_json::to_value(&SessionArchiveResult {
                    block_id: cmd.block_id,
                    archived_bytes,
                    archived_at,
                }).unwrap()))
            })
        }),
    );
}

// ---------------------------------------------------------------------------
// session:restore
// ---------------------------------------------------------------------------

fn register_session_restore_handler(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let filestore = state.filestore.clone();

    engine.register_handler(
        COMMAND_SESSION_RESTORE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandSessionRestoreData = serde_json::from_value(data)
                    .map_err(|e| format!("session:restore: {e}"))?;

                tracing::info!(block_id = %cmd.block_id, "session:restore");

                let restored_bytes = session_archive::restore_session_output(
                    &wstore,
                    &filestore,
                    &cmd.block_id,
                )?;

                Ok(Some(serde_json::to_value(&SessionRestoreResult {
                    block_id: cmd.block_id,
                    restored_bytes,
                }).unwrap()))
            })
        }),
    );
}

// ---------------------------------------------------------------------------
// session:export
// ---------------------------------------------------------------------------

fn register_session_export_handler(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let filestore = state.filestore.clone();

    engine.register_handler(
        COMMAND_SESSION_EXPORT,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandSessionExportData = serde_json::from_value(data)
                    .map_err(|e| format!("session:export: {e}"))?;

                tracing::info!(block_id = %cmd.block_id, "session:export");

                let (raw_bytes, line_count) = session_archive::read_session_output(
                    &wstore,
                    &filestore,
                    &cmd.block_id,
                )?;

                let byte_count = raw_bytes.len() as u64;
                let content = base64::engine::general_purpose::STANDARD.encode(&raw_bytes);

                Ok(Some(serde_json::to_value(&SessionExportResult {
                    content,
                    line_count,
                    byte_count,
                }).unwrap()))
            })
        }),
    );
}

// ---------------------------------------------------------------------------
// session:digest
// ---------------------------------------------------------------------------

fn register_session_digest(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let filestore = state.filestore.clone();
    let broker = state.broker.clone();

    engine.register_handler(
        COMMAND_SESSION_DIGEST,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let filestore = filestore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandSessionDigestData = serde_json::from_value(data)
                    .map_err(|e| format!("session:digest: {e}"))?;

                tracing::info!(block_id = %cmd.block_id, force = ?cmd.force, "session:digest");

                let force = cmd.force.unwrap_or(false);

                // Read block meta
                let block: Block = wstore
                    .get(&cmd.block_id)
                    .map_err(|e| format!("session:digest: {e}"))?
                    .ok_or_else(|| format!("BLOCK_NOT_FOUND: {}", cmd.block_id))?;

                // Check for a valid cached digest
                let cached_summary = block.meta.get("session:digest_summary")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let cached_generated_at = block.meta.get("session:digest_generated_at")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let digest_last_line_count = block.meta.get("session:digest_last_line_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                // Current line count from meta (O(1))
                let current_line_count = block.meta.get("session:line_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                // Serve cache if: not forced, cached digest exists, AND fewer than 20 new lines
                // since the digest was last generated.
                let lines_since_digest = current_line_count.saturating_sub(digest_last_line_count);
                if !force && cached_summary.is_some() && lines_since_digest < 20 {
                    return Ok(Some(serde_json::to_value(&SessionDigestResult {
                        summary: cached_summary.unwrap(),
                        generated_at: cached_generated_at,
                        cached: true,
                    }).unwrap()));
                }

                // --- Generate a new digest ---

                // Read up to the last 200 lines from FileStore, falling back to the WPS ring buffer.
                let all_lines: Vec<String> = {
                    let filestore_lines = match filestore.stat(&cmd.block_id, "output") {
                        Ok(Some(ref wf)) if wf.size > 0 => {
                            match filestore.read_file(&cmd.block_id, "output") {
                                Ok(Some(bytes)) => {
                                    let text = String::from_utf8_lossy(&bytes);
                                    let lines: Vec<String> = text.lines()
                                        .filter(|l| !l.trim().is_empty())
                                        .map(|l| l.to_string())
                                        .collect();
                                    Some(lines)
                                }
                                _ => None,
                            }
                        }
                        _ => None,
                    };

                    if let Some(lines) = filestore_lines {
                        lines
                    } else {
                        // Fallback: WPS ring buffer
                        let scope = format!("block:{}", cmd.block_id);
                        let events = broker.read_event_history(
                            crate::backend::wps::EVENT_BLOCK_FILE,
                            &scope,
                            usize::MAX,
                        );
                        let mut lines: Vec<String> = Vec::new();
                        for event in events {
                            let Some(ref ed) = event.data else { continue };
                            let fname = ed.get("filename").and_then(|v| v.as_str()).unwrap_or("");
                            if fname != "output" { continue; }
                            if let Some(d64) = ed.get("data64").and_then(|v| v.as_str()) {
                                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(d64) {
                                    let text = String::from_utf8_lossy(&bytes);
                                    for line in text.lines() {
                                        if !line.trim().is_empty() {
                                            lines.push(line.to_string());
                                        }
                                    }
                                }
                            }
                        }
                        lines
                    }
                };

                // Take the last 200 lines
                let n = all_lines.len();
                let start = n.saturating_sub(200);
                let window: Vec<&str> = all_lines[start..].iter().map(|s| s.as_str()).collect();

                if window.is_empty() {
                    return Ok(Some(serde_json::to_value(&SessionDigestResult {
                        summary: String::new(),
                        generated_at: 0,
                        cached: false,
                    }).unwrap()));
                }

                // Extract meaningful text (skip system events and raw stream deltas)
                let extracted = extract_digest_text(&window);

                if extracted.is_empty() {
                    return Ok(Some(serde_json::to_value(&SessionDigestResult {
                        summary: String::new(),
                        generated_at: 0,
                        cached: false,
                    }).unwrap()));
                }

                // Locate the Claude CLI (stored in block meta as "cmd" by runLaunchFlow)
                let cli_path = obj::meta_get_string(&block.meta, "cmd", "");
                if cli_path.is_empty() {
                    tracing::warn!(block_id = %cmd.block_id, "session:digest: no CLI path in meta");
                    return Ok(Some(serde_json::to_value(&SessionDigestResult {
                        summary: String::new(),
                        generated_at: 0,
                        cached: false,
                    }).unwrap()));
                }

                // Build the summarization prompt
                let prompt = format!(
                    "Summarize this AI coding session in 3-4 sentences. Focus on: what was worked on, \
                     tools used, any errors encountered, and the current state. Be concise and factual.\n\n\
                     Session content (last 200 events):\n\n{}",
                    extracted
                );

                // Invoke the Claude CLI and extract the summary text
                let summary = match invoke_cli_for_digest(&cli_path, &prompt, &block.meta).await {
                    Ok(text) => text,
                    Err(e) => {
                        tracing::warn!(block_id = %cmd.block_id, error = %e, "session:digest: CLI invocation failed");
                        String::new()
                    }
                };

                if summary.is_empty() {
                    return Ok(Some(serde_json::to_value(&SessionDigestResult {
                        summary: String::new(),
                        generated_at: 0,
                        cached: false,
                    }).unwrap()));
                }

                // Cache in block meta
                let generated_at = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);

                let mut meta_update = obj::MetaMapType::new();
                meta_update.insert("session:digest_summary".to_string(), json!(summary.clone()));
                meta_update.insert("session:digest_generated_at".to_string(), json!(generated_at));
                meta_update.insert("session:digest_last_line_count".to_string(), json!(current_line_count));

                if let Err(e) = crate::server::service::update_object_meta(
                    &wstore,
                    &format!("block:{}", cmd.block_id),
                    &meta_update,
                ) {
                    tracing::warn!(block_id = %cmd.block_id, error = %e, "session:digest: failed to cache in meta");
                }

                Ok(Some(serde_json::to_value(&SessionDigestResult {
                    summary,
                    generated_at,
                    cached: false,
                }).unwrap()))
            })
        }),
    );
}

/// Extract meaningful text from raw stream-json lines for digest summarization.
/// Skips system/result events and raw stream_event deltas; extracts assistant text
/// and tool call summaries.
fn extract_digest_text(lines: &[&str]) -> String {
    let mut parts: Vec<String> = Vec::new();

    for line in lines {
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else { continue };

        let msg_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match msg_type {
            "assistant" => {
                if let Some(content) = val.get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                {
                    for block in content {
                        let btype = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if btype == "text" {
                            if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                                let trimmed = text.trim();
                                if !trimmed.is_empty() {
                                    parts.push(format!("[assistant] {}", trimmed));
                                }
                            }
                        } else if btype == "tool_use" {
                            let tool_name = block.get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown");
                            parts.push(format!("[tool] {}", tool_name));
                        }
                    }
                }
            }
            "user" => {
                if let Some(content) = val.get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                {
                    for block in content {
                        let btype = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if btype == "tool_result" {
                            let is_error = block.get("is_error")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            if is_error {
                                let err_text = block.get("content")
                                    .and_then(|c| c.as_str())
                                    .unwrap_or("(error)")
                                    .chars().take(120).collect::<String>();
                                parts.push(format!("[error] {}", err_text));
                            }
                        } else if btype == "text" {
                            if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                                let trimmed = text.trim();
                                if !trimmed.is_empty() {
                                    parts.push(format!("[user] {}", trimmed));
                                }
                            }
                        }
                    }
                }
            }
            "result" => {
                if let Some(cost) = val.get("total_cost_usd").and_then(|v| v.as_f64()) {
                    if let Some(turns) = val.get("num_turns").and_then(|v| v.as_u64()) {
                        parts.push(format!("[summary] {} turns, ${:.4} total cost", turns, cost));
                    }
                }
            }
            // Skip: system, stream_event (deltas), rate_limit_event
            _ => {}
        }
    }

    parts.join("\n")
}

/// Invoke the Claude CLI with a prompt and extract the text response.
/// Uses `-p --output-format stream-json --verbose` (non-interactive mode).
async fn invoke_cli_for_digest(
    cli_path: &str,
    prompt: &str,
    meta: &obj::MetaMapType,
) -> Result<String, String> {
    // Inherit auth env from block meta (CLAUDE_CONFIG_DIR, etc.)
    let auth_env: std::collections::HashMap<String, String> = match meta.get("cmd:env") {
        Some(serde_json::Value::Object(obj_map)) => obj_map
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect(),
        _ => std::collections::HashMap::new(),
    };

    // Pipe the prompt via stdin rather than passing it as a CLI arg — Linux
    // caps individual argv entries at MAX_ARG_STRLEN (~128 KB), and a digest
    // over 200 lines of session content can easily exceed that.
    // `kill_on_drop(true)` ensures the child is terminated if the timeout
    // future below is dropped — tokio `Child` does NOT kill on drop by default.
    let mut child = crate::server::cli_handlers::make_cli_cmd(cli_path)
        .args(["-p", "--output-format", "stream-json", "--verbose"])
        .envs(&auth_env)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("failed to spawn digest CLI: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin
            .write_all(prompt.as_bytes())
            .await
            .map_err(|e| format!("digest CLI stdin write: {e}"))?;
        stdin
            .shutdown()
            .await
            .map_err(|e| format!("digest CLI stdin shutdown: {e}"))?;
    }

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| "digest CLI timed out after 60s".to_string())?
    .map_err(|e| format!("digest CLI wait: {e}"))?;

    if !output.status.success() {
        return Err(format!("digest CLI exited with status {}", output.status));
    }

    // Parse stream-json output — capture the last assistant text block
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut last_text = String::new();

    for line in stdout.lines() {
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let msg_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if msg_type == "assistant" {
            if let Some(content) = val.get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_array())
            {
                for block in content {
                    if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                        if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                            last_text = text.trim().to_string();
                        }
                    }
                }
            }
        }
    }

    if last_text.is_empty() {
        return Err("no text content in digest CLI response".to_string());
    }

    Ok(last_text)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve a tab ID: use the provided one, or fall back to the first workspace's active tab.
fn resolve_tab_id(wstore: &WaveStore, explicit: Option<&str>) -> Result<String, String> {
    if let Some(tid) = explicit {
        return Ok(tid.to_string());
    }

    // Fall back to first workspace's active tab
    let workspaces: Vec<Workspace> = wstore.get_all::<Workspace>()
        .map_err(|e| format!("agent.open: list workspaces: {e}"))?;

    for ws in &workspaces {
        if !ws.activetabid.is_empty() {
            return Ok(ws.activetabid.clone());
        }
        if let Some(first_tab) = ws.tabids.first() {
            return Ok(first_tab.clone());
        }
    }

    Err("no tabs found in any workspace".to_string())
}

/// Find an existing agent block in a tab by agent ID.
fn find_agent_block(wstore: &WaveStore, tab_id: &str, agent_id: &str) -> Result<Option<Block>, String> {
    let tab: Tab = wstore.must_get(tab_id)
        .map_err(|e| format!("TAB_NOT_FOUND: {e}"))?;

    for block_id in &tab.blockids {
        if let Ok(Some(block)) = wstore.get::<Block>(block_id) {
            let block_agent_id = obj::meta_get_string(&block.meta, "agentId", "");
            if block_agent_id == agent_id {
                return Ok(Some(block));
            }
        }
    }
    Ok(None)
}

/// Atomically allocate an agent working directory.
///
/// Tries to atomically create `desired` via `std::fs::create_dir`. If
/// that fails because the directory already exists, tries `<desired>-1`,
/// `<desired>-2`, …, up to `-99`. The atomic `create_dir` (NOT
/// `create_dir_all` for the leaf) is the reservation mechanism: two
/// concurrent callers competing for the same path race on the OS
/// `mkdir` syscall and one wins; the loser sees `AlreadyExists` and
/// moves on.
///
/// Caller is responsible for distinguishing auto-generated paths from
/// user-specified ones — this function rewrites the path on collision,
/// which would clobber a user's intent if they pointed an agent at
/// `~/projects/myrepo` and that already had a `CLAUDE.md`.
pub fn allocate_agent_workdir(desired: &str) -> Result<String, String> {
    let p = std::path::Path::new(desired);
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("allocate_agent_workdir: parent {}: {e}", parent.display()))?;
        }
    }
    match std::fs::create_dir(p) {
        Ok(()) => return Ok(desired.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(format!("allocate_agent_workdir: create_dir({}): {e}", desired)),
    }
    for n in 1..=99u32 {
        let candidate = format!("{desired}-{n}");
        match std::fs::create_dir(std::path::Path::new(&candidate)) {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("allocate_agent_workdir: create_dir({candidate}): {e}")),
        }
    }
    Err(format!(
        "allocate_agent_workdir: too many collisions (>99) under {desired}-N — clean up old runs"
    ))
}

/// Write agent config files (CLAUDE.md, .mcp.json, etc.) to the working directory.
fn write_agent_config_files(
    wstore: &WaveStore,
    agent: &crate::backend::storage::ForgeAgent,
    agent_slug: &str,
    work_dir: &str,
) -> Result<(), String> {
    // Load forge contents and skills
    let contents = wstore.forge_get_all_content(&agent.id)
        .unwrap_or_default();
    let skills = wstore.forge_list_skills(&agent.id)
        .unwrap_or_default();

    let mut content_map = std::collections::HashMap::new();
    for fc in &contents {
        content_map.insert(fc.content_type.clone(), fc.content.clone());
    }

    let config_files = crate::backend::agent_config::build_config_files(
        &content_map,
        &skills,
        &agent.name,
        &agent.id,
    );

    // Expand ~ in work_dir
    let expanded_dir = if work_dir.starts_with("~/") || work_dir == "~" {
        if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
            format!("{}/{}", home, work_dir.trim_start_matches("~/"))
        } else {
            work_dir.to_string()
        }
    } else {
        work_dir.to_string()
    };

    let base_path = std::path::Path::new(&expanded_dir);
    if !base_path.exists() {
        std::fs::create_dir_all(base_path)
            .map_err(|e| format!("failed to create working dir: {e}"))?;
    }

    for file in &config_files {
        let file_path = base_path.join(&file.filename);
        if let Some(parent) = file_path.parent() {
            if !parent.exists() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
        std::fs::write(&file_path, &file.content)
            .map_err(|e| format!("failed to write {}: {e}", file.filename))?;
    }

    tracing::info!(
        agent_id = %agent.id,
        work_dir = %expanded_dir,
        file_count = config_files.len(),
        "agent.open: wrote config files"
    );

    Ok(())
}

