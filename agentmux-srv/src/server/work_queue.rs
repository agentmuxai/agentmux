// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Muxqueue HTTP surface — slice 2 of
//! `docs/reports/REPORT_UNIVERSAL_AGENT_WORK_QUEUE_2026_09_01.md`.
//!
//!   POST   /agentmux/work                 — enqueue
//!   POST   /agentmux/work/claim           — claim the next eligible item
//!   POST   /agentmux/work/:id/heartbeat   — extend a lease
//!   POST   /agentmux/work/:id/complete    — finish
//!   POST   /agentmux/work/:id/release     — give back
//!   GET    /agentmux/work                 — list
//!   DELETE /agentmux/work/:id             — cancel
//!
//! Auth-gated by the same middleware as the cron and reactive routes.
//!
//! **Store choice:** every handler uses `state.identity_store` — the
//! always-global store — NOT `state.shared_store`, which is what the cron
//! handlers next door still use. That difference is deliberate, not an
//! inconsistency to tidy up later: a per-channel queue would only ever mean
//! "any agent in this channel can pick it up". Cron's own placement is a known
//! unmigrated case (`SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md` step 1b).

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::backend::storage::work_queue::{ClaimFilter, WorkItem};
use crate::backend::wps::WaveEvent;

use super::AppState;

/// Default lease granted by a claim, and the window a heartbeat extends by.
/// Deliberately short relative to a long agent turn: a claimant is expected to
/// heartbeat, and a crashed one should return to the pool in about a minute
/// rather than blocking the item for an hour.
const DEFAULT_LEASE_MS: i64 = 120_000;

/// Emitted whenever the queue changes so any live view can refresh without
/// polling. Named alongside the existing `EVENT_CRON_CHANGED` convention.
const EVENT_WORK_QUEUE_CHANGED: &str = "workqueue:changed";

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn publish_changed(state: &AppState) {
    state.broker.publish(WaveEvent {
        event: EVENT_WORK_QUEUE_CHANGED.to_string(),
        scopes: vec![],
        sender: String::new(),
        persist: 0,
        data: None,
    });
}

fn err(code: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<Value>) {
    (code, Json(json!({ "error": msg.into() })))
}

#[derive(Debug, Deserialize)]
pub(super) struct EnqueueRequest {
    pub title: String,
    pub payload: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub target_agent: String,
    #[serde(default)]
    pub target_group: String,
    #[serde(default)]
    pub priority: i64,
    #[serde(default)]
    pub created_by: String,
    #[serde(default)]
    pub max_attempts: Option<i64>,
    /// ms epoch. Not claimable before this.
    #[serde(default)]
    pub not_before: Option<i64>,
}

pub(super) async fn handle_work_enqueue(
    State(state): State<AppState>,
    Json(req): Json<EnqueueRequest>,
) -> (StatusCode, Json<Value>) {
    if req.title.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "title is required");
    }
    if req.payload.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "payload is required");
    }
    // An item targeted at both a specific agent AND a group is ambiguous —
    // reject rather than silently letting one win, which would look like the
    // queue quietly ignored half the caller's intent.
    if !req.target_agent.is_empty() && !req.target_group.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            "specify target_agent or target_group, not both",
        );
    }

    let now = now_ms();
    let item = WorkItem {
        id: Uuid::new_v4().to_string(),
        title: req.title,
        payload: req.payload,
        kind: req.kind,
        target_agent: req.target_agent,
        target_group: req.target_group,
        priority: req.priority,
        state: String::new(), // set by the store
        claimed_by: String::new(),
        claim_expires: None,
        attempts: 0,
        max_attempts: req.max_attempts.unwrap_or(3),
        created_by: req.created_by,
        created_at: now,
        updated_at: now,
        not_before: req.not_before,
        result: String::new(),
    };

    match state.identity_store.work_queue_enqueue(&item) {
        Ok(()) => {
            publish_changed(&state);
            (StatusCode::OK, Json(json!({ "id": item.id })))
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("enqueue failed: {e}")),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct ClaimRequest {
    /// The claiming agent's id.
    pub agent_id: String,
    #[serde(default)]
    pub kind: Option<String>,
    /// Group ids this agent belongs to. Resolved by the CALLER — group
    /// membership lives in the per-channel store, which the queue module
    /// deliberately does not reach into.
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub lease_ms: Option<i64>,
}

pub(super) async fn handle_work_claim(
    State(state): State<AppState>,
    Json(req): Json<ClaimRequest>,
) -> (StatusCode, Json<Value>) {
    if req.agent_id.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "agent_id is required");
    }
    let now = now_ms();

    // Reap BEFORE claiming, so an expired lease is recovered by the very next
    // agent that asks for work. This is why there is no background reaper
    // task: the only moment a stale claim actually matters is when someone
    // wants the item, and reaping here makes that self-healing. The predicate
    // is indexed (idx_ids_work_queue_claim_expires) and no-ops when nothing
    // has expired.
    if let Err(e) = state.identity_store.work_queue_reap(now) {
        // Not fatal — a claim against a not-yet-reaped queue is still correct,
        // just possibly emptier than it could be. Log and continue rather than
        // failing the caller's request.
        tracing::warn!(target: "workqueue", error = %e, "reap before claim failed");
    }

    let filter = ClaimFilter {
        kind: req.kind.filter(|k| !k.is_empty()),
        agent_id: req.agent_id.clone(),
        groups: req.groups.clone(),
    };
    let lease = req.lease_ms.filter(|&n| n > 0).unwrap_or(DEFAULT_LEASE_MS);

    match state.identity_store.work_queue_claim(&filter, now, lease) {
        Ok(Some(item)) => {
            publish_changed(&state);
            // `attempt` is echoed at the top level as well as inside `item`
            // because every subsequent call (heartbeat/complete/release) MUST
            // pass it back as a fence — see work_queue.rs's ABA note. Making
            // it prominent here is the difference between a caller that
            // fences correctly and one that discovers the requirement from a
            // 404-shaped "not the holder" response.
            (
                StatusCode::OK,
                Json(json!({ "claimed": true, "attempt": item.attempts, "item": item })),
            )
        }
        Ok(None) => (StatusCode::OK, Json(json!({ "claimed": false }))),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("claim failed: {e}")),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct HolderRequest {
    pub agent_id: String,
    /// Fence token — the `attempt` returned by the claim this call belongs to.
    pub attempt: i64,
    #[serde(default)]
    pub result: String,
    #[serde(default)]
    pub lease_ms: Option<i64>,
}

/// Shared shape for the three holder-only transitions. `false` from the store
/// means the caller is not the current holder OR its fence is stale — both are
/// CONFLICT, not NOT_FOUND: the row usually still exists, it just moved on
/// without this caller.
fn holder_result(ok: bool, state: &AppState) -> (StatusCode, Json<Value>) {
    if ok {
        publish_changed(state);
        (StatusCode::OK, Json(json!({ "ok": true })))
    } else {
        (
            StatusCode::CONFLICT,
            Json(json!({
                "ok": false,
                "error": "not the current holder, or this claim has been superseded \
                          (expired and reclaimed) — re-claim before continuing"
            })),
        )
    }
}

pub(super) async fn handle_work_heartbeat(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<HolderRequest>,
) -> (StatusCode, Json<Value>) {
    let lease = req.lease_ms.filter(|&n| n > 0).unwrap_or(DEFAULT_LEASE_MS);
    match state
        .identity_store
        .work_queue_heartbeat(&id, &req.agent_id, req.attempt, now_ms(), lease)
    {
        Ok(ok) => holder_result(ok, &state),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("heartbeat failed: {e}")),
    }
}

pub(super) async fn handle_work_complete(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<HolderRequest>,
) -> (StatusCode, Json<Value>) {
    match state
        .identity_store
        .work_queue_complete(&id, &req.agent_id, req.attempt, &req.result, now_ms())
    {
        Ok(ok) => holder_result(ok, &state),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("complete failed: {e}")),
    }
}

pub(super) async fn handle_work_release(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<HolderRequest>,
) -> (StatusCode, Json<Value>) {
    match state
        .identity_store
        .work_queue_release(&id, &req.agent_id, req.attempt, &req.result, now_ms())
    {
        Ok(ok) => holder_result(ok, &state),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("release failed: {e}")),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct ListQuery {
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

pub(super) async fn handle_work_list(
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> (StatusCode, Json<Value>) {
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    match state.identity_store.work_queue_list(&q.state, limit) {
        Ok(items) => (StatusCode::OK, Json(json!({ "items": items }))),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("list failed: {e}")),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct CancelRequest {
    #[serde(default)]
    pub reason: String,
}

pub(super) async fn handle_work_cancel(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<CancelRequest>,
) -> (StatusCode, Json<Value>) {
    match state.identity_store.work_queue_cancel(&id, &req.reason, now_ms()) {
        Ok(true) => {
            publish_changed(&state);
            (StatusCode::OK, Json(json!({ "ok": true })))
        }
        // Distinct from the holder-conflict case: cancel is deliberately NOT
        // holder-gated (it is an operator action), so a `false` here means the
        // item is already terminal or absent, not that the caller lacks a
        // claim.
        Ok(false) => (
            StatusCode::CONFLICT,
            Json(json!({ "ok": false, "error": "no open or claimed item with that id" })),
        ),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("cancel failed: {e}")),
    }
}
