// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The endpoints Claude Code's `SessionStart` hook calls to deliver an agent's
//! memory (docs/specs/SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md §7 P2). The
//! composition and the part rules are `backend::memory::delivery`.
//!
//! - `POST /api/v1/agent/memory/session-start/part` — one part. Each of the
//!   [`HOOK_PARTS`](delivery::HOOK_PARTS) hook commands asks for its
//!   own part number. They run in parallel, so the first request for a
//!   (block, session, reason) composes the delivery and caches it, and every
//!   other part comes from that same composition.
//! - `POST /api/v1/agent/memory/session-start/ack` — a hook wrote its part to
//!   Claude. Once every part is acknowledged the delivery is complete: the pane
//!   gets one persisted `agentmux_memory_injected` frame naming each entry and
//!   its size (owner decision D11). A part fetched but never acknowledged (the
//!   hook crashed or timed out) leaves the delivery incomplete, and no notice
//!   claims otherwise.
//!
//! The agent comes from the request's credential (`X-Agent-Token` →
//! [`Caller::Agent`](super::caller::Caller)): its Personal Memory is that
//! agent's. Without a token only Global Memory is delivered.

use agentmux_common::tokens::{estimate_tokens, tokens_for_chars};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::backend::container_credential::ContainerGrant;
use crate::backend::memory::delivery::{self, Entry, Reason, Tier};
use crate::backend::memory::fit;
use crate::backend::memory::delivery_state::{
    state_lock, Delivery, DeliveryKey, EntrySize, FallbackClaim, FallbackDelivery, StartupDetail,
    PARKED_NOTICE_WAIT, PENDING_POLL, PENDING_WAIT,
};

use super::caller::Caller;
use super::AppState;

#[derive(Debug, Deserialize)]
pub(crate) struct PartRequest {
    block_id: String,
    session_id: String,
    /// The hook's `source` (`startup`, `resume`, `clear`, `compact`, `fork`).
    source: String,
    /// 1-based.
    part: usize,
    /// The session's working directory and Claude's config dir, from the
    /// hook (its stdin `cwd`, its env `CLAUDE_CONFIG_DIR`): where the CLI
    /// looked for its startup files. Empty from an older hook binary.
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    config_dir: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct PartResponse {
    /// This part's text, or `None` when there is none to deliver (no memory,
    /// a source that gets no delivery, or a part number past the last).
    text: Option<String>,
    part: usize,
    of: usize,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AckRequest {
    block_id: String,
    session_id: String,
    source: String,
    part: usize,
}

/// A container agent asks for its own block's delivery only. Its block is the
/// grant's (an empty one is filled in); another block's is refused, since the
/// delivery is cached and composed per block, and a compaction summary is a
/// block's own conversation.
fn bind_to_grant(grant: Option<&ContainerGrant>, block_id: &mut String) -> Result<(), Response> {
    let Some(grant) = grant else { return Ok(()) };
    if block_id.is_empty() {
        block_id.clone_from(&grant.block_id);
    }
    if *block_id != grant.block_id {
        tracing::warn!(granted = %grant.block_id, asked = %block_id, "container agent asked for another block's memory delivery: refused");
        return Err((StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "this container token is for another block"}))).into_response());
    }
    Ok(())
}

/// `POST /api/v1/agent/memory/session-start/part`
pub(crate) async fn handle_session_start_part(
    State(state): State<AppState>,
    caller: Option<axum::Extension<Caller>>,
    grant: Option<axum::Extension<ContainerGrant>>,
    Json(mut req): Json<PartRequest>,
) -> Response {
    if let Err(refused) = bind_to_grant(grant.as_deref(), &mut req.block_id) {
        return refused;
    }
    session_start_part(state, caller, req, grant.is_some()).await.into_response()
}

/// `from_container`: the request's `cwd` and `config_dir` are paths inside the
/// container, so nothing is read from the host for them. The startup files are
/// the ones in the block's own mounted workspace, as the CLI there read them.
async fn session_start_part(state: AppState, caller: Option<axum::Extension<Caller>>, req: PartRequest, from_container: bool) -> Json<PartResponse> {
    let Some(reason) = Reason::from_hook_source(&req.source) else {
        return Json(PartResponse { text: None, part: req.part, of: 0 });
    };
    let key = DeliveryKey { block_id: req.block_id.clone(), session_id: req.session_id.clone(), reason };
    let now = agentmux_common::time::now_ms();

    let cached = {
        let mut st = state_lock();
        st.prune(now);
        // The frontend's fallback already delivered this event: stand down.
        if st.fallback_claimed(&req.block_id, reason) {
            tracing::info!(block_id = %req.block_id, reason = reason.as_str(), part = req.part, "memory delivery: part stood down, the fallback claimed the event");
            return Json(PartResponse { text: None, part: req.part, of: 0 });
        }
        if st.composed_nothing.contains_key(&key) {
            return Json(PartResponse { text: None, part: req.part, of: 0 });
        }
        st.deliveries.get(&key).cloned()
    };
    let delivery = match cached {
        Some(d) => d,
        None => {
            let gate = state_lock().compose_gates.entry(key.clone()).or_default().clone();
            let _composing = gate.lock().await;
            // Dropped on every way out, a cancelled request included, so the
            // gate never outlives the part that took it.
            let _gate_entry = GateEntry(key.clone());
            // A part that held the gate before this one may have composed it, or
            // the fallback may have claimed the event while this one waited.
            let (composed_meanwhile, claimed, nothing_meanwhile) = {
                let st = state_lock();
                (st.deliveries.get(&key).cloned(), st.fallback_claimed(&req.block_id, reason), st.composed_nothing.contains_key(&key))
            };
            if nothing_meanwhile {
                return Json(PartResponse { text: None, part: req.part, of: 0 });
            }
            if claimed {
                tracing::info!(block_id = %req.block_id, reason = reason.as_str(), part = req.part, "memory delivery: part stood down, the fallback claimed the event");
                return Json(PartResponse { text: None, part: req.part, of: 0 });
            }
            if let Some(d) = composed_meanwhile {
                return finish_part(&state, &key, d, req.part);
            }
            let uid = caller.as_ref().and_then(|c| c.uid()).map(str::to_string);
            let launch = if from_container { LaunchDirs::of_container_block(&state, &req.block_id) } else { LaunchDirs::from_request(&req) };
            let composed = {
                let state = state.clone();
                let block_id = req.block_id.clone();
                tokio::task::spawn_blocking(move || {
                    compose_delivery(&state, &block_id, uid.as_deref(), reason, launch.as_ref(), now)
                })
                    .await
            };
            // A failed compose is not "nothing to deliver": the next part tries again.
            let composed = match composed {
                Ok(composed) => composed,
                Err(e) => {
                    tracing::warn!(error = %e, "memory delivery: composing failed");
                    return Json(PartResponse { text: None, part: req.part, of: 0 });
                }
            };
            let Some(composed) = composed else {
                state_lock().composed_nothing.insert(key.clone(), now);
                tracing::info!(block_id = %req.block_id, session_id = %req.session_id, reason = reason.as_str(), part = req.part, cwd_known = !req.cwd.is_empty(), "memory delivery: nothing to deliver for this session start");
                return Json(PartResponse { text: None, part: req.part, of: 0 });
            };
            tracing::info!(
                block_id = %req.block_id, session_id = %req.session_id, reason = reason.as_str(),
                parts = composed.parts.len(), items = composed.entries.len(), "memory delivery: composed"
            );
            // A concurrent part may have composed first: keep that one, so
            // every part of this session start is from the same composition.
            // And the fallback may have claimed the event meanwhile.
            let mut st = state_lock();
            if st.fallback_claimed(&req.block_id, reason) {
                return Json(PartResponse { text: None, part: req.part, of: 0 });
            }
            st.deliveries.entry(key.clone()).or_insert(composed).clone()
        }
    };
    finish_part(&state, &key, delivery, req.part)
}

/// Removes a delivery's compose gate when the part that took it is done.
struct GateEntry(DeliveryKey);

impl Drop for GateEntry {
    fn drop(&mut self) {
        state_lock().compose_gates.remove(&self.0);
    }
}

/// The reply for one part of a composed delivery. A delivery with nothing for
/// the hook to carry still gets its card, on the first part asked for.
fn finish_part(state: &AppState, key: &DeliveryKey, delivery: Delivery, part: usize) -> Json<PartResponse> {
    if delivery.parts.is_empty() {
        send_partless_notice(state, key);
    }
    Json(part_response(&delivery, part))
}

/// A delivery with nothing for the hook to carry (its Global Memory is in the
/// startup file, and there's no Personal Memory) still has a card: written on
/// the first part request, exactly once, since no part will be acknowledged.
fn send_partless_notice(state: &AppState, key: &DeliveryKey) {
    let frame = {
        let mut st = state_lock();
        match st.deliveries.get_mut(key) {
            Some(d) if d.parts.is_empty() && !d.notice_sent && !d.entries.is_empty() => {
                d.notice_sent = true;
                let (frame, created_ms) = (notice_frame(key, d), d.created_ms);
                st.note_completed(key, created_ms);
                frame
            }
            _ => return,
        }
    };
    crate::backend::cli_notice::append_to_pane(&state.broker, &state.filestore, &state.mstore, &key.block_id, &frame);
}

/// `POST /api/v1/agent/memory/session-start/ack`
pub(crate) async fn handle_session_start_ack(
    State(state): State<AppState>,
    grant: Option<axum::Extension<ContainerGrant>>,
    Json(mut req): Json<AckRequest>,
) -> Response {
    if let Err(refused) = bind_to_grant(grant.as_deref(), &mut req.block_id) {
        return refused;
    }
    session_start_ack(state, req).await.into_response()
}

async fn session_start_ack(state: AppState, req: AckRequest) -> Json<serde_json::Value> {
    let Some(reason) = Reason::from_hook_source(&req.source) else {
        return Json(serde_json::json!({ "complete": false }));
    };
    let key = DeliveryKey { block_id: req.block_id.clone(), session_id: req.session_id.clone(), reason };
    let (complete, write_now, parked_at) = {
        let mut st = state_lock();
        let done = st.deliveries.get_mut(&key).and_then(|d| acknowledge(d, req.part).then(|| (notice_frame(&key, d), d.created_ms)));
        match done {
            None => (false, None, None),
            Some((frame, created_ms)) => {
                st.note_completed(&key, created_ms);
                let line = frame.to_string();
                if reason == Reason::Compact {
                    let now = st.park_compaction_notice(&req.block_id, created_ms, line);
                    let parked = now.is_none().then_some(created_ms);
                    (true, now, parked)
                } else {
                    (true, Some(line), None)
                }
            }
        }
    };
    if complete {
        tracing::info!(block_id = %req.block_id, reason = reason.as_str(), parked = parked_at.is_some(), "memory delivery: complete, notice sent");
    }
    if let Some(line) = write_now {
        append_notice(&state, &req.block_id, &line);
    }
    // The boundary hasn't been written yet: the stdout reader writes the card
    // after it. If no boundary comes, write it anyway.
    if let Some(created_ms) = parked_at {
        let block_id = req.block_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(PARKED_NOTICE_WAIT).await;
            let left = state_lock().take_parked(&block_id, Some(created_ms));
            for line in left {
                tracing::warn!(block_id = %block_id, "memory delivery: no compaction boundary came; card written without it");
                append_notice(&state, &block_id, &line);
            }
        });
    }
    Json(serde_json::json!({ "complete": complete }))
}

/// Appends one notice line to the block's output and its transcript zone.
fn append_notice(state: &AppState, block_id: &str, line: &str) {
    crate::backend::cli_notice::append_line_to_pane(&state.broker, &state.filestore, &state.mstore, block_id, line);
}

/// The event a frontend reinjection reason stands for: a compaction, or a
/// fresh session (a spawn without `--resume`, whose hook `source` is
/// `startup`). `None` for anything else.
fn reason_for_fallback(reason: &str) -> Option<Reason> {
    match reason {
        "compaction" => Some(Reason::Compact),
        "fresh_session" => Some(Reason::Startup),
        _ => None,
    }
}

/// Whether the frontend's hidden-reinjection fallback should deliver this
/// event (`true`) or stand down because the `SessionStart` hook delivered it
/// (`false`). Waits out a hook delivery still in flight for up to
/// [`PENDING_WAIT`], then lets the fallback deliver, so a hook that died
/// part-way never leaves the agent without its memory. With a
/// `boundary_uuid`, only the block whose CLI compacted can deliver. With
/// `event_at_ms` (a fresh session's time), the hook's delivery for that
/// session counts however late the claim comes.
pub(crate) async fn claim_fallback(block_id: &str, reason: &str, boundary_uuid: Option<&str>, event_at_ms: Option<i64>) -> ClaimResult {
    let Some(reason) = reason_for_fallback(reason) else { return ClaimResult { deliver: true, foreign: false } };
    let deadline = tokio::time::Instant::now() + PENDING_WAIT;
    let boundary_uuid = boundary_uuid.filter(|u| !u.is_empty());
    loop {
        let now = agentmux_common::time::now_ms();
        let claim = {
            let mut st = state_lock();
            st.prune(now);
            match (boundary_uuid, event_at_ms) {
                (None, Some(event_at)) => st.claim_event(block_id, reason, event_at, now),
                _ => st.claim_boundary(block_id, reason, boundary_uuid, now),
            }
        };
        match claim {
            FallbackClaim::Deliver => return ClaimResult { deliver: true, foreign: false },
            FallbackClaim::Skip => {
                tracing::info!(block_id, reason = reason.as_str(), "memory delivery: the hook delivered; fallback stands down");
                return ClaimResult { deliver: false, foreign: false };
            }
            FallbackClaim::Foreign => {
                tracing::info!(block_id, reason = reason.as_str(), "memory delivery: not this block's compaction; fallback stands down");
                return ClaimResult { deliver: false, foreign: true };
            }
            FallbackClaim::Pending if tokio::time::Instant::now() >= deadline => {
                tracing::warn!(block_id, reason = reason.as_str(), "memory delivery: hook still unfinished; fallback delivers instead");
                state_lock().take_over_from_hook(block_id, reason, agentmux_common::time::now_ms());
                return ClaimResult { deliver: true, foreign: false };
            }
            FallbackClaim::Pending => tokio::time::sleep(PENDING_POLL).await,
        }
    }
}

/// What a fallback's claim found: `deliver`, or stand down because the hook
/// delivered, or because the event was another block's (`foreign`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClaimResult {
    pub deliver: bool,
    pub foreign: bool,
}

/// Registers `memorydelivery:claim_fallback` — the frontend's hidden memory
/// reinjection asks it right before it would fire — and
/// `memorydelivery:compose`, which composes what it sends.
pub(crate) fn register_memory_delivery_handlers(
    engine: &std::sync::Arc<crate::backend::rpc::engine::WshRpcEngine>,
    state: &AppState,
) {
    use crate::backend::rpc_types::{
        CommandMemoryDeliveryClaimFallbackData, CommandMemoryDeliveryComposeData, MemoryDeliveryClaimFallbackResult,
        MemoryDeliveryComposeResult, COMMAND_MEMORY_DELIVERY_CLAIM_FALLBACK, COMMAND_MEMORY_DELIVERY_COMPOSE,
    };
    engine.register_typed(
        COMMAND_MEMORY_DELIVERY_CLAIM_FALLBACK,
        |req: CommandMemoryDeliveryClaimFallbackData, _ctx| async move {
            Ok::<_, String>({
                let r = claim_fallback(&req.block_id, &req.reason, req.boundary_uuid.as_deref(), req.event_at_ms).await;
                MemoryDeliveryClaimFallbackResult { deliver: r.deliver, foreign: r.foreign.then_some(true) }
            })
        },
    );
    let state = state.clone();
    engine.register_typed(COMMAND_MEMORY_DELIVERY_COMPOSE, move |req: CommandMemoryDeliveryComposeData, _ctx| {
        let state = state.clone();
        async move {
            let now = agentmux_common::time::now_ms();
            if req.catch_up.unwrap_or(false) {
                let kind = delivery::CatchUpKind::from_wire(&req.reason)
                    .ok_or_else(|| format!("memorydelivery:compose: unknown reason {:?}", req.reason))?;
                let memory_reason = kind.memory_reason().filter(|_| req.memory.unwrap_or(true));
                let block_id = req.block_id.clone();
                let composed = match memory_reason {
                    Some(reason) => tokio::task::spawn_blocking(move || compose_fallback_delivery(&state, &block_id, reason, now))
                        .await
                        .map_err(|e| format!("memorydelivery:compose: {e}"))?,
                    None => None,
                };
                return Ok::<_, String>(match composed {
                    Some(mut f) => {
                        f.text = delivery::compose_catch_up(kind, Some(&f.text));
                        let reply = MemoryDeliveryComposeResult {
                            delivery_id: Some(f.delivery_id.clone()),
                            text: Some(f.text.clone()),
                            frame: Some(f.frame.clone()),
                        };
                        let mut st = state_lock();
                        st.prune(now);
                        st.fallbacks.insert(f.delivery_id.clone(), f);
                        reply
                    }
                    // No memory to deliver (the hook did, or the phase has
                    // none): the catch-up block alone, and no card.
                    None => MemoryDeliveryComposeResult {
                        delivery_id: None,
                        text: Some(delivery::compose_catch_up(kind, None)),
                        frame: None,
                    },
                });
            }
            let reason = delivery::FallbackReason::from_wire(&req.reason)
                .ok_or_else(|| format!("memorydelivery:compose: unknown reason {:?}", req.reason))?;
            let composed = tokio::task::spawn_blocking(move || compose_fallback_delivery(&state, &req.block_id, reason, now))
                .await
                .map_err(|e| format!("memorydelivery:compose: {e}"))?;
            Ok::<_, String>(match composed {
                Some(f) => {
                    let reply = MemoryDeliveryComposeResult {
                        delivery_id: Some(f.delivery_id.clone()),
                        text: Some(f.text.clone()),
                        frame: Some(f.frame.clone()),
                    };
                    let mut st = state_lock();
                    st.prune(now);
                    st.fallbacks.insert(f.delivery_id.clone(), f);
                    reply
                }
                None => MemoryDeliveryComposeResult::default(),
            })
        }
    });
}

/// Composes the fallback's hidden message from the same memory the hook
/// delivers (`global_entries`, `personal_entries`), and the card that will
/// stand for it. `None` when there is no memory.
fn compose_fallback_delivery(
    state: &AppState,
    block_id: &str,
    reason: delivery::FallbackReason,
    now: i64,
) -> Option<FallbackDelivery> {
    let global = global_entries(state, block_id);
    let uid = crate::backend::spawn_env::block_identity::persisted_agent_identity(&state.mstore, block_id).map(|i| i.uid);
    // A new session whose startup file carries every current Global section
    // doesn't get them a second time, as the hook doesn't send them (LC3).
    let global_in_file = reason == delivery::FallbackReason::FreshSession
        && uid.as_deref().is_some_and(|uid| global_memory_in_latest_startup_files(uid, &global));
    let mut entries = if global_in_file { Vec::new() } else { global.clone() };
    if let Some(uid) = &uid {
        entries.extend(personal_entries(uid));
    }
    let composed = delivery::compose_fallback_with(&entries, reason, global_in_file)?;
    let mut items = if global_in_file { global.iter().map(in_startup_file_item).collect() } else { Vec::new() };
    items.extend(delivery_items(&entries, &composed, composed.text.chars().count()));
    // A compaction's running summary rides along: srv's input handler appends
    // it to this same message (`continuity_state::with_state_after_compaction`).
    let summary = (reason == delivery::FallbackReason::Compaction)
        .then(|| crate::backend::continuity_state::running_summary_section(&state.mstore, block_id))
        .flatten();
    if let Some(summary) = &summary {
        let (size, tokens) = (summary.len(), estimate_tokens(summary) as usize);
        items.push(EntrySize::running_summary(size, tokens, "full", size, tokens));
    }
    let delivery_id = format!("fb-{}", uuid::Uuid::new_v4());
    let frame = fallback_frame(&delivery_id, reason, &items, summary.as_ref().map_or(0, |s| s.len()), now);
    Some(FallbackDelivery { delivery_id, block_id: block_id.to_string(), text: composed.text, frame, created_ms: now })
}

/// The card for a fallback re-delivery: the hook notice's shape, under the
/// delivery's own id. `fallback: true` tells replay that the hidden message
/// right after it is this delivery's echo, not a second one.
fn fallback_frame(
    delivery_id: &str,
    reason: delivery::FallbackReason,
    entries: &[EntrySize],
    summary_bytes: usize,
    now: i64,
) -> serde_json::Value {
    card_frame(
        format!("memory-injected-{delivery_id}"),
        match reason {
            delivery::FallbackReason::Compaction => "compact",
            delivery::FallbackReason::FreshSession => "resume_fresh",
        },
        entries,
        summary_bytes,
        now,
        serde_json::json!({ "fallback": true, "delivery_id": delivery_id }),
    )
}

/// The `agentmux_memory_injected` frame both delivery routes write: the
/// fields the card reads, then `extra` (the route's own: the hook's session
/// and part count, the fallback's delivery id).
fn card_frame(
    id: String,
    reason: &str,
    entries: &[EntrySize],
    summary_bytes: usize,
    at_ms: i64,
    extra: serde_json::Value,
) -> serde_json::Value {
    let mut frame = serde_json::json!({
        "type": "system",
        "subtype": "agentmux_memory_injected",
        "id": id,
        "reason": reason,
        "entries": entries,
        "summary_bytes": summary_bytes,
        "timestamp": chrono::DateTime::from_timestamp_millis(at_ms).unwrap_or_default().to_rfc3339(),
    });
    if let (Some(fields), serde_json::Value::Object(more)) = (frame.as_object_mut(), extra) {
        fields.extend(more);
    }
    frame
}

/// The card of the fallback delivery `delivery_id`, taken once, when its
/// hidden message reaches srv. `None` for an unknown or expired id, or one
/// composed for another block.
pub(crate) fn take_fallback_frame(block_id: &str, delivery_id: &str) -> Option<serde_json::Value> {
    let mut st = state_lock();
    st.prune(agentmux_common::time::now_ms());
    match st.fallbacks.remove(delivery_id) {
        Some(f) if f.block_id == block_id => Some(f.frame),
        Some(f) => {
            tracing::warn!(block_id, delivery_block = %f.block_id, "memory delivery: a fallback delivery id from another block");
            None
        }
        None => None,
    }
}

/// A Global Memory entry that reached the agent through its startup file:
/// listed in the card, sized 0 (the file's own row counts it).
fn in_startup_file_item(e: &Entry) -> EntrySize {
    EntrySize {
        label: e.label.clone(),
        source: "global",
        size_bytes: 0,
        tokens: 0,
        kind: "global_memory",
        name: e.name.clone(),
        tier: Some(if e.system { "system" } else { "workspace" }),
        bundle_id: e.bundle_id.clone(),
        path: None,
        delivered: "full",
        source_size_bytes: e.text.len(),
        source_tokens: estimate_tokens(&e.text) as usize,
        startup: None,
        via: Some("startup_file"),
    }
}

/// Where a session's CLI looked for its startup files.
#[derive(Debug, Clone)]
struct LaunchDirs {
    cwd: std::path::PathBuf,
    config_dir: Option<std::path::PathBuf>,
    /// A container agent: only the mounted workspace's own startup files count,
    /// not the host's parent folders or the user's host `~/.claude`.
    workspace_only: bool,
}

impl LaunchDirs {
    fn from_request(req: &PartRequest) -> Option<Self> {
        let cwd = req.cwd.trim();
        (!cwd.is_empty()).then(|| Self {
            cwd: cwd.into(),
            config_dir: Some(req.config_dir.trim()).filter(|c| !c.is_empty()).map(Into::into),
            workspace_only: false,
        })
    }

    /// A container's: the workspace srv mounted for the block, which is where its
    /// CLI reads the startup files. The paths the container reports are its own.
    fn of_container_block(state: &AppState, block_id: &str) -> Option<Self> {
        let block = state.mstore.get::<crate::backend::obj::Block>(block_id).ok().flatten()?;
        let cwd = crate::backend::obj::meta_get_string(&block.meta, "cmd:cwd", "");
        (!cwd.trim().is_empty()).then(|| Self { cwd: cwd.trim().into(), config_dir: None, workspace_only: true })
    }
}

/// The files the CLI read by itself at session start.
fn startup_files(launch: &LaunchDirs) -> Vec<crate::backend::startup_files::StartupFile> {
    if launch.workspace_only {
        return crate::backend::startup_files::workspace_startup_files(&launch.cwd);
    }
    let home = dirs::home_dir();
    crate::backend::startup_files::claude_startup_files(&launch.cwd, launch.config_dir.as_deref(), home.as_deref())
}

/// Whether the CLI already got the current Global Memory from a startup
/// file: one AgentMux wrote that holds every current section, byte for byte.
/// A foreign or stale file with some old section doesn't count, or the hook
/// would drop current entries the agent never saw (Codex on #4131).
fn global_memory_in_startup_files(files: &[crate::backend::startup_files::StartupFile], global: &[Entry]) -> bool {
    !global.is_empty()
        && files.iter().any(|f| {
            f.owner == crate::backend::startup_files::Owner::Agentmux && global.iter().all(|e| f.text.contains(&e.text))
        })
}

/// [`global_memory_in_startup_files`] for the agent's latest Claude spawn, from
/// where its continuity segment says the CLI started: the fallback has no
/// hook request to read them from.
fn global_memory_in_latest_startup_files(agent_uid: &str, global: &[Entry]) -> bool {
    let Some(fs) = crate::backend::agent_session::global_transcript_store() else { return false };
    let Some(latest) = crate::backend::continuity_segments::segments(fs.as_ref(), agent_uid)
        .into_iter()
        .filter(|s| s.start.provider == "claude" && !s.start.cwd.is_empty())
        .max_by_key(|s| s.start.started_at_ms)
    else {
        return false;
    };
    let launch = LaunchDirs { cwd: latest.start.cwd.into(), config_dir: latest.start.config_dir.filter(|c| !c.is_empty()).map(Into::into), workspace_only: false };
    global_memory_in_startup_files(&startup_files(&launch), global)
}

fn startup_items_of(files: Vec<crate::backend::startup_files::StartupFile>) -> Vec<EntrySize> {
    files
        .into_iter()
        .map(|f| {
            let (size_bytes, tokens) = (f.text.len(), estimate_tokens(&f.text) as usize);
            EntrySize {
                label: f.name.clone(),
                source: "startup",
                size_bytes,
                tokens,
                kind: "startup_file",
                name: f.name,
                tier: None,
                bundle_id: None,
                path: Some(f.path.to_string_lossy().into_owned()),
                delivered: "full",
                source_size_bytes: size_bytes,
                source_tokens: tokens,
                startup: Some(StartupDetail { role: f.role, owner: f.owner.as_str(), count: f.count, contains: f.contains }),
                via: None,
            }
        })
        .collect()
}

/// Reads the memory and composes the delivery. `None` when there is nothing
/// to deliver. A new session's delivery also lists the startup files the CLI
/// loaded from `launch` (`startup_items_of`), ahead of the memory items.
fn compose_delivery(
    state: &AppState,
    block_id: &str,
    agent_uid: Option<&str>,
    reason: Reason,
    launch: Option<&LaunchDirs>,
    now: i64,
) -> Option<Delivery> {
    // What the CLI loaded itself comes first, then what the hook carried.
    let files = match (reason, launch) {
        (Reason::Startup, Some(launch)) => startup_files(launch),
        _ => Vec::new(),
    };
    let global = global_entries(state, block_id);
    // A startup file the CLI loaded already carries the current Global
    // Memory: the hook doesn't send it a second time (LC3, §4.4). The card
    // still lists each entry, marked as coming through that file.
    let global_in_file = global_memory_in_startup_files(&files, &global);
    let mut items = startup_items_of(files);
    let mut entries = if global_in_file { Vec::new() } else { global.clone() };
    if let Some(uid) = agent_uid {
        entries.extend(personal_entries(uid));
    }
    if global_in_file {
        items.extend(global.iter().map(in_startup_file_item));
    }
    let summary = (reason == Reason::Compact)
        .then(|| crate::backend::continuity_state::running_summary_section(&state.mstore, block_id))
        .flatten();
    // Nothing left for the hook to carry is still a delivery when there are
    // startup items to show: it has no parts, and its card goes out at once.
    let age_days = |e: &Entry| modified_ms_of(e).map(|m| ((now - m).max(0)) / 86_400_000);
    let (parts, delivered) = match fit_and_compose(entries, summary.as_deref(), reason, global_in_file, &age_days) {
        Some((fitted, composed, parts, delivered_chars)) => {
            let mut delivered = delivery_items(&fitted.kept, &composed, delivered_chars);
            apply_capped(&mut delivered, &fitted.capped);
            delivered.extend(fitted.skipped.iter().map(not_sent_item));
            (parts, delivered)
        }
        None if !items.is_empty() => (Vec::new(), Vec::new()),
        None => return None,
    };
    let acked = vec![false; parts.len()];
    items.extend(delivered);
    Some(Delivery {
        entries: items,
        summary_bytes: summary.map_or(0, |s| s.len()),
        parts,
        acked,
        created_ms: now,
        notice_sent: false,
    })
}

/// Global Memory as the startup file carries it — its sections, Operator
/// Config first (`globalmemory:sections`, P1).
fn global_entries(state: &AppState, block_id: &str) -> Vec<Entry> {
    // The same entries the pane's startup file got: Operator Config targeted
    // at another agent kind is left out.
    let bundles = crate::backend::operator_config_seed::global_bundles_for_agent(
        state.id_store.bundle_list_global().unwrap_or_default(),
        &crate::backend::operator_config_seed::agent_mode_of_block(&state.mstore, block_id),
    );
    crate::backend::storage::global_bundle_sections(&bundles)
        .into_iter()
        .map(|s| Entry {
            label: format!("{} {}", if s.is_system { "[AgentMux System]" } else { "[Workspace]" }, s.name),
            tier: Tier::Global,
            text: s.text,
            name: s.name,
            system: s.is_system,
            bundle_id: Some(s.id),
            path: None,
        })
        .collect()
}

/// The agent's Personal Memory files from its latest spawn's memory folder,
/// by name, without the `MEMORY.md` index (the CLI's own table of contents).
fn personal_entries(agent_uid: &str) -> Vec<Entry> {
    let Some(dir) = crate::server::native_memory_handlers::memory_dir_from_spawn(agent_uid) else { return Vec::new() };
    personal_entries_in(&dir)
}

fn personal_entries_in(dir: &std::path::Path) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<(String, std::path::PathBuf)> = read
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path())))
        .filter(|(name, _)| name.ends_with(".md") && name != "MEMORY.md")
        .collect();
    files.sort();
    let mut entries: Vec<Entry> = files
        .into_iter()
        .filter_map(|(name, path)| {
            let text = std::fs::read_to_string(&path).ok()?;
            (!text.trim().is_empty()).then_some(Entry {
                label: name.clone(),
                tier: Tier::Personal,
                text,
                name,
                system: false,
                bundle_id: None,
                path: Some(path.to_string_lossy().into_owned()),
            })
        })
        .collect();
    // Most recently modified first (the sort is stable, so equal times keep name order), so what a cut
    // delivery drops is the oldest, not whatever sorts last by name.
    entries.sort_by_cached_key(|e| std::cmp::Reverse(modified_ms_of(e).unwrap_or(0)));
    entries
}

/// The files that fit one delivery (newest first and capped, the rest named, not dropped silently), composed and
/// split the way the hooks will carry them. The character budget is only a guess at how the parts break at line
/// ends, so when the real split still cuts (the notes at the end are what can tip it), a little more room is
/// kept back and the fit is redone, so the reserved summary and the notes are never the part that is lost.
fn fit_and_compose(
    entries: Vec<Entry>,
    summary: Option<&str>,
    reason: Reason,
    global_in_file: bool,
    age_days: &dyn Fn(&Entry) -> Option<i64>,
) -> Option<(fit::Fitted, delivery::Composed, Vec<String>, usize)> {
    const STEP: usize = 2_000;
    const LAST: usize = 40_000;
    let reserve = summary.map_or(0, |s| s.chars().count());
    let mut slack = 0;
    loop {
        let fitted = fit::fit(entries.clone(), reserve + slack, reason == Reason::Compact, age_days);
        let composed = delivery::compose_items_tail(&fitted.kept, reason, summary, global_in_file, fitted.tail.as_deref())?;
        let (parts, delivered_chars) = delivery::split_into_parts_counted(&composed.text, delivery::MAX_PART_CHARS, delivery::HOOK_PARTS);
        if delivered_chars >= composed.text.chars().count() || slack >= LAST {
            return Some((fitted, composed, parts, delivered_chars));
        }
        slack += STEP;
    }
}

/// A file the cap cut shows on the card with its real size, as partly delivered.
fn apply_capped(rows: &mut [EntrySize], capped: &[fit::Capped]) {
    for row in rows.iter_mut().filter(|r| r.kind == "personal_memory") {
        let Some(c) = capped.iter().find(|c| c.name == row.name) else { continue };
        row.source_size_bytes = c.source_bytes;
        row.source_tokens = tokens_for_chars(c.source_chars as u64) as usize;
        if row.delivered == delivery::Delivered::Full.as_str() {
            row.delivered = delivery::Delivered::Partial(0).as_str();
        }
    }
}

/// When a Personal Memory file was last modified, in Unix ms: its `modified` frontmatter when it has one that
/// parses, else the file's mtime.
fn modified_ms_of(e: &Entry) -> Option<i64> {
    if let Some(v) = fit::frontmatter_value(&e.text, "modified") {
        if let Ok(t) = chrono::DateTime::parse_from_rfc3339(&v) {
            return Some(t.timestamp_millis());
        }
    }
    let mtime = std::fs::metadata(e.path.as_deref()?).ok()?.modified().ok()?;
    i64::try_from(mtime.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis()).ok()
}

/// The card's row for a file left out of the delivery whole.
fn not_sent_item(e: &Entry) -> EntrySize {
    EntrySize {
        label: e.label.clone(),
        source: e.tier.as_str(),
        size_bytes: 0,
        tokens: 0,
        kind: "personal_memory",
        name: e.name.clone(),
        tier: None,
        bundle_id: e.bundle_id.clone(),
        path: e.path.clone(),
        delivered: delivery::Delivered::Omitted.as_str(),
        source_size_bytes: e.text.len(),
        source_tokens: estimate_tokens(&e.text) as usize,
        startup: None,
        via: None,
    }
}

/// The notice's items, in delivery order, sized by what the parts carried.
fn delivery_items(entries: &[Entry], composed: &delivery::Composed, delivered_chars: usize) -> Vec<EntrySize> {
    let chars: Vec<char> = composed.text.chars().collect();
    composed
        .spans
        .iter()
        .map(|span| {
            let delivered = delivery::Delivered::of(span, delivered_chars);
            let slice: String = chars[span.start..span.end.min(delivered_chars).max(span.start)].iter().collect();
            let (size_bytes, tokens) = (slice.len(), estimate_tokens(&slice) as usize);
            let whole: String = chars[span.start..span.end].iter().collect();
            let (source_size_bytes, source_tokens) = (whole.len(), tokens_for_chars((span.end - span.start) as u64) as usize);
            match span.entry.map(|i| &entries[i]) {
                Some(e) => EntrySize {
                    label: e.label.clone(),
                    source: e.tier.as_str(),
                    size_bytes,
                    tokens,
                    kind: match e.tier {
                        Tier::Global => "global_memory",
                        Tier::Personal => "personal_memory",
                    },
                    name: e.name.clone(),
                    tier: (e.tier == Tier::Global).then_some(if e.system { "system" } else { "workspace" }),
                    bundle_id: e.bundle_id.clone(),
                    path: e.path.clone(),
                    delivered: delivered.as_str(),
                    source_size_bytes,
                    source_tokens,
                    startup: None,
                    via: None,
                },
                None => EntrySize::running_summary(size_bytes, tokens, delivered.as_str(), source_size_bytes, source_tokens),
            }
        })
        .collect()
}

fn part_response(d: &Delivery, part: usize) -> PartResponse {
    let text = part.checked_sub(1).and_then(|i| d.parts.get(i)).cloned();
    PartResponse { text, part, of: d.parts.len() }
}

/// Marks `part` written. True exactly once: when this makes every part
/// written and no notice has gone out yet.
fn acknowledge(d: &mut Delivery, part: usize) -> bool {
    if let Some(slot) = part.checked_sub(1).and_then(|i| d.acked.get_mut(i)) {
        *slot = true;
    }
    if d.notice_sent || !d.acked.iter().all(|a| *a) {
        return false;
    }
    d.notice_sent = true;
    true
}

/// The persisted notice (D11): which memory was delivered, and each entry's
/// size. `id` is stable for the delivery, so a live frame and a replay of the
/// same line render once.
fn notice_frame(key: &DeliveryKey, d: &Delivery) -> serde_json::Value {
    card_frame(
        format!("memory-injected-{}-{}-{}", key.session_id, key.reason.as_str(), d.created_ms),
        key.reason.as_str(),
        &d.entries,
        d.summary_bytes,
        d.created_ms,
        serde_json::json!({ "session_id": key.session_id, "parts": d.parts.len() }),
    )
}

#[cfg(test)]
mod tests;
