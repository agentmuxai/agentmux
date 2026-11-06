// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The `object` service's methods, one function each. A method answers `Ok` with its reply, or
//! `Err` with the reply that ends the call early (a bad argument, a refusal).

#![allow(clippy::result_large_err)]

use agentmux_common::ipc::{Command, Event};

use crate::backend::obj::*;
use crate::backend::service::{self, WebCallType, WebReturnType};
use crate::server::AppState;

use super::super::object_helpers::{
    get_object_by_oref, schedule_agent_zoom_mirror, update_object, update_object_meta,
};
use super::super::reducer_helpers::{compensate_via_reducer, dispatch_to_reducer, publish_events};

type Reply = Result<WebReturnType, WebReturnType>;

/// The argument at `idx`, or the reply that refuses the call for it.
fn arg<T: serde::de::DeserializeOwned>(args: &[serde_json::Value], idx: usize) -> Result<T, WebReturnType> {
    service::get_arg(args, idx).map_err(WebReturnType::error)
}

/// The message of the first `Error` event the reducer emitted, if any.
fn reducer_error(events: &[Event]) -> Option<String> {
    events.iter().find_map(|e| match e {
        Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    })
}

/// Writes each event the reducer emitted to the store, stopping at the first failure.
fn apply_events(state: &AppState, events: &[Event]) -> Result<(), String> {
    for ev in events {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn row_update(otype: &str, oid: &str, obj: serde_json::Value) -> MuxObjUpdate {
    MuxObjUpdate { updatetype: "update".into(), otype: otype.to_string(), oid: oid.to_string(), obj: Some(obj) }
}

pub(super) fn get_object(state: &AppState, args: &[serde_json::Value]) -> Reply {
    let oref_str: String = arg(args, 0)?;
    match get_object_by_oref(&state.mstore, &oref_str) {
        Ok(data) => Ok(WebReturnType::success(data)),
        Err(e) => Ok(WebReturnType::error(e)),
    }
}

pub(super) fn get_objects(state: &AppState, args: &[serde_json::Value]) -> Reply {
    let orefs: Vec<String> = arg(args, 0)?;
    let mut results = Vec::new();
    for oref_str in &orefs {
        match get_object_by_oref(&state.mstore, oref_str) {
            Ok(data) => results.push(data),
            Err(_) => results.push(serde_json::Value::Null),
        }
    }
    Ok(WebReturnType::success(serde_json::json!(results)))
}

/// The tab a `CreateBlock` call targets: an explicit `tabId` argument, else the context's active tab.
fn create_block_tab(call: &WebCallType) -> Result<String, WebReturnType> {
    // Optional explicit tab_id at args[2] (args[1] is rtOpts).
    // When present, overrides uicontext.active_tab_id — lets
    // callers like applyTabPreset (frontend) target a specific
    // tab without depending on which tab happens to be active
    // when the RPC's uicontext is serialised. Eliminates the
    // TOCTOU race where the user can switch tabs between the
    // call site and the server-side handler.
    //
    // A *malformed* args[2] (e.g. non-string from a stale SDK)
    // returns an error — silently falling back to uicontext
    // would defeat the explicit-targeting contract and make
    // wrong-tab routing hard to diagnose. Missing/null/empty
    // is fine: treat as "no override" and use uicontext.
    let explicit_tab_id: Option<String> = match service::get_optional_arg::<String>(&call.args, 2) {
        Ok(opt) => opt.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        Err(e) => return Err(WebReturnType::error(format!("invalid tabId arg: {}", e))),
    };
    match explicit_tab_id {
        Some(id) => Ok(id),
        None => match call.uicontext.as_ref().map(|ctx| ctx.active_tab_id.clone()) {
            Some(id) if !id.is_empty() => Ok(id),
            _ => Err(WebReturnType::error("missing uicontext.activetabid")),
        },
    }
}

pub(super) async fn create_block(state: &AppState, call: &WebCallType) -> Reply {
    let store = &state.mstore;
    let mut block_def: BlockDef = arg(&call.args, 0)?;
    crate::server::browser_owner::strip_srv_only_keys(&mut block_def.meta);
    if let Err(e) = crate::server::browser_identity::check_new_tab(state, &block_def.meta) {
        return Err(WebReturnType::error(e));
    }
    let tab_id = create_block_tab(call)?;
    // Phase E.2c.4 — CreateBlock dispatches through the reducer
    // (forward+compensate on SQLite failure). The reducer
    // assigns the block_id; the persist subscriber's apply
    // path writes the Block row with the caller's meta map.
    let meta_value = serde_json::to_value(&block_def.meta).unwrap_or(serde_json::Value::Null);
    let events = dispatch_to_reducer(state, Command::CreateBlock { tab_id: tab_id.clone(), meta: meta_value }).await;
    // Surface reducer Error events (tab not found).
    if let Some(err_msg) = reducer_error(&events) {
        return Err(WebReturnType::error(err_msg));
    }
    let block_id = events.iter().find_map(|e| match e {
        Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
        _ => None,
    });
    if let Err(err) = apply_events(state, &events) {
        if let Some(bid) = block_id.as_ref() {
            compensate_via_reducer(
                state,
                Command::DeleteBlock { tab_id: tab_id.clone(), block_id: bid.clone() },
                store,
            )
            .await;
        }
        return Err(WebReturnType::error(format!("CreateBlock: SQLite write failed: {}", err)));
    }
    publish_events(state, &events);
    match block_id {
        Some(bid) => {
            let mut updates = vec![];
            if let Ok(block) = store.must_get::<Block>(&bid) {
                updates.push(row_update(OTYPE_BLOCK, &bid, mux_obj_to_value(&block)));
            }
            if let Ok(tab) = store.must_get::<Tab>(&tab_id) {
                updates.push(row_update(OTYPE_TAB, &tab_id, mux_obj_to_value(&tab)));
            }
            Ok(WebReturnType::success_data_updates(serde_json::json!(bid), updates))
        }
        None => Err(WebReturnType::error("CreateBlock: reducer did not emit BlockCreated".to_string())),
    }
}

// Phase E.5.7 (Step 5 PR 1) — DeleteBlock saga. The legacy
// SQLite-first pattern (wcore::delete_block + reducer-sync
// dispatch) is replaced by `sagas::delete_block::run`, which
// routes through the reducer + persist subscriber. The saga
// also handles the controller-kill cascade (matches the old
// ordering: controller down → reducer dispatch → SQLite).
pub(super) async fn delete_block(state: &AppState, args: &[serde_json::Value]) -> Reply {
    let block_id: String = arg(args, 0)?;
    // Look up the block's owning tab from server state rather than using
    // uicontext.active_tab_id. Floating pane windows have their own tab
    // context that differs from the block's owning tab, so using the
    // uicontext tab always fails for floating-pane closes.
    let tab_id = {
        let s = state.srv_state.lock().await;
        match s.blocks.get(&block_id) {
            Some(rec) => rec.tab_id.clone(),
            None => return Err(WebReturnType::error(format!("DeleteBlock: block not found: {}", block_id))),
        }
    };
    if let Err(reason) = crate::sagas::delete_block::run(state, tab_id, block_id).await {
        return Err(WebReturnType::error(reason));
    }
    Ok(WebReturnType::success_empty())
}

// `/shutdown` (aliases `/quit`, `/exit`) in an agent's composer: end that
// agent gracefully and close its own tab only
// (SPEC_AGENT_SELF_QUIT_2026_09_24.md §3). Returns the quit summary for
// the "<agent> shut down" notice.
pub(super) async fn quit_agent(state: &AppState, args: &[serde_json::Value]) -> Reply {
    let block_id: String = arg(args, 0)?;
    match crate::sagas::self_quit::run(state, &block_id, crate::sagas::self_quit::QuitOrigin::UserSlash).await {
        Ok(summary) => Ok(WebReturnType::success(serde_json::to_value(&summary).unwrap_or_default())),
        Err(reason) => Ok(WebReturnType::error(reason)),
    }
}

// Shut down from My agents or a pane's menu: the same quit, with the person's answer to
// "stop its background tasks too?" (SPEC_CLOSING_PARKS_AGENTS_SHUTDOWN_IS_EXPLICIT §4.4, §4.6).
pub(super) async fn shutdown_agent(state: &AppState, args: &[serde_json::Value]) -> Reply {
    let block_id: String = arg(args, 0)?;
    let stop_background: bool = service::get_arg(args, 1).unwrap_or(false);
    match crate::sagas::self_quit::run_shutdown(state, &block_id, crate::sagas::self_quit::QuitOrigin::UserMenu, stop_background).await {
        Ok(summary) => Ok(WebReturnType::success(serde_json::to_value(&summary).unwrap_or_default())),
        Err(reason) => Ok(WebReturnType::error(reason)),
    }
}

// What a shutdown would end, for the confirm; nothing is ended.
pub(super) async fn shutdown_preview(state: &AppState, args: &[serde_json::Value]) -> Reply {
    let block_id: String = arg(args, 0)?;
    let preview = crate::sagas::self_quit::preview(state, &block_id).await;
    Ok(WebReturnType::success(serde_json::to_value(&preview).unwrap_or_default()))
}

// A pane's × — every block in the pane's stack, stopped before its
// records are deleted (SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN_2026_09_18.md).
// Closing one tab of a stack sends that one id; the saga keeps the
// pane when other members survive.
pub(super) async fn close_pane(state: &AppState, args: &[serde_json::Value]) -> Reply {
    let block_ids: Vec<String> = arg(args, 0)?;
    // Optional: the frontend kept the pane on screen to show the
    // shutdown log (SPEC_AGENT_SELF_QUIT_2026_09_24.md §5.5), so srv
    // removes it member by member. Absent = the historical flow.
    let frontend_waits: bool = service::get_arg(args, 1).unwrap_or(false);
    let opts = crate::sagas::close_pane::CloseOpts { frontend_waits, ..Default::default() };
    if let Err(reason) = crate::sagas::close_pane::run_with(state, block_ids, opts).await {
        return Err(WebReturnType::error(reason));
    }
    Ok(WebReturnType::success_empty())
}

// `layout.applyIntent` for callers outside a window's websocket (the
// tools/tests harnesses): the same srv edit, the same stored row.
pub(super) async fn apply_layout_intent(state: &AppState, args: &[serde_json::Value]) -> Reply {
    let tab_id: String = arg(args, 0)?;
    let intent: agentmux_common::LayoutIntent = arg(args, 1)?;
    let action_id: String = arg(args, 2)?;
    match super::super::layout_intents::apply_layout_intent(state, &tab_id, intent, &action_id, None).await {
        Ok(outcome) => Ok(WebReturnType::success(serde_json::json!({
            "applied": outcome.applied,
            "error": outcome.error,
            "version": outcome.version,
        }))),
        Err(e) => Ok(WebReturnType::error(e)),
    }
}

pub(super) fn update_object_row(state: &AppState, args: &[serde_json::Value]) -> Reply {
    let mux_obj_value: serde_json::Value = arg(args, 0)?;
    // Whole-row replace, only for the client row: every other row is
    // the reducer's (`update_object` says which RPCs change it).
    match update_object(&state.mstore, mux_obj_value) {
        Ok((otype, oid, obj_val)) => {
            let update = MuxObjUpdate { updatetype: "update".into(), otype, oid, obj: Some(obj_val) };
            Ok(WebReturnType::success_with_updates(vec![update]))
        }
        Err(e) => Ok(WebReturnType::error(e)),
    }
}

/// The reducer command that patches the meta of the object `oref` names, or `None` for an object
/// type the reducer doesn't own.
fn meta_command(oref: &crate::backend::ORef, meta_value: serde_json::Value) -> Option<Command> {
    Some(match oref.otype.as_str() {
        t if t == OTYPE_WORKSPACE => Command::UpdateWorkspaceMeta { workspace_id: oref.oid.clone(), meta_patch: meta_value },
        t if t == OTYPE_TAB => Command::UpdateTabMeta { tab_id: oref.oid.clone(), meta_patch: meta_value },
        t if t == OTYPE_BLOCK => Command::UpdateBlockMeta { block_id: oref.oid.clone(), meta_patch: meta_value },
        t if t == OTYPE_WINDOW => Command::UpdateWindowMeta { window_id: oref.oid.clone(), meta_patch: meta_value },
        _ => return None,
    })
}

/// Per-agent zoom persistence (SPEC_AGENT_ZOOM_PERSISTENCE): when an
/// agent block's `term:zoom` changes, mirror it (debounced) into the
/// agent's per-agent `ui:zoom` content so the zoom survives the block
/// and is restored at `agent.open`. Only blocks carrying `agentId`
/// (agent panes) participate; everything else is untouched. A `null`
/// `term:zoom` (the frontend's reset-to-1.0 convention) deletes the
/// saved value so a default agent stores nothing.
fn mirror_agent_zoom(state: &AppState, oref: &crate::backend::ORef, meta_update: &MetaMapType) {
    if oref.otype == OTYPE_BLOCK && meta_update.contains_key("term:zoom") {
        if let Ok(block) = state.mstore.must_get::<Block>(&oref.oid) {
            let agent_id = block.meta.get("agentId").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if !agent_id.is_empty() {
                let zoom = meta_update.get("term:zoom").and_then(|v| v.as_f64());
                schedule_agent_zoom_mirror(state.mstore.clone(), agent_id, zoom);
            }
        }
    }
}

/// The reply that returns the updated block or tab so the frontend MOS cache stays in sync.
fn updated_row_reply(state: &AppState, oref: &crate::backend::ORef) -> WebReturnType {
    if oref.otype == OTYPE_BLOCK {
        if let Ok(block) = state.mstore.must_get::<Block>(&oref.oid) {
            return WebReturnType::success_with_updates(vec![row_update(OTYPE_BLOCK, &oref.oid, mux_obj_to_value(&block))]);
        }
    }
    if oref.otype == OTYPE_TAB {
        if let Ok(tab) = state.mstore.must_get::<Tab>(&oref.oid) {
            return WebReturnType::success_with_updates(vec![row_update(OTYPE_TAB, &oref.oid, mux_obj_to_value(&tab))]);
        }
    }
    WebReturnType::success_empty()
}

// Phase E.5.3 — UpdateObjectMeta migrated through the
// reducer. Decomposes by otype to the typed Update*Meta
// command. Reducer is pass-through (validates entity exists;
// emits event); subscriber's apply_*_meta_updated does the
// shallow merge against mstore.
pub(super) async fn update_object_meta_patch(state: &AppState, args: &[serde_json::Value]) -> Reply {
    let oref_str: String = arg(args, 0)?;
    let meta_update: MetaMapType = arg(args, 1)?;
    let oref = match crate::backend::ORef::parse(&oref_str) {
        Ok(v) => v,
        Err(e) => return Err(WebReturnType::error(e.to_string())),
    };
    if let Err(e) = crate::server::browser_owner::guard_client_meta_write(&oref_str, &meta_update) {
        return Err(WebReturnType::error(e));
    }
    let meta_value = serde_json::to_value(&meta_update).unwrap_or(serde_json::Value::Null);
    let Some(cmd) = meta_command(&oref, meta_value) else {
        // Remaining otypes (Layout, Client, Temp) aren't
        // meta-mutated via the reducer yet; fall back to
        // wcore for forward-compat. They publish no event,
        // so the MuxObjUpdate bridge can't see them — the
        // frontend cache stays stale until next bootstrap
        // (deemed acceptable since these aren't user-edited).
        // Future Phase E.5.x migrations can add reducer arms
        // for any of these following the OTYPE_WINDOW pattern
        // in `meta_command` (per issue #855 retro).
        return match update_object_meta(&state.mstore, &oref_str, &meta_update) {
            Ok(()) => Ok(WebReturnType::success_empty()),
            Err(e) => Ok(WebReturnType::error(format!(
                "UpdateObjectMeta: unsupported otype {} via reducer; wcore fallback failed: {}",
                oref.otype, e
            ))),
        };
    };
    let events = dispatch_to_reducer(state, cmd).await;
    if let Some(err_msg) = reducer_error(&events) {
        return Err(WebReturnType::error(err_msg));
    }
    if let Err(e) = apply_events(state, &events) {
        return Err(WebReturnType::error(format!("UpdateObjectMeta: SQLite write failed: {}", e)));
    }
    publish_events(state, &events);
    mirror_agent_zoom(state, &oref, &meta_update);
    Ok(updated_row_reply(state, &oref))
}

// Phase E.5.3 — UpdateTabName migrated through the reducer.
pub(super) async fn update_tab_name(state: &AppState, args: &[serde_json::Value]) -> Reply {
    let tab_id: String = arg(args, 0)?;
    let name: String = arg(args, 1)?;
    let events = dispatch_to_reducer(state, Command::RenameTab { tab_id: tab_id.clone(), name }).await;
    if let Some(err_msg) = reducer_error(&events) {
        return Err(WebReturnType::error(err_msg));
    }
    if let Err(e) = apply_events(state, &events) {
        return Err(WebReturnType::error(format!("UpdateTabName: SQLite write failed: {}", e)));
    }
    publish_events(state, &events);
    if let Ok(updated_tab) = state.mstore.must_get::<Tab>(&tab_id) {
        return Ok(WebReturnType::success_with_updates(vec![row_update(OTYPE_TAB, &tab_id, mux_obj_to_value(&updated_tab))]));
    }
    Ok(WebReturnType::success_empty())
}
