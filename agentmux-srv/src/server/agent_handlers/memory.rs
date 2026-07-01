// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
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
use crate::backend::storage::store::{
    Identity, Memory,
};

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
