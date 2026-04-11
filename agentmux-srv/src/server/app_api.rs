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
use crate::backend::storage::wstore::WaveStore;

use super::AppState;

/// Register all App API handlers on the RPC engine.
pub fn register_app_api_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_agent_open(engine, state);
    register_agent_send(engine, state);
    register_agent_stop(engine, state);
    register_agent_status(engine, state);
    register_agent_list(engine, state);
    register_agent_output(engine, state);
}

// ---------------------------------------------------------------------------
// agent.open
// ---------------------------------------------------------------------------

fn register_agent_open(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    let event_bus = state.event_bus.clone();

    engine.register_handler(
        COMMAND_AGENT_OPEN,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            let event_bus = event_bus.clone();
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
                if let Some(existing) = find_agent_block(&wstore, &tab_id, &cmd.agent_id)? {
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
                if !std::path::Path::new(&npm_bin).exists() {
                    return Err(format!(
                        "CLI_NOT_AVAILABLE: {} not installed at {}. Open an agent pane in the UI to trigger installation.",
                        provider.cli_command, npm_bin
                    ));
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
                // Auth dir
                let auth_dir = format!("{}/.agentmux/config/auth/{}", home, provider.auth_dir_name);
                let _ = std::fs::create_dir_all(&auth_dir);
                env_vars.insert(provider.auth_config_dir_env_var.to_string(), json!(auth_dir));
                for (k, v) in provider.auth_extra_env {
                    env_vars.insert(k.to_string(), json!(v));
                }
                // Agent identity
                env_vars.insert("GH_CONFIG_DIR".to_string(), json!(format!("~/.agentmux/config/gh-{}", agent_slug)));
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
                    _ => "claude-stream-json",
                };
                meta.insert("agentOutputFormat".to_string(), json!(output_format));
                meta.insert("controller".to_string(), json!(controller_type));
                meta.insert("cmd".to_string(), json!(&npm_bin));
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

                // 8. Write agent config files
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
                let exit_code = runtime.as_ref().map(|s| s.shellprocexitcode);
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
