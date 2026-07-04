use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_agent_open(engine, state);
}

fn register_agent_open(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    let event_bus = state.event_bus.clone();
    let filestore = state.filestore.clone();
    // Capture the whole (Arc-backed, Clone) AppState so the block can be created
    // through the reducer (#1681) — see the create-block site below.
    let app_state = state.clone();

    engine.register_handler(
        COMMAND_AGENT_OPEN,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            let event_bus = event_bus.clone();
            let filestore = filestore.clone();
            let app_state = app_state.clone();
            Box::pin(async move {
                let cmd: CommandAgentOpenData = serde_json::from_value(data)
                    .map_err(|e| format!("agent.open: {e}"))?;

                tracing::info!(agent_id = %cmd.agent_id, "agent.open");

                // 1. Load the agent definition (by id or name)
                let agents = wstore.agent_def_list()
                    .map_err(|e| format!("agent.open: {e}"))?;
                let agent = agents.iter()
                    .find(|a| a.id == cmd.agent_id || a.name.eq_ignore_ascii_case(&cmd.agent_id))
                    .ok_or_else(|| format!("AGENT_NOT_FOUND: no agent definition with id '{}'", cmd.agent_id))?
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
                        let controller_type = if agent.agent_type == "container" {
                            "subprocess"
                        } else {
                            provider.controller_type_str()
                        };
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
                // Container agents use per-turn docker exec; the subprocess controller
                // is required regardless of what the provider defaults to (claude returns
                // "persistent", which causes AgentInput to skip the container exec path).
                let controller_type = if agent.agent_type == "container" {
                    "subprocess"
                } else {
                    controller_type
                };
                let is_persistent = controller_type == "persistent";
                let mut cli_args: Vec<String> = if is_persistent {
                    provider.persistent_launch_args
                        .unwrap_or(provider.launch_args)
                        .iter().map(|s| s.to_string()).collect()
                } else {
                    provider.launch_args.iter().map(|s| s.to_string()).collect()
                };
                // Append definition-level flags (e.g. --model <value>) stored in provider_flags.
                if !agent.provider_flags.is_empty() {
                    cli_args.extend(agent.provider_flags.split_whitespace().map(str::to_string));
                }

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
                // Auth dir — the DEFAULT provider auth lives in the shared,
                // instance/channel/version-independent providers area so a single
                // login is shared everywhere (the structural fix for the per-channel
                // validate-spin regression). The per-identity bundle override
                // (identity_handlers) still wins for explicit multi-account.
                let auth_dir = agentmux_common::DataPaths::from_env()
                    .map(|p| p.provider_auth_dir(provider.auth_dir_name).to_string_lossy().into_owned())
                    .unwrap_or_else(|| format!("{}/.agentmux/shared/providers/{}", home, provider.auth_dir_name));
                let _ = std::fs::create_dir_all(&auth_dir);
                env_vars.insert(provider.auth_config_dir_env_var.to_string(), json!(auth_dir));
                for (k, v) in provider.auth_extra_env {
                    env_vars.insert(k.to_string(), json!(v));
                }
                // Merge env vars from the definition's persisted env content blob (KEY=VALUE lines).
                // Provider/auth entries inserted above take precedence; definition-level vars
                // are merged after so they can extend (but not override) the auth env.
                if let Ok(Some(env_blob)) = wstore.agent_content_get(&agent.id, "env") {
                    for line in env_blob.content.lines() {
                        if let Some((k, v)) = line.split_once('=') {
                            let k = k.trim();
                            if !k.is_empty() && !env_vars.contains_key(k) {
                                env_vars.insert(k.to_string(), json!(v));
                            }
                        }
                    }
                }
                // Agent identity
                env_vars.insert("GH_CONFIG_DIR".to_string(), json!(format!("{}/gh-{}", config_home, agent_slug)));
                // Use stored slug (stable across renames) for muxbus routing;
                // fall back to the computed slug derived from the display name.
                let routing_id = if !agent.slug.is_empty() { &agent.slug } else { &agent_slug };
                env_vars.insert("AGENTMUX_AGENT_ID".to_string(), json!(routing_id));
                // Exit delay only for subprocess
                if !is_persistent {
                    env_vars.insert("CLAUDE_CODE_EXIT_AFTER_STOP_DELAY".to_string(), json!("30000"));
                }

                let mut meta = MetaMapType::new();
                meta.insert("view".to_string(), json!("agent"));
                meta.insert("agentId".to_string(), json!(&agent.id));
                // Per-agent zoom persistence (SPEC_AGENT_ZOOM_PERSISTENCE): seed
                // the new block's `term:zoom` from the agent's saved `ui:zoom`
                // (per-agent content store, global cross-channel) so reopening
                // the same agent restores its zoom instead of resetting to 1.0.
                // Stored only for non-default zooms; clamp to the frontend's
                // [0.5, 2.0] range so a corrupt value can't escape it.
                if let Ok(Some(c)) = wstore.agent_content_get(&agent.id, "ui:zoom") {
                    if let Some(z) = parse_seed_zoom(&c.content) {
                        meta.insert("term:zoom".to_string(), json!(z));
                    }
                }
                meta.insert("agentProvider".to_string(), json!(&agent.provider));
                meta.insert("agentName".to_string(), json!(&agent.name));
                meta.insert("agentIcon".to_string(), json!(if agent.icon.is_empty() { "sparkles" } else { &agent.icon }));
                meta.insert("agentMode".to_string(), json!(if agent.agent_type.is_empty() { "host" } else { &agent.agent_type }));
                if !agent.container_image.is_empty() {
                    meta.insert("agent:container_image".to_string(), json!(&agent.container_image));
                }
                if agent.container_volumes != "[]" && !agent.container_volumes.is_empty() {
                    meta.insert("agent:container_volumes".to_string(), json!(&agent.container_volumes));
                }
                // Container-local CLI command: the provider's CLI as it resolves
                // INSIDE the image (on the image's PATH, e.g. `claude`). Distinct
                // from `cmd` below, which is the host-resolved absolute npm path
                // (`<config_home>/.../node_modules/.bin/claude`) and does NOT exist
                // in the container — passing it as docker-exec argv[0] would fail
                // with "no such file or directory". Container turns use this.
                meta.insert("agent:container_command".to_string(), json!(provider.cli_command));
                // Derive output format from provider ID (matches frontend providers/index.ts)
                let output_format = match provider.id {
                    "claude" => "claude-stream-json",
                    "codex" => "codex-json",
                    "gemini" => "gemini-json",
                    // Qwen Code is a Gemini-CLI fork → same stream-json schema.
                    "qwen" => "gemini-json",
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

                // 7. Create block + insert into layout tree.
                // Through the reducer (#1681), not wcore-direct: a store-only
                // block is invisible to the reducer-canonical `state.blocks`
                // (only hydrated from SQLite at bootstrap), so tearing this agent
                // pane off later was rejected "block not found". BlockCreated
                // carries meta → apply_block_created writes the wstore Block,
                // which the controller resync below reloads by id.
                let meta_val = serde_json::to_value(&meta)
                    .map_err(|e| format!("agent.open: meta serialize: {e}"))?;
                let create_events = crate::server::service::dispatch_to_reducer(
                    &app_state,
                    agentmux_common::ipc::Command::CreateBlock {
                        tab_id: tab_id.clone(),
                        meta: meta_val,
                    },
                )
                .await;
                if let Some(msg) = create_events.iter().find_map(|e| match e {
                    agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
                    _ => None,
                }) {
                    return Err(format!("agent.open: CreateBlock: {msg}"));
                }
                let block_id = create_events
                    .iter()
                    .find_map(|e| match e {
                        agentmux_common::ipc::Event::BlockCreated { block_id, .. } => {
                            Some(block_id.clone())
                        }
                        _ => None,
                    })
                    .ok_or_else(|| "agent.open: CreateBlock emitted no BlockCreated".to_string())?;
                for ev in &create_events {
                    if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, &wstore) {
                        tracing::warn!("agent.open: CreateBlock wstore apply failed: {e}");
                    }
                }
                crate::server::service::publish_events(&app_state, &create_events);

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
                write_agent_config_files(&wstore, &app_state.id_store, &agent, routing_id, &work_dir)?;

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

/// Write agent config files (CLAUDE.md, .mcp.json, etc.) to the working directory.
pub(super) fn write_agent_config_files(
    wstore: &Store,
    id_store: &Store,
    agent: &crate::backend::storage::AgentDefinition,
    agent_slug: &str,
    work_dir: &str,
) -> Result<(), String> {
    // Load agent content and skills
    let contents = wstore.agent_content_get_all(&agent.id)
        .unwrap_or_default();
    let skills = wstore.agent_skill_list(&agent.id)
        .unwrap_or_default();

    let mut content_map = std::collections::HashMap::new();
    for fc in &contents {
        content_map.insert(fc.content_type.clone(), fc.content.clone());
    }

    // Disable autonomous memory writes only when using the bare slug-fallback workdir
    // (~/.agentmux/agents/<slug>) — that path is shared across same-name multi-tab
    // launches with no collision resolution, risking concurrent MEMORY.md corruption
    // (upstream issue #29051). An empty working_directory field means the caller
    // chose the fallback; any explicitly set workdir is isolated and keeps writes on.
    let workdir_is_shared = agent.working_directory.is_empty();
    if workdir_is_shared {
        let settings_str = content_map
            .entry("settings".to_string())
            .or_insert_with(|| "{}".to_string());
        match serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(settings_str) {
            Ok(mut obj) => {
                obj.entry("autoMemoryEnabled".to_string())
                    .or_insert(json!(false));
                *settings_str =
                    serde_json::to_string(&obj).unwrap_or_else(|_| "{}".to_string());
            }
            Err(e) => {
                tracing::warn!(
                    work_dir = %work_dir,
                    error = %e,
                    "write_agent_config_files: settings JSON unparseable; \
                     autoMemoryEnabled guard skipped — memory writes may be active on shared workdir"
                );
            }
        }
    }

    // Inject global memory bundles (Armory global brain) into CLAUDE.md.
    // All agents get these regardless of per-agent memory selection. Each
    // section carries a `# [Workspace] <name>` heading (see
    // format_global_brain_block) so the rules are attributable to the
    // workspace and ordered per the Brain tab's sort_order.
    let global_bundles = id_store.bundle_memory_list_global().unwrap_or_default();
    let global_block = crate::backend::storage::format_global_brain_block(&global_bundles);
    if !global_block.is_empty() {
        content_map
            .entry("memory".to_string())
            .and_modify(|existing| {
                *existing = format!("{global_block}\n\n---\n\n{existing}");
            })
            .or_insert(global_block);
    }

    // v1 skills: globals are always injected; the agent's OWN ref-bound skills
    // are authoritative when present, otherwise fall back to legacy
    // db_agent_skills. The fallback decision is gated on *own* refs only — NOT
    // the global-inclusive list — so adding a global skill never discards a
    // legacy-only agent's skills.
    // skill_list now returns each skill wrapped with a per-agent bound_to_agent
    // flag (for the App API's "bound to me" indicator, tracked in #1960) —
    // this config-generation path doesn't need it, so unwrap immediately.
    let visible_skills: Vec<crate::backend::storage::Skill> = wstore.skill_list(&agent.id)
        .unwrap_or_default()
        .into_iter()
        .map(|item| item.skill)
        .collect(); // own refs + globals
    let has_own_skill_refs = visible_skills.iter().any(|s| !s.is_global);
    let effective_skills: Vec<crate::backend::storage::AgentSkill> = if has_own_skill_refs {
        // Ref-based path: own bound + globals (the full visible list).
        crate::backend::agent_config::skills_to_agent_skills(&visible_skills, &agent.id)
    } else {
        // Legacy path: keep legacy skills, then inject globals on top.
        // `visible_skills` here contains only globals (no own refs).
        let mut merged = skills;
        merged.extend(crate::backend::agent_config::skills_to_agent_skills(&visible_skills, &agent.id));
        merged
    };

    let mut config_files = crate::backend::agent_config::build_config_files(
        &content_map,
        &effective_skills,
        &agent.name,
        &agent.id,
        agent_slug,
    );

    // v1 MCP: same rule. Globals + synthetic "agentmux" are always emitted; the
    // legacy blob's user servers are merged in ONLY when the agent has no own
    // ref-bound servers (so a global server never wipes a legacy-only agent's
    // .mcp.json). When the agent has own refs, those are authoritative.
    // Same unwrap as visible_skills above — this path doesn't need bound_to_agent.
    let visible_mcp: Vec<crate::backend::storage::McpServer> = wstore.mcp_server_list(&agent.id)
        .unwrap_or_default()
        .into_iter()
        .map(|item| item.server)
        .collect(); // own refs + globals
    let has_own_mcp_refs = visible_mcp.iter().any(|s| !s.is_global);
    if !visible_mcp.is_empty() {
        let blob_for_merge = if has_own_mcp_refs {
            None
        } else {
            content_map.get("mcp").map(|s| s.as_str())
        };
        if let Some(mcp_json) = crate::backend::agent_config::build_mcp_config_from_refs(
            &visible_mcp,
            blob_for_merge,
            agent_slug,
            &agent.agent_bus_id,
        ) {
            if let Some(pos) = config_files.iter().position(|f| f.filename == ".mcp.json") {
                config_files[pos].content = mcp_json;
            } else {
                config_files.push(crate::backend::agent_config::AgentConfigFile {
                    filename: ".mcp.json".to_string(),
                    content: mcp_json,
                });
            }
        }
    }

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
