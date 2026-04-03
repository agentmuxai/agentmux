use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    COMMAND_LIST_FORGE_AGENTS, COMMAND_CREATE_FORGE_AGENT, COMMAND_UPDATE_FORGE_AGENT,
    COMMAND_DELETE_FORGE_AGENT, COMMAND_GET_FORGE_CONTENT, COMMAND_SET_FORGE_CONTENT,
    COMMAND_GET_ALL_FORGE_CONTENT,
    COMMAND_LIST_FORGE_SKILLS, COMMAND_CREATE_FORGE_SKILL, COMMAND_UPDATE_FORGE_SKILL,
    COMMAND_DELETE_FORGE_SKILL,
    COMMAND_APPEND_FORGE_HISTORY, COMMAND_LIST_FORGE_HISTORY, COMMAND_SEARCH_FORGE_HISTORY,
    COMMAND_IMPORT_FORGE_FROM_CLAW,
    COMMAND_RESEED_FORGE_AGENTS,
    CommandCreateForgeAgentData, CommandUpdateForgeAgentData, CommandDeleteForgeAgentData,
    CommandGetForgeContentData, CommandSetForgeContentData, CommandGetAllForgeContentData,
    CommandListForgeSkillsData, CommandCreateForgeSkillData, CommandUpdateForgeSkillData,
    CommandDeleteForgeSkillData,
    CommandAppendForgeHistoryData, CommandListForgeHistoryData, CommandSearchForgeHistoryData,
    CommandImportForgeFromClawData,
};
use crate::backend::storage::{ForgeAgent, ForgeContent, ForgeSkill};

use super::AppState;

pub fn register_forge_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // listforgeagents → return all forge agents
    let wstore_lfa = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_FORGE_AGENTS,
        Box::new(move |_data, _ctx| {
            let wstore = wstore_lfa.clone();
            Box::pin(async move {
                let agents = wstore.forge_list().map_err(|e| format!("listforgeagents: {e}"))?;
                Ok(Some(serde_json::to_value(&agents).unwrap_or_default()))
            })
        }),
    );

    // createforgeagent → insert new agent, broadcast forgeagents:changed
    let wstore_cfa = state.wstore.clone();
    let broker_cfa = state.broker.clone();
    engine.register_handler(
        COMMAND_CREATE_FORGE_AGENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_cfa.clone();
            let broker = broker_cfa.clone();
            Box::pin(async move {
                let cmd: CommandCreateForgeAgentData = serde_json::from_value(data)
                    .map_err(|e| format!("createforgeagent: {e}"))?;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                let agent = ForgeAgent {
                    id: uuid::Uuid::new_v4().to_string(),
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
                };
                wstore.forge_insert(&agent).map_err(|e| format!("createforgeagent: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "forgeagents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&agent).unwrap_or_default()))
            })
        }),
    );

    // updateforgeagent → update existing agent, broadcast forgeagents:changed
    let wstore_ufa = state.wstore.clone();
    let broker_ufa = state.broker.clone();
    engine.register_handler(
        COMMAND_UPDATE_FORGE_AGENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_ufa.clone();
            let broker = broker_ufa.clone();
            Box::pin(async move {
                let cmd: CommandUpdateForgeAgentData = serde_json::from_value(data)
                    .map_err(|e| format!("updateforgeagent: {e}"))?;
                // Fetch existing to preserve created_at
                let existing = wstore.forge_list().map_err(|e| format!("updateforgeagent: {e}"))?;
                let old = existing.iter().find(|a| a.id == cmd.id)
                    .ok_or_else(|| format!("updateforgeagent: agent {} not found", cmd.id))?;
                let agent = ForgeAgent {
                    id: cmd.id,
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
                };
                let found = wstore.forge_update(&agent).map_err(|e| format!("updateforgeagent: {e}"))?;
                if !found {
                    return Err(format!("updateforgeagent: agent {} not found", agent.id));
                }
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "forgeagents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&agent).unwrap_or_default()))
            })
        }),
    );

    // deleteforgeagent → delete agent by id, broadcast forgeagents:changed
    let wstore_dfa = state.wstore.clone();
    let broker_dfa = state.broker.clone();
    engine.register_handler(
        COMMAND_DELETE_FORGE_AGENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_dfa.clone();
            let broker = broker_dfa.clone();
            Box::pin(async move {
                let cmd: CommandDeleteForgeAgentData = serde_json::from_value(data)
                    .map_err(|e| format!("deleteforgeagent: {e}"))?;
                wstore.forge_delete(&cmd.id).map_err(|e| format!("deleteforgeagent: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "forgeagents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(None)
            })
        }),
    );

    // getforgecontent → return a single content blob for an agent
    let wstore_gfc = state.wstore.clone();
    engine.register_handler(
        COMMAND_GET_FORGE_CONTENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_gfc.clone();
            Box::pin(async move {
                let cmd: CommandGetForgeContentData = serde_json::from_value(data)
                    .map_err(|e| format!("getforgecontent: {e}"))?;
                let content = wstore.forge_get_content(&cmd.agent_id, &cmd.content_type)
                    .map_err(|e| format!("getforgecontent: {e}"))?;
                Ok(content.map(|c| serde_json::to_value(&c).unwrap_or_default()))
            })
        }),
    );

    // setforgecontent → upsert a content blob, broadcast forgecontent:changed
    let wstore_sfc = state.wstore.clone();
    let broker_sfc = state.broker.clone();
    engine.register_handler(
        COMMAND_SET_FORGE_CONTENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_sfc.clone();
            let broker = broker_sfc.clone();
            Box::pin(async move {
                let cmd: CommandSetForgeContentData = serde_json::from_value(data)
                    .map_err(|e| format!("setforgecontent: {e}"))?;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                let content = ForgeContent {
                    agent_id: cmd.agent_id,
                    content_type: cmd.content_type,
                    content: cmd.content,
                    updated_at: now,
                };
                wstore.forge_set_content(&content).map_err(|e| format!("setforgecontent: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "forgecontent:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&content).unwrap_or_default()))
            })
        }),
    );

    // getallforgecontent → return all content blobs for an agent
    let wstore_gafc = state.wstore.clone();
    engine.register_handler(
        COMMAND_GET_ALL_FORGE_CONTENT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_gafc.clone();
            Box::pin(async move {
                let cmd: CommandGetAllForgeContentData = serde_json::from_value(data)
                    .map_err(|e| format!("getallforgecontent: {e}"))?;
                let contents = wstore.forge_get_all_content(&cmd.agent_id)
                    .map_err(|e| format!("getallforgecontent: {e}"))?;
                Ok(Some(serde_json::to_value(&contents).unwrap_or_default()))
            })
        }),
    );

    // ── Forge Skills handlers ──────────────────────────────────────────────

    // listforgeskills → return all skills for an agent
    let wstore_lfs = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_FORGE_SKILLS,
        Box::new(move |data, _ctx| {
            let wstore = wstore_lfs.clone();
            Box::pin(async move {
                let cmd: CommandListForgeSkillsData = serde_json::from_value(data)
                    .map_err(|e| format!("listforgeskills: {e}"))?;
                let skills = wstore.forge_list_skills(&cmd.agent_id)
                    .map_err(|e| format!("listforgeskills: {e}"))?;
                Ok(Some(serde_json::to_value(&skills).unwrap_or_default()))
            })
        }),
    );

    // createforgeskill → insert new skill, broadcast forgeskills:changed
    let wstore_cfs = state.wstore.clone();
    let broker_cfs = state.broker.clone();
    engine.register_handler(
        COMMAND_CREATE_FORGE_SKILL,
        Box::new(move |data, _ctx| {
            let wstore = wstore_cfs.clone();
            let broker = broker_cfs.clone();
            Box::pin(async move {
                let cmd: CommandCreateForgeSkillData = serde_json::from_value(data)
                    .map_err(|e| format!("createforgeskill: {e}"))?;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                let skill = ForgeSkill {
                    id: uuid::Uuid::new_v4().to_string(),
                    agent_id: cmd.agent_id,
                    name: cmd.name,
                    trigger: cmd.trigger,
                    skill_type: cmd.skill_type,
                    description: cmd.description,
                    content: cmd.content,
                    created_at: now,
                };
                wstore.forge_insert_skill(&skill).map_err(|e| format!("createforgeskill: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "forgeskills:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&skill).unwrap_or_default()))
            })
        }),
    );

    // updateforgeskill → update existing skill, broadcast forgeskills:changed
    let wstore_ufs = state.wstore.clone();
    let broker_ufs = state.broker.clone();
    engine.register_handler(
        COMMAND_UPDATE_FORGE_SKILL,
        Box::new(move |data, _ctx| {
            let wstore = wstore_ufs.clone();
            let broker = broker_ufs.clone();
            Box::pin(async move {
                let cmd: CommandUpdateForgeSkillData = serde_json::from_value(data)
                    .map_err(|e| format!("updateforgeskill: {e}"))?;
                let existing = wstore.forge_get_skill(&cmd.id)
                    .map_err(|e| format!("updateforgeskill: {e}"))?
                    .ok_or_else(|| format!("updateforgeskill: skill {} not found", cmd.id))?;
                let skill = ForgeSkill {
                    id: cmd.id,
                    agent_id: existing.agent_id,
                    name: cmd.name,
                    trigger: cmd.trigger,
                    skill_type: cmd.skill_type,
                    description: cmd.description,
                    content: cmd.content,
                    created_at: existing.created_at,
                };
                let found = wstore.forge_update_skill(&skill).map_err(|e| format!("updateforgeskill: {e}"))?;
                if !found {
                    return Err(format!("updateforgeskill: skill {} not found", skill.id));
                }
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "forgeskills:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&skill).unwrap_or_default()))
            })
        }),
    );

    // deleteforgeskill → delete skill by id, broadcast forgeskills:changed
    let wstore_dfs = state.wstore.clone();
    let broker_dfs = state.broker.clone();
    engine.register_handler(
        COMMAND_DELETE_FORGE_SKILL,
        Box::new(move |data, _ctx| {
            let wstore = wstore_dfs.clone();
            let broker = broker_dfs.clone();
            Box::pin(async move {
                let cmd: CommandDeleteForgeSkillData = serde_json::from_value(data)
                    .map_err(|e| format!("deleteforgeskill: {e}"))?;
                wstore.forge_delete_skill(&cmd.id).map_err(|e| format!("deleteforgeskill: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "forgeskills:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(None)
            })
        }),
    );

    // ── Forge History handlers ─────────────────────────────────────────────

    // appendforgehistory → append a history entry, broadcast forgehistory:changed
    let wstore_afh = state.wstore.clone();
    let broker_afh = state.broker.clone();
    engine.register_handler(
        COMMAND_APPEND_FORGE_HISTORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore_afh.clone();
            let broker = broker_afh.clone();
            Box::pin(async move {
                let cmd: CommandAppendForgeHistoryData = serde_json::from_value(data)
                    .map_err(|e| format!("appendforgehistory: {e}"))?;
                let entry = wstore.forge_append_history(&cmd.agent_id, &cmd.entry)
                    .map_err(|e| format!("appendforgehistory: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "forgehistory:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&entry).unwrap_or_default()))
            })
        }),
    );

    // listforgehistory → return history entries with pagination
    let wstore_lfh = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_FORGE_HISTORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore_lfh.clone();
            Box::pin(async move {
                let cmd: CommandListForgeHistoryData = serde_json::from_value(data)
                    .map_err(|e| format!("listforgehistory: {e}"))?;
                let entries = wstore.forge_list_history(
                    &cmd.agent_id,
                    cmd.session_date.as_deref(),
                    cmd.limit,
                    cmd.offset,
                ).map_err(|e| format!("listforgehistory: {e}"))?;
                Ok(Some(serde_json::to_value(&entries).unwrap_or_default()))
            })
        }),
    );

    // searchforgehistory → search history entries by query
    let wstore_sfh = state.wstore.clone();
    engine.register_handler(
        COMMAND_SEARCH_FORGE_HISTORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore_sfh.clone();
            Box::pin(async move {
                let cmd: CommandSearchForgeHistoryData = serde_json::from_value(data)
                    .map_err(|e| format!("searchforgehistory: {e}"))?;
                let entries = wstore.forge_search_history(&cmd.agent_id, &cmd.query, cmd.limit)
                    .map_err(|e| format!("searchforgehistory: {e}"))?;
                Ok(Some(serde_json::to_value(&entries).unwrap_or_default()))
            })
        }),
    );

    // ── Forge Import handler ───────────────────────────────────────────────

    // importforgefromclaw → read claw workspace, create agent + content
    let wstore_ifc = state.wstore.clone();
    let broker_ifc = state.broker.clone();
    engine.register_handler(
        COMMAND_IMPORT_FORGE_FROM_CLAW,
        Box::new(move |data, _ctx| {
            let wstore = wstore_ifc.clone();
            let broker = broker_ifc.clone();
            Box::pin(async move {
                let cmd: CommandImportForgeFromClawData = serde_json::from_value(data)
                    .map_err(|e| format!("importforgefromclaw: {e}"))?;

                let workspace_path = std::path::Path::new(&cmd.workspace_path);
                if !workspace_path.exists() {
                    return Err(format!("importforgefromclaw: path does not exist: {}", cmd.workspace_path));
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

                // Create the agent
                let agent = ForgeAgent {
                    id: uuid::Uuid::new_v4().to_string(),
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
                };
                wstore.forge_insert(&agent).map_err(|e| format!("importforgefromclaw: {e}"))?;

                // Read CLAUDE.md → agentmd content
                let claude_md_path = workspace_path.join("CLAUDE.md");
                if claude_md_path.exists() {
                    if let Ok(content) = std::fs::read_to_string(&claude_md_path) {
                        let fc = ForgeContent {
                            agent_id: agent.id.clone(),
                            content_type: "agentmd".to_string(),
                            content,
                            updated_at: now,
                        };
                        let _ = wstore.forge_set_content(&fc);
                    }
                }

                // Read .mcp.json → mcp content
                let mcp_path = workspace_path.join(".mcp.json");
                if mcp_path.exists() {
                    if let Ok(content) = std::fs::read_to_string(&mcp_path) {
                        let fc = ForgeContent {
                            agent_id: agent.id.clone(),
                            content_type: "mcp".to_string(),
                            content,
                            updated_at: now,
                        };
                        let _ = wstore.forge_set_content(&fc);
                    }
                }

                broker.publish(crate::backend::wps::WaveEvent {
                    event: "forgeagents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&agent).unwrap_or_default()))
            })
        }),
    );

    // reseedforgeagents → delete all seeded agents and re-run seed from manifest
    let wstore_rsfa = state.wstore.clone();
    let broker_rsfa = state.broker.clone();
    engine.register_handler(
        COMMAND_RESEED_FORGE_AGENTS,
        Box::new(move |_data, _ctx| {
            let wstore = wstore_rsfa.clone();
            let broker = broker_rsfa.clone();
            Box::pin(async move {
                // Delete all previously seeded agents (cascade deletes content, skills, history)
                let deleted = wstore.forge_delete_seeded()
                    .map_err(|e| format!("reseedforgeagents: delete seeded: {e}"))?;

                // Re-run seed
                let report = crate::backend::forge_seed::seed_forge_agents(&wstore)
                    .map_err(|e| format!("reseedforgeagents: seed: {e}"))?;

                broker.publish(crate::backend::wps::WaveEvent {
                    event: "forgeagents:changed".to_string(),
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
}
