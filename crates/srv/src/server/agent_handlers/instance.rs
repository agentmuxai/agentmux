// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


use std::sync::Arc;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    COMMAND_LIST_AGENT_INSTANCES, COMMAND_GET_AGENT_INSTANCE,
    COMMAND_CREATE_AGENT_INSTANCE, COMMAND_UPDATE_AGENT_INSTANCE,
    COMMAND_DELETE_AGENT_INSTANCE, COMMAND_AGENT_LAST_RUNTIME,
    AgentLastRuntime, CommandAgentLastRuntimeData,
    CommandListAgentInstancesData, CommandGetAgentInstanceData,
    CommandCreateAgentInstanceData, CommandUpdateAgentInstanceData,
    CommandDeleteAgentInstanceData, DeleteAgentInstanceResult,
};
use crate::backend::storage::store::{
    AgentInstance, InstanceStatus,
};

use super::super::AppState;

/// `listagentinstances`.
//
// `Option<_>` rather than the bare struct, which is what the previous
// `unwrap_or_default()` was really buying: both encodings of "no
// filter" -- the `{}` the stub sends and the `null` a client that
// omits `data` sends -- still mean "no filter". What it no longer
// buys is swallowing a MALFORMED filter: `{"status": 5}` used to
// return the unfiltered list, which is the wrong answer to give
// silently, so it is an error now.
pub(crate) fn list_agent_instances(
    state: &AppState,
    cmd: Option<CommandListAgentInstancesData>,
) -> Result<Vec<AgentInstance>, String> {
    let cmd = cmd.unwrap_or_default();
    let rows = state
        .mstore
        .instance_list(cmd.definition_id.as_deref(), cmd.status.as_deref())
        .map_err(|e| format!("listagentinstances: {e}"))?;
    Ok(rows)
}

/// `getagentinstance`.
pub(crate) fn get_agent_instance(
    state: &AppState,
    cmd: CommandGetAgentInstanceData,
) -> Result<AgentInstance, String> {
    // Not-found stays an Err rather than becoming `Option<..>`:
    // the stub types this `Promise<AgentInstance>`, so answering
    // null would be a shape change, not just a typing change.
    state
        .mstore
        .instance_get(&cmd.id)
        .map_err(|e| format!("getagentinstance: {e}"))?
        .ok_or_else(|| format!("getagentinstance: not found id={}", cmd.id))
}

/// Option E (PR 1 of 2) — stamp the agent-anchored
/// session zone reference onto the block meta. Every
/// block of this agent definition reads/writes through
/// `agent:<defId>:current`. Continuation is now
/// structural (same zone, different block) rather than
/// parametric (per-block snapshot copy + --continue).
fn stamp_session_zone(state: &AppState, inst: &AgentInstance) {
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
            &state.mstore, &oref_str, &meta_update,
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
}

/// `createagentinstance`.
pub(crate) fn create_agent_instance(
    state: &AppState,
    cmd: CommandCreateAgentInstanceData,
) -> Result<AgentInstance, String> {
    let mstore = &state.mstore;
    // A fresh launch of a template would put a pane on the template's own transcript
    // zone (`agent:<templateId>:current`, shared by every launch of it), with a row
    // of its own: two keys for one conversation. Templates are cloned into a user
    // agent first (the picker does; `agent.open` refuses them). A continuation of an
    // existing launch folds into its row and stays allowed.
    if cmd.parent_instance_id.is_empty()
        && mstore.agent_def_get(&cmd.definition_id).ok().flatten().is_some_and(|d| d.is_seeded == 1)
    {
        return Err(format!(
            "TEMPLATE_NOT_LAUNCHABLE: '{}' is a template; create an agent from it first",
            cmd.definition_id
        ));
    }
    let now = agentmux_common::time::now_ms();
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
        // PR-F.3: the launch passes through its account +
        // bundle picks. Empty string = blank
        // singleton (no override; the resolver returns
        // immediately on either "" or "blank").
        identity_id: cmd.identity_id,
        memory_id: cmd.memory_id,
        // v8: named-agent continuation. instance_name +
        // working_directory come from the launch
        // overrides via CommandCreateAgentInstanceData
        // (added in the same spec). Empty string for
        // legacy/ambient launches.
        instance_name: cmd.instance_name.clone(),
        working_directory: cmd.working_directory.clone(),
        display_hidden: false,
    };
    // The row the launch landed on — its id can differ from
    // `inst.id` (a launch of a user agent folds into that agent's
    // row; a continuation folds into the row it continues). The
    // frontend stores THIS id as the block's `agentInstanceId`,
    // so the response must carry the canonical row. The session
    // zone stamp and the change event below deliberately keep
    // using the REQUESTED definition id: Option E anchors the
    // zone on the definition the launch started from
    // (`agent:<defId>:current`), and the frontend subscribes to
    // `agentinstances:changed:<defId>` by that same id.
    let created = mstore
        .instance_create(&inst)
        .map_err(|e| format!("createagentinstance: {e}"))?;

    stamp_session_zone(state, &inst);

    state.broker.publish_event_for::<crate::backend::mps_events::AgentInstancesChanged>(&inst.definition_id, &(), vec![], 0);
    Ok(created)
}

/// `updateagentinstance`.
pub(crate) fn update_agent_instance(
    state: &AppState,
    cmd: CommandUpdateAgentInstanceData,
) -> Result<AgentInstance, String> {
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
    let fresh = state
        .mstore
        .instance_update_partial(&cmd.id, &upd)
        .map_err(|e| format!("updateagentinstance: {e}"))?
        .ok_or_else(|| format!("updateagentinstance: not found id={}", cmd.id))?;
    state.broker.publish_event_for::<crate::backend::mps_events::AgentInstancesChanged>(&fresh.definition_id, &(), vec![], 0);
    Ok(fresh)
}

/// `agentlastruntime`.
pub(crate) fn agent_last_runtime(
    state: &AppState,
    cmd: CommandAgentLastRuntimeData,
) -> Result<AgentLastRuntime, String> {
    if let Some(raw) = &cmd.runtime {
        let normalized = crate::backend::storage::agents::normalize_last_runtime(raw)
            .map_err(|e| format!("agentlastruntime: {e}"))?;
        state
            .mstore
            .agent_last_runtime_set(&cmd.id, &normalized)
            .map_err(|e| format!("agentlastruntime: {e}"))?;
    }
    let runtime = state
        .mstore
        .agent_last_runtime_get(&cmd.id)
        .map_err(|e| format!("agentlastruntime: {e}"))?;
    Ok(AgentLastRuntime { runtime })
}

/// `deleteagentinstance`.
pub(crate) async fn delete_agent_instance(
    state: &AppState,
    cmd: CommandDeleteAgentInstanceData,
) -> Result<DeleteAgentInstanceResult, String> {
    // An agent's delete also deletes its parked block, and stops it (as `deleteagent` does).
    crate::sagas::park_agents::delete_parked_blocks_of(state, &cmd.id).await;
    // Read the row first so we can emit a scoped event after.
    let definition_id = state
        .mstore
        .instance_get(&cmd.id)
        .map_err(|e| format!("deleteagentinstance: {e}"))?
        .map(|i| i.definition_id);
    let deleted = state
        .mstore
        .instance_delete(&cmd.id)
        .map_err(|e| format!("deleteagentinstance: {e}"))?;
    // Unconditional, matching the sibling `deleteagent` handler
    // (ReAgent P1 round 2 on PR #3262). Gating on `deleted`
    // skipped the identity-store purge on a RETRY — where the
    // local `db_agents` row is already gone but an earlier
    // attempt's credential/identity-link cleanup failed — which
    // is the "the confirm dialog promises a purge it doesn't
    // deliver" bug, reproduced on this entry point. The purge
    // is a no-op for an id with no rows, so there is nothing to
    // gate on.
    super::purge_identity_store_rows(&state.identity_store, &state.id_store, &cmd.id, "deleteagentinstance");
    if let Some(def_id) = definition_id.filter(|_| deleted) {
        state.broker.publish_event_for::<crate::backend::mps_events::AgentInstancesChanged>(&def_id, &(), vec![], 0);
    }
    Ok(DeleteAgentInstanceResult { deleted })
}

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let s = state.clone();
    engine.register_typed(
        COMMAND_LIST_AGENT_INSTANCES,
        move |cmd: Option<CommandListAgentInstancesData>, _ctx| {
            let s = s.clone();
            async move { list_agent_instances(&s, cmd) }
        },
    );

    let s = state.clone();
    engine.register_typed(COMMAND_GET_AGENT_INSTANCE, move |cmd: CommandGetAgentInstanceData, _ctx| {
        let s = s.clone();
        async move { get_agent_instance(&s, cmd) }
    });

    let s = state.clone();
    engine.register_typed(COMMAND_CREATE_AGENT_INSTANCE, move |cmd: CommandCreateAgentInstanceData, _ctx| {
        let s = s.clone();
        async move { create_agent_instance(&s, cmd) }
    });

    let s = state.clone();
    engine.register_typed(COMMAND_UPDATE_AGENT_INSTANCE, move |cmd: CommandUpdateAgentInstanceData, _ctx| {
        let s = s.clone();
        async move { update_agent_instance(&s, cmd) }
    });

    let s = state.clone();
    engine.register_typed(COMMAND_AGENT_LAST_RUNTIME, move |cmd: CommandAgentLastRuntimeData, _ctx| {
        let s = s.clone();
        async move { agent_last_runtime(&s, cmd) }
    });

    let s = state.clone();
    engine.register_typed(COMMAND_DELETE_AGENT_INSTANCE, move |cmd: CommandDeleteAgentInstanceData, _ctx| {
        let s = s.clone();
        async move { delete_agent_instance(&s, cmd).await }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::rpc::engine::WshRpcEngine;
    use crate::server::tests::test_state;

    /// `deleteagentinstance` is recorded in the engine's schema with its
    /// exact type names — the property the RPC codegen plan depends on.
    /// `DeleteAgentInstanceResult` didn't exist before this migration; the
    /// The four CRUD commands record their request and response types, so the
    /// whole of instance.rs is now in the registry rather than one command of
    /// five. A command that falls back to `register_handler` disappears from
    /// here rather than being recorded wrong.
    #[tokio::test]
    async fn register_typed_records_the_four_instance_crud_commands() {
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

        for (cmd, resp) in [
            (crate::backend::rpc_types::COMMAND_GET_AGENT_INSTANCE, "AgentInstance"),
            (crate::backend::rpc_types::COMMAND_CREATE_AGENT_INSTANCE, "AgentInstance"),
            (crate::backend::rpc_types::COMMAND_UPDATE_AGENT_INSTANCE, "AgentInstance"),
        ] {
            assert_eq!(find(cmd)["responseName"], resp, "{cmd} response");
        }
        let list = find(crate::backend::rpc_types::COMMAND_LIST_AGENT_INSTANCES);
        let list_resp = list["responseName"].as_str().unwrap();
        assert!(
            list_resp.starts_with("alloc::vec::Vec<") && list_resp.contains("AgentInstance"),
            "listagentinstances should answer Vec<AgentInstance>, got {list_resp:?}",
        );
    }

    /// `listagentinstances` used to `unwrap_or_default()` its payload. That
    /// bought two different things, and only one of them was wanted.
    ///
    /// Wanted: both encodings of "no filter" work -- the `{}` the stub sends
    /// and the `null` a client that omits `data` sends. `Option<Req>` keeps
    /// that.
    ///
    /// Not wanted: a MALFORMED filter silently became "no filter", so
    /// `{"status": 5}` returned every row instead of erroring. Answering an
    /// unfiltered list to a request whose filter failed to parse is the wrong
    /// answer to give quietly, so it is an error now. This test pins both
    /// halves so the change is visible rather than incidental.
    #[tokio::test]
    async fn listagentinstances_accepts_no_filter_but_rejects_a_malformed_one() {
        let state = crate::server::tests::test_state();
        let (engine, mut rx) = WshRpcEngine::new();
        register(&engine, &state);

        let call = |payload: serde_json::Value, reqid: &str| {
            engine.handle_message(crate::backend::rpc_types::RpcMessage {
                command: crate::backend::rpc_types::COMMAND_LIST_AGENT_INSTANCES.to_string(),
                reqid: reqid.to_string(),
                data: Some(payload),
                ..Default::default()
            });
        };

        for (i, payload) in [serde_json::json!({}), serde_json::Value::Null]
            .into_iter()
            .enumerate()
        {
            call(payload.clone(), &format!("ok-{i}"));
            let resp = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
                .await
                .unwrap()
                .unwrap();
            assert!(
                resp.error.is_empty(),
                "{payload} means no filter and must be accepted, got {:?}",
                resp.error,
            );
        }

        call(serde_json::json!({ "status": 5 }), "bad");
        let resp = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            !resp.error.is_empty(),
            "a malformed filter must not be silently answered with the unfiltered list",
        );
    }

    /// handler used to answer with an anonymous `json!({"deleted": ..})`.
    #[tokio::test]
    async fn register_typed_records_deleteagentinstance() {
        let state = test_state();
        let (engine, _rx) = WshRpcEngine::new();
        register(&engine, &state);
        let schema = engine.schema_json();
        let rows = schema.as_array().unwrap();
        let row = rows
            .iter()
            .find(|r| r["command"] == COMMAND_DELETE_AGENT_INSTANCE)
            .expect("deleteagentinstance missing from the schema");
        assert_eq!(row["requestName"], "CommandDeleteAgentInstanceData");
        assert_eq!(row["responseName"], "DeleteAgentInstanceResult");
        // The other four instance commands waited for the storage-level
        // `AgentInstance` entity to be generated, which it now is, so they are
        // present rather than absent. The property being asserted is the same
        // one, with the opposite sign --
        // `register_typed_records_the_four_instance_crud_commands` covers
        // their exact types; here just assert instance.rs has nothing left
        // outside the registry.
        for cmd in [
            COMMAND_LIST_AGENT_INSTANCES,
            COMMAND_GET_AGENT_INSTANCE,
            COMMAND_CREATE_AGENT_INSTANCE,
            COMMAND_UPDATE_AGENT_INSTANCE,
        ] {
            assert!(
                rows.iter().any(|r| r["command"] == cmd),
                "{cmd} is migrated and must appear in the schema",
            );
        }
    }

    /// `createagentinstance` refuses a fresh launch of a template and records a user
    /// agent's, so no pane is put on a template's shared transcript zone.
    #[tokio::test]
    async fn createagentinstance_refuses_a_fresh_template_launch() {
        let state = crate::server::tests::test_state();
        for (id, seeded) in [("tpl-x", 1), ("mine-x", 0)] {
            let mut def: crate::backend::storage::AgentDefinition = serde_json::from_value(serde_json::json!({
                "id": id, "name": id, "icon": "wand-magic-sparkles", "provider": "claude",
                "description": "", "created_at": 1, "is_seeded": seeded,
            }))
            .unwrap();
            state.mstore.agent_def_insert(&mut def).unwrap();
        }
        let (engine, mut rx) = WshRpcEngine::new();
        register(&engine, &state);
        let mut call = |n: u32, id: &str| {
            engine.handle_message(crate::backend::rpc_types::RpcMessage {
                command: COMMAND_CREATE_AGENT_INSTANCE.to_string(),
                reqid: format!("r{n}"),
                data: Some(serde_json::json!({ "definition_id": id })),
                ..Default::default()
            });
        };

        call(1, "tpl-x");
        let r = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv()).await.unwrap().unwrap();
        assert!(r.error.starts_with("TEMPLATE_NOT_LAUNCHABLE"), "{:?}", r.error);
        assert!(state.mstore.instance_list(Some("tpl-x"), None).unwrap().is_empty(), "no row was made");

        call(2, "mine-x");
        let r = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv()).await.unwrap().unwrap();
        assert!(r.error.is_empty(), "{:?}", r.error);
    }

    /// `agentlastruntime`: read / replace / forget the remembered runtime, and
    /// refuse a value the launch path could not read.
    #[tokio::test]
    async fn agentlastruntime_reads_replaces_forgets_and_refuses_bad_values() {
        let state = crate::server::tests::test_state();
        let mut agent: crate::backend::storage::AgentDefinition = serde_json::from_value(serde_json::json!({
            "id": "rt-agent",
            "name": "rt-agent",
            "icon": "wand-magic-sparkles",
            "provider": "claude",
            "description": "",
            "created_at": 1,
        }))
        .unwrap();
        state.mstore.agent_def_insert(&mut agent).unwrap();
        let (engine, mut rx) = WshRpcEngine::new();
        register(&engine, &state);

        let mut n = 0;
        let mut call = |payload: serde_json::Value| {
            n += 1;
            engine.handle_message(crate::backend::rpc_types::RpcMessage {
                command: COMMAND_AGENT_LAST_RUNTIME.to_string(),
                reqid: format!("r{n}"),
                data: Some(payload),
                ..Default::default()
            });
        };
        macro_rules! next {
            () => {
                tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
                    .await
                    .unwrap()
                    .unwrap()
            };
        }

        call(serde_json::json!({ "id": "rt-agent" }));
        let r = next!();
        assert!(r.error.is_empty(), "{:?}", r.error);
        assert_eq!(r.data.unwrap()["runtime"], "");

        call(serde_json::json!({ "id": "rt-agent", "runtime": r#"{"model":"opus","effort":"high"}"# }));
        let r = next!();
        assert!(r.error.is_empty(), "{:?}", r.error);
        assert_eq!(r.data.unwrap()["runtime"], r#"{"effort":"high","model":"opus"}"#);

        call(serde_json::json!({ "id": "rt-agent", "runtime": r#"{"colour":"red"}"# }));
        let r = next!();
        assert!(!r.error.is_empty(), "an unknown key is refused");

        call(serde_json::json!({ "id": "rt-agent" }));
        assert_eq!(next!().data.unwrap()["runtime"], r#"{"effort":"high","model":"opus"}"#, "a refused write changes nothing");

        call(serde_json::json!({ "id": "rt-agent", "runtime": "" }));
        assert_eq!(next!().data.unwrap()["runtime"], "");
    }

    fn insert_agent(state: &AppState, id: &str, seeded: i64) {
        let mut def: crate::backend::storage::AgentDefinition = serde_json::from_value(serde_json::json!({
            "id": id, "name": id, "icon": "wand-magic-sparkles", "provider": "claude",
            "description": "", "created_at": 1, "is_seeded": seeded,
        }))
        .unwrap();
        state.mstore.agent_def_insert(&mut def).unwrap();
    }

    fn launch(state: &AppState, def: &str) -> AgentInstance {
        create_agent_instance(state, serde_json::from_value(serde_json::json!({ "definition_id": def })).unwrap()).unwrap()
    }

    /// The handlers run without the engine: a launch makes a running row that
    /// `get` returns and `list` finds by definition, and `list` takes no filter.
    #[tokio::test]
    async fn create_get_and_list_agent_instances_agree() {
        let state = test_state();
        insert_agent(&state, "inst-a", 0);
        insert_agent(&state, "inst-b", 0);
        let a = launch(&state, "inst-a");
        launch(&state, "inst-b");
        assert_eq!(a.status, "running");

        let got = get_agent_instance(&state, serde_json::from_value(serde_json::json!({ "id": a.id })).unwrap()).unwrap();
        assert_eq!(got.definition_id, "inst-a");

        let only_a = list_agent_instances(
            &state,
            Some(serde_json::from_value(serde_json::json!({ "definition_id": "inst-a" })).unwrap()),
        )
        .unwrap();
        assert_eq!(only_a.len(), 1);
        assert_eq!(list_agent_instances(&state, None).unwrap().len(), 2);
    }

    #[tokio::test]
    async fn get_agent_instance_errors_when_the_row_is_missing() {
        let state = test_state();
        let err = get_agent_instance(&state, serde_json::from_value(serde_json::json!({ "id": "nope" })).unwrap()).unwrap_err();
        assert_eq!(err, "getagentinstance: not found id=nope");
    }

    /// A template is refused unless the launch continues an existing one.
    #[tokio::test]
    async fn create_agent_instance_refuses_a_template_but_not_a_continuation() {
        let state = test_state();
        insert_agent(&state, "tpl-direct", 1);
        let fresh = serde_json::from_value(serde_json::json!({ "definition_id": "tpl-direct" })).unwrap();
        let err = create_agent_instance(&state, fresh).unwrap_err();
        assert!(err.starts_with("TEMPLATE_NOT_LAUNCHABLE"), "{err}");
        assert!(state.mstore.instance_list(Some("tpl-direct"), None).unwrap().is_empty());

        let cont = serde_json::from_value(serde_json::json!({
            "definition_id": "tpl-direct", "parent_instance_id": "earlier",
        }))
        .unwrap();
        assert!(create_agent_instance(&state, cont).is_ok());
    }

    /// Only the fields the command names change; an unknown id is an error.
    #[tokio::test]
    async fn update_agent_instance_writes_only_the_named_fields() {
        let state = test_state();
        insert_agent(&state, "inst-u", 0);
        let inst = launch(&state, "inst-u");

        let upd = serde_json::from_value(serde_json::json!({ "id": inst.id, "status": "ended", "ended_at": 42 })).unwrap();
        let fresh = update_agent_instance(&state, upd).unwrap();
        assert_eq!(fresh.status, "ended");
        assert_eq!(fresh.ended_at, 42);
        assert_eq!(fresh.definition_id, "inst-u");

        let missing = serde_json::from_value(serde_json::json!({ "id": "nope", "status": "ended" })).unwrap();
        assert_eq!(update_agent_instance(&state, missing).unwrap_err(), "updateagentinstance: not found id=nope");
    }

    #[tokio::test]
    async fn agent_last_runtime_handler_reads_replaces_and_refuses_bad_values() {
        let state = test_state();
        insert_agent(&state, "rt-direct", 0);
        let call = |v: serde_json::Value| agent_last_runtime(&state, serde_json::from_value(v).unwrap());

        assert_eq!(call(serde_json::json!({ "id": "rt-direct" })).unwrap().runtime, "");
        let set = call(serde_json::json!({ "id": "rt-direct", "runtime": r#"{"model":"opus"}"# })).unwrap();
        assert_eq!(set.runtime, r#"{"model":"opus"}"#);
        assert!(call(serde_json::json!({ "id": "rt-direct", "runtime": r#"{"colour":"red"}"# })).is_err());
        assert_eq!(call(serde_json::json!({ "id": "rt-direct" })).unwrap().runtime, r#"{"model":"opus"}"#);
    }

    /// Deleting reports whether a row went, and a second delete reports false
    /// rather than erroring.
    #[tokio::test]
    async fn delete_agent_instance_reports_whether_a_row_was_removed() {
        let state = test_state();
        insert_agent(&state, "inst-d", 0);
        let inst = launch(&state, "inst-d");
        let del = |id: &str| delete_agent_instance(&state, serde_json::from_value(serde_json::json!({ "id": id })).unwrap());

        assert!(del(&inst.id).await.unwrap().deleted);
        assert!(state.mstore.instance_get(&inst.id).unwrap().is_none());
        assert!(!del(&inst.id).await.unwrap().deleted);
    }
}
