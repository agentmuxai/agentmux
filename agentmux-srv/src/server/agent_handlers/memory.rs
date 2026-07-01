// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

#![allow(unused_imports)]

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
    // ---- Identity bundle CRUD ----

    let wstore = state.id_store.clone();
    engine.register_handler(
        COMMAND_LIST_IDENTITY_BUNDLES,
        Box::new(move |_data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let bundles = wstore
                    .bundle_identity_list()
                    .map_err(|e| format!("listidentitybundles: {e}"))?;
                Ok(Some(serde_json::to_value(&bundles).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.id_store.clone();
    engine.register_handler(
        COMMAND_GET_IDENTITY_BUNDLE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: CommandGetIdentityBundleData = serde_json::from_value(data)
                    .map_err(|e| format!("getidentitybundle: {e}"))?;
                match wstore
                    .bundle_identity_get(&cmd.id)
                    .map_err(|e| format!("getidentitybundle: {e}"))?
                {
                    Some(b) => Ok(Some(serde_json::to_value(&b).unwrap_or_default())),
                    None => Err(format!("getidentitybundle: not found id={}", cmd.id)),
                }
            })
        }),
    );

    let wstore = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_UPSERT_IDENTITY_BUNDLE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let mut bundle: Identity = serde_json::from_value(data)
                    .map_err(|e| format!("upsertidentitybundle: {e}"))?;
                // Guard on BOTH client-supplied is_blank AND id == "blank".
                // Without the id check a caller could send
                // {id:"blank", is_blank:false, name:"evil"} and the
                // ON CONFLICT(id) DO UPDATE path would rename/re-describe
                // the seeded singleton. (reagent P1, 2026-05-08).
                if bundle.is_blank || bundle.id == "blank" {
                    return Err(
                        "upsertidentitybundle: cannot mutate the blank singleton".to_string(),
                    );
                }
                if bundle.id.is_empty() {
                    bundle.id = uuid::Uuid::new_v4().to_string();
                }
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                if bundle.created_at == 0 {
                    bundle.created_at = now;
                }
                bundle.updated_at = now;
                wstore
                    .bundle_identity_upsert(&bundle)
                    .map_err(|e| format!("upsertidentitybundle: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "identitybundles:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&bundle).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_DELETE_IDENTITY_BUNDLE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandDeleteIdentityBundleData = serde_json::from_value(data)
                    .map_err(|e| format!("deleteidentitybundle: {e}"))?;
                let deleted = wstore
                    .bundle_identity_delete(&cmd.id)
                    .map_err(|e| format!("deleteidentitybundle: {e}"))?;
                if deleted {
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: "identitybundles:changed".to_string(),
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

    // ---- Identity bundle bindings (junction with accounts) ----

    let wstore = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_BIND_IDENTITY_ACCOUNT,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandBindIdentityAccountData = serde_json::from_value(data)
                    .map_err(|e| format!("bindidentityaccount: {e}"))?;
                wstore
                    .bundle_identity_bind(&cmd.identity_id, &cmd.provider, &cmd.account_id)
                    .map_err(|e| format!("bindidentityaccount: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: format!("identitybundlebindings:changed:{}", cmd.identity_id),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(None)
            })
        }),
    );

    let wstore = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_UNBIND_IDENTITY_ACCOUNT,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandUnbindIdentityAccountData = serde_json::from_value(data)
                    .map_err(|e| format!("unbindidentityaccount: {e}"))?;
                let removed = wstore
                    .bundle_identity_unbind(&cmd.identity_id, &cmd.provider)
                    .map_err(|e| format!("unbindidentityaccount: {e}"))?;
                if removed {
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: format!("identitybundlebindings:changed:{}", cmd.identity_id),
                        scopes: vec![],
                        sender: String::new(),
                        persist: 0,
                        data: None,
                    });
                }
                Ok(Some(json!({ "unbound": removed })))
            })
        }),
    );

    let wstore = state.id_store.clone();
    engine.register_handler(
        COMMAND_LIST_IDENTITY_BINDINGS,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: CommandListIdentityBindingsData = serde_json::from_value(data)
                    .map_err(|e| format!("listidentitybindings: {e}"))?;
                let bindings = wstore
                    .bundle_identity_bindings(&cmd.identity_id)
                    .map_err(|e| format!("listidentitybindings: {e}"))?;
                Ok(Some(serde_json::to_value(&bindings).unwrap_or_default()))
            })
        }),
    );

    // ---- Memory bundle CRUD ----

    let wstore = state.id_store.clone();
    engine.register_handler(
        COMMAND_LIST_MEMORIES,
        Box::new(move |_data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let memories = wstore
                    .bundle_memory_list()
                    .map_err(|e| format!("listmemories: {e}"))?;
                Ok(Some(serde_json::to_value(&memories).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.id_store.clone();
    engine.register_handler(
        COMMAND_GET_MEMORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: CommandGetMemoryData = serde_json::from_value(data)
                    .map_err(|e| format!("getmemory: {e}"))?;
                match wstore
                    .bundle_memory_get(&cmd.id)
                    .map_err(|e| format!("getmemory: {e}"))?
                {
                    Some(m) => Ok(Some(serde_json::to_value(&m).unwrap_or_default())),
                    None => Err(format!("getmemory: not found id={}", cmd.id)),
                }
            })
        }),
    );

    let wstore = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_UPSERT_MEMORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let mut memory: Memory = serde_json::from_value(data)
                    .map_err(|e| format!("upsertmemory: {e}"))?;
                // Guard on BOTH client-supplied is_blank AND id == "blank".
                // Same bypass as upsertidentitybundle — see that comment.
                // (reagent P1, 2026-05-08).
                if memory.is_blank || memory.id == "blank" {
                    return Err("upsertmemory: cannot mutate the blank singleton".to_string());
                }
                if memory.id.is_empty() {
                    memory.id = uuid::Uuid::new_v4().to_string();
                }
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                if memory.created_at == 0 {
                    memory.created_at = now;
                }
                memory.updated_at = now;
                wstore
                    .bundle_memory_upsert(&memory)
                    .map_err(|e| format!("upsertmemory: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "memories:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&memory).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_DELETE_MEMORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandDeleteMemoryData = serde_json::from_value(data)
                    .map_err(|e| format!("deletememory: {e}"))?;
                let deleted = wstore
                    .bundle_memory_delete(&cmd.id)
                    .map_err(|e| format!("deletememory: {e}"))?;
                if deleted {
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: "memories:changed".to_string(),
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

    let wstore = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_REORDER_GLOBAL_BRAIN,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandReorderGlobalBrainData = serde_json::from_value(data)
                    .map_err(|e| format!("reorderglobalbrain: {e}"))?;
                let updated = wstore
                    .bundle_memory_reorder(&cmd.ids)
                    .map_err(|e| format!("reorderglobalbrain: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "memories:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(json!({ "updated": updated })))
            })
        }),
    );

}
