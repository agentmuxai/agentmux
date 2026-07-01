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
                    // PR-F.3: launch modal passes through Identity +
                    // Memory bundle picks. Empty string = blank
                    // singleton (no override; the resolver returns
                    // immediately on either "" or "blank").
                    identity_id: cmd.identity_id,
                    memory_id: cmd.memory_id,
                    // v8: named-agent continuation. instance_name +
                    // working_directory come from the launch-modal
                    // overrides via CommandCreateAgentInstanceData
                    // (added in the same spec). Empty string for
                    // legacy/ambient launches.
                    instance_name: cmd.instance_name.clone(),
                    working_directory: cmd.working_directory.clone(),
                    display_hidden: false,
                };
                wstore
                    .instance_create(&inst)
                    .map_err(|e| format!("createagentinstance: {e}"))?;

                // Option E (PR 1 of 2) — stamp the agent-anchored
                // session zone reference onto the block meta. Every
                // block of this agent definition reads/writes through
                // `agent:<defId>:current`. Continuation is now
                // structural (same zone, different block) rather than
                // parametric (per-block snapshot copy + --continue).
                // See docs/specs/SPEC_CONTINUATION_SESSION_PERSISTENCE_2026_05_23.md.
                if !inst.block_id.is_empty()
                    && crate::backend::agent_session::is_valid_definition_id(&inst.definition_id)
                {
                    let zone = crate::backend::agent_session::agent_current_zone(
                        &inst.definition_id,
                    );
                    let mut meta_update = crate::backend::obj::MetaMapType::new();
                    meta_update.insert(
                        "agent:sessionZone".to_string(),
                        serde_json::json!(zone),
                    );
                    let oref_str = format!("block:{}", inst.block_id);
                    if let Err(e) = crate::server::service::update_object_meta(
                        &wstore, &oref_str, &meta_update,
                    ) {
                        // Non-fatal — the instance row is the source
                        // of truth, the meta stamp is a frontend
                        // convenience. Log + continue so the launch
                        // doesn't abort mid-flow.
                        tracing::warn!(
                            block_id = %inst.block_id,
                            definition_id = %inst.definition_id,
                            error = %e,
                            "createagentinstance: failed to stamp agent:sessionZone"
                        );
                    }
                }

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
                // Partial write — only the fields the command provided.
                // No fetch-and-merge: this used to `instance_get` the full
                // row to fill the unspecified fields, which was the sole
                // production caller needing `instance_get`'s transient
                // per-launch columns. The store builds a dynamic UPDATE
                // and returns the post-write row (for the event scope +
                // response) from the reload it already runs.
                // SPEC_UPDATEAGENTINSTANCE_PARTIAL_UPDATE_2026_05_29.md.
                let upd = crate::backend::storage::InstanceUpdate {
                    block_id: cmd.block_id,
                    session_id: cmd.session_id,
                    status: cmd.status,
                    github_context: cmd.github_context,
                    ended_at: cmd.ended_at,
                };
                let fresh = wstore
                    .instance_update_partial(&cmd.id, &upd)
                    .map_err(|e| format!("updateagentinstance: {e}"))?
                    .ok_or_else(|| format!("updateagentinstance: not found id={}", cmd.id))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: format!("agentinstances:changed:{}", fresh.definition_id),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&fresh).unwrap_or_default()))
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

}
