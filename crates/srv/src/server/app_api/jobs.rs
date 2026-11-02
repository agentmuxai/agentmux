// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The Warden Jobs tab's websocket RPCs
//! (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §3.3):
//!
//! - `jobs.list`: every job (cron jobs, Drone graphs that have run, and approved
//!   cloud jobs) with its state, next run, recent runs and spend, the cloud events
//!   dropped since start-up, and Pause all.
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

use crate::backend::cron::schedule::JobSchedule;
use crate::backend::job_templates::cloud::CloudExpect;
use crate::backend::job_templates::{self as job_templates, ApprovedTemplate};
use crate::backend::jobs::caps;
use crate::backend::jobs::from_template::{self, TemplatePinView};
use crate::backend::jobs::verdict::{cron_job_state, job_failing, run_failed};
use crate::backend::jobs::{self, RunLedger};
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::*;
use crate::backend::storage::cron::CronJob;
use crate::backend::storage::job_incidents::JobIncident;
use crate::backend::storage::job_runs::{JobFlag, JobRun};
use crate::drone::storage::DroneStore;
use crate::server::caller::Caller;
use crate::server::cron::{handle_cron_delete, handle_cron_patch, handle_cron_run_now};
use crate::server::job_caps;

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
    /// `cron`, `drone`, or `cloud` (an approved cloud template: it runs elsewhere and reports in).
    #[ts(type = r#""cron" | "drone" | "cloud""#)]
    pub source: String,
    pub name: String,
    /// The agent that created it, by name; `""` for a Drone graph (the operator's).
    pub owner: String,
    pub owner_uid: String,
    /// The owner's pane on this instance, when it runs here (Open owner).
    pub owner_block_id: Option<String>,
    /// The agent a cron job messages.
    pub target: String,
    /// The 5-field cron expression, read in `tz`; `""` unless the trigger is
    /// `cron` (a Drone graph is manual).
    pub expression: String,
    /// The trigger's kind: `cron`, `every`, `once` or `manual`.
    #[ts(type = r#""cron" | "every" | "once" | "manual""#)]
    pub trigger_kind: String,
    /// The cron expression's IANA time zone.
    pub tz: String,
    /// `every`: the interval, in seconds.
    #[ts(type = "number | null")]
    pub every_secs: Option<i64>,
    /// `once`: the time (ms epoch).
    #[ts(type = "number | null")]
    pub once_at_ms: Option<i64>,
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
    /// The cap it is over today (`job_cap`, `account_cap` or `message_cap`):
    /// the tab's "Over cap (today)". Its runs resume at the next local midnight.
    pub over_cap: Option<String>,
    /// The most one run may spend, and one day of its runs (§5.3), in dollars.
    #[ts(type = "number | null")]
    pub run_cap_usd: Option<f64>,
    #[ts(type = "number | null")]
    pub day_cap_usd: Option<f64>,
    /// Its agent's provider reports no cost, so the caps can't stop it and only
    /// its timeout limits it: the tab says so.
    pub cost_not_reported: bool,
    /// The template it was made from, and a newer version of it if one is
    /// installed (§5.4).
    pub template: Option<TemplatePinView>,
    /// What a cloud job adds: its reporter, grace, longest run and "Run now". `None` for the others.
    pub cloud: Option<CloudJobView>,
}

/// An approved cloud job (§4.3), as the tab's cloud group shows it. Its runs are the reporter's signed events and the
/// watchdog's misses; nothing here starts it.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CloudJobView {
    /// The reporter's name from the manifest, and whether its key is pinned (an unpinned reporter's events are dropped).
    pub reporter: String,
    pub reporter_pinned: bool,
    /// Minutes after an expected slot before a missing report counts as missed; `None` for a job events start.
    pub grace_minutes: Option<u32>,
    /// The longest a run may go without news before it is `lost`.
    pub max_run_minutes: Option<u32>,
    /// `runNow`: a page that starts it, or what to ask its owner.
    pub run_now_link: Option<String>,
    pub run_now_ask_owner: Option<String>,
    /// When the operator approved it (ms epoch): slots before are not misses.
    #[ts(type = "number")]
    pub approved_at_ms: i64,
}

/// Cloud events refused since this AgentMux started, by why ("unknown job", "bad signature"): never shown as runs.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct DroppedEvents {
    pub reason: String,
    #[ts(type = "number")]
    pub count: u64,
}

/// Settings' caps (`jobs:cap:*`): what a job without figures of its own gets
/// (§5.3). The editor shows them beside the job's own.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CapDefaults {
    pub run_usd: f64,
    pub job_day_usd: f64,
    pub account_day_usd: f64,
    pub messages_per_day: u32,
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobsListResult {
    pub jobs: Vec<JobView>,
    /// Pause all, while the operator has it on.
    pub paused_all: Option<JobFlag>,
    #[ts(type = "number")]
    pub now_ms: i64,
    pub defaults: CapDefaults,
    /// Cloud events dropped since start-up, by reason; empty when none were.
    pub dropped: Vec<DroppedEvents>,
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

/// Local midnight today, ms epoch: a day is the operator's (§10, 7), the same
/// day the spending caps count.
fn local_midnight_ms() -> i64 {
    caps::local_day_start_ms(now_ms())
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
    let next_run_ms = if job.enabled { jobs::next_slot_after(job, now) } else { None };
    JobView {
        id: job.id.clone(),
        source: jobs::SOURCE_CRON.to_string(),
        name: job.name.clone(),
        owner: job.created_by.clone(),
        owner_uid: job.owner_uid().to_string(),
        owner_block_id,
        target: job.target.clone(),
        expression: job.expression.clone(),
        trigger_kind: if job.trigger_kind.is_empty() { "cron".to_string() } else { job.trigger_kind.clone() },
        tz: if job.tz.is_empty() { "UTC".to_string() } else { job.tz.clone() },
        every_secs: job.every_secs,
        once_at_ms: job.once_at_ms,
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
        over_cap: None,
        run_cap_usd: None,
        day_cap_usd: None,
        cost_not_reported: false,
        template: None,
        cloud: None,
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
        trigger_kind: "manual".to_string(),
        tz: "UTC".to_string(),
        every_secs: None,
        once_at_ms: None,
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
        over_cap: None,
        run_cap_usd: None,
        day_cap_usd: None,
        cost_not_reported: false,
        template: None,
        cloud: None,
    }
}

/// An approved cloud template as a job. The owner is the agent the manifest names, found among the agents here.
fn cloud_view(t: &ApprovedTemplate, l: Ledgered, regs: &[crate::backend::reactive::types::AgentRegistration], pinned: bool, now: i64) -> JobView {
    let m = &t.manifest;
    let owner_uid = m.owner.clone().unwrap_or_default();
    let owner = regs.iter().find(|r| r.uid.as_deref() == Some(owner_uid.as_str())).map(|r| r.agent_id.clone()).unwrap_or_else(|| owner_uid.clone());
    let (schedule, grace_minutes) = match &m.expect {
        Some(CloudExpect::Schedule(s)) => (Some(s), Some(s.grace_minutes)),
        _ => (None, None),
    };
    let next_run_ms = schedule.and_then(|s| JobSchedule::cron(&s.cron, &s.tz).ok()).and_then(|sch| sch.next_after(now));
    JobView {
        id: t.id.clone(),
        source: jobs::SOURCE_CLOUD.to_string(),
        name: m.name.clone(),
        owner,
        owner_block_id: block_of(regs, &owner_uid, ""),
        owner_uid,
        target: String::new(),
        expression: schedule.map(|s| s.cron.clone()).unwrap_or_default(),
        trigger_kind: if schedule.is_some() { "cron" } else { "manual" }.to_string(),
        tz: schedule.map(|s| s.tz.clone()).unwrap_or_else(|| "UTC".to_string()),
        every_secs: None,
        once_at_ms: None,
        next_run_ms,
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
        over_cap: None,
        run_cap_usd: None,
        day_cap_usd: None,
        cost_not_reported: false,
        template: None,
        cloud: Some(CloudJobView {
            reporter: m.reporter.as_ref().map(|r| r.name.clone()).unwrap_or_default(),
            reporter_pinned: pinned,
            grace_minutes,
            max_run_minutes: m.max_duration_minutes,
            run_now_link: m.run_now.as_ref().and_then(|r| r.link.clone()),
            run_now_ask_owner: m.run_now.as_ref().and_then(|r| r.ask_owner.clone()),
            approved_at_ms: t.approved_at_ms,
        }),
    }
}

/// Local jobs sorted by next run (none last), then by name (§3.3).
fn sort_jobs(jobs: &mut [JobView]) {
    jobs.sort_by(|a, b| {
        let key = |j: &JobView| j.next_run_ms.unwrap_or(i64::MAX);
        key(a).cmp(&key(b)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

pub(super) async fn list(state: &AppState) -> Result<JobsListResult, String> {
    let ledger = ledger(state)?;
    let jobs_store = crate::server::cron::jobs_store(state);
    let mstore = state.mstore.clone();
    let regs = state.reactive_handler.list_agents();
    let st = state.clone();
    tokio::task::spawn_blocking(move || {
        let now = now_ms();
        let (midnight, week) = (local_midnight_ms(), now - 7 * DAY_MS);
        let paused_all = ledger.paused_all();
        let cron_jobs = match &jobs_store {
            Some(s) => s.cron_list().map_err(|e| e.to_string())?,
            None => Vec::new(),
        };
        let mut templates = from_template::Latest::new();
        let mut views: Vec<JobView> = cron_jobs
            .iter()
            .map(|job| {
                let owner_block = block_of(&regs, job.owner_uid(), &job.created_by);
                let mut view = cron_view(job, ledgered(&ledger, &job.id, midnight, week), paused_all.is_some(), owner_block, now);
                let eff = caps::effective(job.policy.caps.as_ref(), &ledger.limits());
                view.over_cap = ledger.over_cap_today(job).map(str::to_string);
                (view.run_cap_usd, view.day_cap_usd) = (Some(eff.run_usd), Some(eff.day_usd));
                view.cost_not_reported = job_caps::no_cost_provider(&st, job).is_some();
                view.template = job.template.as_ref().map(|pin| from_template::pin_view(pin, &mut templates));
                view
            })
            .collect();
        // Drone graphs are run by hand; list the ones the ledger has runs of.
        for drone in mstore.drone_list().unwrap_or_default() {
            let l = ledgered(&ledger, &drone.id, midnight, week);
            if !l.recent.is_empty() {
                views.push(drone_view(&drone.id, &drone.name, l));
            }
        }
        // Approved cloud templates: jobs that run elsewhere and report in.
        if let Some(service) = job_templates::service() {
            for t in service.approved_clouds() {
                let pinned = t.manifest.reporter.as_ref().is_some_and(|r| service.reporters().fingerprint_of_pinned(&r.key).is_some());
                views.push(cloud_view(&t, ledgered(&ledger, &t.id, midnight, week), &regs, pinned, now));
            }
        }
        sort_jobs(&mut views);
        let limits = job_caps::limits(&st);
        let defaults = CapDefaults {
            run_usd: limits.run_usd,
            job_day_usd: limits.job_day_usd,
            account_day_usd: limits.account_day_usd,
            messages_per_day: limits.messages_per_day,
        };
        let dropped = crate::backend::jobs::cloud_events::drops()
            .into_iter()
            .map(|(reason, count)| DroppedEvents { reason: reason.to_string(), count })
            .collect();
        Ok(JobsListResult { jobs: views, paused_all, now_ms: now, defaults, dropped })
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

/// Who is asking: the operator's own window (`None`), which needs the host's
/// proof, or the agent this connection registered as, by UID. An agent
/// connection whose UID can't be found is refused: it can't be told apart
/// from another agent.
fn caller_of(state: &AppState, ctx: &RpcContext) -> Result<Option<Caller>, String> {
    if ctx.agent_id.is_empty() {
        ctx.operator_only("a job is acted on by the operator or by an agent")?;
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
                action => handle_cron_patch(st, caller, Path(id), Json(serde_json::json!({ "action": action }))).await,
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
