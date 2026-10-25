// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! HTTP handlers for jobs (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md
//! §5.1) and the persistent cron scheduler under them.
//!
//! Routes (all auth-gated via `X-AuthKey`), under `/agentmux/jobs`, and the
//! same under `/agentmux/cron`, kept as aliases for one release:
//!   POST   /agentmux/jobs          — create a job: a job definition
//!                                    (`jobVersion` 1), or the cron shape
//!                                    the Schedule tool sends
//!   DELETE /agentmux/jobs/:id      — delete a job
//!   GET    /agentmux/jobs          — list all jobs, each with its last run
//!   PATCH  /agentmux/jobs/:id      — pause / resume, or edit its definition
//!   GET    /agentmux/jobs/:id/runs — a job's runs, newest first
//!   POST   /agentmux/jobs/:id/run  — run it now (a `manual` run)
//!   POST   /agentmux/jobs/runs/:run_id/report — a run's own verdict
//!
//! The run routes are the run ledger's (§3.2, §3.3 MuxAPI). Who may do what
//! is §7: [`Who`].


use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::backend::cron::schedule::JobSchedule;
use crate::backend::cron::FireOutcome;
use crate::backend::jobs;
use crate::backend::jobs::definition::{Action, Check, JobDefinition, LATER};
use super::job_caps::{self, Origin};
use crate::backend::storage::cron::CronJob;
use crate::backend::storage::job_runs::{JobRun, OUTCOMES};
use crate::backend::storage::store::Store;
use std::sync::Arc;
use super::caller::Caller;
use super::AppState;

#[derive(Debug, Deserialize)]
pub struct CronCreateRequest {
    pub name: String,
    /// 5-field cron expression: "min hour dom mon dow", read in `tz`.
    pub expression: String,
    pub prompt: String,
    /// Target agent id. Required — no implicit self-targeting from HTTP.
    pub target: String,
    /// Identity M3: the target's UID, resolved at the MCP boundary (spec
    /// §5.4 — resolution at authoring time) and CARRIED here. Absent → the
    /// job fires by `target`, counted.
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
    /// `latest`. The same as `policy.catch_up`; this one wins.
    #[serde(default)]
    pub catch_up: Option<String>,
    /// The expression's IANA time zone; UTC when absent.
    #[serde(default)]
    pub tz: Option<String>,
    /// The definition's `policy` (§5.1).
    #[serde(default)]
    pub policy: Option<Value>,
    /// A template to create the job from: refused until templates land.
    #[serde(default)]
    pub template: Option<Value>,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct CronJobView {
    pub id: String,
    pub name: String,
    pub expression: String,
    /// The cron expression's time zone (`UTC` unless it says otherwise).
    pub tz: String,
    pub trigger_kind: String,
    pub action_kind: String,
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
    /// ISO-8601 string of the next scheduled fire (UTC), or null if disabled
    /// or there is none.
    pub next_fire: Option<String>,
    pub catch_up: String,
    /// The job's newest run in the ledger, if it has one.
    pub last_run: Option<JobRun>,
    /// Scheduled fires that reached no one, and the last one's time and
    /// reason (`CronJob::missed_count`).
    pub missed_count: i64,
    pub last_missed_at: Option<i64>,
    pub last_miss_reason: String,
    /// The cap its runs are over today (`job_cap`, `account_cap` or
    /// `message_cap`), if any; they resume at the operator's next midnight.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub over_cap: Option<String>,
    /// The whole job definition (§5.1).
    pub definition: JobDefinition,
}

fn to_view(job: &CronJob) -> CronJobView {
    let expires_in_secs = job.max_age_secs.map(|max_age| {
        let elapsed = Utc::now().timestamp() - job.created_at;
        (max_age - elapsed).max(0)
    });
    let definition = JobDefinition::from_job(job);
    CronJobView {
        id: job.id.clone(),
        name: job.name.clone(),
        expression: job.expression.clone(),
        tz: if job.tz.is_empty() { "UTC".to_string() } else { job.tz.clone() },
        trigger_kind: definition.trigger.kind().to_string(),
        action_kind: definition.action.kind().to_string(),
        prompt: job.prompt.clone(),
        target: job.target.clone(),
        enabled: job.enabled,
        last_fired: job.last_fired,
        fire_count: job.fire_count,
        max_fires: job.max_fires,
        created_at: job.created_at,
        expires_in_secs,
        max_age_secs: job.max_age_secs,
        next_fire: next_fire(job),
        catch_up: job.catch_up.clone(),
        last_run: None,
        missed_count: job.missed_count,
        last_missed_at: job.last_missed_at,
        last_miss_reason: job.last_miss_reason.clone(),
        over_cap: None,
        definition,
    }
}

/// The job's next fire (its trigger's next slot, in its time zone) as an
/// ISO-8601 UTC string; `None` when disabled, when its trigger has no next
/// slot, or when it doesn't read.
fn next_fire(job: &CronJob) -> Option<String> {
    if !job.enabled {
        return None;
    }
    let next = JobSchedule::of(job).ok()?.next_after(agentmux_common::time::now_ms())?;
    Some(chrono::DateTime::from_timestamp_millis(next)?.to_rfc3339())
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
    /// Scheduled fires that reached no one; Swarm's Cron bucket shows them.
    pub missed_count: i64,
    pub last_miss_reason: String,
}

pub(crate) fn to_summary(job: &CronJob, block_id: String) -> CronSummary {
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
        next_fire: next_fire(job),
        missed_count: job.missed_count,
        last_miss_reason: job.last_miss_reason.clone(),
    }
}

/// Where jobs are (`db_jobs`): the shared store, beside the `db_cron_jobs`
/// it replaced, or the work store of a `task dev` build, which keeps its own
/// (`agentmux_common::work_store`). Every job read and write goes through here,
/// so the store can move in one place.
pub(crate) fn jobs_store(state: &AppState) -> Option<Arc<Store>> {
    state.cron_store.clone()
}

/// Where the run ledger is (`db_job_runs`): the identity store, or the work store of
/// a `task dev` build, which keeps its own (`agentmux_common::work_store`).
pub(crate) fn ledger_store(state: &AppState) -> Arc<Store> {
    state.work_store.clone()
}

/// Tests: keep jobs in the test state's own store (which carries every
/// schema), as [`jobs_store`] and [`ledger_store`] would find them.
#[cfg(test)]
pub(crate) fn use_test_store_for_jobs(state: &mut AppState) {
    state.cron_store = Some(state.mstore.clone());
}

/// Where tasks are filed (the work queue and its cards): the identity store.
pub(crate) fn work_store(state: &AppState) -> Arc<Store> {
    state.identity_store.clone()
}

/// [`jobs_store`], or the answer when there is none.
fn jobs_store_or_503(state: &AppState) -> Result<Arc<Store>, Answer> {
    jobs_store(state).ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": "shared store unavailable"}))))
}

fn publish_cron_changed(state: &AppState) {
    state.broker.publish_event::<crate::backend::mps_events::CronChanged>(&(), vec![], 0);
}

/// Who is asking, for the rules of §7. The operator is the app's own
/// client, which carries no agent token (`Caller::Unattributed`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Who {
    Operator,
    Agent { uid: String, slug: String, container: bool },
}

fn who(state: &AppState, caller: Option<&Caller>) -> Who {
    match caller.and_then(Caller::uid) {
        None => Who::Operator,
        Some(uid) => {
            let def = state.mstore.agent_def_get(uid).ok().flatten();
            Who::Agent {
                uid: uid.to_string(),
                slug: def.as_ref().map(|d| d.slug.clone()).unwrap_or_default(),
                container: def.is_some_and(|d| d.agent_type == "container"),
            }
        }
    }
}

/// What a container agent gets for anything but reading (§7).
const CONTAINER_READ_ONLY: &str = "a container agent can list jobs and read their history, not create, change or run them";

fn forbidden(why: impl Into<String>) -> Answer {
    (StatusCode::FORBIDDEN, Json(json!({"error": why.into()})))
}

fn bad_request(why: impl Into<String>) -> Answer {
    (StatusCode::BAD_REQUEST, Json(json!({"error": why.into()})))
}

/// Refuse a container agent's change.
fn refuse_container(state: &AppState, caller: Option<&Caller>) -> Result<(), Answer> {
    match who(state, caller) {
        Who::Agent { container: true, .. } => Err(forbidden(CONTAINER_READ_ONLY)),
        _ => Ok(()),
    }
}

/// Whether `who` may define a job like `def` (§7): the operator any job; an
/// agent a `message` or `task` job it owns. A `spawn` job needs the
/// operator's approval once (the spawn consent rule, made at creation since
/// its runs are unattended), which has no flow yet, so an agent can't create
/// one. `current_owner` is the job's owner on an edit.
pub(crate) fn may_define(who: &Who, def: &JobDefinition, current_owner: Option<&str>) -> Result<(), String> {
    let Who::Agent { uid, container, .. } = who else { return Ok(()) };
    if *container {
        return Err(CONTAINER_READ_ONLY.to_string());
    }
    if let Action::Spawn { leader, .. } = &def.action {
        if leader != uid {
            return Err("an agent can create a spawn job only with itself as the leader".to_string());
        }
        return Err(
            "a spawn job an agent creates needs the operator's approval once, which isn't available yet: ask the operator to create it"
                .to_string(),
        );
    }
    let owner = def.owner.trim();
    let allowed = owner.is_empty() || owner == uid || current_owner == Some(owner);
    if !allowed {
        return Err("an agent's job is owned by that agent: leave owner out, or name yourself".to_string());
    }
    Ok(())
}

/// What this build can't run yet, though the definition is valid. Refused
/// rather than stored and silently ignored: `max_turns` is a flag at the CLI's
/// launch, which a message to a running agent can't set.
pub(crate) fn runnable_yet(def: &JobDefinition) -> Result<(), String> {
    match def.policy.max_turns {
        Some(_) => Err(format!("policy.max_turns is {LATER}")),
        None => Ok(()),
    }
}

/// What a spawn job asks of anyone who isn't the app's own controls: it
/// starts agents that spend until someone stops them.
const SPAWN_FROM_THE_APP: &str =
    "a spawn job starts workers that spend until stopped: only the operator's own host creates and runs it, not this route";

/// A message's `to` as the job's target `(name, uid)`: a UID names that
/// agent; anything else is kept as a name, never resolved here (identity
/// spec §5.4: the job then fires by name, counted).
fn target_of(state: &AppState, to: &str) -> (String, String) {
    match state.mstore.agent_def_get(to) {
        Ok(Some(def)) if !to.is_empty() => (def.slug, to.to_string()),
        _ => {
            if !to.is_empty() {
                crate::backend::agent_resolve::record_uid_fallback("jobs_create.uid_not_carried");
            }
            (to.to_string(), String::new())
        }
    }
}

/// The definition the cron shape describes: a `cron` trigger in `tz` and a
/// `message` action to the target.
fn legacy_definition(req: &CronCreateRequest, check: Check) -> Result<JobDefinition, String> {
    let mut policy = match &req.policy {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(p)) => p.clone(),
        Some(_) => return Err("policy is an object".to_string()),
    };
    if let Some(catch_up) = &req.catch_up {
        if catch_up != jobs::CATCH_UP_SKIP && catch_up != jobs::CATCH_UP_LATEST {
            return Err(format!("invalid catch_up '{catch_up}' (use 'skip' or 'latest')"));
        }
        policy.insert("catch_up".into(), json!(catch_up));
    }
    let to = if req.target_uid.trim().is_empty() { req.target.trim() } else { req.target_uid.trim() };
    let mut def = json!({
        "jobVersion": jobs::definition::JOB_VERSION,
        "name": req.name,
        "trigger": { "kind": "cron", "cron": req.expression, "tz": req.tz.as_deref().unwrap_or("UTC") },
        "action": { "kind": "message", "to": to, "brief": req.prompt },
        "policy": policy,
    });
    for (key, value) in [("template", &req.template), ("params", &req.params)] {
        if let Some(v) = value.as_ref().filter(|v| !v.is_null()) {
            def[key] = v.clone();
        }
    }
    jobs::from_template::apply(&mut def, None)?;
    JobDefinition::parse(&def, check)
}

pub(super) async fn handle_cron_create(
    State(state): State<AppState>,
    caller: Option<axum::Extension<super::caller::Caller>>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    create_job(&state, caller.as_deref(), body, Origin::default())
}

/// Create a job from a definition or the cron shape. `origin.from_host`: the
/// operator's decision, from the host route (`handle_host_jobs`) or the
/// app's own proven window (`jobs.save`): the only way to create a job that
/// starts workers, to raise a cap, or to confirm a job on a provider that
/// reports no cost. A definition that names a template is made from it
/// (`jobs::from_template`).
pub(crate) fn create_job(state: &AppState, caller: Option<&Caller>, body: Value, origin: Origin) -> (StatusCode, Json<Value>) {
    let from_host = origin.from_host;
    let check = Check { fresh: true, now_ms: agentmux_common::time::now_ms() };
    let who = if from_host { Who::Operator } else { who(state, caller) };
    // The cron shape (no `jobVersion`) carries its target, creator and limits
    // beside the definition.
    let legacy = match body.get("jobVersion") {
        Some(_) => None,
        None => match serde_json::from_value::<CronCreateRequest>(body.clone()) {
            Ok(req) => Some(req),
            Err(e) => return bad_request(format!("invalid job: {e}")),
        },
    };
    let parsed = match &legacy {
        Some(req) => {
            super::actor::check_actor(state, caller, super::actor::ActorSite::CronCreate, Some(&req.created_by));
            legacy_definition(req, check)
        }
        None => {
            let mut body = body.clone();
            jobs::from_template::apply(&mut body, None).and_then(|()| JobDefinition::parse(&body, check))
        }
    };
    let mut def = match parsed {
        Ok(def) => def,
        Err(e) => return bad_request(e),
    };
    if let Err(e) = may_define(&who, &def, None) {
        return forbidden(e);
    }
    if matches!(def.action, Action::Spawn { .. }) && !from_host {
        return forbidden(SPAWN_FROM_THE_APP);
    }
    if let Err(e) = runnable_yet(&def) {
        return bad_request(e);
    }
    let notes = match job_caps::apply_rules(state, &mut def, None, origin) {
        Ok(notes) => notes,
        Err(refused) => return refused,
    };

    let (created_by, owner) = match &who {
        Who::Operator => (jobs::BY_OPERATOR.to_string(), def.owner.trim().to_string()),
        Who::Agent { uid, slug, .. } => (if slug.is_empty() { uid.clone() } else { slug.clone() }, uid.clone()),
    };
    let mut job = CronJob {
        id: Uuid::new_v4().to_string(),
        created_by,
        // Identity M4c-1 (§6.5.9): the creator's UID is the request's
        // `Caller` — its token — never a body field; `""` when Unattributed.
        created_by_uid: super::caller::attributed_uid(caller),
        created_at: Utc::now().timestamp(),
        ..Default::default()
    };
    match &legacy {
        Some(req) => {
            // Identity M3: the target's UID is stored only if the caller
            // CARRIED one, resolved at the boundary where the name was typed
            // (spec §5.4). A named target with no carried UID keeps firing by
            // name until M5, counted.
            let target_uid = req.target_uid.trim().to_string();
            if target_uid.is_empty() && !req.target.trim().is_empty() {
                crate::backend::agent_resolve::record_uid_fallback("cron_create.uid_not_carried");
            }
            def.apply_to(&mut job, &|_| (req.target.clone(), target_uid.clone()));
            job.created_by = req.created_by.clone();
            (job.max_fires, job.max_age_secs) = (req.max_fires, req.max_age_secs);
        }
        None => def.apply_to(&mut job, &|to| target_of(state, to)),
    }
    // An agent's job is its own (§7); the operator's has the owner it names.
    job.owner = owner;

    let store = match jobs_store_or_503(state) {
        Ok(store) => store,
        Err(e) => return e,
    };
    if let Err(e) = store.cron_create(&job) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()})));
    }

    // Register the new job with the live scheduler.
    if job.enabled {
        state.cron_scheduler.schedule_job(&job);
    }
    publish_cron_changed(state);

    (StatusCode::CREATED, Json(json!({"job": to_view(&job), "notes": notes})))
}

pub(super) async fn handle_cron_list(
    State(state): State<AppState>,
) -> (StatusCode, Json<Value>) {
    let store = match jobs_store_or_503(&state) {
        Ok(store) => store,
        Err(e) => return e,
    };
    match store.cron_list() {
        Ok(jobs) => {
            let ledger = state.cron_scheduler.runs().cloned();
            let (views, paused_all) = tokio::task::spawn_blocking(move || {
                let views = jobs
                    .iter()
                    .map(|job| {
                        let mut view = to_view(job);
                        view.last_run = ledger.as_ref().and_then(|l| l.store().job_run_last(&job.id).ok().flatten());
                        view.over_cap = ledger.as_ref().and_then(|l| l.over_cap_today(job)).map(str::to_string);
                        view
                    })
                    .collect::<Vec<_>>();
                // Pause all (jobs spec §3.3), so the Schedule tool can say so.
                (views, ledger.as_ref().and_then(|l| l.paused_all()))
            })
            .await
            .unwrap_or_default();
            (StatusCode::OK, Json(json!({"jobs": views, "paused_all": paused_all})))
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))),
    }
}

/// Whether `caller` may act on `job` (Run now): the operator (the app's own
/// client, which carries no agent token) or the agent that created or owns
/// it (jobs spec §7). By UID when the job recorded one, else by the
/// creator's name against the caller's agent slug.
fn may_act_on(state: &AppState, caller: Option<&Caller>, job: &CronJob) -> bool {
    let Some(uid) = caller.and_then(Caller::uid) else {
        return true;
    };
    if job.owner_uid() == uid {
        return true;
    }
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

/// Deliver on this instance (`jekt_files::inject_here`). A delivery another
/// instance forwarded here whose `request_id` is a run another channel
/// fired is followed here, before the answer goes back (#1988).
pub(super) async fn inject_here_following_runs(
    state: &AppState,
    req: &crate::backend::reactive::types::InjectionRequest,
) -> crate::backend::reactive::types::InjectionResponse {
    let delivery_started = agentmux_common::time::now_ms();
    let resp = super::jekt_files::inject_here(state, req);
    let run_id = req.request_id.clone().unwrap_or_default();
    let block_id = resp.block_id.clone().unwrap_or_default();
    if resp.success && req.forward_hops > 0 && !run_id.is_empty() && !block_id.is_empty() {
        if let Some(ledger) = state.cron_scheduler.runs().cloned() {
            let _ = tokio::task::spawn_blocking(move || ledger.follow_forwarded(&run_id, &block_id, delivery_started)).await;
        }
    }
    resp
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
    if let Err(refused) = refuse_container(&state, caller.as_deref()) {
        return refused;
    }
    let store = match jobs_store_or_503(&state) {
        Ok(store) => store,
        Err(e) => return e,
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
    if job.action_kind == "spawn" {
        return forbidden(SPAWN_FROM_THE_APP);
    }
    let triggered_by =
        caller.as_deref().and_then(Caller::uid).map(str::to_string).unwrap_or_else(|| jobs::BY_OPERATOR.to_string());
    run_job_now(&state, &job, &triggered_by).await
}

/// Run `job` now and say how it went.
async fn run_job_now(state: &AppState, job: &CronJob, triggered_by: &str) -> Answer {
    let fired = state.cron_scheduler.run_now(job, triggered_by).await;
    if let Some(why) = fired.waiting {
        let body = json!({"run_id": fired.run_id, "outcome": "queued", "error": null, "waiting_for": why});
        return (StatusCode::OK, Json(body));
    }
    let (outcome, error) = match &fired.outcome {
        Some(FireOutcome::Delivered { .. }) => ("delivered", None),
        Some(FireOutcome::Missed { .. }) => ("missed", Some(format!("agent {} is not running", job.target))),
        Some(FireOutcome::Failed { error }) => ("failed", Some(error.clone())),
        None => ("failed", Some("no way to deliver".to_string())),
    };
    (StatusCode::OK, Json(json!({"run_id": fired.run_id, "outcome": outcome, "error": error})))
}

/// `POST /api/v1/host/jobs`: what the operator decides in the Jobs tab that
/// starts agents, relayed by the host with its own token. `action` is
/// `create` (`definition`: a job definition, a `spawn` job included),
/// `update` (`id`, `fields`: an edit, which may raise the job's caps) or
/// `run_now` (`id`). `confirm_no_cost: true` confirms a job whose agent's
/// provider reports no cost (§5.3).
pub(crate) async fn handle_host_jobs(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if !super::ui_handlers::from_host(&state, &headers).await {
        return forbidden("only the AgentMux host can create, change or run a job that starts workers, or raise a spending cap").into_response();
    }
    let origin = Origin { from_host: true, confirm_no_cost: body.get("confirm_no_cost").and_then(Value::as_bool) == Some(true) };
    match body.get("action").and_then(Value::as_str).unwrap_or("") {
        "create" => create_job(&state, None, body.get("definition").cloned().unwrap_or(Value::Null), origin).into_response(),
        "update" => {
            let id = body.get("id").and_then(Value::as_str).unwrap_or("");
            patch_job(&state, None, id, body.get("fields").cloned().unwrap_or(Value::Null), origin).into_response()
        }
        "run_now" => {
            let id = body.get("id").and_then(Value::as_str).unwrap_or("");
            let store = match jobs_store_or_503(&state) {
                Ok(store) => store,
                Err(e) => return e.into_response(),
            };
            match store.cron_get(id) {
                Ok(Some(job)) => run_job_now(&state, &job, jobs::BY_OPERATOR).await.into_response(),
                Ok(None) => (StatusCode::NOT_FOUND, Json(json!({"error": "job not found"}))).into_response(),
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response(),
            }
        }
        other => bad_request(format!("unknown action {other:?} (use create, update or run_now)")).into_response(),
    }
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

/// The most links one report keeps, and the longest url and label.
const REPORT_LINKS_MAX: usize = 5;
const REPORT_URL_MAX: usize = 2048;
const REPORT_LABEL_MAX: usize = 120;

/// The links a report keeps: https URLs, given as strings or as
/// `{label, url}`, within the length bounds, at most five. Anything else is
/// dropped: the ledger shows them to other agents and the UI.
fn report_links(given: Option<Value>) -> Value {
    let https = |u: &str| u.len() <= REPORT_URL_MAX && u.starts_with("https://") && u.len() > "https://".len();
    let items = match given {
        None | Some(Value::Null) => return Value::Null,
        Some(Value::Array(items)) => items,
        Some(other) => vec![other],
    };
    let kept: Vec<Value> = items
        .into_iter()
        .filter_map(|item| match item {
            Value::String(u) if https(&u) => Some(Value::String(u)),
            Value::Object(o) => {
                let url = o.get("url").and_then(Value::as_str).filter(|u| https(u))?;
                let label = o.get("label").and_then(Value::as_str).unwrap_or("");
                (label.chars().count() <= REPORT_LABEL_MAX).then(|| json!({ "label": label, "url": url }))
            }
            _ => None,
        })
        .take(REPORT_LINKS_MAX)
        .collect();
    Value::Array(kept)
}

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
    let links = report_links(req.links);
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
    // A run whose receiver's UID couldn't be resolved at delivery is
    // reported only from the pane that took it, never by another agent.
    let allowed = match caller.as_deref().and_then(Caller::uid) {
        None => true,
        Some(uid) => {
            let ran_it = if run.target_agent_uid.is_empty() {
                !run.block_id.is_empty() && state.reactive_handler.block_for_uid(uid).as_deref() == Some(run.block_id.as_str())
            } else {
                run.target_agent_uid == uid
            };
            ran_it
                || jobs_store(&state)
                    .and_then(|s| s.cron_get(&run.job_id).ok().flatten())
                    .is_some_and(|job| may_act_on(&state, caller.as_deref(), &job))
        }
    };
    if !allowed {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "only the agent that ran this run can report it"})));
    }
    match ledger.store().job_run_report(&run_id, &req.outcome, &req.summary, &links) {
        Ok(true) => {
            ledger.changed(&run.job_id);
            (StatusCode::OK, Json(json!({"ok": true, "run_id": run_id, "outcome": req.outcome})))
        }
        Ok(false) => (
            StatusCode::CONFLICT,
            Json(json!({"error": format!("run {run_id} never ran ({}): there is nothing to report", run.state)})),
        ),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))),
    }
}

/// Whether `caller` may pause, resume or delete `job`. An agent (a request
/// carrying its token) may change only a job it created or owns, or one
/// aimed at it; everyone else, the UI included, may change any job.
/// Unattributed requests can't be refused: at the same-user boundary one
/// that omits its token looks like the UI (caller.rs). This stops an agent
/// changing another agent's job by mistake, not a determined one. A job with
/// no recorded creator UID (created before identity M4c-1, or unattributed)
/// is open to any caller. Editing the definition is narrower: [`may_edit_job`].
fn may_change_job(caller: Option<&super::caller::Caller>, job: &CronJob) -> bool {
    let Some(uid) = caller.and_then(super::caller::Caller::uid) else {
        return true;
    };
    job.created_by_uid.is_empty() || may_edit_job(caller, job) || job.target_uid == uid
}

/// Whether `caller` may edit `job`'s definition: the operator, or the agent
/// that created or owns it (§7). The agent a job messages may pause it, not
/// rewrite it. A job with no creator (the operator's, or one from before
/// creators were recorded) is the operator's to edit.
fn may_edit_job(caller: Option<&super::caller::Caller>, job: &CronJob) -> bool {
    let Some(uid) = caller.and_then(super::caller::Caller::uid) else {
        return true;
    };
    !uid.is_empty() && (job.created_by_uid == uid || job.owner_uid() == uid)
}

/// The store and the job, or the response refusing the change: no store, a
/// container agent, no such job, or a caller [`may_change_job`] refuses.
fn job_for_change(
    state: &AppState,
    caller: Option<&super::caller::Caller>,
    id: &str,
) -> Result<(Arc<Store>, CronJob), Answer> {
    let store = jobs_store_or_503(state)?;
    refuse_container(state, caller)?;
    match store.cron_get(id) {
        Ok(Some(job)) if may_change_job(caller, &job) => Ok((store, job)),
        Ok(Some(_)) => Err(forbidden("an agent can change only a scheduled job it created or one aimed at it")),
        Ok(None) => Err((StatusCode::NOT_FOUND, Json(json!({"error": "job not found"})))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()})))),
    }
}

pub(super) async fn handle_cron_delete(
    State(state): State<AppState>,
    caller: Option<axum::Extension<super::caller::Caller>>,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let store = match job_for_change(&state, caller.as_deref(), &id) {
        Ok((store, _)) => store,
        Err(refused) => return refused,
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

/// A job's fields an edit can't change: srv sets them.
const FIXED_FIELDS: [&str; 3] = ["id", "jobVersion", "created_by"];

/// `PATCH`: `{"action": "pause" | "resume"}`, or an edit: definition fields
/// (§5.1) laid over the job's definition, which is then read as a create's
/// is. The 15-minute minimum applies only when the edit sets the trigger or
/// the action, so a job kept as it was isn't refused for a rule newer than
/// it.
pub(super) async fn handle_cron_patch(
    State(state): State<AppState>,
    caller: Option<axum::Extension<super::caller::Caller>>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    patch_job(&state, caller.as_deref(), &id, body, Origin::default())
}

/// [`handle_cron_patch`]'s work. `origin.from_host`: the operator's edit,
/// from the host route or the app's own proven window, which may raise caps,
/// confirm a job on a provider that reports no cost, and change a spawn job.
pub(crate) fn patch_job(state: &AppState, caller: Option<&Caller>, id: &str, body: Value, origin: Origin) -> (StatusCode, Json<Value>) {
    let (store, job) = match job_for_change(state, caller, id) {
        Ok(found) => found,
        Err(refused) => return refused,
    };
    if let Some(action) = body.get("action").and_then(Value::as_str) {
        return set_enabled(state, &store, id, action);
    }
    let Some(fields) = body.as_object() else { return bad_request("an edit is a JSON object of job fields") };
    if job.action_kind == "spawn" && !origin.from_host {
        return forbidden("a spawn job can be paused, resumed or deleted here; to change it, delete it and create it again from the Jobs tab");
    }
    if !may_edit_job(caller, &job) {
        return forbidden("an agent can edit only a job it created or owns");
    }
    let before = JobDefinition::from_job(&job);
    let mut merged = serde_json::to_value(&before).unwrap_or_default();
    for (key, value) in fields {
        if FIXED_FIELDS.contains(&key.as_str()) {
            if merged.get(key) != Some(value) {
                return bad_request(format!("{key} can't be changed"));
            }
            continue;
        }
        merged[key.as_str()] = value.clone();
    }
    if jobs::from_template::edit_needs_render(fields, &before) {
        if let Err(e) = jobs::from_template::apply(&mut merged, before.template.as_ref()) {
            return bad_request(e);
        }
    }
    let fresh = fields.contains_key("trigger") || fields.contains_key("action");
    let mut def = match JobDefinition::parse(&merged, Check { fresh, now_ms: agentmux_common::time::now_ms() }) {
        Ok(def) => def,
        Err(e) => return bad_request(e),
    };
    let editor = if origin.from_host { Who::Operator } else { who(state, caller) };
    if let Err(e) = may_define(&editor, &def, Some(job.owner_uid())) {
        return forbidden(e);
    }
    if matches!(def.action, Action::Spawn { .. }) && !(origin.from_host && job.action_kind == "spawn") {
        return forbidden(SPAWN_FROM_THE_APP);
    }
    if let Err(e) = runnable_yet(&def) {
        return bad_request(e);
    }
    let notes = match job_caps::apply_rules(state, &mut def, Some(&before), origin) {
        Ok(notes) => notes,
        Err(refused) => return refused,
    };
    let mut edited = job.clone();
    if fields.contains_key("action") {
        def.apply_to(&mut edited, &|to| target_of(state, to));
    } else {
        // The target as it was: a job by name keeps firing by name.
        def.apply_to(&mut edited, &|_| (job.target.clone(), job.target_uid.clone()));
    }
    match store.cron_update_definition(&edited) {
        Ok(true) => {}
        Ok(false) => return (StatusCode::NOT_FOUND, Json(json!({"error": "job not found"}))),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))),
    }
    if edited.enabled {
        state.cron_scheduler.schedule_job(&edited);
    } else {
        state.cron_scheduler.cancel_job(id);
    }
    publish_cron_changed(state);
    (StatusCode::OK, Json(json!({"ok": true, "enabled": edited.enabled, "job": to_view(&edited), "notes": notes})))
}

/// Pause or resume job `id`.
fn set_enabled(state: &AppState, store: &Store, id: &str, action: &str) -> Answer {
    let enabled = match action {
        "resume" => true,
        "pause" => false,
        other => return bad_request(format!("unknown action '{other}' (use 'pause' or 'resume')")),
    };
    match store.cron_set_enabled(id, enabled) {
        Ok(true) => {}
        Ok(false) => return (StatusCode::NOT_FOUND, Json(json!({"error": "job not found"}))),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))),
    }

    if enabled {
        if let Ok(Some(job)) = store.cron_get(id) {
            state.cron_scheduler.schedule_job(&job);
        }
    } else {
        state.cron_scheduler.cancel_job(id);
    }
    publish_cron_changed(state);

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
            ..Default::default()
        }
    }

    /// Report links keep https URLs only, as strings or `{label, url}`,
    /// within the length bounds, at most five.
    #[test]
    fn report_links_keep_only_bounded_https_urls() {
        let long_url = format!("https://example.com/{}", "a".repeat(REPORT_URL_MAX));
        let given = json!([
            "https://example.com/pr/1",
            "http://example.com/plain",
            "javascript:alert(1)",
            "https://",
            {"label": "Report", "url": "https://example.com/r"},
            {"label": "x".repeat(REPORT_LABEL_MAX + 1), "url": "https://example.com/too-long-label"},
            {"label": "bad", "url": "ftp://example.com"},
            {"url": "https://example.com/no-label"},
            long_url,
            42,
            {"label": "nested", "url": {"href": "https://example.com"}},
        ]);
        assert_eq!(
            report_links(Some(given)),
            json!([
                "https://example.com/pr/1",
                {"label": "Report", "url": "https://example.com/r"},
                {"label": "", "url": "https://example.com/no-label"},
            ])
        );
        let many: Vec<Value> = (0..9).map(|i| json!(format!("https://example.com/{i}"))).collect();
        assert_eq!(report_links(Some(Value::Array(many))).as_array().unwrap().len(), REPORT_LINKS_MAX);
        assert_eq!(report_links(Some(json!("https://example.com/one"))), json!(["https://example.com/one"]));
        assert_eq!(report_links(None), Value::Null);
    }

    #[test]
    fn an_agent_changes_only_its_own_jobs_or_ones_aimed_at_it() {
        use super::super::caller::Caller;
        let mut job = mk_job(true, None, 0);
        job.created_by_uid = "uid-creator".to_string();
        job.target_uid = "uid-target".to_string();
        let agent = |uid: &str| Caller::Agent { uid: uid.to_string() };
        assert!(may_change_job(Some(&agent("uid-creator")), &job));
        assert!(may_change_job(Some(&agent("uid-target")), &job), "the agent it pings may pause it");
        assert!(!may_change_job(Some(&agent("uid-other")), &job));
        // The UI and every other unattributed request can't be told apart: never refused.
        assert!(may_change_job(Some(&Caller::Unattributed), &job));
        assert!(may_change_job(None, &job));
        // A job with no recorded creator is open to any caller.
        job.created_by_uid.clear();
        assert!(may_change_job(Some(&agent("uid-other")), &job));
    }

    #[test]
    fn views_carry_the_miss_record() {
        let mut job = mk_job(true, None, 2);
        job.missed_count = 3;
        job.last_missed_at = Some(1_700_000_100);
        job.last_miss_reason = "target_stopped".to_string();
        let view = to_view(&job);
        assert_eq!((view.missed_count, view.last_missed_at), (3, Some(1_700_000_100)));
        assert_eq!(view.last_miss_reason, "target_stopped");
        let summary = to_summary(&job, "block-1".to_string());
        assert_eq!((summary.missed_count, summary.last_miss_reason.as_str()), (3, "target_stopped"));
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
