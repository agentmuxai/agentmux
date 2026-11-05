use super::*;
use super::agent_open_steps as steps;

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

/// How long `agent.open` waits for a missing pinned CLI to install before telling
/// the caller to retry. The install is not cancelled when this elapses.
pub(super) const AGENT_OPEN_INSTALL_WAIT_SECS: u64 = 150;

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
/// Resolve the CLI argv for spawning `provider`'s process under `controller_type`
/// (the string form: `"persistent"` / `"subprocess"` / `"acp"` / `"app-server"`).
/// Pure — no I/O — so it's directly unit-testable without the full async
/// spawn-time harness this is called from.
///
/// ReAgent P1 on PR #3215: this used to only branch on `"persistent"`, so an
/// `"app-server"` controller silently fell back to `provider.launch_args` —
/// Codex's one-shot `exec --json` args — instead of `provider.app_server`'s
/// `--listen stdio://` args. The spawned child would never complete the App
/// Server handshake once the provider registry flips a provider's controller
/// type to `AppServer`.
pub(super) fn resolve_cli_args(provider: &providers::ProviderConfig, controller_type: &str) -> Vec<String> {
    let args: &[&str] = match controller_type {
        "persistent" => provider.persistent_launch_args.unwrap_or(provider.launch_args),
        "app-server" => provider
            .app_server
            .as_ref()
            .map(|app_server| app_server.launch_args)
            .unwrap_or(provider.launch_args),
        _ => provider.launch_args,
    };
    args.iter().map(|s| s.to_string()).collect()
}

pub(super) fn resolve_vendor_env_override(
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

// resolve_effective_provider_id (and its store-lookup helper) moved to
// Store::resolve_effective_provider_id in backend/storage/agents/definition.rs
// (2026-08-15) — the identical resolution logic was independently
// duplicated in identity/resolver/inject.rs's layer-3 credential gate,
// found reading agent.provider directly (the same mstore-vs-id_store
// class of bug this file's own version was fixed for in round 3 of PR
// #2587's review). Consolidated so both call sites share one
// implementation and can't drift on it separately again — see that
// method's own doc comment for the full history and the tests that
// moved with it.

#[cfg(test)]
mod resolve_vendor_env_override_tests {
    use super::*;

    fn base_agent(model_vendor_base_url: &str) -> AgentDefinition {
        AgentDefinition {
            seat: None,
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
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
            model_vendor_base_url: model_vendor_base_url.to_string(),
            memory_id: String::new(),
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

#[cfg(test)]
mod resolve_cli_args_tests {
    use super::*;

    #[test]
    fn persistent_controller_uses_persistent_launch_args() {
        let provider = providers::get_provider("claude").unwrap();
        assert_eq!(
            resolve_cli_args(provider, "persistent"),
            provider.persistent_launch_args.unwrap().to_vec()
        );
    }

    #[test]
    fn subprocess_controller_uses_the_default_launch_args() {
        let provider = providers::get_provider("codex").unwrap();
        assert_eq!(resolve_cli_args(provider, "subprocess"), provider.launch_args.to_vec());
    }

    /// ReAgent P1 on PR #3215: this is the bug fix. Before it, an
    /// `"app-server"` controller_type fell into the `else` branch and got
    /// Codex's one-shot `exec --json` args — the App Server child would spawn
    /// but never see `--listen stdio://`, so it never speaks the App Server
    /// protocol and the handshake in `AppServerController::start` hangs until
    /// `INITIALIZE_TIMEOUT`.
    #[test]
    fn app_server_controller_uses_the_app_server_launch_args_not_the_one_shot_ones() {
        let provider = providers::get_provider("codex").unwrap();
        let app_server_args = provider.app_server.as_ref().unwrap().launch_args.to_vec();

        let resolved = resolve_cli_args(provider, "app-server");

        assert_eq!(resolved, app_server_args);
        assert_ne!(
            resolved,
            provider.launch_args.to_vec(),
            "must not silently fall back to the one-shot exec args"
        );
    }

    /// Defensive: a provider with no `app_server` config (everything except
    /// Codex today) must not panic if `controller_type` is somehow
    /// `"app-server"` for it — fall back to the ordinary launch args instead.
    #[test]
    fn app_server_controller_type_falls_back_gracefully_for_a_provider_without_one() {
        let provider = providers::get_provider("claude").unwrap();
        assert!(provider.app_server.is_none());
        assert_eq!(resolve_cli_args(provider, "app-server"), provider.launch_args.to_vec());
    }
}

/// The agent `agent.open` opens: a user agent (`is_seeded == 0`), named by
/// definition id or by display name (ASCII case-insensitive) — its id first,
/// then its name. Only when no user agent answers is a template consulted,
/// and then only to refuse it with `TEMPLATE_NOT_OPENABLE`: opening one would
/// launch a session that is no agent's (spec §6.5.8, template sessions); an
/// agent is created from a template in the picker. A template's id is its
/// lowercase name (`claude`), which a user agent created from it shares
/// case-insensitively, so templates must never be matched first.
pub(super) fn select_openable_agent<'a>(
    agents: &'a [AgentDefinition],
    wanted: &str,
) -> Result<&'a AgentDefinition, String> {
    let names = |a: &&AgentDefinition| a.id == wanted || a.name.eq_ignore_ascii_case(wanted);
    let users = || agents.iter().filter(|a| a.is_seeded == 0);
    let found = users()
        .find(|a| a.id == wanted)
        .or_else(|| users().find(|a| a.name.eq_ignore_ascii_case(wanted)))
        .or_else(|| agents.iter().find(names))
        .ok_or_else(|| format!("AGENT_NOT_FOUND: no agent definition with id '{wanted}'"))?;
    if found.is_seeded != 0 {
        return Err(format!(
            "TEMPLATE_NOT_OPENABLE: '{}' is a template; agent.open opens an agent from My Agents — create an agent from this template in the agent picker first",
            found.name
        ));
    }
    Ok(found)
}

/// Identity M4b-4 (spec §6.5.8): a user agent's local row, backfilled from
/// the shared registry if it is another channel's — and **confirmed**, or the
/// open fails: without a local row the spawn would resolve to nothing, with
/// no UID/token and no credential gate, exactly the process M4b closes (Codex
/// P1 on #3584). `agent_def_list` listed the agent, so a missing row here is a
/// failed backfill, not a missing agent.
pub(super) fn require_local_agent_row(mstore: &Store, agent_id: &str) -> Result<(), String> {
    match mstore.agent_row_ensure_local(agent_id) {
        Ok(true) => Ok(()),
        Ok(false) => Err(format!(
            "AGENT_ROW_UNAVAILABLE: could not create a local row for agent '{agent_id}' — not started"
        )),
        Err(e) => Err(format!(
            "AGENT_ROW_UNAVAILABLE: local row for agent '{agent_id}' could not be read or created ({e}) — not started"
        )),
    }
}

/// Identity M4b-4 (spec §6.5.8): record `agent.open`'s launch of user agent
/// `agent_id` on `block_id` — a lifecycle-only update, never
/// `instance_create` with a fresh record (whose fold would blank the agent's
/// account, bundle and workspace) — and stamp the block with the row id.
/// A row that is gone (the agent deleted mid-open — the delete handlers do
/// not take `AGENT_OPEN_LOCKS`) or a failed write **fails the open**: the
/// spawn would otherwise resolve to nothing, tokenless and past the
/// credential gate (Codex P1 on #3584). A failed stamp is only logged — for a
/// user agent the block's `agentId` already names its row.
pub(super) fn record_agent_open_launch(mstore: &Store, agent_id: &str, block_id: &str) -> Result<(), String> {
    let now = agentmux_common::time::now_ms();
    match mstore.instance_record_launch(agent_id, block_id, now) {
        Ok(true) => {
            let mut stamp = obj::MetaMapType::new();
            stamp.insert("agentInstanceId".to_string(), json!(agent_id));
            if let Err(e) = crate::server::service::update_object_meta(
                mstore,
                &format!("block:{block_id}"),
                &stamp,
            ) {
                tracing::warn!(agent_id, block_id, error = %e, "agent.open: stamping the block failed");
            }
        }
        Ok(false) => {
            return Err(format!(
                "AGENT_ROW_UNAVAILABLE: agent '{agent_id}' has no local row to record the launch on (deleted?) — not started"
            ));
        }
        Err(e) => {
            return Err(format!(
                "AGENT_ROW_UNAVAILABLE: recording the launch of agent '{agent_id}' failed ({e}) — not started"
            ));
        }
    }
    Ok(())
}

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_agent_open(engine, state);
}

fn register_agent_open(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let app_state = state.clone();

    engine.register_typed(COMMAND_AGENT_OPEN, move |cmd: CommandAgentOpenData, _ctx| {
        let app_state = app_state.clone();
        async move { open_agent_impl(&app_state, cmd).await }
    });
}

/// The `agent.open` implementation — load the definition, resolve the
/// provider/CLI, create the block, enqueue the layout insert, and register
/// the controller. Extracted from the RPC closure so the HTTP surface
/// (`POST /api/v1/agent/open`, backing the `OpenAgent` MCP tool) shares this
/// exact code path instead of re-deriving it: the TOCTOU serialization via
/// `AGENT_OPEN_LOCKS` (reagent P1 on PR #2059), the ABF provider shadowing,
/// the resume-session seeding guard, and the CLI path resolution all live
/// here once. Same extraction shape as `open_pane` / `handle_pane_open`.
/// See docs/reports/REPORT_AGENT_OPEN_API_GAP_2026_09_06.md.
pub(crate) async fn open_agent_impl(
    state: &AppState,
    cmd: CommandAgentOpenData,
) -> Result<AgentOpenResult, String> {
    open_agent_inner(state, cmd, None, MetaMapType::new()).await
}

/// `agent.open`, but launching into an existing, empty agent pane
/// (`{view:"agent"}`, the picker) instead of creating a new one and
/// inserting it into the layout. Used by `layout.open` and Workspaces → Import…
/// (`SPEC_LAYOUT_FILES_2026_09_25.md` §3.5), which rebuilds the saved split
/// tree first and then launches each agent into its placeholder — so every
/// check `agent.open` makes (admission, one live instance, provider, CLI,
/// resume, config files) applies unchanged. If the agent is already open in
/// this tab, that pane is returned and the placeholder is left as a picker.
pub(crate) async fn open_agent_into_block(
    state: &AppState,
    agent_id: &str,
    tab_id: &str,
    block_id: &str,
) -> Result<AgentOpenResult, String> {
    open_agent_inner(
        state,
        CommandAgentOpenData {
            agent_id: agent_id.to_string(),
            tab_id: Some(tab_id.to_string()),
            split_direction: None,
            split_reference_block_id: None,
            focus: Some(false),
        },
        Some(block_id),
        MetaMapType::new(),
    )
    .await
}

/// `extra_meta` goes on a new block as it's created (srv-only keys: `agent_spawn.rs`).
pub(super) async fn open_agent_inner(
    state: &AppState,
    cmd: CommandAgentOpenData,
    into_block: Option<&str>,
    extra_meta: MetaMapType,
) -> Result<AgentOpenResult, String> {
    let mut extra_meta = extra_meta;
    let headless = extra_meta.remove(super::agent_attach::START_PARKED).is_some(); // a start: no pane
    let mstore = state.mstore.clone();

    tracing::info!(agent_id = %cmd.agent_id, "agent.open");

    // 1. Load the agent definition (by id or name)
    let agent = steps::pick_openable_agent(state, &cmd.agent_id)?;

    // Serialize the rest of this handler per agent definition —
    // held until the function returns (guard drops at every exit
    // path, success or error). See AGENT_OPEN_LOCKS' doc comment.
    let open_lock = agent_open_lock(&agent.id);
    let _open_guard = open_lock.lock().await;

    steps::admit_open(state, &agent).await?;

    // 2. Resolve provider
    let provider = providers::get_provider(&agent.provider)
        .ok_or_else(|| format!("INVALID_PROVIDER: unknown provider '{}'", agent.provider))?;

    // 3-4. The tab, and the agent's own block when it has one: its pane in this tab, a
    //      parked block attached here, or the pane that shows it elsewhere (`agent_attach`).
    let tab_id = match super::agent_attach::tab_or_reuse(state, &agent, provider, &cmd, headless, into_block).await? {
        Ok(tab_id) => tab_id,
        Err(done) => return Ok(done),
    };

    // 5. Resolve the CLI path (see `resolve_cli_path`).
    let resolved_cli_path = steps::resolve_cli_path(provider).await?;

    // 6. Build metadata
    let controller_type = steps::open_controller_type(&agent, provider)?;
    let (seeded, pane_flags) = steps::seed_runtime_args(&mstore, &agent, provider, controller_type);

    let work_dir = steps::agent_work_dir(&agent);
    let routing_id = steps::routing_id(&agent);
    let auth_dir = steps::prepare_auth_dir(provider)?;
    let env_vars = steps::launch_env(&mstore, &agent, provider, &auth_dir, &routing_id, controller_type)?;

    let resume_session_id = steps::resume_session_for(&mstore, &agent).await;
    let mut meta = steps::pane_identity_meta(&mstore, &agent, provider);
    let launch = steps::LaunchMeta { controller_type, cli_path: &resolved_cli_path, seeded, pane_flags: &pane_flags, work_dir: &work_dir, env_vars, resume_session_id: resume_session_id.as_deref(), extra_meta };
    steps::add_launch_meta(&mut meta, provider, launch);

    // 7. Create the block (see `create_agent_block`).
    let meta_val = serde_json::to_value(&meta)
        .map_err(|e| format!("agent.open: meta serialize: {e}"))?;
    let block_id = match into_block {
        Some(target) => steps::fill_existing_pane(state, target, meta_val).await?,
        None => steps::create_agent_block(state, &tab_id, meta_val, headless, &cmd).await?,
    };

    tracing::info!(
        block_id = %block_id,
        agent_id = %cmd.agent_id,
        provider = %agent.provider,
        controller_type = %controller_type,
        "agent.open: block created + layout updated"
    );

    // 8. Write the config files and record the launch (see `write_files_and_record_launch`).
    steps::write_files_and_record_launch(state, &agent, &routing_id, &work_dir, &block_id)?;

    steps::start_controller(state, provider, &tab_id, &block_id, headless)?;

    // 10. Broadcast block + tab + layout updates to frontend
    steps::broadcast_open_updates(state, &block_id, &tab_id);

    let status = steps::launch_status(&block_id);
    Ok(AgentOpenResult {
        block_id,
        tab_id,
        agent_id: cmd.agent_id,
        provider: agent.provider,
        controller_type: controller_type.to_string(),
        status,
        created: true,
    })
}

/// Write agent config files (CLAUDE.md, .mcp.json, etc.) to the working directory.
pub(super) fn write_agent_config_files(
    mstore: &Store,
    id_store: &Store,
    identity_store: &Store,
    agent: &crate::backend::storage::AgentDefinition,
    agent_slug: &str,
    work_dir: &str,
) -> Result<(), String> {
    // An empty working_directory field means the caller chose the fallback `~/.agentmux/agents/<slug>`,
    // shared by every same-name launch.
    let workdir_is_shared = agent.working_directory.is_empty();
    let config_files = super::agent_config_render::render_agent_config_files(
        mstore,
        id_store,
        identity_store,
        agent,
        &agent.name,
        agent_slug,
        work_dir,
        workdir_is_shared,
    );
    write_rendered_config_files(mstore, agent, config_files, work_dir)
}

/// Write `config_files` into `work_dir` and record what the agent will read as project instructions.
fn write_rendered_config_files(
    mstore: &Store,
    agent: &crate::backend::storage::AgentDefinition,
    config_files: Vec<crate::backend::agent_config::AgentConfigFile>,
    work_dir: &str,
) -> Result<(), String> {
    // Expand ~ in work_dir
    let expanded_dir = if let Some(under_override) = crate::backend::base::expand_under_home_override(work_dir) {
        under_override.to_string_lossy().to_string()
    } else if let (true, Ok(home)) = (work_dir.starts_with("~/") || work_dir == "~", std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"))) {
        format!("{}/{}", home, work_dir.trim_start_matches("~/"))
    } else {
        work_dir.to_string()
    };

    let base_path = std::path::Path::new(&expanded_dir);
    if !base_path.exists() {
        std::fs::create_dir_all(base_path) // workdir-fs: creates the workdir itself
            .map_err(|e| format!("failed to create working dir: {e}"))?;
    }
    let wd = crate::backend::workdir_fs::Workdir::open(base_path)
        .map_err(|e| format!("failed to open working dir {}: {e}", base_path.display()))?;

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
        super::agent_config_render::write_config_file(&wd, base_path, &expanded_dir, file)?;
    }

    crate::backend::agent_config::write_managed_skill_file_manifest(base_path, &new_managed_skill_paths);

    // Record what this agent will read as project instructions, now that
    // AgentMux's own files are in their final state for this launch — observed
    // before the write, this would record the previous launch's content.
    //
    // Phase 3 of SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md.
    // Observing at open is the spec's §8 decision 2: it costs nothing, catches
    // the case that matters (the repository changed since last launch), and
    // avoids a watcher. Drift *during* a session is deliberately not covered
    // yet — revisit only if it shows up in practice.
    //
    // Best-effort: an agent must never fail to launch because a tracking row
    // could not be written.
    observe_project_instructions(mstore, agent, &expanded_dir);

    tracing::info!(
        agent_id = %agent.id,
        work_dir = %expanded_dir,
        file_count = config_files.len(),
        "agent.open: wrote config files"
    );

    Ok(())
}

/// Resolve and record this agent's project instructions.
///
/// Split out so the launch path reads as one line and this can be tested on
/// its own. Never returns an error: see the call site.
pub(super) fn observe_project_instructions(
    mstore: &crate::backend::storage::store::Store,
    agent: &crate::backend::storage::AgentDefinition,
    working_dir: &str,
) {
    use crate::backend::storage::project_instructions::ProjectInstructionObservation;

    let files = crate::backend::project_instructions::resolve_project_instructions(
        &agent.provider,
        working_dir,
    );
    if files.is_empty() {
        return;
    }
    let now = agentmux_common::time::now_ms();
    let observations: Vec<ProjectInstructionObservation> = files
        .iter()
        .map(|f| ProjectInstructionObservation {
            path: f.path.clone(),
            content_hash: f.content_hash.clone(),
            size_bytes: f.size_bytes as i64,
            owner: match f.owner {
                crate::backend::project_instructions::InstructionOwner::Agentmux => "agentmux",
                crate::backend::project_instructions::InstructionOwner::Foreign => "foreign",
            }
            .to_string(),
            existed: f.exists,
            observed_at: now,
        })
        .collect();

    if let Err(e) = mstore.project_instructions_record(&agent.id, &observations) {
        tracing::warn!(
            agent_id = %agent.id, error = %e,
            "agent.open: could not record project-instruction observations"
        );
    }
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
            seat: None,
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
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
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
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
        let mstore = make_store();
        let id_store = make_store();
        let work_dir = tempfile::tempdir().unwrap();
        let work_dir_str = work_dir.path().to_str().unwrap();

        let mut agent = make_agent("agent-1", work_dir_str);
        mstore.agent_def_insert(&mut agent).unwrap();
        mstore.skill_upsert_unique(&mstore, "agent-1", &make_skill("agent-skill"), true).unwrap();

        write_agent_config_files(&mstore, &id_store, &mstore, &agent, "test-agent", work_dir_str).unwrap();
        let skill_md = work_dir.path().join(".claude/skills/deploy/SKILL.md");
        assert!(skill_md.exists(), "expected .claude/skills/deploy/SKILL.md to be written");

        // Flip the same skill to "prompt" format and relaunch.
        let mut prompt_skill = make_skill("prompt");
        prompt_skill.updated_at = 1_700_000_000_001;
        mstore.skill_upsert_unique(&mstore, "agent-1", &prompt_skill, true).unwrap();
        write_agent_config_files(&mstore, &id_store, &mstore, &agent, "test-agent", work_dir_str).unwrap();

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
        let mstore = make_store();
        let id_store = make_store();
        let work_dir = tempfile::tempdir().unwrap();
        let work_dir_str = work_dir.path().to_str().unwrap();

        let mut agent = make_agent("agent-1", work_dir_str);
        mstore.agent_def_insert(&mut agent).unwrap();

        let user_file = work_dir.path().join(".claude/commands/my-own-command.md");
        std::fs::create_dir_all(user_file.parent().unwrap()).unwrap();
        std::fs::write(&user_file, "# hand-authored, not from AgentMux").unwrap();

        mstore.skill_upsert_unique(&mstore, "agent-1", &make_skill("agent-skill"), true).unwrap();
        write_agent_config_files(&mstore, &id_store, &mstore, &agent, "test-agent", work_dir_str).unwrap();
        write_agent_config_files(&mstore, &id_store, &mstore, &agent, "test-agent", work_dir_str).unwrap();

        assert!(user_file.exists(), "hand-authored file outside AgentMux's manifest must survive");
    }

    /// Defense-in-depth: even if a manifest somehow contains a path-traversal
    /// entry (e.g. from a bypass of the trigger sanitization added upstream
    /// in agent_config.rs, or manual tampering with the manifest file
    /// itself), the cleanup pass must never delete anything outside the
    /// agent's working directory (reagent P1, PR #2322).
    #[test]
    fn cleanup_refuses_to_delete_a_manifest_path_that_escapes_the_working_dir() {
        let mstore = make_store();
        let id_store = make_store();
        let work_dir = tempfile::tempdir().unwrap();
        let work_dir_str = work_dir.path().to_str().unwrap();

        let mut agent = make_agent("agent-1", work_dir_str);
        mstore.agent_def_insert(&mut agent).unwrap();

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
        write_agent_config_files(&mstore, &id_store, &mstore, &agent, "test-agent", work_dir_str).unwrap();

        assert!(sentinel.exists(), "file outside the working directory must never be deleted");
    }

    /// A `.claude` that links outside the workdir: the generic file loop
    /// (skill files, settings) used to write straight through it, with only
    /// a lexical check (SPEC_WORKDIR_SAFE_WRITES_2026_10_01 §1). Now refused
    /// and skipped, and the launch still succeeds.
    #[test]
    fn config_files_are_never_written_through_a_claude_folder_linking_outside() {
        let mstore = make_store();
        let id_store = make_store();
        let work_dir = tempfile::tempdir().unwrap();
        let work_dir_str = work_dir.path().to_str().unwrap();
        let outside = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(outside.path(), work_dir.path().join(".claude")).is_ok();
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_dir(outside.path(), work_dir.path().join(".claude")).is_ok();
        if !linked {
            return; // this OS user can't create links
        }

        let mut agent = make_agent("agent-1", work_dir_str);
        mstore.agent_def_insert(&mut agent).unwrap();
        mstore.skill_upsert_unique(&mstore, "agent-1", &make_skill("agent-skill"), true).unwrap();
        write_agent_config_files(&mstore, &id_store, &mstore, &agent, "test-agent", work_dir_str).unwrap();

        let leaked: Vec<_> = walkdir_files(outside.path());
        assert!(leaked.is_empty(), "nothing may be written outside the workdir, found {leaked:?}");
    }

    fn walkdir_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                out.extend(walkdir_files(&p));
            } else {
                out.push(p);
            }
        }
        out
    }

    /// A bundle carries no MCP servers
    /// (SPEC_BUNDLE_CONTENTS_MEMORY_NOT_MCP_2026_10_07.md §3.1): a bundle ref
    /// an older build left behind neither reaches `.mcp.json` nor counts as
    /// the agent's own, so the agent's legacy blob is still merged.
    #[test]
    fn a_leftover_bundle_mcp_ref_neither_launches_nor_drops_the_agents_legacy_mcp_blob() {
        let mstore = make_store();
        let id_store = make_store();
        let work_dir = tempfile::tempdir().unwrap();
        let work_dir_str = work_dir.path().to_str().unwrap();

        let mut agent = make_agent("agent-1", work_dir_str);
        mstore.agent_def_insert(&mut agent).unwrap();
        mstore.agent_bundles_set("agent-1", &["bundle-1".to_string()]).unwrap();

        mstore
            .bundle_upsert(&crate::backend::storage::bundles::Bundle {
                id: "bundle-1".to_string(),
                name: "Bundle 1".to_string(),
                description: String::new(),
                is_blank: false,
                is_global: false,
                provider: "claude".to_string(),
                model: "anthropic".to_string(),
                instructions: String::new(),
                instructions_by_provider: "{}".to_string(),
                context_files: "[]".to_string(),
                mcp_servers: "[]".to_string(),
                skills: "[]".to_string(),
                sort_order: 0,
                created_at: 1_700_000_000_000,
                updated_at: 1_700_000_000_000,
                is_system: false,
            })
            .unwrap();
        leftover_bundle_mcp_ref(&mstore, "bundle-1", "bundle-private-server", "bundle-server", "bundle-tool");
        // agent-1 itself has NO own db_mcp_servers ref — only its bundle does.

        mstore
            .agent_content_set(&crate::backend::storage::content::AgentContent {
                agent_id: "agent-1".to_string(),
                content_type: "mcp".to_string(),
                content: r#"{"mcpServers":{"legacy-server":{"command":"legacy-tool"}}}"#.to_string(),
                updated_at: 1_700_000_000_000,
            })
            .unwrap();

        write_agent_config_files(&mstore, &id_store, &mstore, &agent, "test-agent", work_dir_str).unwrap();

        let mcp_json_path = work_dir.path().join(".mcp.json");
        let mcp_json = std::fs::read_to_string(&mcp_json_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&mcp_json).unwrap();
        assert!(
            parsed["mcpServers"].get("legacy-server").is_some(),
            "a leftover bundle ref must not drop the agent's legacy .mcp.json blob: {mcp_json}"
        );
        assert!(
            parsed["mcpServers"].get("bundle-server").is_none(),
            "a bundle's server must not launch: {mcp_json}"
        );
    }

    /// A bundle MCP ref of the kind an older build wrote; nothing writes one now.
    fn leftover_bundle_mcp_ref(store: &Store, bundle_id: &str, id: &str, name: &str, command: &str) {
        store
            .mcp_server_insert_raw(&crate::backend::storage::McpServer {
                id: id.to_string(),
                name: name.to_string(),
                transport: "stdio".to_string(),
                config: format!(r#"{{"command":"{command}"}}"#),
                is_global: false,
                created_at: 1_700_000_000_000,
                updated_at: 1_700_000_000_000,
            })
            .unwrap();
        store
            .conn()
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO db_bundle_mcp_ref (bundle_id, mcp_id) VALUES (?1, ?2)",
                rusqlite::params![bundle_id, id],
            )
            .unwrap();
    }

    fn make_bundle(id: &str, name: &str, instructions: &str, is_global: bool) -> crate::backend::storage::bundles::Bundle {
        crate::backend::storage::bundles::Bundle {
            id: id.to_string(),
            name: name.to_string(),
            description: String::new(),
            is_blank: false,
            is_global,
            provider: String::new(),
            model: String::new(),
            instructions: instructions.to_string(),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            sort_order: 0,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
            is_system: false,
        }
    }

    /// The bundles picked for an agent reach its launch
    /// (SPEC_RENAME_KNOWLEDGE_TO_MEMORY_2026_10_06.md §3.6): their
    /// instructions as `# [Bundle]` sections after Global Memory, in list
    /// order; a listed bundle that no longer exists is skipped. Not MCP
    /// servers: a bundle carries none.
    #[test]
    fn picked_bundles_reach_the_startup_file_but_not_the_mcp_config() {
        let mstore = make_store();
        let id_store = make_store();
        let work_dir = tempfile::tempdir().unwrap();
        let work_dir_str = work_dir.path().to_str().unwrap();

        let mut agent = make_agent("agent-1", work_dir_str);
        mstore.agent_def_insert(&mut agent).unwrap();
        id_store.bundle_upsert(&make_bundle("global-1", "Rules", "global rule", true)).unwrap();
        id_store.bundle_upsert(&make_bundle("b1", "Reviewer", "review carefully", false)).unwrap();
        id_store.bundle_upsert(&make_bundle("b2", "Writer", "write plainly", false)).unwrap();
        leftover_bundle_mcp_ref(&mstore, "b2", "b2-server", "writer-tools", "writer-tool");
        mstore
            .agent_bundles_set("agent-1", &["b2".to_string(), "gone".to_string(), "b1".to_string()])
            .unwrap();

        write_agent_config_files(&mstore, &id_store, &mstore, &agent, "test-agent", work_dir_str).unwrap();

        let claude_md = std::fs::read_to_string(work_dir.path().join("CLAUDE.md")).unwrap();
        let global = claude_md.find("# [Workspace] Rules").expect("Global Memory section");
        let writer = claude_md.find("# [Bundle] Writer\n\nwrite plainly").expect("first pick's section");
        let reviewer = claude_md.find("# [Bundle] Reviewer\n\nreview carefully").expect("second pick's section");
        assert!(global < writer && writer < reviewer, "Global Memory, then the picks in order: {claude_md}");

        let mcp_json = std::fs::read_to_string(work_dir.path().join(".mcp.json")).unwrap_or_default();
        assert!(!mcp_json.contains("writer-tools"), "a picked bundle's server must not launch: {mcp_json}");
    }

    /// `memory:launchcost` counts what a launch writes: its parts sum to the
    /// estimate of the written startup file (SPEC_MEMORY_PANE_TOKENS_2026_10_15.md §8).
    #[test]
    fn launch_cost_is_the_estimate_of_the_startup_file_a_launch_writes() {
        let (mstore, id_store) = (make_store(), make_store());
        let work_dir = tempfile::tempdir().unwrap();
        let mut agent = make_agent("agent-1", work_dir.path().to_str().unwrap());
        agent.provider = "codex".to_string();
        mstore.agent_def_insert(&mut agent).unwrap();
        id_store.bundle_upsert(&make_bundle("global-1", "Rules", "global rule", true)).unwrap();
        id_store.bundle_upsert(&make_bundle("b1", "Reviewer", "review carefully", false)).unwrap();
        mstore.agent_bundles_set("agent-1", &["b1".to_string()]).unwrap();
        let (agent_id, content_type) = ("agent-1".to_string(), "soul".to_string());
        let content = "You are {{AGENT}}, a careful reviewer.".to_string();
        let soul = crate::backend::storage::content::AgentContent { agent_id, content_type, content, updated_at: 1 };
        mstore.agent_content_set(&soul).unwrap();

        write_agent_config_files(&mstore, &id_store, &mstore, &agent, "test-agent", work_dir.path().to_str().unwrap()).unwrap();
        let written = std::fs::read_to_string(work_dir.path().join("AGENTS.md")).unwrap();
        let cost = crate::server::memory_launch_cost::launch_cost(&mstore, &id_store, &mstore, &agent, Some(&written), &[]);
        let (_marker, body) = written.split_once("\n\n").unwrap();
        assert_eq!(cost.total_tokens_est, agentmux_common::tokens::estimate_tokens(body));
        let kinds: Vec<&str> = cost.parts.iter().map(|p| p.kind.as_str()).collect();
        assert_eq!(kinds, ["workspace_global", "bundle", "soul", "formatting"]);
        assert!(cost.delivered_by_file && cost.startup_file.as_deref() == Some("AGENTS.md"));
    }
}

#[cfg(test)]
mod record_agent_open_launch_tests {
    use super::*;
    use crate::backend::storage::agents::test_agent_def;

    /// Identity M4b-4 (spec §6.5.8): an `agent.open` of a user agent records
    /// the launch on its row (lifecycle only) and stamps the block, so the
    /// block resolves to that row — the row's UID and token at its spawn.
    /// Codex P1 on #3584: an open whose local row cannot be confirmed
    /// (here: an agent that exists neither locally nor in a registry) fails
    /// instead of spawning row-less; a local agent passes.
    #[test]
    fn an_open_without_a_confirmed_local_row_is_refused() {
        let store = Store::open_in_memory().unwrap();
        let err = require_local_agent_row(&store, "agent-nowhere").unwrap_err();
        assert!(err.starts_with("AGENT_ROW_UNAVAILABLE"), "{err}");
        let mut def = test_agent_def("agent-here", "Here", "claude", "agent", 1, "");
        store.agent_def_insert(&mut def).unwrap();
        assert!(require_local_agent_row(&store, "agent-here").is_ok());
    }

    /// Codex P1 on #3584: an agent deleted between the row check and the
    /// record (the delete handlers do not take the open lock) fails the open
    /// rather than spawning a block that names a missing row.
    #[test]
    fn an_open_whose_agent_was_deleted_mid_open_is_refused() {
        let store = Store::open_in_memory().unwrap();
        let mut def = test_agent_def("agent-gone-mid", "GoneMid", "claude", "agent", 1, "");
        store.agent_def_insert(&mut def).unwrap();
        assert!(require_local_agent_row(&store, "agent-gone-mid").is_ok());
        assert!(store.agent_def_delete("agent-gone-mid").unwrap());
        let err =
            record_agent_open_launch(&store, "agent-gone-mid", &uuid::Uuid::new_v4().to_string())
                .unwrap_err();
        assert!(err.starts_with("AGENT_ROW_UNAVAILABLE"), "{err}");
    }

    #[test]
    fn an_agent_open_launch_is_recorded_stamped_and_resolves() {
        let store = Store::open_in_memory().unwrap();
        let mut def = test_agent_def("agent-open-x", "OpenX", "claude", "agent", 1, "");
        store.agent_def_insert(&mut def).unwrap();
        // ORef::parse, behind update_object_meta, needs a UUID-shaped oid.
        let block_id = uuid::Uuid::new_v4().to_string();
        let mut block = Block {
            oid: block_id.clone(),
            parentoref: String::new(),
            version: 0,
            runtimeopts: None,
            stickers: None,
            meta: {
                let mut m = obj::MetaMapType::new();
                m.insert("agentId".to_string(), json!("agent-open-x"));
                m
            },
            subblockids: None,
        };
        store.insert(&mut block).unwrap();

        record_agent_open_launch(&store, "agent-open-x", &block_id).unwrap();

        let stamped: Block = store.must_get(&block_id).unwrap();
        assert_eq!(
            stamped.meta.get("agentInstanceId"),
            Some(&json!("agent-open-x"))
        );
        let row = store.instance_get("agent-open-x").unwrap().unwrap();
        assert_eq!(row.block_id, block_id);
        assert_eq!(row.status, "running");
        let resolved = store
            .instance_get_active_for_block(&block_id)
            .unwrap()
            .map(|r| r.id);
        assert_eq!(resolved.as_deref(), Some("agent-open-x"));
    }

    fn def(id: &str, name: &str, is_seeded: i64) -> AgentDefinition {
        let mut d = test_agent_def(id, name, "claude", "agent", 1, "");
        d.is_seeded = is_seeded;
        d
    }

    /// `agent.open` opens user agents only: a template is refused by id or by
    /// name, and a user agent is preferred over a template it shares a name
    /// with — including the template's id, which is its lowercase name
    /// (adversarial review of #3600) — whatever the list order.
    #[test]
    fn agent_open_selects_my_agents_and_refuses_templates() {
        let agents = vec![
            def("claude", "Claude", 1),
            def("agent-mine", "Claude", 0),
            def("codex", "Codex", 1),
            def("agent-other", "agent-mine", 0),
        ];
        let pick = |wanted| select_openable_agent(&agents, wanted).map(|a| a.id.as_str());

        assert_eq!(pick("agent-mine"), Ok("agent-mine"), "an id wins over a name");
        assert_eq!(pick("claude"), Ok("agent-mine"), "not the template's id");
        assert_eq!(pick("CLAUDE"), Ok("agent-mine"));
        for wanted in ["codex", "Codex"] {
            let err = pick(wanted).unwrap_err();
            assert!(err.starts_with("TEMPLATE_NOT_OPENABLE"), "{wanted}: {err}");
        }
        let err = pick("nobody").unwrap_err();
        assert!(err.starts_with("AGENT_NOT_FOUND"), "{err}");
    }
}

#[cfg(test)]
#[path = "agent_open_single_live_instance_tests.rs"]
mod single_live_instance_tests;
