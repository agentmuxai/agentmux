// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


use std::sync::Arc;


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
    let state_list = state.clone();
    engine.register_typed(COMMAND_LIST_AGENT_SKILLS, move |cmd: CommandListAgentSkillsData, _ctx| {
        let state = state_list.clone();
        async move { list_agent_skills(&state, cmd) }
    });

    let state_create = state.clone();
    engine.register_typed(COMMAND_CREATE_AGENT_SKILL, move |cmd: CommandCreateAgentSkillData, _ctx| {
        let state = state_create.clone();
        async move { create_agent_skill(&state, cmd) }
    });

    let state_update = state.clone();
    engine.register_typed(COMMAND_UPDATE_AGENT_SKILL, move |cmd: CommandUpdateAgentSkillData, _ctx| {
        let state = state_update.clone();
        async move { update_agent_skill(&state, cmd) }
    });

    let state_delete = state.clone();
    engine.register_typed(COMMAND_DELETE_AGENT_SKILL, move |cmd: CommandDeleteAgentSkillData, _ctx| {
        let state = state_delete.clone();
        async move { delete_agent_skill(&state, cmd) }
    });
}

/// listagentskills → return this agent's EFFECTIVE skills (legacy
/// db_agent_skills, or its own ref-bound db_skills + globals when any own
/// refs exist — see Store::effective_skills). Window-scoped (no
/// `check_s1`) because this is called from
/// `agent-model.ts`'s pre-launch `launchAgentDefinition`, before any
/// agent connection exists to authenticate as — that's also exactly why
/// this must reuse the same merge algorithm as the Rust
/// `write_agent_config_files` path rather than the frontend calling the
/// agent-scoped `skill.list` RPC directly (it would fail check_s1 pre-
/// launch). Previously returned only agent_skill_list (legacy-only),
/// silently hiding every standalone/Memory → Skills skill from the actual
/// launch flow (reagent P0 on PR #2322).
pub(crate) fn list_agent_skills(state: &AppState, cmd: CommandListAgentSkillsData) -> Result<Vec<AgentSkill>, String> {
    Ok(state.mstore.effective_skills(&state.identity_store, &cmd.agent_id))
}

/// createagentskill → insert new skill, broadcast agentskills:changed
pub(crate) fn create_agent_skill(state: &AppState, cmd: CommandCreateAgentSkillData) -> Result<AgentSkill, String> {
    let now = agentmux_common::time::now_ms();
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
    state.mstore.agent_skill_insert(&skill).map_err(|e| format!("createagentskill: {e}"))?;
    state.broker.publish_event::<crate::backend::mps_events::AgentSkillsChanged>(&(), vec![], 0);
    Ok(skill)
}

/// updateagentskill → update existing skill, broadcast agentskills:changed
pub(crate) fn update_agent_skill(state: &AppState, cmd: CommandUpdateAgentSkillData) -> Result<AgentSkill, String> {
    let existing = state
        .mstore
        .agent_skill_get(&cmd.id)
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
    let found = state.mstore.agent_skill_update(&skill).map_err(|e| format!("updateagentskill: {e}"))?;
    if !found {
        return Err(format!("updateagentskill: skill {} not found", skill.id));
    }
    state.broker.publish_event::<crate::backend::mps_events::AgentSkillsChanged>(&(), vec![], 0);
    Ok(skill)
}

/// deleteagentskill → delete skill by id, broadcast agentskills:changed
pub(crate) fn delete_agent_skill(state: &AppState, cmd: CommandDeleteAgentSkillData) -> Result<(), String> {
    state.mstore.agent_skill_delete(&cmd.id).map_err(|e| format!("deleteagentskill: {e}"))?;
    state.broker.publish_event::<crate::backend::mps_events::AgentSkillsChanged>(&(), vec![], 0);
    Ok(())
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

    /// A skill belongs to an agent row, so the row has to exist.
    fn state_with_agent(id: &str) -> AppState {
        let state = crate::server::tests::test_state();
        let mut def = crate::backend::storage::agents::test_agent_def(id, id, "claude", "agent", 1, "");
        state.mstore.agent_def_insert(&mut def).unwrap();
        state
    }

    fn create_req(agent_id: &str, name: &str) -> CommandCreateAgentSkillData {
        serde_json::from_value(serde_json::json!({ "agent_id": agent_id, "name": name, "content": "do it" })).unwrap()
    }

    #[tokio::test]
    async fn a_created_skill_is_stored_and_listed_for_its_agent() {
        let state = state_with_agent("agent-s1");
        let skill = create_agent_skill(&state, create_req("agent-s1", "review")).unwrap();
        assert_eq!((skill.agent_id.as_str(), skill.name.as_str(), skill.skill_type.as_str()), ("agent-s1", "review", "prompt"));
        assert!(!skill.id.is_empty());

        let listed = list_agent_skills(&state, CommandListAgentSkillsData { agent_id: "agent-s1".into() }).unwrap();
        assert!(listed.iter().any(|s| s.id == skill.id), "{listed:?}");
        let other = list_agent_skills(&state, CommandListAgentSkillsData { agent_id: "agent-s2".into() }).unwrap();
        assert!(!other.iter().any(|s| s.id == skill.id));
    }

    #[tokio::test]
    async fn an_update_replaces_the_skill_and_keeps_its_agent_and_creation_time() {
        let state = state_with_agent("agent-s1");
        let created = create_agent_skill(&state, create_req("agent-s1", "review")).unwrap();
        let req: CommandUpdateAgentSkillData =
            serde_json::from_value(serde_json::json!({ "id": created.id, "name": "renamed", "content": "new" })).unwrap();
        let updated = update_agent_skill(&state, req).unwrap();
        assert_eq!((updated.name.as_str(), updated.content.as_str()), ("renamed", "new"));
        assert_eq!((updated.agent_id.as_str(), updated.created_at), ("agent-s1", created.created_at));
        assert_eq!(state.mstore.agent_skill_get(&created.id).unwrap().unwrap().name, "renamed");
    }

    #[tokio::test]
    async fn updating_a_skill_that_is_not_there_names_the_command_and_the_id() {
        let state = crate::server::tests::test_state();
        let req: CommandUpdateAgentSkillData = serde_json::from_value(serde_json::json!({ "id": "nope", "name": "x" })).unwrap();
        assert_eq!(update_agent_skill(&state, req).unwrap_err(), "updateagentskill: skill nope not found");
    }

    #[tokio::test]
    async fn a_deleted_skill_is_gone() {
        let state = state_with_agent("agent-s1");
        let created = create_agent_skill(&state, create_req("agent-s1", "review")).unwrap();
        delete_agent_skill(&state, CommandDeleteAgentSkillData { id: created.id.clone() }).unwrap();
        assert!(state.mstore.agent_skill_get(&created.id).unwrap().is_none());
    }
}
