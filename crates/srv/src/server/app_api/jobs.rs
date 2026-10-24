// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The Warden Jobs tab's websocket RPCs
//! (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §3.3):
//!
//! - `jobs.list`: every job (cron jobs, and Drone graphs that have run) with
//!   its state, next run, recent runs and spend, and Pause all.
//! - `jobs.runs`: one job's runs, newest first, paged by `before`.
//! - `jobs.act`: `run_now`, `pause`, `resume`, `delete` on a cron job, and
//!   the operator's `pause_all` / `resume_all`.
//!
//! The per-job actions go through the Schedule routes' own handlers
//! (`server::cron`), so the caller check (#5392) and the scheduler calls are
//! the same whichever way a job is changed. A connection registered as an
//! agent acts as that agent there; Pause all and Resume all refuse it.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use agentmux_common::time::now_ms;

use crate::backend::jobs::verdict::{cron_job_state, job_failing, run_failed};
use crate::backend::jobs::{self, RunLedger};
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::*;
use crate::backend::storage::cron::CronJob;
use crate::backend::storage::job_incidents::JobIncident;
use crate::backend::storage::job_runs::{JobFlag, JobRun};
use crate::drone::storage::DroneStore;
use crate::server::caller::Caller;
use crate::server::cron::{handle_cron_delete, handle_cron_patch, handle_cron_run_now, CronPatchRequest};

use super::AppState;

/// Runs a row's strip shows (§3.3, "Recent").
const RECENT: i64 = 14;
/// The most runs one `jobs.runs` returns, and its default.
const RUNS_MAX: i64 = 200;
const RUNS_DEFAULT: i64 = 50;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// A run as the tab shows it: the ledger row, and whether it failed (§3.4).
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobRunRow {
    #[serde(flatten)]
    #[ts(flatten)]
    pub run: JobRun,
    pub failed: bool,
}

impl From<JobRun> for JobRunRow {
    fn from(run: JobRun) -> Self {
        JobRunRow { failed: run_failed(&run), run }
    }
}

/// One job in the tab's list (§3.3).
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobView {
    pub id: String,
    /// `cron` or `drone`.
    #[ts(type = r#""cron" | "drone""#)]
    pub source: String,
    pub name: String,
    /// The agent that created it, by name; `""` for a Drone graph (the operator's).
    pub owner: String,
    pub owner_uid: String,
    /// The owner's pane on this instance, when it runs here (Open owner).
    pub owner_block_id: Option<String>,
    /// The agent a cron job messages.
    pub target: String,
    /// The 5-field UTC cron expression; `""` for a Drone graph (manual).
    pub expression: String,
    /// The next scheduled fire (ms epoch), for an enabled cron job.
    #[ts(type = "number | null")]
    pub next_run_ms: Option<i64>,
    /// `active`, `paused`, `paused_all` (Pause all) or `ended` (its own limit).
    #[ts(type = r#""active" | "paused" | "paused_all" | "ended""#)]
    pub state: String,
    /// `skip` or `latest` (§5.2); `""` for a Drone graph.
    pub catch_up: String,
    #[ts(type = "number")]
    pub fire_count: i64,
    #[ts(type = "number | null")]
    pub max_fires: Option<i64>,
    #[ts(type = "number")]
    pub missed_count: i64,
    /// Its newest runs, newest first, at most 14.
    pub recent: Vec<JobRunRow>,
    /// Its newest decisive run failed (§3.4).
    pub failing: bool,
    /// Its open incident (§3.4): the Kanban task its failures went to.
    pub incident: Option<JobIncident>,
    /// Spend since local midnight, and over the last 7 days.
    pub cost_today_usd: f64,
    pub cost_week_usd: f64,
    /// Any of those runs reported a cost; when none did the tab shows "—".
    pub cost_known: bool,
    /// It takes Run now, Pause, Resume and Delete here: a cron job does; a
    /// Drone graph, run from its own pane, doesn't.
    pub actions: bool,
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobsListResult {
    pub jobs: Vec<JobView>,
    /// Pause all, while the operator has it on.
    pub paused_all: Option<JobFlag>,
    #[ts(type = "number")]
    pub now_ms: i64,
}

#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobsRunsRequest {
    pub job_id: String,
    #[serde(default)]
    #[ts(optional, type = "number")]
    pub limit: Option<i64>,
    /// Page: runs started before this (ms epoch).
    #[serde(default)]
    #[ts(optional, type = "number")]
    pub before: Option<i64>,
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobsRunsResult {
    pub runs: Vec<JobRunRow>,
}

#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobsActRequest {
    #[ts(type = r#""run_now" | "pause" | "resume" | "delete" | "pause_all" | "resume_all""#)]
    pub action: String,
    /// The job, for every action but `pause_all` and `resume_all`.
    #[serde(default)]
    #[ts(optional)]
    pub job_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobsActResult {
    pub ok: bool,
    /// `run_now`: the run it started.
    pub run_id: Option<String>,
    /// `run_now`: `delivered`, `missed` or `failed`.
    pub outcome: Option<String>,
    /// Why it didn't happen, in words.
    pub error: Option<String>,
}

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let s = state.clone();
    engine.register_typed(COMMAND_JOBS_LIST, move |_data: Value, _ctx| {
        let s = s.clone();
        async move { list(&s).await }
    });
    let s = state.clone();
    engine.register_typed(COMMAND_JOBS_RUNS, move |req: JobsRunsRequest, _ctx| {
        let s = s.clone();
        async move { runs(&s, req).await }
    });
    let s = state.clone();
    engine.register_typed(COMMAND_JOBS_ACT, move |req: JobsActRequest, ctx| {
        let s = s.clone();
        async move { act(&s, req, &ctx).await }
    });
}

fn ledger(state: &AppState) -> Result<Arc<RunLedger>, String> {
    state.cron_scheduler.runs().cloned().ok_or_else(|| "the run ledger isn't ready".to_string())
}

/// Local midnight today, ms epoch: a day is the operator's (§10, 7).
fn local_midnight_ms() -> i64 {
    let now = chrono::Local::now();
    now.date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|m| m.and_local_timezone(chrono::Local).earliest())
        .map(|m| m.timestamp_millis())
        .unwrap_or_else(|| now.timestamp_millis() - DAY_MS)
}

/// The pane of the registered agent with this UID, else this name.
fn block_of(regs: &[crate::backend::reactive::types::AgentRegistration], uid: &str, name: &str) -> Option<String> {
    let by_uid = (!uid.is_empty()).then(|| regs.iter().find(|r| r.uid.as_deref() == Some(uid))).flatten();
    let by_name = || (!name.is_empty()).then(|| regs.iter().find(|r| r.agent_id.eq_ignore_ascii_case(name))).flatten();
    by_uid.or_else(by_name).map(|r| r.block_id.clone())
}

/// What the list needs from the ledger for one job.
struct Ledgered {
    recent: Vec<JobRunRow>,
    failing: bool,
    incident: Option<JobIncident>,
    cost_today_usd: f64,
    cost_week_usd: f64,
    cost_known: bool,
}

fn ledgered(ledger: &RunLedger, job_id: &str, midnight: i64, week: i64) -> Ledgered {
    let store = ledger.store();
    let runs = store.job_runs_for_job(job_id, RECENT, None).unwrap_or_default();
    // From the newest decisive run, not the strip: a failure stays failing
    // behind any number of skips (§3.4).
    let failing = job_failing(store.job_run_last_decisive(job_id).ok().flatten().as_ref());
    let (today, known_today) = store.job_runs_cost_since(job_id, midnight).unwrap_or((0.0, false));
    let (week_usd, known_week) = store.job_runs_cost_since(job_id, week).unwrap_or((0.0, false));
    Ledgered {
        recent: runs.into_iter().map(JobRunRow::from).collect(),
        failing,
        incident: store.job_incident_open_for(job_id).ok().flatten(),
        cost_today_usd: today,
        cost_week_usd: week_usd,
        cost_known: known_today || known_week,
    }
}

fn cron_view(job: &CronJob, l: Ledgered, paused_all: bool, owner_block_id: Option<String>, now: i64) -> JobView {
    let next_run_ms = if job.enabled { jobs::next_slot_after(&job.expression, now) } else { None };
    JobView {
        id: job.id.clone(),
        source: jobs::SOURCE_CRON.to_string(),
        name: job.name.clone(),
        owner: job.created_by.clone(),
        owner_uid: job.created_by_uid.clone(),
        owner_block_id,
        target: job.target.clone(),
        expression: job.expression.clone(),
        next_run_ms,
        state: cron_job_state(job, paused_all, now / 1000).to_string(),
        catch_up: job.catch_up.clone(),
        fire_count: job.fire_count,
        max_fires: job.max_fires,
        missed_count: job.missed_count,
        recent: l.recent,
        failing: l.failing,
        incident: l.incident,
        cost_today_usd: l.cost_today_usd,
        cost_week_usd: l.cost_week_usd,
        cost_known: l.cost_known,
        actions: true,
    }
}

fn drone_view(id: &str, name: &str, l: Ledgered) -> JobView {
    JobView {
        id: id.to_string(),
        source: jobs::SOURCE_DRONE.to_string(),
        name: name.to_string(),
        owner: String::new(),
        owner_uid: String::new(),
        owner_block_id: None,
        target: String::new(),
        expression: String::new(),
        next_run_ms: None,
        state: crate::backend::jobs::verdict::job_state::ACTIVE.to_string(),
        catch_up: String::new(),
        fire_count: 0,
        max_fires: None,
        missed_count: 0,
        recent: l.recent,
        failing: l.failing,
        incident: l.incident,
        cost_today_usd: l.cost_today_usd,
        cost_week_usd: l.cost_week_usd,
        cost_known: l.cost_known,
        actions: false,
    }
}

/// Local jobs sorted by next run (none last), then by name (§3.3).
fn sort_jobs(jobs: &mut [JobView]) {
    jobs.sort_by(|a, b| {
        let key = |j: &JobView| j.next_run_ms.unwrap_or(i64::MAX);
        key(a).cmp(&key(b)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

async fn list(state: &AppState) -> Result<JobsListResult, String> {
    let ledger = ledger(state)?;
    let shared = state.shared_store.clone();
    let mstore = state.mstore.clone();
    let regs = state.reactive_handler.list_agents();
    tokio::task::spawn_blocking(move || {
        let now = now_ms();
        let (midnight, week) = (local_midnight_ms(), now - 7 * DAY_MS);
        let paused_all = ledger.paused_all();
        let cron_jobs = match &shared {
            Some(s) => s.cron_list().map_err(|e| e.to_string())?,
            None => Vec::new(),
        };
        let mut views: Vec<JobView> = cron_jobs
            .iter()
            .map(|job| {
                let owner_block = block_of(&regs, &job.created_by_uid, &job.created_by);
                cron_view(job, ledgered(&ledger, &job.id, midnight, week), paused_all.is_some(), owner_block, now)
            })
            .collect();
        // Drone graphs are run by hand; list the ones the ledger has runs of.
        for drone in mstore.drone_list().unwrap_or_default() {
            let l = ledgered(&ledger, &drone.id, midnight, week);
            if !l.recent.is_empty() {
                views.push(drone_view(&drone.id, &drone.name, l));
            }
        }
        sort_jobs(&mut views);
        Ok(JobsListResult { jobs: views, paused_all, now_ms: now })
    })
    .await
    .map_err(|e| e.to_string())?
}

async fn runs(state: &AppState, req: JobsRunsRequest) -> Result<JobsRunsResult, String> {
    let ledger = ledger(state)?;
    let limit = req.limit.unwrap_or(RUNS_DEFAULT).clamp(1, RUNS_MAX);
    tokio::task::spawn_blocking(move || {
        let runs = ledger.store().job_runs_for_job(&req.job_id, limit, req.before).map_err(|e| e.to_string())?;
        Ok(JobsRunsResult { runs: runs.into_iter().map(JobRunRow::from).collect() })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Who is asking: the operator's own client (`None`), or the agent this
/// connection registered as, by UID. An agent connection whose UID can't be
/// found is refused: it can't be told apart from another agent.
fn caller_of(state: &AppState, ctx: &RpcContext) -> Result<Option<Caller>, String> {
    if ctx.operator_only("").is_ok() {
        return Ok(None);
    }
    state
        .reactive_handler
        .list_agents()
        .into_iter()
        .find(|r| r.agent_id.eq_ignore_ascii_case(&ctx.agent_id))
        .and_then(|r| r.uid)
        .filter(|u| !u.is_empty())
        .map(|uid| Some(Caller::Agent { uid }))
        .ok_or_else(|| "FORBIDDEN: this agent connection has no identity to act with".to_string())
}

/// A Schedule route's answer as the RPC's: the route's `error` when it
/// refused.
fn answered(status: axum::http::StatusCode, body: &Value) -> Result<JobsActResult, String> {
    let error = body.get("error").and_then(Value::as_str).map(str::to_string);
    if !status.is_success() {
        return Err(error.unwrap_or_else(|| format!("refused ({status})")));
    }
    let outcome = body.get("outcome").and_then(Value::as_str).map(str::to_string);
    let ok = outcome.as_deref().is_none_or(|o| o == "delivered");
    let run_id = body.get("run_id").and_then(Value::as_str).map(str::to_string);
    Ok(JobsActResult { ok, run_id, outcome, error })
}

async fn act(state: &AppState, req: JobsActRequest, ctx: &RpcContext) -> Result<JobsActResult, String> {
    match req.action.as_str() {
        "pause_all" | "resume_all" => {
            ctx.operator_only("only the operator can pause or resume every job")?;
            let on = req.action == "pause_all";
            let ledger = ledger(state)?;
            tokio::task::spawn_blocking(move || ledger.set_paused_all(on, jobs::BY_OPERATOR))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
            state.broker.publish_event::<crate::backend::mps_events::CronChanged>(&(), vec![], 0);
            Ok(JobsActResult { ok: true, ..Default::default() })
        }
        "run_now" | "pause" | "resume" | "delete" => {
            let id = req.job_id.clone().filter(|id| !id.is_empty()).ok_or("which job? (job_id)")?;
            let caller = caller_of(state, ctx)?.map(Extension);
            let st = State(state.clone());
            let (status, Json(body)) = match req.action.as_str() {
                "run_now" => handle_cron_run_now(st, caller, Path(id)).await,
                "delete" => handle_cron_delete(st, caller, Path(id)).await,
                action => handle_cron_patch(st, caller, Path(id), Json(CronPatchRequest { action: action.to_string() })).await,
            };
            let out = answered(status, &body);
            if let Some(ledger) = state.cron_scheduler.runs() {
                ledger.changed(req.job_id.as_deref().unwrap_or(""));
            }
            out
        }
        other => Err(format!("jobs.act: unknown action {other:?}")),
    }
}

#[cfg(test)]
#[path = "jobs_tests.rs"]
mod tests;
