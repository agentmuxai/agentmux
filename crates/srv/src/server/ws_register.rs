// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The commands every WebSocket connection registers, in the order `register_handlers` adds them.
//! Moved out of `websocket.rs` unchanged; each command with a body of its own is a named handler.

use std::sync::Arc;

use crate::backend::background_task_feed::publish_background_task_updated;
use crate::backend::blockcontroller;
use crate::backend::base::normalize_working_dir;
use crate::backend::obj::{Block, MuxObjUpdate, mux_obj_to_value};
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    AgentShutdownKeepResult, AiRateLimitResult, AppInfoResult, CommandAgentAnswerData, CommandAgentCancelData,
    CommandAgentShutdownKeepData, CommandAmbientNarrateData, CommandBackgroundTaskCompletionData,
    CommandBackgroundTaskPidData, CommandBlockInputData, CommandControllerResyncData, CommandCreateSubBlockData,
    CommandDeleteSubBlockData, CommandDockNodeStatusData, CommandEventReadHistoryData, CommandGetMetaData,
    CommandListBackgroundTasksData, CommandSetMetaData, CommandToolDecisionData, NoArgsReq, RpcContext,
    COMMAND_AGENT_ANSWER, COMMAND_AGENT_CANCEL, COMMAND_AGENT_SHUTDOWN_KEEP, COMMAND_AMBIENT_NARRATE, COMMAND_APP_INFO,
    COMMAND_BACKGROUND_TASK_COMPLETION, COMMAND_BACKGROUND_TASK_PID, COMMAND_CONTROLLER_INPUT,
    COMMAND_CONTROLLER_RESYNC, COMMAND_CREATE_SUB_BLOCK, COMMAND_DELETE_SUB_BLOCK, COMMAND_DOCK_NODE_STATUS,
    COMMAND_EVENT_READ_HISTORY, COMMAND_EVENT_SUB, COMMAND_EVENT_UNSUB, COMMAND_EVENT_UNSUB_ALL,
    COMMAND_GET_AI_RATE_LIMIT, COMMAND_GET_FULL_CONFIG, COMMAND_GET_META, COMMAND_LIST_BACKGROUND_TASKS,
    COMMAND_ROUTE_ANNOUNCE, COMMAND_ROUTE_UNANNOUNCE, COMMAND_SET_CONFIG, COMMAND_SET_META, COMMAND_TOOL_DECISION,
};
use super::service::{schedule_agent_zoom_mirror, update_object_meta};
use super::websocket::{parse_block_input, with_question_controller};
use super::AppState;

/// How far back the fleet list (`COMMAND_LIST_BACKGROUND_TASKS` with an empty
/// `blockid`) reaches for tasks that already ended. Comfortably longer than the
/// longest finished-row retention the renderer applies (15 s for an error), so
/// a finished row is still in the list for as long as it is shown.
const FLEET_ENDED_WINDOW_MS: i64 = 60_000;

pub(super) fn register_handlers(engine: &Arc<WshRpcEngine>, state: AppState, conn_id: String) {
    register_connection_and_config(engine, &state, &conn_id);
    register_object_meta(engine, &state);
    register_block_controller(engine, &state);
    register_tool_decision_and_narration(engine, &state);
    register_background_tasks(engine, &state);
    register_background_task_list(engine, &state);
    register_agent_questions(engine, &state);
    register_feature_handlers(engine, &state);
    register_event_history_and_settings(engine, &state);
    register_service_handlers(engine, &state, &conn_id);
}

/// The connection itself: the full config, route announcements and the event subscriptions.
fn register_connection_and_config(engine: &Arc<WshRpcEngine>, state: &AppState, conn_id: &str) {
    let conn_id = conn_id.to_string();
    // getfullconfig → return full config as JSON
    // The RESPONSE stays `serde_json::Value`, deliberately. Redaction runs on
    // the serialized form (`redact_full_config_for_renderer` walks the JSON and
    // strips secrets), so what goes on the wire is NOT `FullConfigType` -- it
    // is `FullConfigType` minus fields that a type would still claim are
    // there. Generating `FullConfigType` here would produce a binding that
    // promises the renderer secrets it will never receive, which is worse
    // than `unknown`.
    let s = state.clone();
    engine.register_typed(
        COMMAND_GET_FULL_CONFIG,
        move |_req: Option<NoArgsReq>, _ctx| {
            let s = s.clone();
            async move { get_full_config(&s) }
        },
    );

    // routeannounce → log + no-op (fire-and-forget, may have no reqid)
    // `Value` request: fire-and-forget, may arrive with no reqid and any
    // payload, and it only logs. A struct would start rejecting shapes this has
    // always accepted.
    engine.register_typed(
        COMMAND_ROUTE_ANNOUNCE,
        |data: serde_json::Value, _ctx| {
            async move {
                tracing::debug!("routeannounce: {:?}", data);
                Ok(())
            }
        },
    );

    // routeunannounce → no-op
    engine.register_typed(
        COMMAND_ROUTE_UNANNOUNCE,
        |_data: serde_json::Value, _ctx| async move { Ok(()) },
    );

    // eventsub → register subscription with the MPS broker
    let broker_sub = state.broker.clone();
    let conn_id_sub = conn_id.clone();
    engine.register_typed(
        COMMAND_EVENT_SUB,
        move |sub: crate::backend::mps::SubscriptionRequest, _ctx| {
            let broker = broker_sub.clone();
            let conn_id = conn_id_sub.clone();
            async move {
                tracing::debug!("eventsub: event={} scopes={:?} allscopes={}", sub.event, sub.scopes, sub.allscopes);
                broker.subscribe(&conn_id, sub);
                Ok(())
            }
        },
    );

    // eventunsub → unsubscribe from the MPS broker
    let broker_unsub = state.broker.clone();
    let conn_id_unsub = conn_id.clone();
    // The request is a BARE STRING, not an object -- the only command in the
    // app shaped that way, which is exactly the sort of thing a hand-written
    // stub gets wrong. Typing it as `String` makes the generated binding say
    // so. `Option<String>` rather than `String` because the old handler
    // answered a non-string payload with "" (no-op) rather than an error, and
    // that tolerance is cheap to keep.
    engine.register_typed(
        COMMAND_EVENT_UNSUB,
        move |data: serde_json::Value, _ctx| {
            let broker = broker_unsub.clone();
            let conn_id = conn_id_unsub.clone();
            async move {
                // `as_str().unwrap_or("")` -- byte-identical to the untyped
                // handler. A number/object/array is a silent no-op, as before,
                // NOT a deserialize error.
                let event_name = data.as_str().unwrap_or("").to_string();
                if !event_name.is_empty() {
                    broker.unsubscribe(&conn_id, &event_name);
                }
                Ok(())
            }
        },
    );

    // eventunsuball → unsubscribe all from the MPS broker
    let broker_unsub_all = state.broker.clone();
    let conn_id_unsub_all = conn_id.clone();
    engine.register_typed(
        COMMAND_EVENT_UNSUB_ALL,
        move |_req: Option<NoArgsReq>, _ctx| {
            let broker = broker_unsub_all.clone();
            let conn_id = conn_id_unsub_all.clone();
            async move {
                broker.unsubscribe_all(&conn_id);
                Ok(())
            }
        },
    );
}

/// `getfullconfig`: the config as JSON, with the secrets the renderer must not see removed.
fn get_full_config(state: &AppState) -> Result<serde_json::Value, String> {
    let cw = state.config_watcher.clone();
    let config = cw.get_full_config();
    let mut v = serde_json::to_value(config.as_ref())
        .map_err(|e| format!("failed to serialize config: {}", e))?;
    crate::backend::wconfig::redact_full_config_for_renderer(&mut v);
    Ok(v)
}

/// `setmeta` and `getmeta` on wave objects, and the two small info commands.
fn register_object_meta(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // setmeta → update object metadata in the DB, broadcast update event
    // `Value` request, parsed into its struct below (which uses the hand-written ORef/MetaType); the stub keeps its
    // hand-written request type (generated-manifest.json `dataType`).
    let s = state.clone();
    engine.register_typed(COMMAND_SET_META, move |data: serde_json::Value, _ctx| {
        let s = s.clone();
        async move { set_meta(&s, data) }
    });

    // getmeta → return metadata for a wave object
    // `Value` request and response: the meta map is what the frontend models as
    // MetaType (generated-manifest.json `dataType` / `returns`).
    let s = state.clone();
    engine.register_typed(COMMAND_GET_META, move |data: serde_json::Value, _ctx| {
        let s = s.clone();
        async move { get_meta(&s, data) }
    });

    // waveinfo → return version and build info
    let version_info = state.version.clone();
    engine.register_typed(
        COMMAND_APP_INFO,
        move |_req: Option<NoArgsReq>, _ctx| {
            let version = version_info.clone();
            async move { Ok(AppInfoResult { version }) }
        },
    );

    // getwaveairatelimit → AgentMux has no rate limits; return unlimited/unknown
    engine.register_typed(
        COMMAND_GET_AI_RATE_LIMIT,
        |_req: Option<NoArgsReq>, _ctx| async move {
            Ok(AiRateLimitResult {
                req: 9999,
                reqlimit: 9999,
                preq: 9999,
                preqlimit: 9999,
                resetepoch: 0,
                unknown: true,
            })
        },
    );
}

/// `setmeta`: update an object's metadata, then tell every client about the new block.
fn set_meta(state: &AppState, data: serde_json::Value) -> Result<(), String> {
    let mstore = state.mstore.clone();
    let event_bus = state.event_bus.clone();
    let cmd: CommandSetMetaData =
        serde_json::from_value(data).map_err(|e| format!("setmeta: {e}"))?;
    let oref_str = cmd.oref.to_string();
    let meta_keys: Vec<&String> = cmd.meta.keys().collect();
    // debug, not info: fires on every metadata write (zoom, title,
    // view-state edits); at info this became a meaningful slice of
    // an unrotated launcher-log mirror (SPEC_WIN10_PAGEFILE_OOM_
    // CRASH_2026_06_29 P1). Default production filter is info, so
    // this is now suppressed unless RUST_LOG=debug is set.
    tracing::debug!(oref = %oref_str, keys = ?meta_keys, "SetMeta");
    crate::server::browser_owner::guard_client_meta_write(&oref_str, &cmd.meta)?;
    update_object_meta(&mstore, &oref_str, &cmd.meta)?;
    // Per-agent zoom persistence (SPEC_AGENT_ZOOM_PERSISTENCE): the
    // frontend writes term:zoom via this WebSocket path, not the HTTP
    // UpdateObjectMeta handler where the mirror was originally placed.
    // Mirror term:zoom → ui:zoom here so the zoom survives pane close.
    let oref_parsed = crate::backend::ORef::parse(&oref_str)
        .map_err(|e| e.to_string())?;
    if oref_parsed.otype == "block" && cmd.meta.contains_key("term:zoom") {
        if let Ok(block) = mstore.must_get::<Block>(&oref_parsed.oid) {
            let agent_id = block.meta.get("agentId")
                .and_then(|v| v.as_str()).unwrap_or("").to_string();
            if !agent_id.is_empty() {
                let zoom = cmd.meta.get("term:zoom").and_then(|v| v.as_f64());
                schedule_agent_zoom_mirror(mstore.clone(), agent_id, zoom);
            }
        }
    }
    // Read the updated object and broadcast a proper MuxObjUpdate
    // so all WS clients refresh their atoms with the new data.
    let oref = crate::backend::ORef::parse(&oref_str)
        .map_err(|e| e.to_string())?;
    // Only a block's update carries its object; any other went out with no data,
    // which the frontend ignores, so it isn't sent.
    if oref.otype == "block" {
        if let Ok(block) = mstore.must_get::<Block>(&oref.oid) {
            let update = MuxObjUpdate {
                updatetype: "update".into(),
                otype: oref.otype.clone(),
                oid: oref.oid.clone(),
                obj: Some(mux_obj_to_value(&block)),
            };
            event_bus.broadcast_event(&crate::backend::eventbus::EventBus::typed_ws_event::<crate::backend::mps_events::MuxObjUpdated>(&update, oref_str));
        }
    }
    Ok(())
}

/// `getmeta`: an object's metadata (the whole object when it has no `meta` field).
fn get_meta(state: &AppState, data: serde_json::Value) -> Result<serde_json::Value, String> {
    let mstore = state.mstore.clone();
    let cmd: CommandGetMetaData =
        serde_json::from_value(data).map_err(|e| format!("getmeta: {e}"))?;
    let obj: Option<serde_json::Value> = mstore
        .get_raw(&cmd.oref.otype, &cmd.oref.oid)
        .map_err(|e| format!("getmeta: {e}"))?;
    match obj {
        Some(val) => {
            // Return the "meta" field if present, otherwise the full object
            let meta = val.get("meta").cloned().unwrap_or(val);
            Ok(meta)
        }
        None => Err(format!("getmeta: object {} not found", cmd.oref)),
    }
}

/// Starting, resyncing and feeding block controllers, and the headless sub-blocks under a block.
fn register_block_controller(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // controllerresync → load block from DB, create/restart controller with PTY
    // `Value` request, parsed into its struct below (termsize and the rest); the stub keeps its
    // hand-written request type (generated-manifest.json `dataType`).
    let s = state.clone();
    engine.register_typed(COMMAND_CONTROLLER_RESYNC, move |data: serde_json::Value, _ctx| {
        let s = s.clone();
        async move { controller_resync(&s, data) }
    });

    // controllerinput → route keyboard input / signals / resize to block controller
    //
    // Drops input for a block currently under an active PtyShell agent lock
    // (`term:agentlockuntil`, docs/specs/SPEC_AGENT_INTERACTIVE_PTY_SHELL_API_2026_09_10.md
    // §10.3) rather than forwarding it — Codex P1 on PR #3194: the
    // frontend's own gate (`AgentShellSubblock.tsx` dropping its
    // `sendDataHandler` calls while locked) is necessarily eventually-
    // consistent — it reacts to a WS-pushed meta value, not something
    // synchronous with the backend's own state — so a human keystroke
    // already in flight when the lock is set could otherwise still reach
    // `send_input` and interleave with the agent's own write, exactly the
    // collision the lease exists to prevent. This check, made at the
    // moment this handler actually runs against the backend's own current
    // state, is the real enforcement; the frontend gate is the UX layer
    // that keeps it from happening in the first place, not the
    // correctness boundary.
    //
    // Keep this check I/O-free. It reads `blockcontroller::agent_lock`, an
    // in-memory map, NOT the block's persisted meta: the original revision
    // called `mstore.get::<Block>()` here, putting a synchronous SQLite
    // query behind `Store`'s single process-wide `Mutex<Connection>` inside
    // an async handler, inline on a Tokio worker thread with no
    // `block_in_place` — the same failure class as the sysinfo incident
    // (#1782). This handler carries the Stop button's SIGINT and
    // `usePtyWidth`'s debounced resizes (which burst while a pane is being
    // dragged); it is NOT the keystroke path — those go through the
    // `blockinput` arm below, which has the same lock check via
    // `dispatch_blockinput` (closed 2026-09-16, previously `agent_lock`'s
    // documented "Known gap" — see its module doc history). See also
    // docs/analysis/ANALYSIS_CROSS_PANE_INPUT_DELAY_REGRESSION_2026_09_15.md.
    engine.register_typed(
        COMMAND_CONTROLLER_INPUT,
        move |cmd: CommandBlockInputData, _ctx| {
            async move {
                if blockcontroller::agent_lock::is_locked(&cmd.blockid) {
                    tracing::debug!(block_id = %cmd.blockid, "controllerinput: dropped — agent lock active");
                    return Ok(());
                }
                let input = parse_block_input(&cmd)?;
                blockcontroller::send_input(&cmd.blockid, input, cmd.seq)?;
                Ok(())
            }
        },
    );

    // createsubblock → create a headless sub-block (no tab/layout entry)
    // parented to an existing block, e.g. a `term`-view PTY embedded in an
    // agent pane's details drawer.
    // `Value` request, parsed into its struct below (a BlockDef); the stub keeps its
    // hand-written request type (generated-manifest.json `dataType`).
    let s = state.clone();
    engine.register_typed(COMMAND_CREATE_SUB_BLOCK, move |data: serde_json::Value, _ctx| {
        let s = s.clone();
        async move { create_sub_block(&s, data) }
    });

    // deletesubblock → tear down a sub-block created via createsubblock.
    // Kill-first ordering matches sagas/delete_block.rs; unlike that saga,
    // this does NOT touch tab bookkeeping — sub-blocks are never
    // tab-referenced, so delete_block.rs's precondition would reject them.
    let s = state.clone();
    engine.register_typed(
        COMMAND_DELETE_SUB_BLOCK,
        move |cmd: CommandDeleteSubBlockData, _ctx| {
            let s = s.clone();
            async move { delete_sub_block(&s, cmd) }
        },
    );
}

/// `controllerresync`: load the block from the store and create or restart its controller.
fn controller_resync(state: &AppState, data: serde_json::Value) -> Result<(), String> {
    let mstore = state.mstore.clone();
    let broker = state.broker.clone();
    let event_bus = state.event_bus.clone();
    let filestore = state.filestore.clone();
    let id_store = state.id_store.clone();
    let identity_store = state.identity_store.clone();
    let boot_id = state.boot_id.clone();
    let auth_key = state.auth_key.clone();
    let config_watcher = state.config_watcher.clone();
    let cmd: CommandControllerResyncData = serde_json::from_value(data)
        .map_err(|e| format!("controllerresync: {e}"))?;
    tracing::info!(
        block_id = %cmd.blockid,
        tab_id = %cmd.tabid,
        forcerestart = cmd.forcerestart,
        norespawn = cmd.norespawn,
        "ControllerResync"
    );
    let block: Block = mstore
        .get(&cmd.blockid)
        .map_err(|e| format!("controllerresync: load block: {e}"))?
        .ok_or_else(|| format!("controllerresync: block {} not found", cmd.blockid))?;
    let registry = mstore.shared_agent_registry();
    blockcontroller::resync_controller(
        &block,
        &cmd.tabid,
        cmd.rtopts,
        cmd.forcerestart,
        // respawn_if_done — see `norespawn`'s own doc comment.
        // Default (false → true here) preserves TermResyncHandler's
        // crash-recovery behaviour for every existing caller.
        !cmd.norespawn,
        Some(broker),
        Some(event_bus),
        Some(mstore),
        Some(filestore),
        Some(id_store),
        Some(identity_store),
        registry,
        boot_id,
        &auth_key,
        Some(config_watcher),
    )?;
    Ok(())
}

/// `createsubblock`: a headless sub-block parented to an existing block; resolves to its `block:<id>`.
fn create_sub_block(state: &AppState, data: serde_json::Value) -> Result<String, String> {
    let mstore = state.mstore.clone();
    let cmd: CommandCreateSubBlockData = serde_json::from_value(data)
        .map_err(|e| format!("createsubblock: {e}"))?;

    let mut meta = cmd.blockdef.meta.clone();
    let raw_cwd = meta
        .get(blockcontroller::META_KEY_CMD_CWD)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if let Some(raw_cwd) = raw_cwd {
        // The PTY spawn path (blockcontroller/shell/lifecycle.rs)
        // reads cmd:cwd raw with no MSYS→Windows conversion, unlike
        // shellexec — normalize here so a Git-Bash-style path
        // doesn't hit os error 267 on Windows. Mirrors
        // server/mod.rs's shellexec cwd-fallback precedent: an
        // invalid/non-absolute value degrades to no cwd rather
        // than failing the whole call — this is a best-effort
        // convenience derived from the parent's meta, not a
        // caller-supplied value worth hard-rejecting.
        match normalize_working_dir(&raw_cwd).filter(|p| std::path::Path::new(p).is_absolute()) {
            Some(norm) => {
                meta.insert(
                    blockcontroller::META_KEY_CMD_CWD.to_string(),
                    serde_json::Value::String(norm),
                );
            }
            None => {
                tracing::warn!(
                    raw_cwd = %raw_cwd,
                    "createsubblock: invalid or non-absolute cmd:cwd, dropping (no cwd)"
                );
                meta.remove(blockcontroller::META_KEY_CMD_CWD);
            }
        }
    }

    let child_id = uuid::Uuid::new_v4().to_string();
    let mut block = Block {
        oid: child_id.clone(),
        parentoref: format!("block:{}", cmd.parentblockid),
        meta,
        ..Default::default()
    };
    mstore
        .insert(&mut block)
        .map_err(|e| format!("createsubblock: insert: {e}"))?;

    // Best-effort link into the parent's subblockids. Not a CAS
    // (Store::update is a blind version-bump) — an acceptable
    // race for a single lazily-created shell per agent pane.
    if let Ok(mut parent) = mstore.must_get::<Block>(&cmd.parentblockid) {
        parent
            .subblockids
            .get_or_insert_with(Vec::new)
            .push(child_id.clone());
        let _ = mstore.update(&mut parent);
    }

    tracing::info!(
        child_id = %child_id,
        parent_id = %cmd.parentblockid,
        "CreateSubBlock"
    );
    // TS `CreateSubBlockCommand` resolves `Promise<ORef>`, and
    // `ORef` (srv-types.d.ts) is a plain string — return the
    // bare "block:<id>" string, not a wrapper object.
    Ok(format!("block:{child_id}"))
}

/// `deletesubblock`: tear down a sub-block made by `createsubblock`.
fn delete_sub_block(state: &AppState, cmd: CommandDeleteSubBlockData) -> Result<(), String> {
    let mstore = state.mstore.clone();
    // Kill process FIRST — a lingering PTY tree is worse than a
    // delayed row delete.
    crate::sagas::agent_teardown::discard(&cmd.blockid);

    let parent_id = mstore
        .get::<Block>(&cmd.blockid)
        .ok()
        .flatten()
        .and_then(|b| b.parentoref.strip_prefix("block:").map(str::to_string));

    mstore
        .delete::<Block>(&cmd.blockid)
        .map_err(|e| format!("deletesubblock: delete: {e}"))?;

    // Best-effort unlink from the parent's subblockids — the
    // controller is already dead and the row is already gone
    // either way, so don't fail the call over this step.
    if let Some(parent_id) = parent_id {
        if let Ok(mut parent) = mstore.must_get::<Block>(&parent_id) {
            if let Some(ids) = parent.subblockids.as_mut() {
                ids.retain(|id| id != &cmd.blockid);
                let _ = mstore.update(&mut parent);
            }
        }
    }

    tracing::info!(block_id = %cmd.blockid, "DeleteSubBlock");
    Ok(())
}

/// `tooldecision` and the ambient narration line.
fn register_tool_decision_and_narration(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // tooldecision → reply to a per-tool-call permission gate via the Agent SDK
    // **control protocol** — the `AgentDecisionPanel` Allow/Deny buttons, once
    // `should_route_to_decision_panel` (persistent/mod.rs) is flipped on for real
    // (that gate is currently hardcoded false, so `decide_tool_permission`
    // below will fail every call today with "no pending tool-permission
    // request" — expected until the Phase 2 policy decision is made; see
    // that function's own doc comment).
    //
    // The original PR-3a draft tried to write `y\n` / `n\n` to the
    // subprocess's stdin via `blockcontroller::send_input`. Codex P1
    // on PR #557 caught that this would fail: `SubprocessController::
    // send_input` (and `PersistentSubprocessController::send_input`)
    // both reject raw `input_data` with an `Err`. The deeper truth is
    // that AgentMux runs the agent CLI in non-interactive `--print`
    // mode — the CLI shows no y/n prompt and stdin carries only the
    // structured message stream, so a y/n write would be meaningless even if
    // the controller accepted it. Real delivery is the same
    // control_response mechanism `agent.answer` uses below for
    // AskUserQuestion. See SPEC_DECISION_PROMPT_2026_04_24.md §9.1
    // and SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
    //
    // `cmd.request_id` is used here as the `tool_use_id` key
    // `pending_permissions`/`decide_tool_permission` are keyed on — NOT
    // as a separate correlator against the speculative
    // `PermissionRequestEvent` §4.1 describes, which was never built.
    // The eventual mechanism (still unbuilt — see the Phase 2 gate)
    // correlates a `pending_approval` ToolNode by `tool_use_id`, exactly
    // like AskUserQuestion's `awaiting_answer` already does; whatever
    // populates that field on the frontend side should set it to the
    // ToolNode's `tool_use_id`.
    //
    // `scope` is still validated but not yet acted on: §6's rules-
    // persistence layer (`permissions.json`) does not exist anywhere in
    // this tree. That is separate, still-open work — this PR only makes
    // the *transport* (control_response delivery) real, not repeat-
    // decision memory.
    engine.register_typed(
        COMMAND_TOOL_DECISION,
        move |cmd: CommandToolDecisionData, _ctx| async move { tool_decision(cmd) },
    );

    // Ambient narration — generate a short user-facing line about something
    // AgentMux just did on its own, and broadcast it to the pane.
    //
    // Fire-and-forget in both directions: the renderer does not await a reply,
    // and the work runs in a detached task so a slow or capped model call never
    // holds the RPC open. Every failure path is silence — the narrated action
    // has already happened and the UI already reflects it, so a missing line is
    // a missing nicety, not a missing state change.
    let s = state.clone();
    engine.register_typed(
        COMMAND_AMBIENT_NARRATE,
        move |cmd: CommandAmbientNarrateData, _ctx| {
            let s = s.clone();
            async move { ambient_narrate(&s, cmd) }
        },
    );
}

/// `tooldecision`: deliver an Allow or Deny over the control protocol.
fn tool_decision(cmd: CommandToolDecisionData) -> Result<(), String> {
    match cmd.outcome.as_str() {
        "allow" | "deny" => {}
        other => {
            return Err(format!(
                "tooldecision: invalid outcome '{}' (expected 'allow' or 'deny')",
                other
            ));
        }
    }
    // Validate scope so a future rules-persistence layer (§6)
    // can trust the value without re-checking. Reagent P1
    // round-3 on PR #557.
    match cmd.scope.as_str() {
        "once" | "session" | "project" | "global" => {}
        other => {
            return Err(format!(
                "tooldecision: invalid scope '{}' (expected 'once'/'session'/'project'/'global')",
                other
            ));
        }
    }
    tracing::info!(
        block_id = %cmd.blockid,
        request_id = %cmd.request_id,
        outcome = %cmd.outcome,
        scope = %cmd.scope,
        has_feedback = cmd.feedback.is_some(),
        "[tooldecision] delivering via control protocol"
    );
    let ctrl = blockcontroller::get_controller(&cmd.blockid)
        .ok_or_else(|| format!("tooldecision: no controller for block {}", cmd.blockid))?;
    if let Some(persistent_ctrl) = ctrl
        .as_any()
        .downcast_ref::<blockcontroller::persistent::PersistentSubprocessController>()
    {
        persistent_ctrl.decide_tool_permission(cmd.request_id, &cmd.outcome, cmd.feedback)
    } else {
        Err("tooldecision: UNSUPPORTED_CONTROLLER: per-tool permission decisions \
             require a persistent (host) agent; container/one-shot agents are not \
             yet supported (same limitation as agent.answer)".to_string())
    }
}

/// `ambientnarrate`: generate a short line about something AgentMux just did and publish it, off the RPC.
fn ambient_narrate(state: &AppState, cmd: CommandAmbientNarrateData) -> Result<(), String> {
    let mstore = state.mstore.clone();
    let broker = state.broker.clone();
    let narrated = state.narrated_events.clone();
    // Dedupe per narrated event. A ToolNode can be re-observed, and
    // narrating one event twice is both noise and a wasted model
    // call. Checked before spawning so a repeat costs nothing.
    if !cmd.dedupe_key.is_empty() && !narrated.mark_new(&cmd.dedupe_key) {
        return Ok(());
    }

    tokio::spawn(async move {
        let generation = agentmux_common::time::now_ms_u64();
        let Some(text) = crate::ambient::tasks::generate_ambient_narration(
            &mstore,
            &cmd.blockid,
            &cmd.dedupe_key,
            generation,
            &cmd.kind,
            &cmd.context,
        )
        .await
        else {
            return;
        };
        // A short task can finish while the model call is still
        // in flight, so by now the dock may already show it done.
        // Narrating it at that point is worse than silence: the line
        // describes a launch the user has already seen conclude.
        //
        // Belt and braces with the prompt, which is phrased to stay
        // true after completion — this drops the message entirely
        // rather than relying on wording alone. (codex P2 on #3169.)
        //
        // Only skips on a DEFINITE completion: an unknown id (no
        // registry row — the task was never observed, or this is a
        // narration kind that has none) falls through and publishes,
        // since absence of a row is not evidence of completion.
        if !cmd.dedupe_key.is_empty() {
            if let Ok(Some(task)) = mstore.background_task_get(&cmd.dedupe_key) {
                if task.status != crate::backend::storage::background_tasks::BackgroundTaskStatus::Running {
                    return;
                }
            }
        }
        // Carries the text itself, unlike background-task-updated
        // next to it: there is no list query to re-read, so an
        // invalidation ping would have nothing to invalidate.
        let narration = crate::backend::mps_events::AmbientNarrationEvent { block_id: cmd.blockid.clone(), kind: cmd.kind.clone(), text: text.to_string() };
        broker.publish_event::<crate::backend::mps_events::AmbientNarration>(&narration, vec![format!("block:{}", cmd.blockid)], 0);
    });
    Ok(())
}

/// Declared-background tasks: the dock push, completion, pid and the list.
fn register_background_tasks(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // docknodestatus → fire-and-forget push of a ToolNode's latest status,
    // cached in-memory per block for `muxspect dock` to read. See
    // docs/specs/SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06.md §3.1.
    //
    // Also mirrors a declared-background task's liveness into the durable
    // `db_background_tasks` registry (see
    // docs/status/STATUS_ATTACHED_TASK_AXIS_AND_DEV_LOOP_2026_08_15.md) —
    // `DockSnapshotCache` above is intentionally ephemeral (never persisted,
    // 1-hour eviction on read); the registry is the durable source of truth
    // consumers that need to survive a cache eviction or a session
    // reconnect (Swarm, #2492's teardown-survival) read from instead.
    //
    // `run_in_background == Some(true)` is ONLY ever sent once
    // `isAcceptedBackgroundLaunch()` holds client-side (`tool-adapter.ts`),
    // which by its own definition requires `status == "success"` — checking
    // `status == "running"` here (an earlier version of this handler did)
    // made the observe call unreachable, since the flag and that status
    // value are never sent together (reagentx P0 / codex P1 on PR #2590).
    // Best-effort: a registry write failure never blocks the dock push it
    // rides alongside — `dock_snapshots.push_delta` below already gives the
    // user the live UI signal regardless. Completion is NOT detected here —
    // see `COMMAND_BACKGROUND_TASK_COMPLETION` below for why this handler
    // structurally can't see it (the originating ToolNode's status never
    // changes again after acceptance).
    let s = state.clone();
    engine.register_typed(
        COMMAND_DOCK_NODE_STATUS,
        move |cmd: CommandDockNodeStatusData, _ctx| {
            let s = s.clone();
            async move { dock_node_status(&s, cmd) }
        },
    );

    // backgroundtaskcompletion → fire-and-forget push of a declared-
    // background task's REAL terminal outcome, parsed client-side from its
    // `<task-notification>` message (`tool-adapter.ts`'s
    // `parseTaskNotification`). This is a separate command from
    // `docknodestatus` above on purpose: the originating ToolNode's own
    // `status` field goes "success" at acceptance and never changes again
    // (that's the raw tool_result's outcome, not the background task's) —
    // there is no later `docknodestatus` push this handler could key off of
    // to learn a background task actually finished (reagentx P0 / codex P1
    // on PR #2590, corrected here). Routing this through `docknodestatus`
    // instead of a dedicated command would also corrupt `DockSnapshotCache`:
    // `push_delta` fully overwrites a node's snapshot, and this event has no
    // `tool_name`/original `run_in_background` value to carry forward — the
    // exact bug class #2520 already fixed once for a different call site.
    let s = state.clone();
    engine.register_typed(
        COMMAND_BACKGROUND_TASK_COMPLETION,
        move |cmd: CommandBackgroundTaskCompletionData, _ctx| {
            let s = s.clone();
            async move { background_task_completion(&s, cmd) }
        },
    );

    // backgroundtaskpid → fire-and-forget push of a declared-background
    // task's real OS pid, relayed from `agentmux-bashwrap`'s own MPS "pid"
    // chunk. Closes the gap where `db_background_tasks.pid` existed but
    // nothing in production ever wrote it (Phase A of
    // docs/specs/SPEC_BACKGROUND_TASK_PID_CAPTURE_2026_08_20.md). Best-effort,
    // same as the two handlers above: a write failure here doesn't block
    // anything else and is only ever a diagnostic/teardown-survival input,
    // never something the model-visible tool_result depends on.
    //
    // bashwrap publishes this essentially at process start — routinely
    // BEFORE `COMMAND_DOCK_NODE_STATUS` above has created this task's
    // `db_background_tasks` row (that requires a full round-trip through
    // the frontend recognizing an accepted background launch first).
    // `background_task_set_pid` silently no-ops on a missing row
    // (`Ok(false)`), which would lose the pid permanently with no way to
    // retry a write that will never succeed — stash it instead, and
    // `COMMAND_DOCK_NODE_STATUS`'s handler applies it the moment the row
    // exists. See Codex/reagentx findings on PR #2681.
    let s = state.clone();
    engine.register_typed(
        COMMAND_BACKGROUND_TASK_PID,
        move |cmd: CommandBackgroundTaskPidData, _ctx| {
            let s = s.clone();
            async move { background_task_pid(&s, cmd) }
        },
    );

}

/// `listbackgroundtasks`.
fn register_background_task_list(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // listbackgroundtasks → request/response: this block's current
    // db_background_tasks rows, so the frontend can seed its attachedTask
    // axis from the durable registry on mount/reconnect instead of only
    // ever re-deriving it from this tab's own live transcript replay
    // (which has no way to know about a task that survived a session
    // restart under a controller generation with no transcript history of
    // ever launching it — Phase B of
    // docs/specs/SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL_2026_08_20.md).
    // See docs/specs/SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20.md §3.1.
    let s = state.clone();
    engine.register_typed(
        COMMAND_LIST_BACKGROUND_TASKS,
        move |cmd: CommandListBackgroundTasksData, _ctx| {
            let s = s.clone();
            async move { list_background_tasks(&s, cmd) }
        },
    );
}

/// `docknodestatus`: cache a ToolNode's latest status, and record a declared-background task durably.
fn dock_node_status(state: &AppState, cmd: CommandDockNodeStatusData) -> Result<(), String> {
    let dock_snapshots = state.dock_snapshots.clone();
    let mstore = state.mstore.clone();
    let broker = state.broker.clone();
    let observed_at = agentmux_common::time::now_ms();

    if cmd.run_in_background == Some(true) {
        match mstore.background_task_observe(
            &cmd.node_id,
            &cmd.blockid,
            &cmd.tool_name,
            observed_at,
            observed_at,
        ) {
            Ok(()) => publish_background_task_updated(&broker, &cmd.blockid),
            Err(e) => tracing::warn!(
                target: "background_tasks",
                node_id = %cmd.node_id,
                error = %e,
                "failed to observe declared-background task in the durable registry",
            ),
        }
        // bashwrap's own pid publish (COMMAND_BACKGROUND_TASK_PID
        // below) routinely races ahead of the observe call above —
        // it fires at bashwrap's process start, before the frontend
        // even sees an accepted-background tool result. Check for
        // (and apply) a pid stashed ahead of this row's existence.
        // Runs unconditionally on observe failure too — a stashed
        // pid may still apply if the row already existed from an
        // earlier retry. Atomic with the pid handler's own
        // set_or_stash call for the same id — see
        // pending_background_pids.rs's module doc for why that
        // matters (Codex/reagentx findings on PR #2681).
        crate::backend::background_task_pid::apply_stashed(&mstore, &cmd.node_id, observed_at);
    }

    dock_snapshots.push_delta(
        &cmd.blockid,
        crate::backend::dock_snapshot::DockNodeSnapshot {
            node_id: cmd.node_id,
            tool_name: cmd.tool_name,
            status: cmd.status,
            timestamp: cmd.timestamp,
            observed_at,
            run_in_background: cmd.run_in_background,
        },
    );
    Ok(())
}

/// `backgroundtaskcompletion`: a declared-background task's real terminal outcome.
fn background_task_completion(state: &AppState, cmd: CommandBackgroundTaskCompletionData) -> Result<(), String> {
    let mstore = state.mstore.clone();
    let broker = state.broker.clone();
    let ended_at = cmd.timestamp.unwrap_or_else(|| {
        agentmux_common::time::now_ms()
    });
    let status = crate::backend::storage::background_tasks::BackgroundTaskStatus::from_str(&cmd.status);
    match mstore.background_task_complete(&cmd.node_id, status, ended_at) {
        Ok(_) => publish_background_task_updated(&broker, &cmd.blockid),
        Err(e) => tracing::warn!(
            target: "background_tasks",
            node_id = %cmd.node_id,
            error = %e,
            "failed to mark background task terminal in the durable registry",
        ),
    }
    Ok(())
}

/// `backgroundtaskpid`: a declared-background task's OS pid, stashed when its row does not exist yet.
fn background_task_pid(state: &AppState, cmd: CommandBackgroundTaskPidData) -> Result<(), String> {
    crate::backend::background_task_pid::file_pid(&state.mstore, Some(&state.broker), &cmd.blockid, &cmd.node_id, cmd.pid as i64, agentmux_common::time::now_ms());
    Ok(())
}

/// `listbackgroundtasks`: a block's rows from the durable registry, or the whole fleet for an empty block id.
fn list_background_tasks(state: &AppState, cmd: CommandListBackgroundTasksData) -> Result<Vec<super::muxspect_handlers::BackgroundTaskView>, String> {
    let mstore = state.mstore.clone();
    // An empty blockid asks for the whole fleet (Swarm): every
    // running task plus those that ended recently enough to still
    // show a finished row. SPEC_BACKGROUND_TASK_STRUCTURED_FEED_
    // AND_SWARM_OWNERSHIP_2026_09_27.md §3.
    let tasks = if cmd.blockid.is_empty() {
        let now_ms = agentmux_common::time::now_ms();
        mstore.background_task_list_fleet(now_ms - FLEET_ENDED_WINDOW_MS)
    } else {
        mstore.background_task_list_for_block(&cmd.blockid)
    }
    .map_err(|e| format!("listbackgroundtasks: {e}"))?;
    let views: Vec<super::muxspect_handlers::BackgroundTaskView> =
        tasks.into_iter().map(Into::into).collect();
    Ok(views)
}

/// Answering or declining an AskUserQuestion, and the pending-shutdown banner's "Keep running".
fn register_agent_questions(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // agent.answer → deliver an AskUserQuestion answer to the running agent CLI
    // via the Agent SDK **control protocol**: the persistent controller replies
    // to the CLI's parked `can_use_tool` control_request with a control_response
    // carrying `updatedInput.answers`. (Delivering a `tool_result` on stdin does
    // NOT work — the CLI auto-rejects AskUserQuestion within the turn; see spec
    // §2/§3.) Only the persistent controller speaks the control protocol;
    // container/one-shot subprocess agents return UNSUPPORTED_CONTROLLER (Phase 2).
    // Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
    // `Value` request, parsed into its struct below (an answer to a question); the stub keeps its
    // hand-written request type (generated-manifest.json `dataType`).
    engine.register_typed(COMMAND_AGENT_ANSWER, move |data: serde_json::Value, _ctx| async move { agent_answer(data) });

    // agent.cancel → a REAL protocol-level decline of a pending AskUserQuestion
    // (Cancel button / Escape in AgentQuestionPanel.tsx), not a UI-only dismiss.
    // Sends `behavior: "deny"` over the same control protocol agent.answer uses
    // for `behavior: "allow"` — see `deny_question`'s doc comment for why this
    // is a general, documented Agent SDK mechanism rather than something
    // special-cased for AskUserQuestion. Its error strings come from
    // `with_question_controller`, shared with agent.answer (`no controller for
    // block`, `UNSUPPORTED_CONTROLLER`): the frontend's SAFE_TO_RETRY_VIA_FOLLOWUP
    // allowlist (useAgentQuestions.ts) matches on these substrings for both
    // commands. Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
    engine.register_typed(
        COMMAND_AGENT_CANCEL,
        move |cmd: CommandAgentCancelData, _ctx| async move { agent_cancel(cmd) },
    );

    // The pending-shutdown banner's "Keep running"
    // (docs/specs/SPEC_AGENT_SELF_QUIT_2026_09_24.md §6.5).
    let state_keep = state.clone();
    engine.register_typed(
        COMMAND_AGENT_SHUTDOWN_KEEP,
        move |cmd: CommandAgentShutdownKeepData, _ctx| {
            let state = state_keep.clone();
            async move {
                let outcome = crate::sagas::pending_shutdown::keep(&state, &cmd.blockid, &cmd.request_id);
                Ok(AgentShutdownKeepResult { outcome: outcome.to_string() })
            }
        },
    );
}

/// `agent.answer`: deliver an AskUserQuestion answer over the control protocol.
fn agent_answer(data: serde_json::Value) -> Result<(), String> {
    let cmd: CommandAgentAnswerData = serde_json::from_value(data)
        .map_err(|e| format!("agent.answer: {e}"))?;
    with_question_controller("agent.answer", "answering", &cmd.blockid, &cmd.tool_use_id, |ctrl| {
        ctrl.answer_question(cmd.tool_use_id.clone(), cmd.answers)
    })?;
    tracing::info!(
        block_id = %cmd.blockid,
        tool_use_id = %cmd.tool_use_id,
        "[agent.answer] control_response delivered to persistent stdin"
    );
    Ok(())
}

/// `agent.cancel`: decline a pending AskUserQuestion over the control protocol.
fn agent_cancel(cmd: CommandAgentCancelData) -> Result<(), String> {
    with_question_controller("agent.cancel", "canceling", &cmd.blockid, &cmd.tool_use_id, |ctrl| {
        ctrl.deny_question(
            cmd.tool_use_id.clone(),
            blockcontroller::persistent::ASK_USER_QUESTION_DENY_MESSAGE.to_string(),
        )
    })?;
    tracing::info!(
        block_id = %cmd.blockid,
        tool_use_id = %cmd.tool_use_id,
        "[agent.cancel] deny control_response delivered to persistent stdin"
    );
    Ok(())
}

/// The features that register their own commands: agent input, shells, files, LSP, CLI, widgets, tools.
fn register_feature_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // Agent input/stop + subprocess spawn handlers
    super::agent_handlers::register_agent_input_handlers(engine, &state);

    // `/btw` slash command — one-shot, tool-less side question.
    super::agent_handlers::register_side_question_handlers(engine, &state);

    // Shell exec/stop handlers
    super::shell_handlers::register_shell_handlers(engine, &state);

    // Editor/file-ops + write_agent_config handlers
    super::editor_handlers::register_editor_handlers(engine, &state);

    // Files pane: fs.list/places/watch/unwatch + mutations
    // (SPEC_FILE_BROWSER_PANE_2026_10_01.md §6.3)
    super::fs_handlers::register_fs_handlers(engine, &state);

    // LSP handlers (lspstart, lspsend, lspstop)
    super::lsp_handlers::register_lsp_handlers(engine, &state);

    // CLI handlers (resolvecli, checkcliauth, runclilogin), then widget packages (widgets.*)
    super::cli_handlers::register_cli_handlers(engine, &state);
    super::widget_handlers::register_widget_handlers(engine, &state);

    // Tool store handlers (gettoolstatus, installtool)
    super::tool_handlers::register_tool_handlers(engine, &state);

    // Provider model catalog (providers.models → authoritative /v1/models list)
    super::providers_handlers::register_providers_handlers(engine, &state);

    // reactive.registrations → Stash "Registration" tab live status (#2696)
    super::reactive::register_reactive_ws_handlers(engine, &state);
}

/// `eventreadhistory` and `setconfig`.
fn register_event_history_and_settings(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // eventreadhistory → read persisted event history from the MPS broker
    let broker_history = state.broker.clone();
    engine.register_typed(
        COMMAND_EVENT_READ_HISTORY,
        move |cmd: CommandEventReadHistoryData, _ctx| {
            let broker = broker_history.clone();
            async move {
                let max_items = if cmd.maxitems == 0 { 1024 } else { cmd.maxitems };
                Ok(broker.read_event_history(&cmd.event, &cmd.scope, max_items))
            }
        },
    );

    // setconfig → merge settings keys into settings.json AND update in-memory config immediately.
    // Writing to disk + broadcasting directly gives instant UI response without waiting for
    // the fs watcher (which has a ~300-800ms debounce + polling delay on Windows).
    // The fs watcher's subsequent reload is a no-op (settings already up to date).
    // `Value` request, parsed into its struct below (a partial settings object); the stub keeps its
    // hand-written request type (generated-manifest.json `dataType`).
    let s = state.clone();
    engine.register_typed(COMMAND_SET_CONFIG, move |data: serde_json::Value, ctx| {
        let s = s.clone();
        async move { set_config(&s, data, &ctx) }
    });
}

/// `setconfig`: merge settings keys into `settings.json` and the in-memory config, then broadcast it.
fn set_config(state: &AppState, data: serde_json::Value, ctx: &RpcContext) -> Result<(), String> {
    let cw = state.config_watcher.clone();
    let eb = state.event_bus.clone();
    let lan_listeners = state.lan_listeners.clone();
    let new_keys = crate::backend::supervisor::settings::parse_setconfig(data, &ctx.agent_id)?;

    // 1. Write to disk (fs watcher will re-broadcast, harmlessly)
    crate::backend::config_watcher_fs::merge_settings_to_disk(new_keys.clone())
        .map_err(|e| format!("setconfig write: {e}"))?;

    // 2. Update in-memory config immediately
    let merged_settings = crate::backend::config_watcher_fs::merge_settings_into_current(&cw, new_keys);
    let lan_enabled = merged_settings.network_lan_discovery;
    cw.update_settings(merged_settings);

    // 3. Live-toggle LAN discovery if the key changed. `apply` is
    //    idempotent so it's safe to call unconditionally — when the
    //    daemon is already in the requested state, this is a no-op.
    //    See docs/specs/lan-discovery-toggle.md.
    //
    //    Only the LISTENER supervisor is driven here. It owns the
    //    advertise/reachable pairing and re-gates `lan` itself on
    //    every reconcile, so mDNS never advertises an address
    //    nothing is listening on ("discovered but unreachable").
    //    Calling `lan.apply` here as well would fight that gate —
    //    it would advertise immediately, before any listener is
    //    bound, which is the exact state being designed out.
    lan_listeners.apply(lan_enabled);

    // 4. Broadcast updated config now — no waiting for fs watcher.
    //    Shared with the settings-watcher reload path and the
    //    browser-start-page watcher — see its own doc comment
    //    for why this used to be duplicated inline here.
    crate::backend::config_watcher_fs::broadcast_full_config(&cw, &eb);
    Ok(())
}

/// The registrants for agents, drones, sign-in, installs, MuxAPI, MuxBus, memory and notifications.
fn register_service_handlers(engine: &Arc<WshRpcEngine>, state: &AppState, conn_id: &str) {
    // Agent handlers (definitions, content, skills, history, import, reseed)
    super::agent_handlers::register_agent_handlers(engine, &state);

    // Drone handlers (issue #753 — Drone pane DAG executor)
    super::drone_handlers::register_drone_handlers(engine, &state);
    super::drone_template_handlers::register(engine, &state);

    // Pre-launch OAuth handlers (auth.start / poll / submitcallback /
    // cancel — see docs/specs/SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md)
    super::identity_handlers::register_identity_handlers(engine, &state);

    // Install handlers (install.start / install.cancel — see
    // docs/specs/SPEC_AGENT_INSTALL_STAGE_2026_05_17.md)
    super::install_handlers::register_install_handlers(engine, &state);
    // Toolchain's Uninstall of an agent CLI (install.uninstall).
    super::uninstall_handlers::register_uninstall_handlers(engine, &state);

    // System-toolchain install handlers (toolchain.resolve_install_command /
    // toolchain.install_system_tool — see
    // docs/specs/SPEC_SYSTEM_TOOLCHAIN_INSTALLER_2026_08_24.md). Shares
    // state.install_sessions with the handlers just above (same
    // InstallSessionRegistry — install.cancel already works unchanged for
    // these sessions too, no new cancel command needed).
    super::system_install_handlers::register_system_install_handlers(engine, &state);

    // MuxAPI handlers (agent.open, agent.send, agent.stop, agent.status, agent.list, agent.output)
    super::app_api::register_app_api_handlers(engine, &state);

    // MuxBus cloud connectivity (muxbus.login / muxbus.login.cancel / muxbus.status / muxbus.disconnect / muxbus.cloudconfig)
    super::muxbus_handlers::register_muxbus_handlers(engine, &state);

    // Native memory file browser (agent:memory:list / read_file / write_file)
    super::native_memory_handlers::register_native_memory_handlers(engine, &state);
    super::native_memory_adopt_handlers::register(engine, &state);

    // Memory delivery: the hidden reinjection's claim against the
    // SessionStart hook (memorydelivery:claim_fallback).
    super::memory_delivery_handlers::register_memory_delivery_handlers(engine, &state);

    // OS notification Router (notify.emit / focus / ack / test / takeactivation
    // — docs/specs/SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md §3.3).
    super::notify_handlers::register_notify_handlers(engine, &state, conn_id.to_string());

    // operator.challenge: this connection's first step in proving it is the AgentMux window.
    super::operator_proof::register_challenge(engine);
}

#[cfg(test)]
#[path = "tests/ws_register_tests.rs"]
mod ws_register_tests;
