// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    COMMAND_LIST_AGENT_INSTANCES, COMMAND_GET_AGENT_INSTANCE,
    COMMAND_CREATE_AGENT_INSTANCE, COMMAND_UPDATE_AGENT_INSTANCE,
    COMMAND_DELETE_AGENT_INSTANCE,
    CommandListAgentInstancesData, CommandGetAgentInstanceData,
    CommandCreateAgentInstanceData, CommandUpdateAgentInstanceData,
    CommandDeleteAgentInstanceData, DeleteAgentInstanceResult,
};
use crate::backend::storage::store::{
    AgentInstance, InstanceStatus,
};

use super::super::AppState;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // ---- Agent instance CRUD ----

    let mstore = state.mstore.clone();
    engine.register_typed(
        COMMAND_LIST_AGENT_INSTANCES,
        // `Option<_>` rather than the bare struct, which is what the previous
        // `unwrap_or_default()` was really buying: both encodings of "no
        // filter" -- the `{}` the stub sends and the `null` a client that
        // omits `data` sends -- still mean "no filter". What it no longer
        // buys is swallowing a MALFORMED filter: `{"status": 5}` used to
        // return the unfiltered list, which is the wrong answer to give
        // silently, so it is an error now.
        move |cmd: Option<CommandListAgentInstancesData>, _ctx| {
            let mstore = mstore.clone();
            async move {
                let cmd = cmd.unwrap_or_default();
                let rows = mstore
                    .instance_list(cmd.definition_id.as_deref(), cmd.status.as_deref())
                    .map_err(|e| format!("listagentinstances: {e}"))?;
                Ok(rows)
            }
        },
    );

    let mstore = state.mstore.clone();
    engine.register_typed(
        COMMAND_GET_AGENT_INSTANCE,
        move |cmd: CommandGetAgentInstanceData, _ctx| {
            let mstore = mstore.clone();
            async move {
                // Not-found stays an Err rather than becoming `Option<..>`:
                // the stub types this `Promise<AgentInstance>`, so answering
                // null would be a shape change, not just a typing change.
                mstore
                    .instance_get(&cmd.id)
                    .map_err(|e| format!("getagentinstance: {e}"))?
                    .ok_or_else(|| format!("getagentinstance: not found id={}", cmd.id))
            }
        },
    );

    let mstore = state.mstore.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_CREATE_AGENT_INSTANCE,
        move |cmd: CommandCreateAgentInstanceData, _ctx| {
            let mstore = mstore.clone();
            let broker = broker.clone();
            async move {
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
                    // Bundle picks. Empty string = blank
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
                // The row the launch landed on — its id can differ from
                // `inst.id` (a launch of a user agent folds into that agent's
                // row; a continuation folds into the row it continues). The
                // frontend stores THIS id as the block's `agentInstanceId`,
                // so the response must carry the canonical row. The session
                // zone stamp and the change event below deliberately keep
                // using the REQUESTED definition id: Option E anchors the
                // zone on the definition the launch modal opened
                // (`agent:<defId>:current`), and the frontend subscribes to
                // `agentinstances:changed:<defId>` by that same id.
                let created = mstore
                    .instance_create(&inst)
                    .map_err(|e| format!("createagentinstance: {e}"))?;

                // Option E (PR 1 of 2) — stamp the agent-anchored
                // session zone reference onto the block meta. Every
                // block of this agent definition reads/writes through
                // `agent:<defId>:current`. Continuation is now
                // structural (same zone, different block) rather than
                // parametric (per-block snapshot copy + --continue).
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
                        &mstore, &oref_str, &meta_update,
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

                broker.publish(crate::backend::mps::MuxEvent {
                    event: format!("agentinstances:changed:{}", inst.definition_id),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(created)
            }
        },
    );

    let mstore = state.mstore.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_UPDATE_AGENT_INSTANCE,
        move |cmd: CommandUpdateAgentInstanceData, _ctx| {
            let mstore = mstore.clone();
            let broker = broker.clone();
            async move {
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
                let fresh = mstore
                    .instance_update_partial(&cmd.id, &upd)
                    .map_err(|e| format!("updateagentinstance: {e}"))?
                    .ok_or_else(|| format!("updateagentinstance: not found id={}", cmd.id))?;
                broker.publish(crate::backend::mps::MuxEvent {
                    event: format!("agentinstances:changed:{}", fresh.definition_id),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(fresh)
            }
        },
    );

    let mstore = state.mstore.clone();
    let broker = state.broker.clone();
    let identity_store_del = state.identity_store.clone();
    engine.register_typed(
        COMMAND_DELETE_AGENT_INSTANCE,
        move |cmd: CommandDeleteAgentInstanceData, _ctx| {
            let mstore = mstore.clone();
            let broker = broker.clone();
            let identity_store = identity_store_del.clone();
            async move {
                // Read the row first so we can emit a scoped event after.
                let definition_id = mstore
                    .instance_get(&cmd.id)
                    .map_err(|e| format!("deleteagentinstance: {e}"))?
                    .map(|i| i.definition_id);
                let deleted = mstore
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
                super::purge_identity_store_rows(&identity_store, &cmd.id, "deleteagentinstance");
                if let Some(def_id) = definition_id.filter(|_| deleted) {
                    broker.publish(crate::backend::mps::MuxEvent {
                        event: format!("agentinstances:changed:{}", def_id),
                        scopes: vec![],
                        sender: String::new(),
                        persist: 0,
                        data: None,
                    });
                }
                Ok(DeleteAgentInstanceResult { deleted })
            }
        },
    );

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
}
