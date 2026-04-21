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
    // v6 identity / instance / fork
    COMMAND_LIST_IDENTITY_ACCOUNTS, COMMAND_GET_IDENTITY_ACCOUNT,
    COMMAND_UPSERT_IDENTITY_ACCOUNT, COMMAND_DELETE_IDENTITY_ACCOUNT,
    COMMAND_LINK_AGENT_IDENTITY, COMMAND_UNLINK_AGENT_IDENTITY,
    COMMAND_LIST_AGENT_IDENTITIES,
    COMMAND_LIST_AGENT_INSTANCES, COMMAND_GET_AGENT_INSTANCE,
    COMMAND_CREATE_AGENT_INSTANCE, COMMAND_UPDATE_AGENT_INSTANCE,
    COMMAND_DELETE_AGENT_INSTANCE,
    COMMAND_FORK_AGENT_DEFINITION,
    CommandListIdentityAccountsData, CommandGetIdentityAccountData,
    CommandDeleteIdentityAccountData,
    CommandLinkAgentIdentityData, CommandUnlinkAgentIdentityData,
    CommandListAgentIdentitiesData,
    CommandListAgentInstancesData, CommandGetAgentInstanceData,
    CommandCreateAgentInstanceData, CommandUpdateAgentInstanceData,
    CommandDeleteAgentInstanceData,
    CommandForkAgentDefinitionData,
};
use crate::backend::storage::{ForgeAgent, ForgeContent, ForgeSkill};
use crate::backend::storage::wstore::{AgentInstance, IdentityAccount, InstanceStatus};

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
                // slug is empty here — forge_insert auto-derives it
                // from name AND collision-resolves AND mutates the
                // struct so we serialize the resolved value back to
                // the frontend (not "").
                let mut agent = ForgeAgent {
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
                };
                wstore.forge_insert(&mut agent).map_err(|e| format!("createforgeagent: {e}"))?;
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
                // slug is preserved from the existing row — it's
                // immutable after creation. The update path never
                // accepts a new slug from the client.
                let agent = ForgeAgent {
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
                    // that only update name/icon/etc. (ForgeForm, AgentPicker rename)
                    // don't carry accounts, so falling back to old.accounts prevents
                    // silently wiping saved assignments.
                    accounts: if cmd.accounts.is_empty() { old.accounts.clone() } else { cmd.accounts },
                    // parent_id + branch_label describe provenance and
                    // are immutable post-insert (forks are separate rows,
                    // not in-place edits).
                    parent_id: old.parent_id.clone(),
                    branch_label: old.branch_label.clone(),
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

                // Create the agent — slug is empty, forge_insert will
                // auto-derive from agent_name and mutate the struct
                // so the resolved slug is returned to the frontend.
                let mut agent = ForgeAgent {
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
                };
                wstore.forge_insert(&mut agent).map_err(|e| format!("importforgefromclaw: {e}"))?;

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

    register_v6_handlers(engine, state);
}

/// v6 handlers — identity accounts, agent instances, definition branching.
/// See specs/SPEC_FORGE_IDENTITY_AGENT_INSTANCES_IMPL_2026_04_20.md §Phase 3.
fn register_v6_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // ---- Identity account CRUD ----

    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_IDENTITY_ACCOUNTS,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: CommandListIdentityAccountsData =
                    serde_json::from_value(data).unwrap_or_default();
                let accounts = wstore
                    .identity_list(cmd.provider.as_deref())
                    .map_err(|e| format!("listidentityaccounts: {e}"))?;
                Ok(Some(serde_json::to_value(&accounts).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_GET_IDENTITY_ACCOUNT,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: CommandGetIdentityAccountData =
                    serde_json::from_value(data).map_err(|e| format!("getidentityaccount: {e}"))?;
                match wstore
                    .identity_get(&cmd.id)
                    .map_err(|e| format!("getidentityaccount: {e}"))?
                {
                    Some(a) => Ok(Some(serde_json::to_value(&a).unwrap_or_default())),
                    None => Err(format!("getidentityaccount: not found id={}", cmd.id)),
                }
            })
        }),
    );

    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_UPSERT_IDENTITY_ACCOUNT,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                // Accept the full IdentityAccount payload. Missing `id` → mint
                // a fresh UUID; `created_at` and `updated_at` are server-set
                // so callers don't have to know the current time.
                let mut account: IdentityAccount = serde_json::from_value(data)
                    .map_err(|e| format!("upsertidentityaccount: {e}"))?;
                if account.id.is_empty() {
                    account.id = uuid::Uuid::new_v4().to_string();
                }
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                if account.created_at == 0 {
                    account.created_at = now;
                }
                account.updated_at = now;
                wstore
                    .identity_upsert(&account)
                    .map_err(|e| format!("upsertidentityaccount: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "identityaccounts:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&account).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_DELETE_IDENTITY_ACCOUNT,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandDeleteIdentityAccountData = serde_json::from_value(data)
                    .map_err(|e| format!("deleteidentityaccount: {e}"))?;
                let deleted = wstore
                    .identity_delete(&cmd.id)
                    .map_err(|e| format!("deleteidentityaccount: {e}"))?;
                if deleted {
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: "identityaccounts:changed".to_string(),
                        scopes: vec![],
                        sender: String::new(),
                        persist: 0,
                        data: None,
                    });
                }
                Ok(Some(json!({ "deleted": deleted })))
            })
        }),
    );

    // ---- Agent ↔ Identity junction ----

    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_LINK_AGENT_IDENTITY,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandLinkAgentIdentityData = serde_json::from_value(data)
                    .map_err(|e| format!("linkagentidentity: {e}"))?;
                wstore
                    .agent_identity_link(&cmd.agent_id, &cmd.account_id, &cmd.provider)
                    .map_err(|e| format!("linkagentidentity: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: format!("agentidentities:changed:{}", cmd.agent_id),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(None)
            })
        }),
    );

    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_UNLINK_AGENT_IDENTITY,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandUnlinkAgentIdentityData = serde_json::from_value(data)
                    .map_err(|e| format!("unlinkagentidentity: {e}"))?;
                let removed = wstore
                    .agent_identity_unlink(&cmd.agent_id, &cmd.provider)
                    .map_err(|e| format!("unlinkagentidentity: {e}"))?;
                if removed {
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: format!("agentidentities:changed:{}", cmd.agent_id),
                        scopes: vec![],
                        sender: String::new(),
                        persist: 0,
                        data: None,
                    });
                }
                Ok(Some(json!({ "unlinked": removed })))
            })
        }),
    );

    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_AGENT_IDENTITIES,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: CommandListAgentIdentitiesData = serde_json::from_value(data)
                    .map_err(|e| format!("listagentidentities: {e}"))?;
                let rows = wstore
                    .agent_identity_list_for_agent(&cmd.agent_id)
                    .map_err(|e| format!("listagentidentities: {e}"))?;
                Ok(Some(serde_json::to_value(&rows).unwrap_or_default()))
            })
        }),
    );

    // ---- Agent instance CRUD ----

    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_AGENT_INSTANCES,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: CommandListAgentInstancesData =
                    serde_json::from_value(data).unwrap_or_default();
                let rows = wstore
                    .instance_list(cmd.definition_id.as_deref(), cmd.status.as_deref())
                    .map_err(|e| format!("listagentinstances: {e}"))?;
                Ok(Some(serde_json::to_value(&rows).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_GET_AGENT_INSTANCE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: CommandGetAgentInstanceData = serde_json::from_value(data)
                    .map_err(|e| format!("getagentinstance: {e}"))?;
                match wstore
                    .instance_get(&cmd.id)
                    .map_err(|e| format!("getagentinstance: {e}"))?
                {
                    Some(i) => Ok(Some(serde_json::to_value(&i).unwrap_or_default())),
                    None => Err(format!("getagentinstance: not found id={}", cmd.id)),
                }
            })
        }),
    );

    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_CREATE_AGENT_INSTANCE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandCreateAgentInstanceData = serde_json::from_value(data)
                    .map_err(|e| format!("createagentinstance: {e}"))?;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let inst = AgentInstance {
                    id: uuid::Uuid::new_v4().to_string(),
                    definition_id: cmd.definition_id,
                    parent_instance_id: cmd.parent_instance_id,
                    block_id: cmd.block_id,
                    session_id: String::new(),
                    status: InstanceStatus::Running.as_str().to_string(),
                    github_context: String::new(),
                    started_at: now,
                    ended_at: 0,
                    created_at: now,
                };
                wstore
                    .instance_create(&inst)
                    .map_err(|e| format!("createagentinstance: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: format!("agentinstances:changed:{}", inst.definition_id),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&inst).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_UPDATE_AGENT_INSTANCE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandUpdateAgentInstanceData = serde_json::from_value(data)
                    .map_err(|e| format!("updateagentinstance: {e}"))?;
                let existing = wstore
                    .instance_get(&cmd.id)
                    .map_err(|e| format!("updateagentinstance: {e}"))?
                    .ok_or_else(|| format!("updateagentinstance: not found id={}", cmd.id))?;
                let merged = AgentInstance {
                    id: existing.id.clone(),
                    definition_id: existing.definition_id.clone(),
                    parent_instance_id: existing.parent_instance_id.clone(),
                    block_id: cmd.block_id.unwrap_or(existing.block_id),
                    session_id: cmd.session_id.unwrap_or(existing.session_id),
                    status: cmd.status.unwrap_or(existing.status),
                    github_context: cmd.github_context.unwrap_or(existing.github_context),
                    started_at: existing.started_at,
                    ended_at: cmd.ended_at.unwrap_or(existing.ended_at),
                    created_at: existing.created_at,
                };
                wstore
                    .instance_update(&merged)
                    .map_err(|e| format!("updateagentinstance: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: format!("agentinstances:changed:{}", merged.definition_id),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&merged).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_DELETE_AGENT_INSTANCE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandDeleteAgentInstanceData = serde_json::from_value(data)
                    .map_err(|e| format!("deleteagentinstance: {e}"))?;
                // Read the row first so we can emit a scoped event after.
                let definition_id = wstore
                    .instance_get(&cmd.id)
                    .map_err(|e| format!("deleteagentinstance: {e}"))?
                    .map(|i| i.definition_id);
                let deleted = wstore
                    .instance_delete(&cmd.id)
                    .map_err(|e| format!("deleteagentinstance: {e}"))?;
                if let Some(def_id) = definition_id.filter(|_| deleted) {
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: format!("agentinstances:changed:{}", def_id),
                        scopes: vec![],
                        sender: String::new(),
                        persist: 0,
                        data: None,
                    });
                }
                Ok(Some(json!({ "deleted": deleted })))
            })
        }),
    );

    // ---- Definition fork ----

    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_FORK_AGENT_DEFINITION,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandForkAgentDefinitionData = serde_json::from_value(data)
                    .map_err(|e| format!("forkagentdefinition: {e}"))?;

                // Find the source definition by id.
                let source = wstore
                    .forge_list()
                    .map_err(|e| format!("forkagentdefinition: {e}"))?
                    .into_iter()
                    .find(|a| a.id == cmd.source_id)
                    .ok_or_else(|| format!("forkagentdefinition: source not found: {}", cmd.source_id))?;

                // Build a new definition that shares the source's content but
                // has a fresh id/slug and records the lineage. Seed-bit is
                // cleared — forks are always user-owned, not built-in.
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let branch_slug_part = if cmd.branch_label.is_empty() {
                    "fork".to_string()
                } else {
                    crate::backend::storage::wstore::derive_slug(&cmd.branch_label)
                };
                let mut fork = ForgeAgent {
                    id: uuid::Uuid::new_v4().to_string(),
                    // Empty slug → forge_insert derives + resolves collisions.
                    slug: format!("{}-{}", source.slug, branch_slug_part),
                    name: if cmd.branch_label.is_empty() {
                        format!("{} (fork)", source.name)
                    } else {
                        format!("{} [{}]", source.name, cmd.branch_label)
                    },
                    icon: source.icon.clone(),
                    provider: source.provider.clone(),
                    description: source.description.clone(),
                    working_directory: String::new(), // force re-resolve via agentmuxHome()
                    shell: source.shell.clone(),
                    provider_flags: source.provider_flags.clone(),
                    auto_start: 0, // forks don't auto-start; explicit launch only
                    restart_on_crash: source.restart_on_crash,
                    idle_timeout_minutes: source.idle_timeout_minutes,
                    created_at: now,
                    agent_type: source.agent_type.clone(),
                    environment: source.environment.clone(),
                    agent_bus_id: String::new(), // fresh bus id so broadcasts don't cross
                    is_seeded: 0,
                    accounts: String::new(),
                    parent_id: source.id.clone(),
                    branch_label: cmd.branch_label.clone(),
                };
                wstore
                    .forge_insert(&mut fork)
                    .map_err(|e| format!("forkagentdefinition: {e}"))?;

                // Deep-copy content blobs + skills from source. Cascade foreign
                // keys on the source are unaffected — we're copying out, not
                // moving.
                let source_contents = wstore
                    .forge_get_all_content(&source.id)
                    .map_err(|e| format!("forkagentdefinition content: {e}"))?;
                for c in source_contents {
                    let new_content = ForgeContent {
                        agent_id: fork.id.clone(),
                        content_type: c.content_type,
                        content: c.content,
                        updated_at: now,
                    };
                    wstore
                        .forge_set_content(&new_content)
                        .map_err(|e| format!("forkagentdefinition content: {e}"))?;
                }
                let source_skills = wstore
                    .forge_list_skills(&source.id)
                    .map_err(|e| format!("forkagentdefinition skills: {e}"))?;
                for s in source_skills {
                    let new_skill = ForgeSkill {
                        id: uuid::Uuid::new_v4().to_string(),
                        agent_id: fork.id.clone(),
                        name: s.name,
                        trigger: s.trigger,
                        skill_type: s.skill_type,
                        description: s.description,
                        content: s.content,
                        created_at: now,
                    };
                    wstore
                        .forge_insert_skill(&new_skill)
                        .map_err(|e| format!("forkagentdefinition skill: {e}"))?;
                }

                broker.publish(crate::backend::wps::WaveEvent {
                    event: "forgeagents:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });

                Ok(Some(serde_json::to_value(&fork).unwrap_or_default()))
            })
        }),
    );
}
