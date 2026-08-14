use super::*;

/// Per-agent-definition async lock serializing `agent.open`'s "check for a
/// live block / seed resume session / create block / register controller"
/// sequence.
///
/// Without this, two concurrent `agent.open` calls for the same agent (a
/// double-invocation, or two tabs opened for the same agent close together)
/// can both observe "not live yet" before either controller actually
/// registers — `CreateBlock` dispatch, the layout-action enqueue, and
/// `write_agent_config_files` all await in between — and both seed the same
/// `resume_session_id`, spawning two controllers that `--resume` the
/// identical provider session concurrently. That's the TOCTOU reagent
/// flagged on PR #2059's first concurrency-guard attempt (the earlier
/// single-point-in-time `agent_live_elsewhere` check closed the
/// already-live case but not the still-racing-to-become-live case).
///
/// Scope note: this only serializes calls handled by THIS process — like
/// the in-memory `CONTROLLER_REGISTRY` it guards, it can't see a genuinely
/// different AgentMux instance/channel racing the same registry entry.
/// Same boundary the live-elsewhere check already accepted.
static AGENT_OPEN_LOCKS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

fn agent_open_lock(agent_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut locks = AGENT_OPEN_LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    locks
        .entry(agent_id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

/// Resolve the `(env_var, value)` pair to inject for this agent's model
/// vendor override, if any — redirects the harness at a non-default
/// backend (e.g. `ANTHROPIC_BASE_URL` for a `claude`-provider agent).
/// Pure — no I/O — so it's directly unit-testable without the full
/// async spawn-time harness this is called from.
///
/// `None` when there's nothing to override (empty `model_vendor_base_url`)
/// OR the provider doesn't declare `base_url_env_var` — the latter should
/// already be impossible by the time an agent reaches spawn (rejected at
/// `agent.define`), but this stays defensive rather than trusting that
/// write-time validation is the only thing that can ever set this field.
fn resolve_vendor_env_override(
    provider: &providers::ProviderConfig,
    agent: &AgentDefinition,
) -> Option<(&'static str, String)> {
    if agent.model_vendor_base_url.is_empty() {
        return None;
    }
    provider
        .base_url_env_var
        .map(|var| (var, agent.model_vendor_base_url.clone()))
}

#[cfg(test)]
mod resolve_vendor_env_override_tests {
    use super::*;

    fn base_agent(model_vendor_base_url: &str) -> AgentDefinition {
        AgentDefinition {
            id: "a1".to_string(),
            slug: "a1".to_string(),
            name: "T".to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 0,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: model_vendor_base_url.to_string(),
        }
    }

    #[test]
    fn returns_none_when_override_is_unset() {
        let provider = providers::get_provider("claude").unwrap();
        let agent = base_agent("");
        assert!(resolve_vendor_env_override(provider, &agent).is_none());
    }

    #[test]
    fn returns_the_env_var_and_value_for_a_supporting_provider() {
        let provider = providers::get_provider("claude").unwrap();
        let agent = base_agent("https://my-proxy.example.com");
        assert_eq!(
            resolve_vendor_env_override(provider, &agent),
            Some(("ANTHROPIC_BASE_URL", "https://my-proxy.example.com".to_string()))
        );
    }

    #[test]
    fn returns_none_for_a_non_supporting_provider_even_if_the_field_is_set() {
        // Defensive: shouldn't be reachable in practice (agent.define
        // rejects this combination), but a non-supporting provider must
        // never get a spurious env var injected.
        let provider = providers::get_provider("codex").unwrap();
        let agent = base_agent("https://my-proxy.example.com");
        assert!(resolve_vendor_env_override(provider, &agent).is_none());
    }
}

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

                // Serialize the rest of this handler per agent definition —
                // held until the function returns (guard drops at every exit
                // path, success or error). See AGENT_OPEN_LOCKS' doc comment.
                let open_lock = agent_open_lock(&agent.id);
                let _open_guard = open_lock.lock().await;

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
                            Some(filestore.clone()), wstore.shared_agent_registry(),
                            app_state.boot_id.clone(),
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
                // Model vendor override (harness vs. model-vendor decoupling)
                // — e.g. ANTHROPIC_BASE_URL for a claude-provider agent
                // pointed at a proxy/Bedrock/OpenRouter instead of
                // Anthropic's default endpoint. Inserted before the
                // free-form env blob below so it wins if a definition-level
                // KEY=VALUE line collides with it (structured field over
                // free-form blob, same precedence as the auth entries above).
                if let Some((var, value)) = resolve_vendor_env_override(provider, &agent) {
                    env_vars.insert(var.to_string(), json!(value));
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
                // Mirror under MUXBUS_AGENT_ID too, matching the same
                // additive fix in agent_handlers/input.rs (ARCH-002) --
                // this App API path (agent.open -> agent.send, via
                // agent_io.rs reading this back from persisted cmd:env)
                // builds its own env_vars independently of input.rs's spawn
                // path, so it needs the same mirror applied here rather than
                // inheriting it (codex P2 on PR #2345).
                env_vars.insert("MUXBUS_AGENT_ID".to_string(), json!(routing_id));
                // Exit delay only for subprocess
                if !is_persistent {
                    env_vars.insert("CLAUDE_CODE_EXIT_AFTER_STOP_DELAY".to_string(), json!("30000"));
                }

                // Cross-tab/cross-restart continuity: this agent may already have
                // a captured provider session sitting in the shared registry
                // (backfilled from its provider transcript, or captured live by a
                // prior block) even though no block for it exists in THIS tab —
                // e.g. after a full app restart lands on a different/rehydrated
                // tab. Seed the new block's agent:sessionid meta so its FIRST
                // turn resumes that conversation instead of silently starting
                // fresh and orphaning the original — the same thing the
                // picker's explicit "Continue" flow already does via
                // continueOfInstanceId (RecentSessionRow.session_id), extended
                // here to the default open path.
                // See docs/retro/retro-cross-channel-conversation-continuity-regression-2026-06-16.md
                // ("Mechanism 2 — continuity"), action item 4.
                //
                // Concurrency guard (reagent P1 on PR #2059): only seed
                // agent:sessionid when no OTHER block for this same agent
                // definition currently has a LIVE controller registered
                // anywhere in this process — not just this tab (find_agent_block
                // above only scoped the "reuse" check to the target tab).
                // Without this, opening the same named agent in a second tab
                // while the first is still live would seed the new block with
                // the SAME session_id, letting two controllers concurrently
                // `--resume` one provider session — risking transcript
                // corruption or exactly the orphaning bug this fix exists to
                // prevent. A block with no registered controller (e.g. right
                // after an app restart, before anything has resynced) is not
                // "live" and doesn't block seeding.
                let agent_live_elsewhere = wstore.get_all::<Block>()
                    .map(|blocks| {
                        blocks.iter().any(|b| {
                            obj::meta_get_string(&b.meta, "agentId", "") == agent.id
                                && blockcontroller::get_controller(&b.oid).is_some()
                        })
                    })
                    .unwrap_or(false);
                let resume_session_id: Option<String> = if agent_live_elsewhere {
                    None
                } else {
                    wstore.shared_agent_registry()
                        .and_then(|reg| reg.list_active().ok())
                        .and_then(|records| {
                            records.into_iter()
                                .filter(|r| r.data.definition_id == agent.id)
                                .filter(|r| r.data.session_id.as_deref().map_or(false, |s| !s.is_empty()))
                                .max_by_key(|r| r.data.last_launched_at_ms)
                        })
                        .and_then(|r| r.data.session_id)
                };

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
                // Per-agent color (SPEC_AGENT_COLOR_2026_08_08.md): seed the
                // new block's frame border colors from the agent's stored
                // ui:color so the existing pane-frame rendering shows it —
                // full-strength on the focused border
                // (frame:activebordercolor), dimmed on the unfocused one
                // (frame:bordercolor) so the color is visible either way
                // while focus stays distinguishable by brightness.
                // Assign-if-missing write-through covers defs created by
                // paths the createagent handler doesn't own (forks,
                // imports) and any def that predates migration m0020 on
                // this channel. Strict #rrggbb validation so a corrupt row
                // can't inject arbitrary CSS.
                {
                    use crate::backend::agent_color::{dim_agent_color, is_valid_agent_color, pick_agent_color};
                    let stored = wstore
                        .agent_content_get(&agent.id, "ui:color")
                        .ok()
                        .flatten()
                        .map(|c| c.content.trim().to_string())
                        .filter(|c| is_valid_agent_color(c));
                    let color = match stored {
                        Some(c) => c,
                        None => {
                            let picked = pick_agent_color(&agent.id).to_string();
                            let now_ms = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_millis() as i64)
                                .unwrap_or(0);
                            // Best-effort: a store error here shouldn't block
                            // opening the agent — the pane just stays uncolored
                            // this session and we retry next open.
                            let _ = wstore.agent_content_set(&crate::backend::storage::store::AgentContent {
                                agent_id: agent.id.clone(),
                                content_type: "ui:color".to_string(),
                                content: picked.clone(),
                                updated_at: now_ms,
                            });
                            picked
                        }
                    };
                    meta.insert("frame:activebordercolor".to_string(), json!(&color));
                    meta.insert("frame:bordercolor".to_string(), json!(dim_agent_color(&color)));
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
                meta.insert("agent:resume_strategy".to_string(), json!(provider.resume_strategy_str()));
                meta.insert("agent:session_id_field".to_string(), json!(provider.session_id_field));
                if let Some(sid) = &resume_session_id {
                    meta.insert("agent:sessionid".to_string(), json!(sid));
                }

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
                // SPEC_864 Phase 4 — append through the reducer (single
                // writer of db_layout). Best-effort like the store-direct
                // write it replaces.
                {
                    let action = obj::LayoutActionData {
                        actiontype: "insert".to_string(),
                        actionid: uuid::Uuid::new_v4().to_string(),
                        blockid: block_id.clone(),
                        nodesize: None,
                        nodesizefraction: None,
                        indexarr: None,
                        focused: true,
                        magnified: false,
                        ephemeral: false,
                        targetblockid: String::new(),
                        position: String::new(),
                    };
                    if let Err(e) = crate::server::service::queue_layout_actions_via_reducer(
                        &app_state,
                        &tab_id,
                        vec![action],
                    )
                    .await
                    {
                        tracing::warn!("agent.open: layout action enqueue failed: {e}");
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
                    wstore.shared_agent_registry(),
                    app_state.boot_id.clone(),
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
    // Load agent content
    let contents = wstore.agent_content_get_all(&agent.id)
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
    // db_agent_skills. See Store::effective_skills for the merge algorithm —
    // shared with the `listagentskills` RPC handler so the frontend's
    // pre-launch skill fetch and this materialization path never diverge.
    let effective_skills = wstore.effective_skills(&agent.id);

    let mut config_files = crate::backend::agent_config::build_config_files(
        &content_map,
        &effective_skills,
        &agent.name,
        &agent.id,
        agent_slug,
        work_dir,
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

    // Host-tier jekt sender signing key (SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md
    // §2.2) — inject AGENTMUX_JEKT_KEY into the agentmux MCP server's own env,
    // right alongside AGENTMUX_AGENT_ID, so `SendMessage`/`Loop` can sign
    // outgoing jekts as this agent. Ensured (minted on first use, reused after)
    // via the same Store this function already has; never returned over any
    // RPC, never written anywhere but this ONE agent's own process env and
    // srv's own local table. Best-effort: a failure here must never block
    // agent spawn — the agent still launches, just without a key, and its
    // jekts render TRUST=self-declared instead of host-verified until the
    // next successful spawn.
    if let Ok(key) = wstore.agent_jekt_key_ensure(agent_slug) {
        if let Some(pos) = config_files.iter().position(|f| f.filename == ".mcp.json") {
            let key_b64 = {
                use base64::Engine as _;
                base64::engine::general_purpose::STANDARD.encode(&key)
            };
            match serde_json::from_str::<serde_json::Value>(&config_files[pos].content) {
                Ok(mut mcp_json) => {
                    if let Some(env) = mcp_json
                        .pointer_mut("/mcpServers/agentmux/env")
                        .and_then(|v| v.as_object_mut())
                    {
                        env.insert("AGENTMUX_JEKT_KEY".to_string(), serde_json::json!(key_b64));
                        if let Ok(rewritten) = serde_json::to_string_pretty(&mcp_json) {
                            config_files[pos].content = rewritten;
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        agent_id = %agent.id,
                        error = %e,
                        "agent_open: .mcp.json failed to parse — AGENTMUX_JEKT_KEY not injected, \
                         this agent's jekts will render TRUST=self-declared instead of host-verified"
                    );
                }
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

    // Remove skill-derived files (.claude/commands/*.md, .claude/skills/*/SKILL.md)
    // that WE wrote on a previous launch but aren't part of this run's output --
    // e.g. a skill's format switched between "prompt" and "agent-skill", or a
    // skill was renamed/removed. Shared with `writeagentconfig` (editor_handlers.rs)
    // so the two RPC paths that materialize config files never drift out of sync
    // on this (reagent P1 + Codex P1/P2, PR #2322).
    let new_managed_skill_paths = crate::backend::agent_config::managed_skill_file_paths(
        config_files.iter().map(|f| f.filename.as_str()),
    );
    crate::backend::agent_config::cleanup_stale_managed_skill_files(base_path, &new_managed_skill_paths);

    for file in &config_files {
        // Same defense-in-depth join as the cleanup pass above.
        let Ok(file_path) = crate::backend::base::safe_join_within_base(base_path, &file.filename)
        else {
            tracing::warn!(
                work_dir = %expanded_dir,
                path = %file.filename,
                "write_agent_config_files: refusing to write a config path that \
                 escapes the working directory"
            );
            continue;
        };
        if let Some(parent) = file_path.parent() {
            if !parent.exists() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
        std::fs::write(&file_path, &file.content)
            .map_err(|e| format!("failed to write {}: {e}", file.filename))?;
    }

    crate::backend::agent_config::write_managed_skill_file_manifest(base_path, &new_managed_skill_paths);

    tracing::info!(
        agent_id = %agent.id,
        work_dir = %expanded_dir,
        file_count = config_files.len(),
        "agent.open: wrote config files"
    );

    Ok(())
}

#[cfg(test)]
mod write_agent_config_files_tests {
    use super::*;
    use crate::backend::storage::Skill;

    fn make_store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn make_agent(id: &str, working_directory: &str) -> AgentDefinition {
        AgentDefinition {
            id: id.to_string(),
            slug: String::new(),
            name: "Test Agent".to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: working_directory.to_string(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1_700_000_000_000,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1_700_000_000_000,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
        }
    }

    fn make_skill(skill_type: &str) -> Skill {
        Skill {
            id: "skill-1".to_string(),
            name: "Deploy".to_string(),
            trigger: "deploy".to_string(),
            skill_type: skill_type.to_string(),
            description: "Deploy the app".to_string(),
            content: "1. Test\n2. Deploy".to_string(),
            is_global: false,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
        }
    }

    /// reagent P1 + Codex P1/P2, PR #2322: switching a skill's `skill_type`
    /// between "prompt" and "agent-skill" must remove the artifact from the
    /// PREVIOUS format, not just write the new one -- otherwise the stale
    /// file stays active alongside the newly selected format.
    #[test]
    fn switching_skill_format_removes_the_stale_artifact() {
        let wstore = make_store();
        let id_store = make_store();
        let work_dir = tempfile::tempdir().unwrap();
        let work_dir_str = work_dir.path().to_str().unwrap();

        let mut agent = make_agent("agent-1", work_dir_str);
        wstore.agent_def_insert(&mut agent).unwrap();
        wstore.skill_upsert_unique("agent-1", &make_skill("agent-skill"), true).unwrap();

        write_agent_config_files(&wstore, &id_store, &agent, "test-agent", work_dir_str).unwrap();
        let skill_md = work_dir.path().join(".claude/skills/deploy/SKILL.md");
        assert!(skill_md.exists(), "expected .claude/skills/deploy/SKILL.md to be written");

        // Flip the same skill to "prompt" format and relaunch.
        let mut prompt_skill = make_skill("prompt");
        prompt_skill.updated_at = 1_700_000_000_001;
        wstore.skill_upsert_unique("agent-1", &prompt_skill, true).unwrap();
        write_agent_config_files(&wstore, &id_store, &agent, "test-agent", work_dir_str).unwrap();

        let command_md = work_dir.path().join(".claude/commands/deploy.md");
        assert!(command_md.exists(), "expected .claude/commands/deploy.md to be written");
        assert!(
            !skill_md.exists(),
            "stale .claude/skills/deploy/SKILL.md must be removed when the skill switches to prompt format"
        );
        assert!(
            !skill_md.parent().unwrap().exists(),
            "the now-empty .claude/skills/deploy/ directory should be cleaned up too"
        );
    }

    /// A file the user hand-authored under .claude/commands or .claude/skills
    /// (never part of any AgentMux-written manifest) must never be deleted by
    /// the stale-cleanup pass.
    #[test]
    fn user_authored_files_outside_the_manifest_are_never_touched() {
        let wstore = make_store();
        let id_store = make_store();
        let work_dir = tempfile::tempdir().unwrap();
        let work_dir_str = work_dir.path().to_str().unwrap();

        let mut agent = make_agent("agent-1", work_dir_str);
        wstore.agent_def_insert(&mut agent).unwrap();

        let user_file = work_dir.path().join(".claude/commands/my-own-command.md");
        std::fs::create_dir_all(user_file.parent().unwrap()).unwrap();
        std::fs::write(&user_file, "# hand-authored, not from AgentMux").unwrap();

        wstore.skill_upsert_unique("agent-1", &make_skill("agent-skill"), true).unwrap();
        write_agent_config_files(&wstore, &id_store, &agent, "test-agent", work_dir_str).unwrap();
        write_agent_config_files(&wstore, &id_store, &agent, "test-agent", work_dir_str).unwrap();

        assert!(user_file.exists(), "hand-authored file outside AgentMux's manifest must survive");
    }

    /// Defense-in-depth: even if a manifest somehow contains a path-traversal
    /// entry (e.g. from a bypass of the trigger sanitization added upstream
    /// in agent_config.rs, or manual tampering with the manifest file
    /// itself), the cleanup pass must never delete anything outside the
    /// agent's working directory (reagent P1, PR #2322).
    #[test]
    fn cleanup_refuses_to_delete_a_manifest_path_that_escapes_the_working_dir() {
        let wstore = make_store();
        let id_store = make_store();
        let work_dir = tempfile::tempdir().unwrap();
        let work_dir_str = work_dir.path().to_str().unwrap();

        let mut agent = make_agent("agent-1", work_dir_str);
        wstore.agent_def_insert(&mut agent).unwrap();

        // A sentinel file OUTSIDE the working directory that a traversal
        // attempt would target.
        let sentinel_dir = tempfile::tempdir().unwrap();
        let sentinel = sentinel_dir.path().join("outside-file.txt");
        std::fs::write(&sentinel, "must survive").unwrap();

        // Simulate a manifest that (however it got there) records an escape.
        std::fs::create_dir_all(work_dir.path().join(".claude")).unwrap();
        let traversal_rel = format!(
            "../{}/outside-file.txt",
            sentinel_dir.path().file_name().unwrap().to_str().unwrap()
        );
        let malicious_manifest = serde_json::to_string(&vec![traversal_rel]).unwrap();
        std::fs::write(
            work_dir.path().join(crate::backend::agent_config::MANAGED_SKILL_FILES_MANIFEST),
            malicious_manifest,
        )
        .unwrap();

        // No skills at all this run, so the manifest's one entry is "stale"
        // and would normally be deleted.
        write_agent_config_files(&wstore, &id_store, &agent, "test-agent", work_dir_str).unwrap();

        assert!(sentinel.exists(), "file outside the working directory must never be deleted");
    }
}
