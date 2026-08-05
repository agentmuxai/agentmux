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
use crate::backend::wps::{WaveEvent, EVENT_CRON_CHANGED};
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
    /// Hard expiry bound in seconds since creation. Optional — omit for no
    /// expiry (matching native CronCreate's 7-day default is a caller
    /// choice, not enforced here; AgentMux's cross-agent jobs are often
    /// meant to run indefinitely, e.g. a recurring standup check).
    #[serde(default)]
    pub max_age_secs: Option<i64>,
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
    pub max_age_secs: Option<i64>,
    /// Seconds remaining before `max_age_secs` expiry, computed server-side
    /// (clamped to 0, never negative) so callers don't need their own clock/
    /// date-math to display something meaningful — `None` when there's no
    /// `max_age_secs` bound. Fixes a review finding on the first cut of this
    /// field: the MCP CLI was printing the raw `max_age_secs` bound under an
    /// "expires_in" label, which is only correct at the instant of creation.
    pub expires_in_secs: Option<i64>,
    /// ISO-8601 string of the next scheduled fire (UTC), or null if disabled.
    pub next_fire: Option<String>,
}

fn to_view(job: &CronJob) -> CronJobView {
    let next_fire = if job.enabled {
        next_fire_utc(&job.expression)
    } else {
        None
    };
    let expires_in_secs = job.max_age_secs.map(|max_age| {
        let elapsed = Utc::now().timestamp() - job.created_at;
        (max_age - elapsed).max(0)
    });
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
        expires_in_secs,
        max_age_secs: job.max_age_secs,
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

/// Swarm-pane row shape for one cron job (SPEC_SWARM_LONG_RUNNING_PROCESS_
/// ROWS_2026_07_20 Phase 2) — `CronJobView` plus the `created_by` agent's
/// resolved `block_id`, so the frontend can group it under that agent's row
/// the same way `ShellSummary` does for the Shell bucket.
#[derive(Debug, Serialize)]
pub struct CronSummary {
    pub id: String,
    pub block_id: String,
    pub name: String,
    pub expression: String,
    pub target: String,
    pub created_by: String,
    pub enabled: bool,
    pub last_fired: Option<i64>,
    pub fire_count: i64,
    pub max_fires: Option<i64>,
    /// ISO-8601 string of the next scheduled fire (UTC), or null if disabled.
    pub next_fire: Option<String>,
}

pub(crate) fn to_summary(job: &CronJob, block_id: String) -> CronSummary {
    let next_fire = if job.enabled {
        next_fire_utc(&job.expression)
    } else {
        None
    };
    CronSummary {
        id: job.id.clone(),
        block_id,
        name: job.name.clone(),
        expression: job.expression.clone(),
        target: job.target.clone(),
        created_by: job.created_by.clone(),
        enabled: job.enabled,
        last_fired: job.last_fired,
        fire_count: job.fire_count,
        max_fires: job.max_fires,
        next_fire,
    }
}

fn publish_cron_changed(state: &AppState) {
    state.broker.publish(WaveEvent {
        event: EVENT_CRON_CHANGED.to_string(),
        scopes: vec![],
        sender: String::new(),
        persist: 0,
        data: None,
    });
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
        max_age_secs: req.max_age_secs,
    };

    if let Err(e) = store.cron_create(&job) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()})));
    }

    // Register the new job with the live scheduler.
    state.cron_scheduler.schedule_job(&job);
    publish_cron_changed(&state);

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
        Ok(true) => {
            publish_cron_changed(&state);
            (StatusCode::OK, Json(json!({"ok": true})))
        }
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
    publish_cron_changed(&state);

    (StatusCode::OK, Json(json!({"ok": true, "enabled": enabled})))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_job(enabled: bool, max_fires: Option<i64>, fire_count: i64) -> CronJob {
        CronJob {
            id: "job-1".to_string(),
            name: "nightly build".to_string(),
            expression: "0 9 * * *".to_string(),
            prompt: "run the build".to_string(),
            target: "target-agent".to_string(),
            created_by: "creator-agent".to_string(),
            enabled,
            last_fired: Some(1_700_000_000),
            fire_count,
            max_fires,
            created_at: 1_699_000_000,
            max_age_secs: None,
        }
    }

    #[test]
    fn to_summary_carries_the_resolved_block_id() {
        let job = mk_job(true, None, 3);
        let summary = to_summary(&job, "block-42".to_string());
        assert_eq!(summary.id, "job-1");
        assert_eq!(summary.block_id, "block-42");
        assert_eq!(summary.created_by, "creator-agent");
        assert_eq!(summary.fire_count, 3);
        assert_eq!(summary.max_fires, None);
    }

    #[test]
    fn to_summary_omits_next_fire_when_disabled() {
        let job = mk_job(false, Some(5), 5);
        let summary = to_summary(&job, "block-1".to_string());
        assert!(!summary.enabled);
        assert_eq!(summary.next_fire, None);
    }

    #[test]
    fn to_summary_computes_next_fire_when_enabled() {
        let job = mk_job(true, None, 0);
        let summary = to_summary(&job, "block-1".to_string());
        assert!(summary.enabled);
        assert!(summary.next_fire.is_some());
    }
}
