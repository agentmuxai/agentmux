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
    let mstore_lfs = state.mstore.clone();
    let identity_store_lfs = state.identity_store.clone();
    engine.register_typed(
        COMMAND_LIST_AGENT_SKILLS,
        move |cmd: CommandListAgentSkillsData, _ctx| {
            let mstore = mstore_lfs.clone();
            let identity_store = identity_store_lfs.clone();
            async move {
                let skills = mstore.effective_skills(&identity_store, &cmd.agent_id);
                Ok(skills)
            }
        },
    );

    // createagentskill → insert new skill, broadcast agentskills:changed
    let mstore_cfs = state.mstore.clone();
    let broker_cfs = state.broker.clone();
    engine.register_typed(
        COMMAND_CREATE_AGENT_SKILL,
        move |cmd: CommandCreateAgentSkillData, _ctx| {
            let mstore = mstore_cfs.clone();
            let broker = broker_cfs.clone();
            async move {
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
                mstore.agent_skill_insert(&skill).map_err(|e| format!("createagentskill: {e}"))?;
                broker.publish(crate::backend::mps::MuxEvent {
                    event: "agentskills:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(skill)
            }
        },
    );

    // updateagentskill → update existing skill, broadcast agentskills:changed
    let mstore_ufs = state.mstore.clone();
    let broker_ufs = state.broker.clone();
    engine.register_typed(
        COMMAND_UPDATE_AGENT_SKILL,
        move |cmd: CommandUpdateAgentSkillData, _ctx| {
            let mstore = mstore_ufs.clone();
            let broker = broker_ufs.clone();
            async move {
                let existing = mstore.agent_skill_get(&cmd.id)
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
                let found = mstore.agent_skill_update(&skill).map_err(|e| format!("updateagentskill: {e}"))?;
                if !found {
                    return Err(format!("updateagentskill: skill {} not found", skill.id));
                }
                broker.publish(crate::backend::mps::MuxEvent {
                    event: "agentskills:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(skill)
            }
        },
    );

    // deleteagentskill → delete skill by id, broadcast agentskills:changed
    let mstore_dfs = state.mstore.clone();
    let broker_dfs = state.broker.clone();
    engine.register_typed(
        COMMAND_DELETE_AGENT_SKILL,
        move |cmd: CommandDeleteAgentSkillData, _ctx| {
            let mstore = mstore_dfs.clone();
            let broker = broker_dfs.clone();
            async move {
                mstore.agent_skill_delete(&cmd.id).map_err(|e| format!("deleteagentskill: {e}"))?;
                broker.publish(crate::backend::mps::MuxEvent {
                    event: "agentskills:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(())
            }
        },
    );

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::rpc_types::{
        CommandCreateAgentSkillData, CommandUpdateAgentSkillData,
    };

    /// All four skill commands record their request and response types.
    #[tokio::test]
    async fn register_typed_records_every_skill_command() {
        let state = crate::server::tests::test_state();
        let (engine, _rx) = WshRpcEngine::new();
        register(&engine, &state);
        let schema = engine.schema_json();
        let rows = schema.as_array().unwrap();
        let find = |cmd: &str| {
            rows.iter()
                .find(|r| r["command"] == cmd)
                .unwrap_or_else(|| panic!("{cmd} missing from the schema"))
        };

        for (cmd, req, resp) in [
            (COMMAND_CREATE_AGENT_SKILL, "CommandCreateAgentSkillData", "AgentSkill"),
            (COMMAND_UPDATE_AGENT_SKILL, "CommandUpdateAgentSkillData", "AgentSkill"),
            (COMMAND_DELETE_AGENT_SKILL, "CommandDeleteAgentSkillData", "()"),
        ] {
            let row = find(cmd);
            assert_eq!(row["requestName"], req, "{cmd} request");
            assert_eq!(row["responseName"], resp, "{cmd} response");
        }

        let list = find(COMMAND_LIST_AGENT_SKILLS);
        assert_eq!(list["requestName"], "CommandListAgentSkillsData");
        let list_resp = list["responseName"].as_str().unwrap();
        assert!(
            list_resp.starts_with("alloc::vec::Vec<") && list_resp.contains("AgentSkill"),
            "listagentskills should answer Vec<AgentSkill>, got {list_resp:?}",
        );
    }

    /// `skill_type` defaults differently on create and update, and the
    /// hand-written TS typed both `skill_type?: string`, which hid it.
    ///
    /// `createagentskill` fills in "prompt"; `updateagentskill` fills in "".
    /// Since update is a full replace -- it rebuilds the whole `AgentSkill`
    /// from the request and keeps only `agent_id`/`created_at` -- a
    /// create-then-update round trip that omits the field silently blanks a
    /// value the create had set.
    ///
    /// This is documenting existing behaviour rather than changing it: the
    /// fix is a product decision and `updateagentskill` has no caller today.
    /// What this test does is make the asymmetry fail loudly if someone
    /// "tidies" one of the two defaults without meaning to change the other.
    #[test]
    fn create_and_update_disagree_about_the_default_skill_type() {
        let created: CommandCreateAgentSkillData = serde_json::from_value(serde_json::json!({
            "agent_id": "a1",
            "name": "review",
        }))
        .unwrap();
        assert_eq!(created.skill_type, "prompt", "createagentskill defaults to prompt");

        let updated: CommandUpdateAgentSkillData = serde_json::from_value(serde_json::json!({
            "id": "s1",
            "name": "review",
        }))
        .unwrap();
        assert_eq!(
            updated.skill_type, "",
            "updateagentskill defaults to empty -- if this ever becomes \"prompt\", \
             the two are consistent and the warning on CommandUpdateAgentSkillData \
             should be removed with it",
        );

        // The other three content fields agree: both default to empty.
        for (c, u) in [
            (created.trigger.as_str(), updated.trigger.as_str()),
            (created.description.as_str(), updated.description.as_str()),
            (created.content.as_str(), updated.content.as_str()),
        ] {
            assert_eq!(c, "");
            assert_eq!(u, "");
        }
    }
}
