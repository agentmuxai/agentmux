// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! HTTP handlers for the persistent cron scheduler.
//!
//! Routes (all auth-gated via `X-AuthKey`):
//!   POST   /agentmux/cron          — create a job
//!   DELETE /agentmux/cron/:id      — delete a job
//!   GET    /agentmux/cron          — list all jobs, each with its last run
//!   PATCH  /agentmux/cron/:id      — pause / resume
//!   GET    /agentmux/cron/:id/runs — a job's runs, newest first
//!   POST   /agentmux/cron/:id/run  — run it now (a `manual` run)
//!   POST   /agentmux/cron/runs/:run_id/report — a run's own verdict
//!
//! The run routes are the run ledger's (SPEC_JOBS_AND_TEMPLATES_2026_10_24.md
//! §3.2, §3.3 MuxAPI).

use std::str::FromStr;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Json;
use chrono::Utc;
use cron::Schedule;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::backend::cron::FireOutcome;
use crate::backend::jobs;
use crate::backend::storage::cron::CronJob;
use crate::backend::storage::job_runs::{JobRun, OUTCOMES};
use super::caller::Caller;
use super::AppState;

#[derive(Debug, Deserialize)]
pub struct CronCreateRequest {
    pub name: String,
    /// 5-field UTC cron expression: "min hour dom mon dow"
    pub expression: String,
    pub prompt: String,
    /// Target agent id. Required — no implicit self-targeting from HTTP.
    pub target: String,
    /// Identity M3: the target's UID, resolved at the MCP boundary (spec
    /// §5.4 — resolution at authoring time) and CARRIED here. Absent → the
    /// server resolves `target` itself as a counted fallback.
    #[serde(default)]
    pub target_uid: String,
    #[serde(default)]
    pub created_by: String,
    pub max_fires: Option<i64>,
    /// Hard expiry bound in seconds since creation. Optional — omit for no
    /// expiry (matching native CronCreate's 7-day default is a caller
    /// choice, not enforced here; AgentMux's cross-agent jobs are often
    /// meant to run indefinitely, e.g. a recurring standup check).
    #[serde(default)]
    pub max_age_secs: Option<i64>,
    /// What an undelivered fire leaves (jobs spec §5.2): `skip` (default) or
    /// `latest`.
    #[serde(default)]
    pub catch_up: Option<String>,
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
    pub catch_up: String,
    /// The job's newest run in the ledger, if it has one.
    pub last_run: Option<JobRun>,
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
        catch_up: job.catch_up.clone(),
        last_run: None,
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
    state.broker.publish_event::<crate::backend::mps_events::CronChanged>(&(), vec![], 0);
}

pub(super) async fn handle_cron_create(
    State(state): State<AppState>,
    caller: Option<axum::Extension<super::caller::Caller>>,
    Json(req): Json<CronCreateRequest>,
) -> (StatusCode, Json<Value>) {
    super::actor::check_actor(
        &state,
        caller.as_deref(),
        super::actor::ActorSite::CronCreate,
        Some(&req.created_by),
    );
    // Validate expression.
    let full_expr = format!("0 {}", req.expression);
    if Schedule::from_str(&full_expr).is_err() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("invalid cron expression: '{}'", req.expression)})),
        );
    }

    let catch_up = req.catch_up.clone().unwrap_or_else(|| jobs::CATCH_UP_SKIP.to_string());
    if catch_up != jobs::CATCH_UP_SKIP && catch_up != jobs::CATCH_UP_LATEST {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("invalid catch_up '{catch_up}' (use 'skip' or 'latest')")})),
        );
    }

    let store = match &state.shared_store {
        Some(s) => s.clone(),
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": "shared store unavailable"}))),
    };

    // Identity M3: the target's UID is stored only if the caller CARRIED
    // one, resolved at the boundary where the name was typed (spec §5.4).
    // This handler never turns a name into an identity (§2 rule 1): the job
    // fires by `target_uid` when set, so a server-side guess would deliver
    // to whichever agent it found, in any channel (review on #3563). A named
    // target with no carried UID keeps firing by name until M5, counted.
    let target_uid = req.target_uid.trim().to_string();
    if target_uid.is_empty() && !req.target.trim().is_empty() {
        crate::backend::agent_resolve::record_uid_fallback("cron_create.uid_not_carried");
    }

    let job = CronJob {
        id: Uuid::new_v4().to_string(),
        name: req.name.clone(),
        expression: req.expression.clone(),
        prompt: req.prompt.clone(),
        target: req.target.clone(),
        target_uid,
        created_by: req.created_by.clone(),
        // Identity M4c-1 (§6.5.9): the creator's UID is the request's
        // `Caller` — its token — never a body field; `""` when Unattributed.
        created_by_uid: super::caller::attributed_uid(caller.as_deref()),
        enabled: true,
        last_fired: None,
        fire_count: 0,
        max_fires: req.max_fires,
        created_at: Utc::now().timestamp(),
        max_age_secs: req.max_age_secs,
        catch_up,
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
            let ledger = state.cron_scheduler.runs().cloned();
            let views = tokio::task::spawn_blocking(move || {
                jobs.iter()
                    .map(|job| {
                        let mut view = to_view(job);
                        view.last_run = ledger.as_ref().and_then(|l| l.store().job_run_last(&job.id).ok().flatten());
                        view
                    })
                    .collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();
            (StatusCode::OK, Json(json!({"jobs": views})))
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))),
    }
}

/// Whether `caller` may act on `job` (Run now): the operator (the app's own
/// client, which carries no agent token) or the agent that created it
/// (jobs spec §7). By UID when the job recorded one, else by the creator's
/// name against the caller's agent slug.
fn may_act_on(state: &AppState, caller: Option<&Caller>, job: &CronJob) -> bool {
    let Some(uid) = caller.and_then(Caller::uid) else {
        return true;
    };
    if !job.created_by_uid.is_empty() {
        return job.created_by_uid == uid;
    }
    match state.mstore.agent_def_get(uid) {
        Ok(Some(def)) => !job.created_by.is_empty() && def.slug.eq_ignore_ascii_case(&job.created_by),
        _ => false,
    }
}

#[derive(Debug, Deserialize)]
pub struct CronRunsQuery {
    #[serde(default)]
    pub limit: Option<i64>,
    /// Page: runs started before this (ms epoch).
    #[serde(default)]
    pub before: Option<i64>,
}

/// The most runs one history call returns.
const RUNS_MAX: i64 = 200;

type Answer = (StatusCode, Json<Value>);

fn ledger_or_503(state: &AppState) -> Result<std::sync::Arc<jobs::RunLedger>, Answer> {
    state
        .cron_scheduler
        .runs()
        .cloned()
        .ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": "run ledger unavailable"}))))
}

pub(super) async fn handle_cron_runs(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<CronRunsQuery>,
) -> Answer {
    let ledger = match ledger_or_503(&state) {
        Ok(l) => l,
        Err(e) => return e,
    };
    let limit = q.limit.unwrap_or(20).clamp(1, RUNS_MAX);
    match tokio::task::spawn_blocking(move || ledger.store().job_runs_for_job(&id, limit, q.before)).await {
        Ok(Ok(runs)) => (StatusCode::OK, Json(json!({"runs": runs}))),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))),
    }
}

pub(super) async fn handle_cron_run_now(
    State(state): State<AppState>,
    caller: Option<axum::Extension<Caller>>,
    Path(id): Path<String>,
) -> Answer {
    let store = match &state.shared_store {
        Some(s) => s.clone(),
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": "shared store unavailable"}))),
    };
    let job = match store.cron_get(&id) {
        Ok(Some(job)) => job,
        Ok(None) => return (StatusCode::NOT_FOUND, Json(json!({"error": "job not found"}))),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))),
    };
    if !may_act_on(&state, caller.as_deref(), &job) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "only the agent that created this job, or the operator, can run it now"})),
        );
    }
    let triggered_by =
        caller.as_deref().and_then(Caller::uid).map(str::to_string).unwrap_or_else(|| jobs::BY_OPERATOR.to_string());
    let fired = state.cron_scheduler.run_now(&job, &triggered_by).await;
    let (outcome, error) = match &fired.outcome {
        Some(FireOutcome::Delivered { .. }) => ("delivered", None),
        Some(FireOutcome::Missed { .. }) => ("missed", Some(format!("agent {} is not running", job.target))),
        Some(FireOutcome::Failed { error }) => ("failed", Some(error.clone())),
        None => ("failed", Some("no way to deliver".to_string())),
    };
    (StatusCode::OK, Json(json!({"run_id": fired.run_id, "outcome": outcome, "error": error})))
}

#[derive(Debug, Deserialize)]
pub struct CronReportRequest {
    /// `ok`, `findings` or `failed`.
    pub outcome: String,
    #[serde(default)]
    pub summary: String,
    /// Up to five links (strings or `{label, url}`), kept as given.
    #[serde(default)]
    pub links: Option<Value>,
}

/// The most links one report keeps.
const REPORT_LINKS_MAX: usize = 5;

pub(super) async fn handle_cron_report(
    State(state): State<AppState>,
    caller: Option<axum::Extension<Caller>>,
    Path(run_id): Path<String>,
    Json(req): Json<CronReportRequest>,
) -> Answer {
    if req.outcome == "unknown" || !OUTCOMES.contains(&req.outcome.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("invalid outcome '{}' (use ok, findings or failed)", req.outcome)})),
        );
    }
    let links = match req.links {
        None | Some(Value::Null) => Value::Null,
        Some(Value::Array(items)) => Value::Array(items.into_iter().take(REPORT_LINKS_MAX).collect()),
        Some(other) => Value::Array(vec![other]),
    };
    let ledger = match ledger_or_503(&state) {
        Ok(l) => l,
        Err(e) => return e,
    };
    let run = match ledger.store().job_run_get(&run_id) {
        Ok(Some(run)) => run,
        Ok(None) => return (StatusCode::NOT_FOUND, Json(json!({"error": "run not found"}))),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))),
    };
    // The agent that ran it reports; so may the job's creator, or the operator.
    let allowed = match caller.as_deref().and_then(Caller::uid) {
        None => true,
        Some(uid) if run.target_agent_uid == uid => true,
        Some(_) => state
            .shared_store
            .as_ref()
            .and_then(|s| s.cron_get(&run.job_id).ok().flatten())
            .is_some_and(|job| may_act_on(&state, caller.as_deref(), &job)),
    };
    if !allowed {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "only the agent that ran this run can report it"})));
    }
    match ledger.store().job_run_report(&run_id, &req.outcome, &req.summary, &links) {
        Ok(true) => (StatusCode::OK, Json(json!({"ok": true, "run_id": run_id, "outcome": req.outcome}))),
        Ok(false) => (
            StatusCode::CONFLICT,
            Json(json!({"error": format!("run {run_id} never ran ({}): there is nothing to report", run.state)})),
        ),
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
            target_uid: String::new(),
            created_by: "creator-agent".to_string(),
            created_by_uid: String::new(),
            enabled,
            last_fired: Some(1_700_000_000),
            fire_count,
            max_fires,
            created_at: 1_699_000_000,
            max_age_secs: None,
            catch_up: "skip".to_string(),
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

#[cfg(test)]
#[path = "cron_tests.rs"]
mod route_tests;
