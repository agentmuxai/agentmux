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
    COMMAND_LIST_AGENT_SKILLS, COMMAND_CREATE_AGENT_SKILL, COMMAND_UPDATE_AGENT_SKILL,
    COMMAND_DELETE_AGENT_SKILL,
    COMMAND_APPEND_AGENT_HISTORY, COMMAND_LIST_AGENT_HISTORY, COMMAND_SEARCH_AGENT_HISTORY,
    COMMAND_IMPORT_AGENT_FROM_CLAW, COMMAND_IMPORT_AGENTS, COMMAND_EXPORT_AGENTS,
    COMMAND_RESEED_AGENTS,
    // Two-tier picker — Phase 1 (SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md)
    COMMAND_AGENT_DEF_CREATE_FROM_TEMPLATE,
    COMMAND_CONTAINER_RUNTIME_AVAILABLE,
    CommandAgentDefCreateFromTemplateData, AgentDefCreateFromTemplateResult,
    CommandListAgentDefinitionsData,
    // Two-tier picker — Phase 2 (SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md
    // Q2 Decision Y: hide templates).
    COMMAND_AGENT_DEF_HIDE, COMMAND_AGENT_DEF_UNHIDE,
    COMMAND_AGENT_DEF_LIST_HIDDEN_TEMPLATES,
    CommandAgentDefHideData, AgentDefHideResult,
    CommandCreateAgentDefinitionData, CommandUpdateAgentDefinitionData, CommandDeleteAgentDefinitionData,
    CommandGetAgentContentData, CommandSetAgentContentData, CommandGetAllAgentContentData,
    CommandListAgentSkillsData, CommandCreateAgentSkillData, CommandUpdateAgentSkillData,
    CommandDeleteAgentSkillData,
    CommandAppendAgentHistoryData, CommandListAgentHistoryData, CommandSearchAgentHistoryData,
    CommandImportAgentFromClawData,
    CommandImportAgentDefinitionsData, ImportAgentDefinitionsResult,
    ExportAgentDefinitionsResult, AgentDefinitionExport, AgentSkillExport,
    // v6 identity / instance / fork
    COMMAND_LIST_IDENTITY_ACCOUNTS, COMMAND_GET_IDENTITY_ACCOUNT,
    COMMAND_UPSERT_IDENTITY_ACCOUNT, COMMAND_DELETE_IDENTITY_ACCOUNT,
    COMMAND_ACCOUNT_KEY_VERIFY,
    COMMAND_ACCOUNT_OAUTH_START, COMMAND_ACCOUNT_OAUTH_POLL, COMMAND_ACCOUNT_OAUTH_CANCEL,
    COMMAND_LINK_AGENT_IDENTITY, COMMAND_UNLINK_AGENT_IDENTITY,
    COMMAND_LIST_AGENT_IDENTITIES,
    COMMAND_LIST_AGENT_INSTANCES, COMMAND_GET_AGENT_INSTANCE,
    COMMAND_CREATE_AGENT_INSTANCE, COMMAND_UPDATE_AGENT_INSTANCE,
    COMMAND_DELETE_AGENT_INSTANCE,
    COMMAND_LIST_NAMED_AGENTS, COMMAND_HIDE_NAMED_AGENT,
    CommandListNamedAgentsData, CommandHideNamedAgentData,
    NamedAgentRow,
    COMMAND_LIST_RECENT_SESSIONS, CommandListRecentSessionsData,
    RecentSessionRow,
    // Option E (PR 1 of 2) — agent-anchored session zones.
    COMMAND_AGENT_SESSION_READ, COMMAND_AGENT_SESSION_WRITE_STATE,
    COMMAND_AGENT_SESSION_APPEND_OUTPUT, COMMAND_AGENT_SESSION_ARCHIVE,
    COMMAND_AGENT_SESSION_LIST_ARCHIVES,
    CommandAgentSessionReadData, AgentSessionReadResult,
    CommandAgentSessionWriteStateData, AgentSessionWriteStateResult,
    CommandAgentSessionAppendOutputData, AgentSessionAppendOutputResult,
    CommandAgentSessionArchiveData, AgentSessionArchiveResult,
    CommandAgentSessionListArchivesData, AgentArchiveRow,
    COMMAND_FORK_AGENT_DEFINITION,
    COMMAND_FORK_AGENT_DEFINITION_SUGGEST,
    CommandForkAgentDefinitionSuggestData, ForkAgentDefinitionSuggestResult,
    CommandListIdentityAccountsData, CommandGetIdentityAccountData,
    CommandDeleteIdentityAccountData,
    CommandLinkAgentIdentityData, CommandUnlinkAgentIdentityData,
    CommandListAgentIdentitiesData,
    CommandListAgentInstancesData, CommandGetAgentInstanceData,
    CommandCreateAgentInstanceData, CommandUpdateAgentInstanceData,
    CommandDeleteAgentInstanceData,
    CommandForkAgentDefinitionData,
    // v7 Identity bundles + Memory
    COMMAND_LIST_IDENTITY_BUNDLES, COMMAND_GET_IDENTITY_BUNDLE,
    COMMAND_UPSERT_IDENTITY_BUNDLE, COMMAND_DELETE_IDENTITY_BUNDLE,
    COMMAND_BIND_IDENTITY_ACCOUNT, COMMAND_UNBIND_IDENTITY_ACCOUNT,
    COMMAND_LIST_IDENTITY_BINDINGS,
    COMMAND_LIST_MEMORIES, COMMAND_GET_MEMORY,
    COMMAND_UPSERT_MEMORY, COMMAND_DELETE_MEMORY, COMMAND_REORDER_GLOBAL_BRAIN,
    CommandGetIdentityBundleData, CommandDeleteIdentityBundleData,
    CommandBindIdentityAccountData, CommandUnbindIdentityAccountData,
    CommandListIdentityBindingsData,
    CommandGetMemoryData, CommandDeleteMemoryData, CommandReorderGlobalBrainData,
};
use crate::backend::storage::{AgentDefinition, AgentContent, AgentSkill};
use crate::backend::storage::store::{
    AgentInstance, Identity, IdentityAccount, InstanceStatus, Memory, SecretRef,
};
use crate::backend::rpc_types::{
    COMMAND_SUBPROCESS_SPAWN, COMMAND_AGENT_INPUT, COMMAND_AGENT_STOP,
    CommandSubprocessSpawnData, CommandAgentInputData, CommandAgentStopData,
};
use crate::backend::obj::Block;
use crate::backend::blockcontroller;

use super::super::AppState;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // appendagenthistory → append a history entry, broadcast agenthistory:changed
    let wstore_afh = state.wstore.clone();
    let broker_afh = state.broker.clone();
    engine.register_handler(
        COMMAND_APPEND_AGENT_HISTORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore_afh.clone();
            let broker = broker_afh.clone();
            Box::pin(async move {
                let cmd: CommandAppendAgentHistoryData = serde_json::from_value(data)
                    .map_err(|e| format!("appendagenthistory: {e}"))?;
                let entry = wstore.agent_history_append(&cmd.agent_id, &cmd.entry)
                    .map_err(|e| format!("appendagenthistory: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agenthistory:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&entry).unwrap_or_default()))
            })
        }),
    );

    // listagenthistory → return history entries with pagination
    let wstore_lfh = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_AGENT_HISTORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore_lfh.clone();
            Box::pin(async move {
                let cmd: CommandListAgentHistoryData = serde_json::from_value(data)
                    .map_err(|e| format!("listagenthistory: {e}"))?;
                let entries = wstore.agent_history_list(
                    &cmd.agent_id,
                    cmd.session_date.as_deref(),
                    cmd.limit,
                    cmd.offset,
                ).map_err(|e| format!("listagenthistory: {e}"))?;
                Ok(Some(serde_json::to_value(&entries).unwrap_or_default()))
            })
        }),
    );

    // searchagenthistory → search history entries by query
    let wstore_sfh = state.wstore.clone();
    engine.register_handler(
        COMMAND_SEARCH_AGENT_HISTORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore_sfh.clone();
            Box::pin(async move {
                let cmd: CommandSearchAgentHistoryData = serde_json::from_value(data)
                    .map_err(|e| format!("searchagenthistory: {e}"))?;
                let entries = wstore.agent_history_search(&cmd.agent_id, &cmd.query, cmd.limit)
                    .map_err(|e| format!("searchagenthistory: {e}"))?;
                Ok(Some(serde_json::to_value(&entries).unwrap_or_default()))
            })
        }),
    );

}
