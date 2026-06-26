// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! HTTP handlers for the persistent cron scheduler.
//!
//! Routes (all auth-gated via `X-AuthKey`):
//!   POST   /agentmux/cron          — create a job
//!   DELETE /agentmux/cron/:id      — delete a job
//!   GET    /agentmux/cron          — list all jobs
//!   PATCH  /agentmux/cron/:id      — pause / resume

use std::str::FromStr;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Json;
use chrono::Utc;
use cron::Schedule;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::backend::storage::cron::CronJob;
use super::AppState;

#[derive(Debug, Deserialize)]
pub struct CronCreateRequest {
    pub name: String,
    /// 5-field UTC cron expression: "min hour dom mon dow"
    pub expression: String,
    pub prompt: String,
    /// Target agent id. Required — no implicit self-targeting from HTTP.
    pub target: String,
    #[serde(default)]
    pub created_by: String,
    pub max_fires: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CronPatchRequest {
    /// "pause" or "resume"
    pub action: String,
}

#[derive(Debug, Serialize)]
pub struct CronJobView {
    pub id: String,
    pub name: String,
    pub expression: String,
    pub prompt: String,
    pub target: String,
    pub enabled: bool,
    pub last_fired: Option<i64>,
    pub fire_count: i64,
    pub max_fires: Option<i64>,
    pub created_at: i64,
    /// ISO-8601 string of the next scheduled fire (UTC), or null if disabled.
    pub next_fire: Option<String>,
}

fn to_view(job: &CronJob) -> CronJobView {
    let next_fire = if job.enabled {
        next_fire_utc(&job.expression)
    } else {
        None
    };
    CronJobView {
        id: job.id.clone(),
        name: job.name.clone(),
        expression: job.expression.clone(),
        prompt: job.prompt.clone(),
        target: job.target.clone(),
        enabled: job.enabled,
        last_fired: job.last_fired,
        fire_count: job.fire_count,
        max_fires: job.max_fires,
        created_at: job.created_at,
        next_fire,
    }
}

/// Compute the next UTC fire time for a 5-field cron expression, returned
/// as an ISO-8601 string. Returns `None` on parse error.
fn next_fire_utc(expression: &str) -> Option<String> {
    let full = format!("0 {}", expression);
    let schedule = Schedule::from_str(&full).ok()?;
    let next = schedule.upcoming(chrono::Utc).next()?;
    Some(next.to_rfc3339())
}

pub(super) async fn handle_cron_create(
    State(state): State<AppState>,
    Json(req): Json<CronCreateRequest>,
) -> (StatusCode, Json<Value>) {
    // Validate expression.
    let full_expr = format!("0 {}", req.expression);
    if Schedule::from_str(&full_expr).is_err() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("invalid cron expression: '{}'", req.expression)})),
        );
    }

    let store = match &state.shared_store {
        Some(s) => s.clone(),
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": "shared store unavailable"}))),
    };

    let job = CronJob {
        id: Uuid::new_v4().to_string(),
        name: req.name.clone(),
        expression: req.expression.clone(),
        prompt: req.prompt.clone(),
        target: req.target.clone(),
        created_by: req.created_by.clone(),
        enabled: true,
        last_fired: None,
        fire_count: 0,
        max_fires: req.max_fires,
        created_at: Utc::now().timestamp(),
    };

    if let Err(e) = store.cron_create(&job) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()})));
    }

    // Register the new job with the live scheduler.
    state.cron_scheduler.schedule_job(&job);

    (StatusCode::CREATED, Json(json!({"job": to_view(&job)})))
}

pub(super) async fn handle_cron_list(
    State(state): State<AppState>,
) -> (StatusCode, Json<Value>) {
    let store = match &state.shared_store {
        Some(s) => s.clone(),
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": "shared store unavailable"}))),
    };

    match store.cron_list() {
        Ok(jobs) => {
            let views: Vec<_> = jobs.iter().map(to_view).collect();
            (StatusCode::OK, Json(json!({"jobs": views})))
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))),
    }
}

pub(super) async fn handle_cron_delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let store = match &state.shared_store {
        Some(s) => s.clone(),
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": "shared store unavailable"}))),
    };

    // Cancel live task first.
    state.cron_scheduler.cancel_job(&id);

    match store.cron_delete(&id) {
        Ok(true) => (StatusCode::OK, Json(json!({"ok": true}))),
        Ok(false) => (StatusCode::NOT_FOUND, Json(json!({"error": "job not found"}))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))),
    }
}

pub(super) async fn handle_cron_patch(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<CronPatchRequest>,
) -> (StatusCode, Json<Value>) {
    let store = match &state.shared_store {
        Some(s) => s.clone(),
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": "shared store unavailable"}))),
    };

    let enabled = match req.action.as_str() {
        "resume" => true,
        "pause" => false,
        other => return (StatusCode::BAD_REQUEST, Json(json!({"error": format!("unknown action '{}' (use 'pause' or 'resume')", other)}))),
    };

    match store.cron_set_enabled(&id, enabled) {
        Ok(true) => {}
        Ok(false) => return (StatusCode::NOT_FOUND, Json(json!({"error": "job not found"}))),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))),
    }

    if enabled {
        if let Ok(Some(job)) = store.cron_get(&id) {
            state.cron_scheduler.schedule_job(&job);
        }
    } else {
        state.cron_scheduler.cancel_job(&id);
    }

    (StatusCode::OK, Json(json!({"ok": true, "enabled": enabled})))
}
