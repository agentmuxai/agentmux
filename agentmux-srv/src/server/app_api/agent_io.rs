use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_agent_send(engine, state);
    register_agent_stop(engine, state);
    register_agent_status(engine, state);
    register_agent_list(engine, state);
    register_agent_output(engine, state);
    register_agent_process_list(engine, state);
    register_agent_tracked_blocks(engine, state);
    register_agent_kill_process(engine, state);
    register_agent_kill_tree(engine, state);
}

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
    let process_broker = state.process_broker.clone();
    engine.register_handler(
        COMMAND_AGENT_TRACKED_BLOCKS,
        Box::new(move |_data, _ctx| {
            let process_broker = process_broker.clone();
            Box::pin(async move {
                // Was: `process_tracker.list_all_blocks()` unioned with
                // `reactive::get_global_handler().list_active_blocks()` — two
                // structurally different, independently-populated registries
                // concatenated with no reconciliation (a provider-chain
                // anti-pattern — see
                // docs/specs/REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK_2026_07_22.md
                // §1/§3.0.2). `ProcessBroker::list_agent_panes()` sources
                // discovery from `blockcontroller::get_all_controllers()`
                // instead (authoritative for every controller type, closing
                // the coverage gap the old `process_tracker`-only half had
                // for `shell`/`acp` blocks) filtered down to agent panes —
                // `list()` alone would also include plain terminals, which
                // this RPC's contract has never included (reagent/codex P1).
                let block_ids: Vec<String> = process_broker
                    .list_agent_panes()
                    .into_iter()
                    .map(|status| status.block_id)
                    .collect();
                Ok(Some(serde_json::to_value(&AgentTrackedBlocksResult {
                    block_ids,
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

fn register_agent_send(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let id_store = state.id_store.clone();
    let broker = state.broker.clone();
    let container_manager = state.container_manager.clone();
    let filestore = state.filestore.clone();

    engine.register_handler(
        COMMAND_AGENT_SEND,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let id_store = id_store.clone();
            let broker = broker.clone();
            let container_manager = container_manager.clone();
            let filestore = filestore.clone();
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
                let mut env_vars: std::collections::HashMap<String, String> = match block.meta.get("cmd:env") {
                    Some(serde_json::Value::Object(obj)) => obj
                        .iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect(),
                    _ => std::collections::HashMap::new(),
                };
                // Identity injection — same path as the `agentinput`
                // handler. See identity/resolver.rs. Passes
                // the broker so the OAuth-class branch can publish a
                // `identitybundlebindings:changed:<bundle_id>` event
                // when the expiry probe updates an account's status
                // (PR D — spec §4.4). Oauth-class resolution failures are
                // BLOCKING unless the agent opted into ambient login
                // (layer-3 spawn gate, SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4
                // _2026_07_14.md §2.2): surface the error in the agent pane
                // via the `error_during_execution` frame (rendered as an
                // agent_error node) and abort before the CLI is created.
                env_vars = match crate::identity::resolver::inject_identity_env_async(
                    wstore.clone(),
                    id_store.clone(),
                    Some(broker.clone()),
                    cmd.block_id.clone(),
                    env_vars,
                )
                .await
                {
                    Ok(env) => env,
                    Err(gate) => {
                        let error_frame = serde_json::json!({
                            "type": "result",
                            "is_error": true,
                            "subtype": "error_during_execution",
                            "error": {"message": format!("[AgentMux] {gate}")}
                        })
                        .to_string();
                        // Some(filestore): the frame must be PERSISTED to the
                        // block file, not just live-broadcast — otherwise the
                        // error vanishes on pane reload/reconnect (reagent P1,
                        // PR #2164 round 2).
                        crate::backend::blockcontroller::shell::handle_append_block_file(
                            &broker,
                            &cmd.block_id,
                            crate::backend::blockcontroller::subprocess::SUBPROCESS_OUTPUT_SUBJECT,
                            format!("{error_frame}\n").as_bytes(),
                            Some(&filestore),
                            None,
                        );
                        return Err(format!("identity spawn gate: {gate}"));
                    }
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
                    // Container agents use per-turn docker exec — incompatible with a
                    // long-lived persistent subprocess. Fail loudly instead of silently
                    // spawning the CLI on the host.
                    let agent_mode = obj::meta_get_string(&block.meta, "agentMode", "host");
                    if agent_mode == "container" {
                        return Err("container agents require a subprocess controller; this provider uses a persistent controller".to_string());
                    }
                    // Resume parity with the subprocess path: pass the resume
                    // flag + captured session id so a respawn (e.g. after a
                    // /model change) continues the same conversation.
                    let resume_flag = obj::meta_get_string(
                        &block.meta, "agent:resume_flag", "--resume",
                    );
                    let persisted_session_id = obj::meta_get_string(
                        &block.meta, "agent:sessionid", "",
                    );
                    let config = blockcontroller::persistent::PersistentSpawnConfig {
                        cli_command,
                        cli_args,
                        working_dir,
                        env_vars,
                        session_id_field,
                        resume_flag,
                        session_id: persisted_session_id,
                        message_id: None,
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
                    // Picker reattach (parallel of the websocket-path
                    // logic): hydrate the persisted session id from
                    // block meta so spawn_turn appends --resume <sid>
                    // on the FIRST turn after reattach.
                    let persisted_session_id = obj::meta_get_string(
                        &block.meta, "agent:sessionid", "",
                    );

                    // Container agent branch: use Docker socket API exec (P1a: no
                    // secrets in argv). Host agent branch: regular CLI subprocess.
                    let agent_mode = obj::meta_get_string(&block.meta, "agentMode", "host");
                    // Cross-process session-lease key (registry::LeaseStore) —
                    // read once for both branches below. Only the host
                    // branch's spawn_turn enforces it in this PR; the
                    // container branch's field is unused for now.
                    let instance_id = obj::meta_get_string(&block.meta, "agentId", "");
                    if agent_mode == "container" {
                        let cm = container_manager.get().await
                            .ok_or_else(|| "Docker not available on this host; cannot start container agent".to_string())?;
                        let container_image = {
                            let img = obj::meta_get_string(&block.meta, "agent:container_image", "");
                            if img.is_empty() { "ghcr.io/agentmuxai/agent-claude:latest".to_string() } else { img }
                        };
                        // Use agentId (UUID) — always valid as a Docker name; display names can have spaces.
                        let agent_id = obj::meta_get_string(&block.meta, "agentId", "");
                        let container_name = crate::backend::container::container_name_for_slug(&agent_id);
                        let volumes_json = obj::meta_get_string(&block.meta, "agent:container_volumes", "[]");
                        let volumes: Vec<String> = serde_json::from_str(&volumes_json).unwrap_or_default();

                        // Ensure container is alive (pull image if needed — P1b).
                        cm.ensure_running(&container_name, &container_image, &volumes, &[]).await
                            .map_err(|e| format!("container ensure_running failed: {e}"))?;


                        tracing::info!(
                            container = %container_name,
                            image = %container_image,
                            "container agent turn: bollard exec (env via Docker socket, not argv)",
                        );

                        // Env is passed via CreateExecOptions.env (Docker socket API),
                        // NOT as -e KEY=VALUE argv args — this prevents CWE-214 exposure.
                        // spawn_container_turn filters config.env_vars (denylist) per
                        // turn, so cmd:cwd (host path) and host-path vars never reach
                        // the container, and each queued turn uses its own env.

                        // Base cmd: [container_command, ...cli_args]. The command
                        // is the provider CLI resolved INSIDE the image (on PATH,
                        // e.g. `claude`) — NOT `cli_command`/`cmd`, which is the
                        // host-resolved absolute npm path and does not exist in the
                        // container (docker exec would fail "no such file or
                        // directory"). cli_args are format flags + provider flags —
                        // no host paths, safe as-is. spawn_container_turn appends
                        // --resume <sid> internally.
                        let container_command = obj::meta_get_string(
                            &block.meta, "agent:container_command", "claude",
                        );
                        let mut base_cmd = vec![container_command];
                        base_cmd.extend(cli_args);

                        let config = blockcontroller::subprocess::SubprocessSpawnConfig {
                            cli_command: String::new(), // unused by spawn_container_turn
                            cli_args: vec![],           // unused by spawn_container_turn
                            working_dir: String::new(), // unused — container has own cwd
                            env_vars,
                            message: cmd.message,
                            resume_flag,
                            session_id_field,
                            message_id: None,
                            session_id: if persisted_session_id.is_empty() {
                                None
                            } else {
                                Some(persisted_session_id)
                            },
                            instance_id: instance_id.clone(),
                        };
                        subprocess_ctrl.spawn_container_turn(cm.clone(), container_name, base_cmd, config)?;
                    } else {
                        // Host agent: regular CLI subprocess (env set on child process, not in argv).
                        let config = blockcontroller::subprocess::SubprocessSpawnConfig {
                            cli_command,
                            cli_args,
                            working_dir,
                            env_vars,
                            message: cmd.message,
                            resume_flag,
                            session_id_field,
                            message_id: None,
                            session_id: if persisted_session_id.is_empty() {
                                None
                            } else {
                                Some(persisted_session_id)
                            },
                            instance_id,
                        };
                        subprocess_ctrl.spawn_turn(config)?;
                    }
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
