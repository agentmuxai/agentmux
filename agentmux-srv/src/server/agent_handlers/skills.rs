// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};


use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    COMMAND_LIST_AGENT_SKILLS, COMMAND_CREATE_AGENT_SKILL, COMMAND_UPDATE_AGENT_SKILL,
    COMMAND_DELETE_AGENT_SKILL,
    CommandListAgentSkillsData, CommandCreateAgentSkillData, CommandUpdateAgentSkillData,
    CommandDeleteAgentSkillData,
};
use crate::backend::storage::AgentSkill;

use super::super::AppState;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // listagentskills → return this agent's EFFECTIVE skills (legacy
    // db_agent_skills, or its own ref-bound db_skills + globals when any own
    // refs exist — see Store::effective_skills). Window-scoped (no
    // `check_s1`, hence `_ctx` unused) because this is called from
    // `agent-model.ts`'s pre-launch `launchAgentDefinition`, before any
    // agent connection exists to authenticate as — that's also exactly why
    // this must reuse the same merge algorithm as the Rust
    // `write_agent_config_files` path rather than the frontend calling the
    // agent-scoped `skill.list` RPC directly (it would fail check_s1 pre-
    // launch). Previously returned only agent_skill_list (legacy-only),
    // silently hiding every standalone/Armory-catalog skill from the actual
    // launch flow (reagent P0 on PR #2322).
    let wstore_lfs = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_AGENT_SKILLS,
        Box::new(move |data, _ctx| {
            let wstore = wstore_lfs.clone();
            Box::pin(async move {
                let cmd: CommandListAgentSkillsData = serde_json::from_value(data)
                    .map_err(|e| format!("listagentskills: {e}"))?;
                let skills = wstore.effective_skills(&cmd.agent_id);
                Ok(Some(serde_json::to_value(&skills).unwrap_or_default()))
            })
        }),
    );

    // createagentskill → insert new skill, broadcast agentskills:changed
    let wstore_cfs = state.wstore.clone();
    let broker_cfs = state.broker.clone();
    engine.register_handler(
        COMMAND_CREATE_AGENT_SKILL,
        Box::new(move |data, _ctx| {
            let wstore = wstore_cfs.clone();
            let broker = broker_cfs.clone();
            Box::pin(async move {
                let cmd: CommandCreateAgentSkillData = serde_json::from_value(data)
                    .map_err(|e| format!("createagentskill: {e}"))?;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                let skill = AgentSkill {
                    id: uuid::Uuid::new_v4().to_string(),
                    agent_id: cmd.agent_id,
                    name: cmd.name,
                    trigger: cmd.trigger,
                    skill_type: cmd.skill_type,
                    description: cmd.description,
                    content: cmd.content,
                    created_at: now,
                };
                wstore.agent_skill_insert(&skill).map_err(|e| format!("createagentskill: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agentskills:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&skill).unwrap_or_default()))
            })
        }),
    );

    // updateagentskill → update existing skill, broadcast agentskills:changed
    let wstore_ufs = state.wstore.clone();
    let broker_ufs = state.broker.clone();
    engine.register_handler(
        COMMAND_UPDATE_AGENT_SKILL,
        Box::new(move |data, _ctx| {
            let wstore = wstore_ufs.clone();
            let broker = broker_ufs.clone();
            Box::pin(async move {
                let cmd: CommandUpdateAgentSkillData = serde_json::from_value(data)
                    .map_err(|e| format!("updateagentskill: {e}"))?;
                let existing = wstore.agent_skill_get(&cmd.id)
                    .map_err(|e| format!("updateagentskill: {e}"))?
                    .ok_or_else(|| format!("updateagentskill: skill {} not found", cmd.id))?;
                let skill = AgentSkill {
                    id: cmd.id,
                    agent_id: existing.agent_id,
                    name: cmd.name,
                    trigger: cmd.trigger,
                    skill_type: cmd.skill_type,
                    description: cmd.description,
                    content: cmd.content,
                    created_at: existing.created_at,
                };
                let found = wstore.agent_skill_update(&skill).map_err(|e| format!("updateagentskill: {e}"))?;
                if !found {
                    return Err(format!("updateagentskill: skill {} not found", skill.id));
                }
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agentskills:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&skill).unwrap_or_default()))
            })
        }),
    );

    // deleteagentskill → delete skill by id, broadcast agentskills:changed
    let wstore_dfs = state.wstore.clone();
    let broker_dfs = state.broker.clone();
    engine.register_handler(
        COMMAND_DELETE_AGENT_SKILL,
        Box::new(move |data, _ctx| {
            let wstore = wstore_dfs.clone();
            let broker = broker_dfs.clone();
            Box::pin(async move {
                let cmd: CommandDeleteAgentSkillData = serde_json::from_value(data)
                    .map_err(|e| format!("deleteagentskill: {e}"))?;
                wstore.agent_skill_delete(&cmd.id).map_err(|e| format!("deleteagentskill: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "agentskills:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(None)
            })
        }),
    );

}
