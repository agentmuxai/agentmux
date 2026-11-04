// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps `agent.open` runs in order, one function each. `open_agent_inner` (agent_open.rs) calls them;
//! the code is moved from it unchanged.

use super::*;
use super::agent_open::{record_agent_open_launch, require_local_agent_row, resolve_cli_args, resolve_vendor_env_override, select_openable_agent, write_agent_config_files, AGENT_OPEN_INSTALL_WAIT_SECS};

/// Step 1: the definition to open, by id or name, with its provider settled.
pub(super) fn pick_openable_agent(state: &AppState, agent_ref: &str) -> Result<AgentDefinition, String> {
    let agents = state.mstore.agent_def_list()
        .map_err(|e| format!("agent.open: {e}"))?;
    let mut agent = select_openable_agent(&agents, agent_ref)?.clone();

    // Settle `agent.provider` once through the shared resolver
    // (the agent's own, or its bundle's when it has none; see
    // Store::resolve_effective_provider_id) instead of at every
    // downstream read in this handler. MUST be
    // app_state.id_store, never mstore.
    agent.provider = state.id_store.resolve_effective_provider_id(&agent);
    Ok(agent)
}

/// The checks before anything is written for the agent: no live instance elsewhere on this host, and a local row.
pub(super) async fn admit_open(state: &AppState, agent: &AgentDefinition) -> Result<(), String> {
    let mstore = &state.mstore;
    // One live instance per agent across every AgentMux instance
    // on this host (SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24
    // Phase 1, I9) — checked before this handler writes anything
    // on the agent's behalf. The in-process checks below only see
    // this srv; this sees the others.
    crate::backend::agent_admission::check_before_spawn(
        crate::backend::agent_admission::lease_store_for(mstore.shared_agent_registry()),
        &agent.id,
        &agent.name,
        &state.boot_id,
    )
    .await
    .map_err(|refusal| format!("AGENT_LIVE_ELSEWHERE: {refusal}"))?;

    // Identity M4b-4 (spec §6.5.8): a user agent known only from
    // the shared registry (another channel's) gets its local row
    // before any spawn, so its block resolves to it. Templates
    // were refused above.
    require_local_agent_row(&mstore, &agent.id)?;
    Ok(())
}

/// Step 5: the provider's CLI on disk, installing the pinned version when it is missing.
pub(super) async fn resolve_cli_path(provider: &providers::ProviderConfig) -> Result<String, String> {
    let paths = agentmux_common::DataPaths::from_env()
        .ok_or_else(|| "DataPaths::from_env() failed".to_string())?;
    let npm_bin = crate::backend::cli_install::npm_bin(
        &crate::backend::cli_install::install_dir(&paths, &provider.id, provider.pinned_version),
        provider.cli_command,
    )
    .to_string_lossy()
    .to_string();
    // The one lookup the picker and the Toolchain pane use too:
    // AgentMux's install, else the user's own copy for a provider
    // AgentMux doesn't install (Kimi, Goose, …).
    let mut resolved_cli_path = crate::server::cli_handlers::locate_provider_cli(provider.id)
        .await
        .map(|l| l.path)
        .unwrap_or_else(|| npm_bin.clone());
    if !std::path::Path::new(&resolved_cli_path).exists() {
        // An npm-backed provider whose pinned CLI isn't there yet
        // (a fresh install, or the minute or two after an upgrade
        // moved the pin): install it, as the UI's launch flow does,
        // instead of making the caller go and do that first.
        if provider.npm_package.is_empty() || provider.pinned_version.is_empty() {
            return Err(format!(
                "CLI_NOT_AVAILABLE: {} not found at {}. Install it, then try again.",
                provider.cli_command, npm_bin
            ));
        }
        let req = crate::backend::cli_install::NpmInstallRequest {
            provider_id: provider.id.to_string(),
            npm_package: provider.npm_package.to_string(),
            pinned_version: provider.pinned_version.to_string(),
            cli_command: provider.cli_command.to_string(),
            background: false,
        };
        match crate::backend::cli_install::ensure_installed(
            paths.clone(),
            req,
            std::time::Duration::from_secs(AGENT_OPEN_INSTALL_WAIT_SECS),
        )
        .await
        {
            Ok(bin) => resolved_cli_path = bin.to_string_lossy().to_string(),
            Err(crate::backend::cli_install::EnsureError::StillInstalling) => {
                return Err(format!(
                    "CLI_INSTALLING: {} {} is still being installed. Try again in a minute; \
                     the install carries on in the background.",
                    provider.cli_command, provider.pinned_version
                ));
            }
            Err(crate::backend::cli_install::EnsureError::Failed(why)) => {
                return Err(format!(
                    "CLI_NOT_AVAILABLE: {} {} could not be installed: {why}",
                    provider.cli_command, provider.pinned_version
                ));
            }
        }
    }
    Ok(resolved_cli_path)
}

/// The CLI args for the pane: catalog args, the runtime the menu shows, then the definition's own flags.
pub(super) fn seed_runtime_args(
    mstore: &Store,
    agent: &AgentDefinition,
    provider: &providers::ProviderConfig,
    controller_type: &str,
) -> (crate::backend::agent_runtime::SeededLaunch, String) {
    // Catalog args, the runtime the menu will show applied on top, then the
    // definition's own flags. Without the middle step a pane opened here
    // ran on the CLI's default model while its menu read Sonnet — see
    // backend/agent_runtime.rs.
    //
    // What the user last picked in the menu for this agent sits above the
    // definition's flags (and replaces them in this pane's copy), so
    // reopening an agent starts where it was left. A read failure only
    // means "nothing remembered".
    let remembered = crate::backend::agent_runtime::Remembered::parse(
        provider.id,
        &mstore.agent_last_runtime_get(&agent.id).unwrap_or_default(),
    );
    let (base_args, pane_flags, remembered_mode) = crate::backend::agent_runtime::apply_remembered(
        provider.id,
        resolve_cli_args(&provider, controller_type),
        &agent.provider_flags,
        &remembered,
    );
    let mut seeded =
        crate::backend::agent_runtime::seed_launch(provider.id, base_args, &pane_flags);
    if let (Some(mode), Some(rt)) = (remembered_mode, seeded.runtime.as_mut()) {
        rt["permissionMode"] = json!(mode);
    }
    (seeded, pane_flags)
}

/// The provider's default auth directory, created with the isolation it needs before a CLI is pointed at it.
pub(super) fn prepare_auth_dir(provider: &providers::ProviderConfig) -> Result<String, String> {
    // Auth dir — the DEFAULT provider auth lives in the shared,
    // instance/channel/version-independent providers area so a single
    // login is shared everywhere (the structural fix for the per-channel
    // validate-spin regression). The per-account auth dir
    // (identity_handlers) still wins for explicit multi-account.
    let auth_dir = providers::default_auth_dir(provider)?;
    // Create the dir AND apply its isolation guarantees in one
    // inseparable step. Claude Code CLI's own user-level CLAUDE.md
    // discovery falls through to the real $HOME/.claude/CLAUDE.md
    // when this isolated dir has none of its own — CLAUDE_CONFIG_DIR
    // relocates credential/session/project storage but NOT that
    // lookup (proven live, three-arm experiment:
    // docs/reports/REPORT_CLAUDE_CONFIG_DIR_ISOLATION_EVIDENCE_2026_09_01.md).
    //
    // Fail closed (block the spawn) rather than warn-and-continue:
    // continuing would launch the agent with exactly the
    // unprotected condition this exists to close (Codex P1 +
    // ReAgent P1, PR #2854). Non-claude providers get the dir
    // created and nothing else, so this never blocks their spawns.
    // See SPEC_ISOLATE_HOST_CLAUDE_MD_2026_08_31.md.
    providers::prepare_provider_auth_dir(provider, &auth_dir).map_err(|e| {
        format!(
            "failed to isolate this agent's Claude Code config ({auth_dir}): {e}. \
             Refusing to launch with an unprotected config dir."
        )
    })?;
    Ok(auth_dir)
}

/// The environment the agent's process starts with: provider and auth entries, the vendor override, the definition's
/// own variables and its identity.
pub(super) fn launch_env(
    mstore: &Store,
    agent: &AgentDefinition,
    provider: &providers::ProviderConfig,
    auth_dir: &str,
    routing_id: &str,
    controller_type: &str,
) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    let mut env_vars = serde_json::Map::new();
    for key in provider.unset_env {
        env_vars.insert(key.to_string(), json!(""));
    }
    if !provider.auth_config_dir_env_var.is_empty() { env_vars.insert(provider.auth_config_dir_env_var.to_string(), json!(auth_dir)); }
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
    if let Ok(Some(env_blob)) = mstore.agent_content_get(&agent.id, "env") {
        // Read the way the My Agents editor shows it (`backend::agent_env`): comments
        // and lines without a valid `NAME=` are skipped, and a repeated name takes its last value.
        for (k, v) in crate::backend::agent_env::effective(&env_blob.content) {
            if !env_vars.contains_key(&k) {
                env_vars.insert(k, json!(v));
            }
        }
    }
    // Agent identity. `GH_CONFIG_DIR` is not persisted here: it is
    // a reserved variable set on every spawn by `backend::gh_guard`,
    // which this env reaches through `build_persistent_spawn_env`.
    // Use stored slug (stable across renames) for muxbus routing;
    // fall back to the computed slug derived from the display name.
    env_vars.insert("AGENTMUX_AGENT_ID".to_string(), json!(routing_id));
    // Mirror under MUXBUS_AGENT_ID too, matching the same
    // additive fix in agent_handlers/input.rs (ARCH-002) --
    // this MuxAPI path (agent.open -> agent.send, via
    // agent_io.rs reading this back from persisted cmd:env)
    // builds its own env_vars independently of input.rs's spawn
    // path, so it needs the same mirror applied here rather than
    // inheriting it (codex P2 on PR #2345).
    env_vars.insert("MUXBUS_AGENT_ID".to_string(), json!(routing_id));
    // Exit delay only for subprocess
    if controller_type != "persistent" {
        env_vars.insert("CLAUDE_CODE_EXIT_AFTER_STOP_DELAY".to_string(), json!("30000"));
    }
    Ok(env_vars)
}

/// The provider session this open resumes: the agent's newest captured one, unless another block of it is live.
pub(super) async fn resume_session_for(mstore: &Store, agent: &AgentDefinition) -> Option<String> {
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
    // A block of this agent that is mid-close has already left
    // CONTROLLER_REGISTRY but its process may still be exiting
    // (up to the close's grace period). Wait for it, so the check
    // below sees it gone AND no second `--resume` starts on the
    // session while the old process still holds it
    // (SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN_2026_09_18.md §9.6).
    let closing_blocks: Vec<String> = mstore
        .get_all::<Block>()
        .map(|blocks| {
            blocks
                .into_iter()
                .filter(|b| {
                    obj::meta_get_string(&b.meta, "agentId", "") == agent.id
                        && blockcontroller::is_closing(&b.oid)
                })
                .map(|b| b.oid)
                .collect()
        })
        .unwrap_or_default();
    for closing in &closing_blocks {
        blockcontroller::wait_closing_stopped(
            closing,
            blockcontroller::SHUTDOWN_GRACE + std::time::Duration::from_secs(2),
        )
        .await;
    }
    let agent_live_elsewhere = mstore.get_all::<Block>()
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
        mstore.shared_agent_registry()
            .and_then(|reg| reg.list_active().ok())
            .and_then(|records| {
                records.into_iter()
                    .filter(|r| r.data.definition_id == agent.id)
                    .filter(|r| r.data.session_id.as_deref().map_or(false, |s| !s.is_empty()))
                    .max_by_key(|r| r.data.last_launched_at_ms)
            })
            .and_then(|r| r.data.session_id)
    };
    resume_session_id
}

/// The pane meta that says which agent this is: its zoom, colour, seat, icon and container settings.
pub(super) fn pane_identity_meta(mstore: &Store, agent: &AgentDefinition, provider: &providers::ProviderConfig) -> MetaMapType {
    let mut meta = MetaMapType::new();
    meta.insert("view".to_string(), json!("agent"));
    meta.insert("agentId".to_string(), json!(&agent.id));
    // Per-agent zoom persistence (SPEC_AGENT_ZOOM_PERSISTENCE): seed
    // the new block's `term:zoom` from the agent's saved `ui:zoom`
    // (per-agent content store, global cross-channel) so reopening
    // the same agent restores its zoom instead of resetting to 1.0.
    // Stored only for non-default zooms; clamp to the frontend's
    // [0.5, 2.0] range so a corrupt value can't escape it.
    if let Ok(Some(c)) = mstore.agent_content_get(&agent.id, "ui:zoom") {
        if let Some(z) = parse_seed_zoom(&c.content) {
            meta.insert("term:zoom".to_string(), json!(z));
        }
    }
    // Per-agent color (SPEC_AGENT_COLOR_2026_08_08.md): seed the new block's frame border
    // colors from the agent's stored ui:color so the existing pane-frame rendering shows
    // it (frame:activebordercolor; the unfocused border is the same hue's own tone, drawn
    // by pane-color-scheme.ts, so nothing dimmed is stored). Assign-if-missing write-through covers defs
    // created by paths the createagent handler doesn't own (forks, imports) and any def
    // that predates migration m0020 on this channel. Strict #rrggbb validation so a
    // corrupt row can't inject arbitrary CSS.
    {
        use crate::backend::agent_color::{color_for_new_agent, is_valid_agent_color};
        let stored = mstore
            .agent_content_get(&agent.id, "ui:color")
            .ok()
            .flatten()
            .map(|c| c.content.trim().to_string())
            .filter(|c| is_valid_agent_color(c));
        let color = match stored {
            Some(c) => c,
            None => {
                let picked = color_for_new_agent(&mstore, &agent.id);
                let now_ms = agentmux_common::time::now_ms();
                // Best-effort: a store error here shouldn't block
                // opening the agent — the pane just stays uncolored
                // this session and we retry next open.
                let _ = mstore.agent_content_set(&crate::backend::storage::store::AgentContent {
                    agent_id: agent.id.clone(),
                    content_type: "ui:color".to_string(),
                    content: picked.clone(),
                    updated_at: now_ms,
                });
                picked
            }
        };
        meta.insert("frame:activebordercolor".to_string(), json!(&color));
    }
    meta.insert("agentProvider".to_string(), json!(&agent.provider));
    meta.insert("agentName".to_string(), json!(&agent.name));
    // Its seat, for the header tooltip; agent.setSeat refreshes it (SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §4).
    crate::backend::agent_seat::seed_meta(&mstore, &agent.id, &mut meta);
    meta.insert("agentIcon".to_string(), json!(if agent.icon.is_empty() { "wand-magic-sparkles" } else { &agent.icon }));
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
    meta
}

/// What the pane needs to launch the CLI, added to its identity meta.
pub(super) struct LaunchMeta<'a> {
    pub controller_type: &'a str,
    pub cli_path: &'a str,
    pub seeded: crate::backend::agent_runtime::SeededLaunch,
    pub pane_flags: &'a str,
    pub work_dir: &'a str,
    pub env_vars: serde_json::Map<String, serde_json::Value>,
    pub resume_session_id: Option<&'a str>,
    pub extra_meta: MetaMapType,
}

/// Step 6, the launch half: command, args, working directory, env, resume settings, then the caller's extra keys.
pub(super) fn add_launch_meta(meta: &mut MetaMapType, provider: &providers::ProviderConfig, launch: LaunchMeta<'_>) {
    let LaunchMeta { controller_type, cli_path, seeded, pane_flags, work_dir, env_vars, resume_session_id, extra_meta } = launch;
    // The provider's own format, as the UI launch writes it
    // (`agent-model.ts`). A hard-coded map here stamped every
    // provider it didn't list (Antigravity, the ACP ones) as Claude.
    meta.insert("agentOutputFormat".to_string(), json!(provider.styled_output_format));
    meta.insert("controller".to_string(), json!(controller_type));
    meta.insert("cmd".to_string(), json!(cli_path));
    // cmd:args + agent:runtime + agent:provider_flags, written together.
    crate::backend::agent_runtime::apply_to_meta(meta, seeded, pane_flags);
    meta.insert("cmd:cwd".to_string(), json!(work_dir));
    meta.insert("cmd:env".to_string(), serde_json::Value::Object(env_vars));
    meta.insert("agent:resume_flag".to_string(), json!(provider.resume_flag.unwrap_or("")));
    meta.insert("agent:resume_strategy".to_string(), json!(provider.resume_strategy_str()));
    meta.insert("agent:session_id_field".to_string(), json!(provider.session_id_field));
    meta.insert("agent:prompt_arg_flag".to_string(), json!(provider.prompt_arg_flag().unwrap_or("")));
    if let Some(sid) = resume_session_id {
        meta.insert("agent:sessionid".to_string(), json!(sid));
    }

    providers::apply_terminal_pane_meta(provider, meta);
    meta.extend(extra_meta);
}

/// Step 7 for `layout.open` and Import: give an empty agent pane that already sits in the layout the agent's meta.
pub(super) async fn fill_existing_pane(state: &AppState, target: &str, meta_val: serde_json::Value) -> Result<String, String> {
    let mstore = &state.mstore;
    // `layout.open` and Import…: the pane already exists and sits
    // in the rebuilt tree — give it the agent's meta through
    // the reducer (like `pane.open`'s editor reuse) and skip
    // the layout insert below.
    let target_block = mstore
        .get::<Block>(target)
        .map_err(|e| format!("agent.open: read target pane: {e}"))?
        .ok_or_else(|| format!("agent.open: no such pane: {target}"))?;
    if obj::meta_get_string(&target_block.meta, "view", "") != "agent"
        || !obj::meta_get_string(&target_block.meta, "agentId", "").is_empty()
    {
        return Err("agent.open: the target pane is not an empty agent pane".to_string());
    }
    let mut patch = meta_val.clone();
    if let Some(p) = patch.as_object_mut() {
        // The layout's "which agent goes here" hint is spent.
        p.insert(crate::backend::layout_file::META_LAYOUT_AGENT.to_string(), serde_json::Value::Null);
    }
    let meta_events = crate::server::service::dispatch_to_reducer(
        state,
        agentmux_common::ipc::Command::UpdateBlockMeta { block_id: target.to_string(), meta_patch: patch },
    )
    .await;
    if let Some(msg) = meta_events.iter().find_map(|e| match e {
        agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    }) {
        return Err(format!("agent.open: UpdateBlockMeta: {msg}"));
    }
    for ev in &meta_events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_mstore(ev, mstore) {
            tracing::warn!("agent.open: UpdateBlockMeta mstore apply failed: {e}");
        }
    }
    crate::server::service::publish_events(state, &meta_events);
    Ok(target.to_string())
}

/// Step 7 for a new pane: create the block through the reducer, then place it in the layout (or park it for a start).
///
/// Through the reducer (#1681), not wcore-direct: a store-only block is invisible to the reducer-canonical
/// `state.blocks` (only hydrated from SQLite at bootstrap), so tearing this agent pane off later was rejected
/// "block not found". `BlockCreated` carries meta, and `apply_block_created` writes the mstore Block, which the
/// controller resync reloads by id.
pub(super) async fn create_agent_block(
    state: &AppState,
    tab_id: &str,
    meta_val: serde_json::Value,
    headless: bool,
    cmd: &CommandAgentOpenData,
) -> Result<String, String> {
    let mstore = &state.mstore;
    let create_events = crate::server::service::dispatch_to_reducer(
        state,
        agentmux_common::ipc::Command::CreateBlock {
            tab_id: tab_id.to_string(),
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
        if let Err(e) = crate::persist_subscriber::apply_event_to_mstore(ev, mstore) {
            tracing::warn!("agent.open: CreateBlock mstore apply failed: {e}");
        }
    }
    crate::server::service::publish_events(state, &create_events);

    // Place it: srv's tree, then the queued `insert` for the
    // window. Best-effort: a failure leaves the block unplaced,
    // for the orphan reaper. `focus` defaults to true, as the UI
    // flow does (#3032). A split of a named pane, as `pane.open`.
    if headless {
        super::agent_attach::park_new(state, &tab_id, &block_id).await?; // a start: parked, no pane
    } else {
        let placement = super::pane::resolve_placement(cmd.split_direction.as_deref(), cmd.split_reference_block_id.as_deref(), cmd.focus.unwrap_or(true));
        if let Err(e) = crate::server::service::layout_intents::place_block(state, &tab_id, &block_id, placement).await {
            tracing::warn!("agent.open: placing the pane failed: {e}");
        }
    }
    Ok(block_id)
}

/// Step 9: register the block's controller (not for a terminal-tier CLI: its term view starts it once attached).
pub(super) fn start_controller(state: &AppState, provider: &providers::ProviderConfig, tab_id: &str, block_id: &str, headless: bool) -> Result<(), String> {
    let mstore = state.mstore.clone();
    let broker = state.broker.clone();
    let event_bus = state.event_bus.clone();
    let filestore = state.filestore.clone();
    let app_state = state;
    let block_for_resync = mstore.must_get::<Block>(block_id)
        .map_err(|e| format!("agent.open: reload block: {e}"))?;
    super::agent_attach::set_desired(&mstore, block_id, super::agent_attach::DESIRED_RUNNING);
    if provider.controller_type != providers::ControllerType::Terminal { blockcontroller::resync_controller(
        &block_for_resync,
        if headless { "" } else { tab_id },
        None,
        true,
        true,
        Some(broker.clone()),
        Some(event_bus.clone()),
        Some(mstore.clone()),
        Some(filestore.clone()),
        Some(app_state.id_store.clone()),
        Some(app_state.identity_store.clone()),
        mstore.shared_agent_registry(),
        app_state.boot_id.clone(),
        &app_state.auth_key,
        Some(app_state.config_watcher.clone()),
    )?; }
    Ok(())
}

/// Step 10: tell every window about the new block, its tab and the tab's layout, in one batched frame.
pub(super) fn broadcast_open_updates(state: &AppState, block_id: &str, tab_id: &str) {
    let mstore = &state.mstore;
    let event_bus = &state.event_bus;
    let mut updates = Vec::new();
    if let Ok(updated_block) = mstore.must_get::<Block>(block_id) {
        updates.push(obj::MuxObjUpdate {
            updatetype: "update".into(),
            otype: "block".into(),
            oid: block_id.to_string(),
            obj: Some(obj::mux_obj_to_value(&updated_block)),
        });
    }
    if let Ok(updated_tab) = mstore.must_get::<Tab>(tab_id) {
        updates.push(obj::MuxObjUpdate {
            updatetype: "update".into(),
            otype: "tab".into(),
            oid: tab_id.to_string(),
            obj: Some(obj::mux_obj_to_value(&updated_tab)),
        });
        if let Ok(updated_layout) = mstore.must_get::<obj::LayoutState>(&updated_tab.layoutstate) {
            updates.push(obj::MuxObjUpdate {
                updatetype: "update".into(),
                otype: "layout".into(),
                oid: updated_tab.layoutstate.clone(),
                obj: Some(obj::mux_obj_to_value(&updated_layout)),
            });
        }
    }
    // One batched frame so the renderer applies all of them in
    // a single reactive flush — see
    // EventBus::broadcast_mux_obj_updates.
    event_bus.broadcast_mux_obj_updates(&updates);
}

/// The controller type a new pane gets for this agent's runtime.
pub(super) fn open_controller_type(agent: &AgentDefinition, provider: &providers::ProviderConfig) -> Result<&'static str, String> {
    let runtime = crate::backend::runtime_adapter::RuntimeKind::of_agent_type(&agent.agent_type).adapter();
    if !runtime.can_host(provider.controller_type) { return Err(format!("{} runs on the host, not in a container", provider.id)); }
    let controller_type = provider.controller_type_str();
    // Container agents use per-turn docker exec; the subprocess controller
    // is required regardless of what the provider defaults to (claude returns
    // "persistent", which causes AgentInput to skip the container exec path).
    Ok(runtime.controller_type_for_open(controller_type))
}

/// Where the agent runs.
pub(super) fn agent_work_dir(agent: &AgentDefinition) -> String {
    // Shared with native-memory resolution so the two can never
    // disagree about where a blank-working_directory agent
    // actually runs — they did, and Personal Bundle broke for the
    // common case as a result (ReAgent/Codex P1, PR #2901; see
    // SPEC_FIX_PERSONAL_MEMORY_EMPTY_WORKDIR_2026_09_01.md).
    if agent.working_directory.is_empty() {
        crate::backend::storage::agents::default_agent_working_dir(&agent.name)
    } else {
        agent.working_directory.clone()
    }
}

/// The id the agent is known by on the message bus: its stored slug (stable across renames), else one derived from its name.
pub(super) fn routing_id(agent: &AgentDefinition) -> String {
    if agent.slug.is_empty() { agentmux_common::slug::path_slug(&agent.name) } else { agent.slug.clone() }
}

/// Step 8: write the agent's config files, then record the launch and stamp the block before the resync.
pub(super) fn write_files_and_record_launch(state: &AppState, agent: &AgentDefinition, routing_id: &str, work_dir: &str, block_id: &str) -> Result<(), String> {
    // No collision resolution in this path — the function creates the dir if
    // missing and overwrites whatever's there. Same-name same-hour launches
    // will share a workdir; proper allocation is tracked as a follow-up.
    write_agent_config_files(&state.mstore, &state.id_store, &state.identity_store, agent, routing_id, work_dir)?;
    // Identity M4b-4: a controller can report its session id during the
    // resync, and that capture finds the row by block (Codex P2 on #3584).
    record_agent_open_launch(&state.mstore, &agent.id, block_id)
}

/// The status of the pane's controller, or `init` when it has none yet. A pane seeded with a registry-derived
/// `agent:sessionid` can come back already `running` (codex P2 on PR #3513), so this is read, not assumed.
pub(super) fn launch_status(block_id: &str) -> String {
    blockcontroller::get_block_controller_status(block_id)
        .map(|s| s.shellprocstatus)
        .unwrap_or_else(|| "init".to_string())
}
