// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


use std::sync::Arc;
use chrono::Utc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    COMMAND_LIST_AGENTS, COMMAND_CREATE_AGENT, COMMAND_UPDATE_AGENT,
    COMMAND_DELETE_AGENT, COMMAND_GET_AGENT_CONTENT, COMMAND_SET_AGENT_CONTENT,
    COMMAND_GET_ALL_AGENT_CONTENT,
    COMMAND_IMPORT_AGENT_FROM_CLAW, COMMAND_IMPORT_AGENTS, COMMAND_EXPORT_AGENTS,
    COMMAND_RESEED_AGENTS,
    COMMAND_CONTAINER_RUNTIME_AVAILABLE,
    CommandListAgentDefinitionsData,
    CommandCreateAgentDefinitionData, CommandUpdateAgentDefinitionData, CommandDeleteAgentDefinitionData,
    CommandGetAgentContentData, CommandSetAgentContentData, CommandGetAllAgentContentData,
    CommandImportAgentFromClawData,
    CommandImportAgentDefinitionsData, ImportAgentDefinitionsResult,
    ExportAgentDefinitionsResult, AgentDefinitionExport, AgentSkillExport,
};
use crate::backend::storage::{AgentDefinition, AgentContent, AgentSkill};

use super::super::AppState;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // listagents → return all agent definitions, optionally filtered by
    // `is_seeded`. Filter input is backward-compatible: callers that
    // pass `null` / `{}` (every existing caller) get the full list.
    // The two-tier picker (Phase 1) passes `{ is_seeded: 0 }` for the
    // "My Agents" section and `{ is_seeded: 1 }` for the "Templates"
    // section — see SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md.
    //
    // Phase 2 (Q2 Decision Y — hide templates): templates with
    // `user_hidden = 1` are filtered out by default. Callers that want
    // them back (the settings panel's "Hidden templates" surface) pass
    // `include_hidden: true`. Hide filter applies ONLY to templates —
    // user-owned definitions are unaffected (their `user_hidden` is
    // always 0 by backend invariant; `agent_def_set_hidden` rejects
    // non-template ids).
    let wstore_lfa = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_AGENTS,
        Box::new(move |data, _ctx| {
            let wstore = wstore_lfa.clone();
            Box::pin(async move {
                // unwrap_or_default — both `null` and `{}` deserialize
                // to the default (no filter). Anything malformed falls
                // back to no-filter rather than erroring; older clients
                // never sent a body for this RPC and we can't know
                // which JSON shape they're on.
                let cmd: CommandListAgentDefinitionsData =
                    serde_json::from_value(data).unwrap_or_default();
                let agents = wstore.agent_def_list().map_err(|e| format!("listagents: {e}"))?;
                let is_seeded_filter = cmd.is_seeded;
                let include_hidden = cmd.include_hidden;
                let filtered: Vec<_> = agents
                    .into_iter()
                    .filter(|a| match is_seeded_filter {
                        Some(flag) => a.is_seeded == flag,
                        None => true,
                    })
                    // Default behaviour: drop hidden templates. The
                    // settings panel opts back in with include_hidden.
                    // User-owned rows (is_seeded == 0) are never
                    // hideable; the conditional below is a no-op for
                    // them.
                    .filter(|a| {
                        include_hidden || a.is_seeded != 1 || a.user_hidden == 0
                    })
                    .collect();
                Ok(Some(serde_json::to_value(&filtered).unwrap_or_default()))
            })
        }),
    );

    // createagent → insert new agent, broadcast agents:changed
    let wstore_cfa = state.wstore.clone();
    let broker_cfa = state.broker.clone();
    engine.register_handler(
        COMMAND_CREATE_AGENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_cfa.clone();
            let broker = broker_cfa.clone();
            Box::pin(async move {
                let cmd: CommandCreateAgentDefinitionData = serde_json::from_value(data)
                    .map_err(|e| format!("createagent: {e}"))?;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                // slug is empty here — agent_def_insert auto-derives it
                // from name AND collision-resolves AND mutates the
                // struct so we serialize the resolved value back to
                // the frontend (not "").
                let mut agent = AgentDefinition {
                    id: uuid::Uuid::new_v4().to_string(),
                    slug: String::new(),
                    name: cmd.name,
                    icon: cmd.icon,
                    provider: cmd.provider,
                    description: cmd.description,
                    working_directory: cmd.working_directory,
                    shell: cmd.shell,
                    provider_flags: cmd.provider_flags,
                    auto_start: cmd.auto_start,
                    restart_on_crash: cmd.restart_on_crash,
                    idle_timeout_minutes: cmd.idle_timeout_minutes,
                    created_at: now,
                    agent_type: cmd.agent_type,
                    environment: cmd.environment,
                    agent_bus_id: cmd.agent_bus_id,
                    is_seeded: 0,
                    accounts: String::new(),
                    parent_id: String::new(),
                    branch_label: String::new(),
                    updated_at: now,
                    user_hidden: 0,
                    container_image: String::new(),
                    container_volumes: "[]".to_string(),
                    container_name: String::new(),
                    // New agents fail-by-default on missing oauth creds; the
                    // ambient opt-in is an explicit per-agent toggle (spec
                    // §2.2 edge case — no implicit ambient for fresh agents).
                    use_ambient_login: 0,
                    // Not settable via this RPC yet — only `agent.define`
                    // (the App-API/MCP path) can set a model vendor override.
                    model_vendor_base_url: String::new(),
                };
                wstore.agent_def_insert(&mut agent).map_err(|e| format!("createagent: {e}"))?;
                // Assign the agent its display color at creation
                // (SPEC_AGENT_COLOR_2026_08_08.md). Best-effort: a failure
                // here shouldn't fail the create — agent.open assigns a
                // color on first open as the fallback.
                let _ = wstore.agent_content_set(&crate::backend::storage::store::AgentContent {
                    agent_id: agent.id.clone(),
                    content_type: "ui:color".to_string(),
                    content: crate::backend::agent_color::pick_agent_color(&agent.id).to_string(),
                    updated_at: now,
                });
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&agent).unwrap_or_default()))
            })
        }),
    );

    // updateagent → update existing agent, broadcast agents:changed
    let wstore_ufa = state.wstore.clone();
    let broker_ufa = state.broker.clone();
    engine.register_handler(
        COMMAND_UPDATE_AGENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_ufa.clone();
            let broker = broker_ufa.clone();
            Box::pin(async move {
                let cmd: CommandUpdateAgentDefinitionData = serde_json::from_value(data)
                    .map_err(|e| format!("updateagent: {e}"))?;
                // Fetch existing to preserve created_at
                let existing = wstore.agent_def_list().map_err(|e| format!("updateagent: {e}"))?;
                let old = existing.iter().find(|a| a.id == cmd.id)
                    .ok_or_else(|| format!("updateagent: agent {} not found", cmd.id))?;
                // slug is preserved from the existing row — it's
                // immutable after creation. The update path never
                // accepts a new slug from the client.
                let mut agent = AgentDefinition {
                    id: cmd.id,
                    slug: old.slug.clone(),
                    name: cmd.name,
                    icon: cmd.icon,
                    provider: cmd.provider,
                    description: cmd.description,
                    working_directory: cmd.working_directory,
                    shell: cmd.shell,
                    provider_flags: cmd.provider_flags,
                    auto_start: cmd.auto_start,
                    restart_on_crash: cmd.restart_on_crash,
                    idle_timeout_minutes: cmd.idle_timeout_minutes,
                    created_at: old.created_at,
                    agent_type: cmd.agent_type,
                    environment: cmd.environment,
                    agent_bus_id: cmd.agent_bus_id,
                    is_seeded: old.is_seeded,
                    // Preserve existing accounts when the caller omits the field
                    // (cmd.accounts defaults to "" via #[serde(default)]). Callers
                    // that only update name/icon/etc. (AgentDefForm, AgentPicker rename)
                    // don't carry accounts, so falling back to old.accounts prevents
                    // silently wiping saved assignments.
                    accounts: if cmd.accounts.is_empty() { old.accounts.clone() } else { cmd.accounts },
                    // parent_id + branch_label describe provenance and
                    // are immutable post-insert (forks are separate rows,
                    // not in-place edits).
                    parent_id: old.parent_id.clone(),
                    branch_label: old.branch_label.clone(),
                    // Placeholder — agent_def_update self-stamps the real
                    // timestamp and writes it back into `agent` below, so
                    // the response body carries the fresh value.
                    updated_at: old.updated_at,
                    // Preserve user_hidden — updateagent edits the
                    // definition payload, not the per-user view-state
                    // flag. Hide/unhide go through their dedicated RPCs
                    // (`agentdefhide` / `agentdefunhide`). Phase 2 of
                    // SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md.
                    user_hidden: old.user_hidden,
                    // Preserve existing container config when the caller omits the field
                    // (cmd.container_image defaults to "" via #[serde(default)]). Callers
                    // that only update name/icon/etc. (AgentDefForm) don't carry container
                    // fields, so falling back to old values prevents silently wiping a
                    // container agent's image and volumes — same guard as `accounts` above.
                    container_image: if cmd.container_image.is_empty() { old.container_image.clone() } else { cmd.container_image },
                    container_volumes: if cmd.container_volumes == "[]" { old.container_volumes.clone() } else { cmd.container_volumes },
                    // container_name is server-managed; preserve the existing value.
                    container_name: old.container_name.clone(),
                    // Preserve the ambient-login opt-in when the caller omits
                    // the field (Option — most callers only edit name/icon/
                    // accounts). The Agent setup modal's Accounts tab sends
                    // Some(0|1) to flip it. Spec §2.3.
                    use_ambient_login: cmd.use_ambient_login.unwrap_or(old.use_ambient_login),
                    // Not carried by CommandUpdateAgentDefinitionData yet —
                    // always preserve, same as container_name/parent_id
                    // above, so a UI-driven save never silently wipes a
                    // vendor override set via `agent.define`.
                    model_vendor_base_url: old.model_vendor_base_url.clone(),
                };
                let found = wstore.agent_def_update(&mut agent).map_err(|e| format!("updateagent: {e}"))?;
                if !found {
                    return Err(format!("updateagent: agent {} not found", agent.id));
                }
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&agent).unwrap_or_default()))
            })
        }),
    );

    // deleteagent → delete agent by id, broadcast agents:changed
    let wstore_dfa = state.wstore.clone();
    let broker_dfa = state.broker.clone();
    engine.register_handler(
        COMMAND_DELETE_AGENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_dfa.clone();
            let broker = broker_dfa.clone();
            Box::pin(async move {
                let cmd: CommandDeleteAgentDefinitionData = serde_json::from_value(data)
                    .map_err(|e| format!("deleteagent: {e}"))?;
                wstore.agent_def_delete(&cmd.id).map_err(|e| format!("deleteagent: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(None)
            })
        }),
    );

    // containerruntimeavailable → does the Docker DAEMON answer a ping
    // right now? Returns `{ available: bool }`. The create-from-template
    // modal (via the frontend's shared toolchain-capabilities store) uses
    // this to gate/default the container runtime. Distinct from
    // `resolvecli docker`, which only confirms the CLI binary is on PATH —
    // that false-positives when Docker is installed but the daemon is
    // stopped, steering users into a container agent that can't start
    // (codex P1 on #1576). `ContainerRuntimeHandle::is_available()` retries
    // the connection on demand, so a daemon that came up or went down
    // since launch is reflected without an app restart.
    let container_manager_cra = state.container_manager.clone();
    engine.register_handler(
        COMMAND_CONTAINER_RUNTIME_AVAILABLE,
        Box::new(move |_data, _ctx| {
            let cm = container_manager_cra.clone();
            Box::pin(async move {
                let available = cm.is_available().await;
                Ok(Some(serde_json::json!({ "available": available })))
            })
        }),
    );

    // getagentcontent → return a single content blob for an agent
    let wstore_gfc = state.wstore.clone();
    engine.register_handler(
        COMMAND_GET_AGENT_CONTENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_gfc.clone();
            Box::pin(async move {
                let cmd: CommandGetAgentContentData = serde_json::from_value(data)
                    .map_err(|e| format!("getagentcontent: {e}"))?;
                let content = wstore.agent_content_get(&cmd.agent_id, &cmd.content_type)
                    .map_err(|e| format!("getagentcontent: {e}"))?;
                Ok(content.map(|c| serde_json::to_value(&c).unwrap_or_default()))
            })
        }),
    );

    // setagentcontent → upsert a content blob, broadcast agentcontent:changed
    let wstore_sfc = state.wstore.clone();
    let broker_sfc = state.broker.clone();
    engine.register_handler(
        COMMAND_SET_AGENT_CONTENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_sfc.clone();
            let broker = broker_sfc.clone();
            Box::pin(async move {
                let cmd: CommandSetAgentContentData = serde_json::from_value(data)
                    .map_err(|e| format!("setagentcontent: {e}"))?;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                let content = AgentContent {
                    agent_id: cmd.agent_id,
                    content_type: cmd.content_type,
                    content: cmd.content,
                    updated_at: now,
                };
                wstore.agent_content_set(&content).map_err(|e| format!("setagentcontent: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agentcontent:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&content).unwrap_or_default()))
            })
        }),
    );

    // getallagentcontent → return all content blobs for an agent
    let wstore_gafc = state.wstore.clone();
    engine.register_handler(
        COMMAND_GET_ALL_AGENT_CONTENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_gafc.clone();
            Box::pin(async move {
                let cmd: CommandGetAllAgentContentData = serde_json::from_value(data)
                    .map_err(|e| format!("getallagentcontent: {e}"))?;
                let contents = wstore.agent_content_get_all(&cmd.agent_id)
                    .map_err(|e| format!("getallagentcontent: {e}"))?;
                Ok(Some(serde_json::to_value(&contents).unwrap_or_default()))
            })
        }),
    );

    // ── Agent Import handler ───────────────────────────────────────────────

    // importagentfromclaw → read claw workspace, create agent + content
    let wstore_ifc = state.wstore.clone();
    let broker_ifc = state.broker.clone();
    engine.register_handler(
        COMMAND_IMPORT_AGENT_FROM_CLAW,
        Box::new(move |data, _ctx| {
            let wstore = wstore_ifc.clone();
            let broker = broker_ifc.clone();
            Box::pin(async move {
                let cmd: CommandImportAgentFromClawData = serde_json::from_value(data)
                    .map_err(|e| format!("importagentfromclaw: {e}"))?;

                let workspace_path = std::path::Path::new(&cmd.workspace_path);
                if !workspace_path.exists() {
                    return Err(format!("importagentfromclaw: path does not exist: {}", cmd.workspace_path));
                }

                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;

                // Detect provider from .claude/settings.json if present
                let mut provider = "claude".to_string();
                let settings_path = workspace_path.join(".claude").join("settings.json");
                if settings_path.exists() {
                    if let Ok(settings_str) = std::fs::read_to_string(&settings_path) {
                        if let Ok(settings) = serde_json::from_str::<serde_json::Value>(&settings_str) {
                            if let Some(p) = settings.get("provider").and_then(|v| v.as_str()) {
                                provider = p.to_string();
                            }
                        }
                    }
                }

                // Create the agent — slug is empty, agent_def_insert will
                // auto-derive from agent_name and mutate the struct
                // so the resolved slug is returned to the frontend.
                let mut agent = AgentDefinition {
                    id: uuid::Uuid::new_v4().to_string(),
                    slug: String::new(),
                    name: cmd.agent_name.clone(),
                    icon: "\u{2726}".to_string(),
                    provider,
                    description: format!("Imported from {}", cmd.workspace_path),
                    working_directory: cmd.workspace_path.clone(),
                    shell: String::new(),
                    provider_flags: String::new(),
                    auto_start: 0,
                    restart_on_crash: 0,
                    idle_timeout_minutes: 0,
                    created_at: now,
                    agent_type: "standalone".to_string(),
                    environment: String::new(),
                    agent_bus_id: String::new(),
                    is_seeded: 0,
                    accounts: String::new(),
                    parent_id: String::new(),
                    branch_label: String::new(),
                    updated_at: now,
                    user_hidden: 0,
                    container_image: String::new(),
                    container_volumes: "[]".to_string(),
                    container_name: String::new(),
                    use_ambient_login: 0,
                    model_vendor_base_url: String::new(),
                };
                wstore.agent_def_insert(&mut agent).map_err(|e| format!("importagentfromclaw: {e}"))?;

                // Read CLAUDE.md → agentmd content
                let claude_md_path = workspace_path.join("CLAUDE.md");
                if claude_md_path.exists() {
                    if let Ok(content) = std::fs::read_to_string(&claude_md_path) {
                        let fc = AgentContent {
                            agent_id: agent.id.clone(),
                            content_type: "agentmd".to_string(),
                            content,
                            updated_at: now,
                        };
                        let _ = wstore.agent_content_set(&fc);
                    }
                }

                // Read .mcp.json → mcp content
                let mcp_path = workspace_path.join(".mcp.json");
                if mcp_path.exists() {
                    if let Ok(content) = std::fs::read_to_string(&mcp_path) {
                        let fc = AgentContent {
                            agent_id: agent.id.clone(),
                            content_type: "mcp".to_string(),
                            content,
                            updated_at: now,
                        };
                        let _ = wstore.agent_content_set(&fc);
                    }
                }

                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&agent).unwrap_or_default()))
            })
        }),
    );

    // reseedagents → delete all seeded agents and re-run seed from manifest
    let wstore_rsfa = state.wstore.clone();
    let broker_rsfa = state.broker.clone();
    engine.register_handler(
        COMMAND_RESEED_AGENTS,
        Box::new(move |_data, _ctx| {
            let wstore = wstore_rsfa.clone();
            let broker = broker_rsfa.clone();
            Box::pin(async move {
                // Delete all previously seeded agents (cascade deletes content, skills, history)
                let deleted = wstore.agent_def_delete_seeded()
                    .map_err(|e| format!("reseedagents: delete seeded: {e}"))?;

                // Re-run seed
                let report = crate::backend::agent_seed::seed_agents(&wstore)
                    .map_err(|e| format!("reseedagents: seed: {e}"))?;

                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(json!({
                    "deleted": deleted,
                    "created": report.created,
                    "skipped": report.skipped,
                })))
            })
        }),
    );

    // importagents — bulk import from JSON export format
    let wstore_ifa = state.wstore.clone();
    let broker_ifa = state.broker.clone();
    engine.register_handler(
        COMMAND_IMPORT_AGENTS,
        Box::new(move |data, _ctx| {
            let wstore = wstore_ifa.clone();
            let broker = broker_ifa.clone();
            Box::pin(async move {
                let cmd: CommandImportAgentDefinitionsData = serde_json::from_value(data)
                    .map_err(|e| format!("importagents: {e}"))?;

                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;

                let mut imported: Vec<String> = Vec::new();
                let mut skipped: Vec<String> = Vec::new();
                let mut failed: Vec<String> = Vec::new();

                for agent_import in cmd.agents {
                    // Check for existing agent by slug (id field from export)
                    let existing = wstore.agent_def_list()
                        .unwrap_or_default()
                        .into_iter()
                        .any(|a| a.slug == agent_import.id);

                    if existing {
                        skipped.push(agent_import.name.clone());
                        continue;
                    }

                    let mut agent = AgentDefinition {
                        id: uuid::Uuid::new_v4().to_string(),
                        slug: agent_import.id.clone(),
                        name: agent_import.name.clone(),
                        icon: agent_import.icon.clone(),
                        provider: agent_import.provider.clone(),
                        description: agent_import.description.clone(),
                        working_directory: agent_import.working_directory.clone(),
                        shell: agent_import.shell.clone(),
                        provider_flags: String::new(),
                        auto_start: 0,
                        restart_on_crash: if agent_import.restart_on_crash { 1 } else { 0 },
                        idle_timeout_minutes: 0,
                        created_at: now,
                        agent_type: agent_import.agent_type.clone(),
                        environment: agent_import.environment.clone(),
                        agent_bus_id: agent_import.agent_bus_id.clone(),
                        is_seeded: 0,
                        accounts: String::new(),
                        parent_id: String::new(),
                        branch_label: String::new(),
                        updated_at: now,
                        user_hidden: 0,
                        container_image: String::new(),
                        container_volumes: "[]".to_string(),
                        container_name: String::new(),
                        use_ambient_login: 0,
                        model_vendor_base_url: String::new(),
                    };

                    if let Err(e) = wstore.agent_def_insert(&mut agent) {
                        failed.push(format!("{}: {e}", agent_import.name));
                        continue;
                    }

                    // Insert content types
                    let mut content_ok = true;
                    for (content_type, content) in &agent_import.content {
                        let fc = AgentContent {
                            agent_id: agent.id.clone(),
                            content_type: content_type.clone(),
                            content: content.clone(),
                            updated_at: now,
                        };
                        if let Err(e) = wstore.agent_content_set(&fc) {
                            tracing::warn!("import: failed to set content for agent {}: {e}", agent.id);
                            content_ok = false;
                        }
                    }

                    // Insert skills
                    let mut skills_ok = true;
                    for skill_import in &agent_import.skills {
                        let skill = AgentSkill {
                            id: uuid::Uuid::new_v4().to_string(),
                            agent_id: agent.id.clone(),
                            name: skill_import.name.clone(),
                            trigger: skill_import.trigger.clone(),
                            skill_type: skill_import.skill_type.clone(),
                            description: skill_import.description.clone(),
                            content: skill_import.content.clone(),
                            created_at: now,
                        };
                        if let Err(e) = wstore.agent_skill_insert(&skill) {
                            tracing::warn!("import: failed to insert skill '{}' for agent {}: {e}", skill.name, agent.id);
                            skills_ok = false;
                        }
                    }

                    if content_ok && skills_ok {
                        imported.push(agent_import.name.clone());
                    } else {
                        failed.push(agent_import.name.clone());
                    }
                }

                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });

                let result = ImportAgentDefinitionsResult { imported, skipped, failed };
                Ok(Some(serde_json::to_value(&result).unwrap_or_default()))
            })
        }),
    );

    // exportagents — export all agent definitions with content and skills
    let wstore_efa = state.wstore.clone();
    engine.register_handler(
        COMMAND_EXPORT_AGENTS,
        Box::new(move |_data, _ctx| {
            let wstore = wstore_efa.clone();
            Box::pin(async move {
                let agents = wstore.agent_def_list()
                    .map_err(|e| format!("exportagents: list: {e}"))?;

                let mut agent_exports: Vec<AgentDefinitionExport> = Vec::new();

                for agent in agents {
                    let content_map = wstore.agent_content_get_all(&agent.id)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|fc| (fc.content_type, fc.content))
                        .collect::<std::collections::HashMap<String, String>>();

                    let skills = wstore.agent_skill_list(&agent.id)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|s| AgentSkillExport {
                            name: s.name,
                            trigger: s.trigger,
                            skill_type: s.skill_type,
                            description: s.description,
                            content: s.content,
                        })
                        .collect::<Vec<_>>();

                    agent_exports.push(AgentDefinitionExport {
                        id: agent.slug.clone(),
                        name: agent.name,
                        icon: agent.icon,
                        description: agent.description,
                        provider: agent.provider,
                        shell: agent.shell,
                        working_directory: agent.working_directory,
                        agent_bus_id: agent.agent_bus_id,
                        agent_type: agent.agent_type,
                        environment: agent.environment,
                        restart_on_crash: agent.restart_on_crash != 0,
                        content: content_map,
                        skills,
                    });
                }

                let exported_at = Utc::now().to_rfc3339();

                let result = ExportAgentDefinitionsResult {
                    version: 4,
                    exported_at,
                    source: "agentmux-export".to_string(),
                    agents: agent_exports,
                };
                Ok(Some(serde_json::to_value(&result).unwrap_or_default()))
            })
        }),
    );

}
