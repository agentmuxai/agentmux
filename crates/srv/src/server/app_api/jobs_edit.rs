// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The Jobs tab's editor RPCs (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md
//! §3.3, §5.1 "Dry run", §5.2, §5.4):
//!
//! - `jobs.get`: one job's definition, for Edit and Clone.
//! - `jobs.save`: create a job, or edit one, from a definition. The
//!   operator's own window only (`operator_only`): it may start workers
//!   (`spawn`), raise a cap and confirm a job on a provider that reports no
//!   cost, which nothing else can. It goes through the same create and edit
//!   code as the Schedule routes, as the operator.
//! - `jobs.dryrun`: what a definition (or a stored job) would do, without
//!   starting an agent.
//! - `jobs.templates.update_diff`: what accepting a template's newer version
//!   would change in a job. Accepting is a `jobs.save` that names it.

use std::sync::Arc;

use axum::http::StatusCode;
use chrono::TimeZone;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use agentmux_common::time::now_ms;

use crate::backend::cron::schedule::{parse_tz, JobSchedule};
use crate::backend::jobs::definition::{Action, Check, JobDefinition};
use crate::backend::jobs::from_template::{self, TemplateUpdateDiff};
use crate::backend::jobs::{caps, policy};
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::*;
use crate::backend::storage::cron::CronJob;
use crate::server::cron::{create_job, jobs_store, patch_job, runnable_yet};
use crate::server::job_caps::{self, Origin, NO_COST_TIMEOUT_MINUTES};

use super::AppState;

/// How many fire times a dry run lists.
const DRY_RUN_FIRES: usize = 5;
/// A definition's fields an edit can't change: srv sets them.
const FIXED_FIELDS: [&str; 3] = ["id", "jobVersion", "created_by"];

#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobsGetRequest {
    pub job_id: String,
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobsGetResult {
    /// The job's definition (`schema/agentmux-job.v1.schema.json`), as stored.
    #[ts(type = "Record<string, unknown>")]
    pub definition: Value,
}

#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobsSaveRequest {
    /// The job to edit; a new job when absent.
    #[serde(default)]
    #[ts(optional)]
    pub job_id: Option<String>,
    /// The job definition. On an edit, only the fields that differ from the
    /// stored job count, so a job kept as it was isn't refused for a rule
    /// newer than it. One that names a template is made from it.
    #[ts(type = "Record<string, unknown>")]
    pub definition: Value,
    /// Confirms a job whose agent's provider reports no cost (§5.3).
    #[serde(default)]
    #[ts(optional)]
    pub confirm_no_cost: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobsSaveResult {
    pub ok: bool,
    /// The job created or edited.
    pub job_id: String,
    /// What srv changed or filled in, in words.
    pub notes: Vec<String>,
    /// The job needs the operator's confirmation first: `no_cost`.
    pub needs_confirmation: Option<String>,
    /// The provider that reports no cost, with `needs_confirmation`.
    pub provider: Option<String>,
    /// Why the confirmation is needed, in words.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobsDryRunRequest {
    /// A stored job, alone or with the edits `definition` makes to it.
    #[serde(default)]
    #[ts(optional)]
    pub job_id: Option<String>,
    #[serde(default)]
    #[ts(optional, type = "Record<string, unknown>")]
    pub definition: Option<Value>,
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct DryRunFire {
    #[ts(type = "number")]
    pub at_ms: i64,
    /// In the job's time zone: `Mon 2026-10-26 06:00 EDT`.
    pub local: String,
}

/// The figures that would apply (§5.3), in dollars.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct DryRunCaps {
    pub run_usd: f64,
    pub day_usd: f64,
    pub account_day_usd: f64,
    pub messages_per_day: u32,
    /// An agent's higher figures would be held to the defaults.
    pub clamped: bool,
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct DryRunSecret {
    pub name: String,
    /// The Vault entry it names (never a value).
    pub vault: String,
}

#[derive(Debug, Clone, Default, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobsDryRunResult {
    /// The definition is one a save would accept.
    pub ok: bool,
    /// Why it isn't, in words.
    pub error: Option<String>,
    pub name: String,
    /// `message`, `task` or `spawn`.
    pub action_kind: String,
    /// Who it goes to: the message's target, the task's, the spawn's leader.
    pub target: String,
    /// The brief as the agent would get it (a task's payload).
    pub brief: String,
    /// The trigger's time zone.
    pub tz: String,
    pub fires: Vec<DryRunFire>,
    pub caps: Option<DryRunCaps>,
    pub outputs: Vec<String>,
    /// What it declares. A dry run can't see the Vault's grants for a run:
    /// each is made, or refused, when the run first uses it.
    pub secrets: Vec<DryRunSecret>,
    /// The concurrency group its runs would wait in.
    pub group: Option<String>,
    /// Things to know before saving, in words.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobsUpdateDiffRequest {
    pub job_id: String,
}

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let s = state.clone();
    engine.register_typed(COMMAND_JOBS_GET, move |req: JobsGetRequest, _ctx| {
        let s = s.clone();
        async move { get(&s, &req.job_id).map(|d| JobsGetResult { definition: d }) }
    });
    let s = state.clone();
    engine.register_typed(COMMAND_JOBS_SAVE, move |req: JobsSaveRequest, ctx| {
        let s = s.clone();
        async move { save(&s, req, &ctx) }
    });
    let s = state.clone();
    engine.register_typed(COMMAND_JOBS_DRYRUN, move |req: JobsDryRunRequest, ctx| {
        let s = s.clone();
        async move { dry_run(&s, req, &ctx) }
    });
    let s = state.clone();
    engine.register_typed(COMMAND_JOBS_TEMPLATES_UPDATE_DIFF, move |req: JobsUpdateDiffRequest, _ctx| {
        let s = s.clone();
        async move { update_diff(&s, &req.job_id) }
    });
}

fn stored_job(state: &AppState, id: &str) -> Result<CronJob, String> {
    let store = jobs_store(state).ok_or("the shared store isn't available")?;
    store.cron_get(id).map_err(|e| e.to_string())?.ok_or_else(|| "job not found".to_string())
}

fn stored(state: &AppState, id: &str) -> Result<JobDefinition, String> {
    stored_job(state, id).map(|job| JobDefinition::from_job(&job))
}

fn get(state: &AppState, id: &str) -> Result<Value, String> {
    serde_json::to_value(stored(state, id)?).map_err(|e| e.to_string())
}

/// The fields of `sent` that differ from the stored `before`, without the
/// ones srv sets.
fn changed_fields(before: &JobDefinition, sent: &Value) -> Result<Map<String, Value>, String> {
    let sent = sent.as_object().ok_or("a job definition is a JSON object")?;
    let current = serde_json::to_value(before).map_err(|e| e.to_string())?;
    Ok(sent
        .iter()
        .filter(|(k, v)| !FIXED_FIELDS.contains(&k.as_str()) && current.get(k.as_str()) != Some(*v))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect())
}

fn save(state: &AppState, req: JobsSaveRequest, ctx: &RpcContext) -> Result<JobsSaveResult, String> {
    ctx.operator_only("only the operator creates or edits a job here")?;
    let origin = Origin { from_host: true, confirm_no_cost: req.confirm_no_cost == Some(true) };
    let (status, axum::Json(body)) = match req.job_id.as_deref().filter(|id| !id.is_empty()) {
        None => create_job(state, None, req.definition, origin),
        Some(id) => {
            let fields = changed_fields(&stored(state, id)?, &req.definition)?;
            patch_job(state, None, id, Value::Object(fields), origin)
        }
    };
    saved(status, &body)
}

/// A create's or edit's answer as the RPC's: the job it made, or why not.
fn saved(status: StatusCode, body: &Value) -> Result<JobsSaveResult, String> {
    let error = body.get("error").and_then(Value::as_str).map(str::to_string);
    if let Some(kind) = body.get("needs_confirmation").and_then(Value::as_str) {
        let provider = body.get("provider").and_then(Value::as_str).map(str::to_string);
        return Ok(JobsSaveResult { needs_confirmation: Some(kind.to_string()), provider, error, ..Default::default() });
    }
    if !status.is_success() {
        return Err(error.unwrap_or_else(|| format!("refused ({status})")));
    }
    let job_id = body.pointer("/job/id").and_then(Value::as_str).unwrap_or("").to_string();
    let notes = body.get("notes").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    Ok(JobsSaveResult { ok: true, job_id, notes, ..Default::default() })
}

fn update_diff(state: &AppState, job_id: &str) -> Result<TemplateUpdateDiff, String> {
    Ok(from_template::update_diff(&stored(state, job_id)?))
}

// ── Dry run ─────────────────────────────────────────────────────────────────

/// The definition a dry run reads and whether its trigger or action is being
/// set (the rules a save would check), from the stored job and/or the sent
/// definition, with its template applied.
fn dry_run_input(state: &AppState, req: &JobsDryRunRequest) -> Result<(Value, bool, Option<JobDefinition>), String> {
    let before = req.job_id.as_deref().filter(|id| !id.is_empty()).map(|id| stored(state, id)).transpose()?;
    match (&before, &req.definition) {
        (None, None) => Err("which job? give job_id or a definition".to_string()),
        (None, Some(sent)) => {
            let mut value = sent.clone();
            from_template::apply(&mut value, None)?;
            Ok((value, true, None))
        }
        (Some(b), None) => Ok((serde_json::to_value(b).map_err(|e| e.to_string())?, false, before)),
        (Some(b), Some(sent)) => {
            let fields = changed_fields(b, sent)?;
            let mut merged = serde_json::to_value(b).map_err(|e| e.to_string())?;
            for (k, v) in &fields {
                merged[k.as_str()] = v.clone();
            }
            if from_template::edit_needs_render(&fields, b) {
                from_template::apply(&mut merged, b.template.as_ref())?;
            }
            let fresh = fields.contains_key("trigger") || fields.contains_key("action");
            Ok((merged, fresh, before))
        }
    }
}

fn target_of(action: &Action) -> String {
    match action {
        Action::Message { to, .. } => to.clone(),
        Action::Task { target, .. } => target.clone(),
        Action::Spawn { leader, .. } => leader.clone(),
        Action::Drone { .. } => String::new(),
    }
}

fn brief_of(action: &Action) -> String {
    match action {
        Action::Message { brief, .. } | Action::Spawn { brief, .. } => brief.clone(),
        Action::Task { payload, .. } => payload.clone(),
        Action::Drone { .. } => String::new(),
    }
}

/// `at_ms` as the zone `tz` reads it: `Mon 2026-10-26 06:00 EDT`.
fn local_time(tz: &chrono_tz::Tz, at_ms: i64) -> String {
    tz.timestamp_millis_opt(at_ms).single().map(|t| t.format("%a %Y-%m-%d %H:%M %Z").to_string()).unwrap_or_default()
}

fn dry_run(state: &AppState, req: JobsDryRunRequest, ctx: &RpcContext) -> Result<JobsDryRunResult, String> {
    let refused = |error: String| JobsDryRunResult { error: Some(error), ..Default::default() };
    let (value, fresh, before) = match dry_run_input(state, &req) {
        Ok(input) => input,
        Err(e) => return Ok(refused(e)),
    };
    let now = now_ms();
    let def = match JobDefinition::parse(&value, Check { fresh, now_ms: now }).and_then(|d| runnable_yet(&d).map(|()| d)) {
        Ok(def) => def,
        Err(e) => return Ok(refused(e)),
    };
    let mut notes = Vec::new();
    let mut job = CronJob {
        id: before.as_ref().map(|b| b.id.clone()).filter(|id| !id.is_empty()).unwrap_or_else(|| "dry-run".to_string()),
        created_at: now / 1000,
        ..Default::default()
    };
    def.apply_to(&mut job, &|to| (to.to_string(), String::new()));
    let tz = parse_tz(&job.tz).unwrap_or(chrono_tz::Tz::UTC);
    let fires: Vec<DryRunFire> = match JobSchedule::of(&job) {
        Ok(schedule) => schedule.after(now).take(DRY_RUN_FIRES).map(|at_ms| DryRunFire { at_ms, local: local_time(&tz, at_ms) }).collect(),
        Err(e) => return Ok(refused(e)),
    };
    if fires.is_empty() && !matches!(def.trigger, crate::backend::jobs::definition::Trigger::Manual) {
        notes.push("this trigger has no time ahead: it won't fire".to_string());
    }

    // The caps that would apply: an agent's higher figures are held to the
    // defaults (the operator's window keeps them).
    let limits = job_caps::limits(state);
    let mut policy_caps = def.policy.caps.clone();
    let mut clamped = false;
    if !ctx.agent_id.is_empty() {
        let (kept, clamp_notes) = caps::clamp_to_defaults(policy_caps, &limits, before.as_ref().and_then(|b| b.policy.caps.as_ref()));
        clamped = !clamp_notes.is_empty();
        notes.extend(clamp_notes);
        policy_caps = kept;
    }
    let eff = caps::effective(policy_caps.as_ref(), &limits);
    if let Some(provider) = job_caps::action_provider(state, &def.action).filter(|p| !p.is_empty() && !caps::reports_cost(p)) {
        let timeout = def.policy.timeout_minutes.unwrap_or(NO_COST_TIMEOUT_MINUTES);
        notes.push(format!(
            "its agent runs on {provider}, which doesn't report what a run costs: the spending caps can't stop it, only its {timeout}-minute timeout does"
        ));
    }
    if matches!(def.action, Action::Spawn { .. }) {
        notes.push("it starts a worker for each run; a dry run starts none".to_string());
    }
    Ok(JobsDryRunResult {
        ok: true,
        error: None,
        name: def.name.trim().to_string(),
        action_kind: def.action.kind().to_string(),
        target: target_of(&def.action),
        brief: brief_of(&def.action),
        tz: tz.name().to_string(),
        fires,
        caps: Some(DryRunCaps {
            run_usd: eff.run_usd,
            day_usd: eff.day_usd,
            account_day_usd: limits.account_day_usd,
            messages_per_day: eff.messages_per_day,
            clamped,
        }),
        outputs: def.outputs.clone(),
        secrets: def.secrets.iter().map(|s| DryRunSecret { name: s.name.clone(), vault: s.vault.clone() }).collect(),
        group: policy::group_of(&job),
        notes,
    })
}

#[cfg(test)]
#[path = "jobs_edit_tests.rs"]
mod tests;
