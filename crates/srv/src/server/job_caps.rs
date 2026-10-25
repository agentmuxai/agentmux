// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Spending caps at the server's edge (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md
//! §5.3, §7): Settings' figures and where a run would go for the ledger's
//! reservation, and what a job's author may ask for.
//!
//! **Who raises a cap.** Only the operator. A request that carries no agent
//! token looks like the app's own window at the same-user boundary, so it
//! proves nothing; the operator's decision arrives only through the host
//! route (`POST /api/v1/host/jobs`, which needs the host's own token). Every
//! other path holds a job's `policy.caps` to the defaults and says so.
//!
//! **A provider that reports no cost.** Its runs count $0 toward the dollar
//! caps and only the timeout bounds them, so the operator confirms such a job
//! (`confirm_no_cost`, passed through the host route). An agent can't.

use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::Json;
use serde_json::{json, Value};

use crate::backend::jobs::caps::{self, Limits};
use crate::backend::jobs::definition::{Action, JobDefinition};
use crate::backend::jobs::reservation::Target;
use crate::backend::jobs::RunLedger;
use crate::backend::obj::{self, Block};
use crate::backend::providers::resolve_provider_alias;
use crate::backend::storage::agents::AgentDefinition;
use crate::backend::storage::cron::CronJob;

use super::app_api::agent_spawn::{self, RUN_ACCOUNT_META_KEY};
use super::AppState;

type Answer = (StatusCode, Json<Value>);

/// The timeout a job on a provider that reports no cost gets when it names
/// none: the only limit on what it spends.
pub(crate) const NO_COST_TIMEOUT_MINUTES: u32 = 60;

/// Who asks, as far as the caps are concerned.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Origin {
    /// The operator's decision, relayed by the host (`handle_host_jobs`).
    pub from_host: bool,
    /// The operator confirmed a job on a provider that reports no cost.
    pub confirm_no_cost: bool,
}

/// Settings' caps (`jobs:cap:*`).
pub(crate) fn limits(state: &AppState) -> Limits {
    Limits::from_settings(&state.config_watcher.get_settings())
}

/// Weigh job runs against Settings' caps, finding where each would go.
pub(crate) fn install(state: &AppState, ledger: &RunLedger) {
    let (for_limits, for_target) = (state.clone(), state.clone());
    ledger.install_caps(Arc::new(move || limits(&for_limits)), Arc::new(move |job| target_of(&for_target, job)));
}

/// The agent definition `who` names: a UID, else a slug or name.
fn agent_named(state: &AppState, who: &str) -> Option<AgentDefinition> {
    let who = who.trim();
    if who.is_empty() {
        return None;
    }
    if let Ok(Some(def)) = state.mstore.agent_def_get(who) {
        return Some(def);
    }
    state.mstore.agent_def_list().ok()?.into_iter().find(|d| d.slug.eq_ignore_ascii_case(who) || d.name.eq_ignore_ascii_case(who))
}

fn provider_of(state: &AppState, def: &AgentDefinition) -> String {
    resolve_provider_alias(&state.id_store.resolve_effective_provider_id(def)).to_string()
}

/// The agent an action starts or messages: a message's target, a spawn's leader.
fn agent_of(action: &Action) -> Option<&str> {
    match action {
        Action::Message { to, .. } => Some(to),
        Action::Spawn { leader, .. } => Some(leader),
        _ => None,
    }
}

/// The provider of the agent a definition's action starts or messages, when
/// it can be told.
pub(crate) fn action_provider(state: &AppState, action: &Action) -> Option<String> {
    agent_named(state, agent_of(action)?).map(|def| provider_of(state, &def))
}

/// The provider of `job`'s agent when its runs report no cost.
pub(crate) fn no_cost_provider(state: &AppState, job: &CronJob) -> Option<String> {
    let action = JobDefinition::from_job(job).action;
    action_provider(state, &action).filter(|p| !p.is_empty() && !caps::reports_cost(p))
}

/// Where `job`'s next run would go: a message's target pane (its account and
/// its provider), the account a spawn would pick.
fn target_of(state: &AppState, job: &CronJob) -> Target {
    match job.action_kind.as_str() {
        "" | "message" => pane_target(state, job),
        "spawn" => match serde_json::from_value::<Action>(job.action.clone()) {
            Ok(Action::Spawn { leader, account, .. }) => {
                let asked = Some(account.as_str()).filter(|a| !a.is_empty());
                agent_spawn::account_for_job(state, &leader, asked)
                    .map(|(account, provider)| Target { account, provider })
                    .unwrap_or_default()
            }
            _ => Target::default(),
        },
        _ => Target::default(),
    }
}

fn pane_target(state: &AppState, job: &CronJob) -> Target {
    let block_id = if job.target_uid.is_empty() {
        state.reactive_handler.list_agents().into_iter().find(|r| r.agent_id.eq_ignore_ascii_case(&job.target)).map(|r| r.block_id)
    } else {
        state.reactive_handler.block_for_uid(&job.target_uid)
    };
    let Some(block) = block_id.and_then(|b| state.mstore.get::<Block>(&b).ok().flatten()) else { return Target::default() };
    let account = obj::meta_get_string(&block.meta, RUN_ACCOUNT_META_KEY, "");
    let provider = agent_named(state, &obj::meta_get_string(&block.meta, "agentId", ""))
        .map(|def| provider_of(state, &def))
        .unwrap_or_default();
    Target { account, provider }
}

fn refuse(status: StatusCode, body: Value) -> Answer {
    (status, Json(body))
}

/// Apply what the caps ask of a job's author to `def`, before it is stored:
/// - an agent's higher caps are clamped to the defaults (the operator's, via
///   the host, are kept);
/// - a job on a provider that reports no cost needs the operator's
///   confirmation, unless it is the same agent as before, and gets a timeout
///   when it names none.
///
/// `before` is the job's definition on an edit. Returns the sentences the
/// reply should carry.
pub(crate) fn apply_rules(
    state: &AppState,
    def: &mut JobDefinition,
    before: Option<&JobDefinition>,
    origin: Origin,
) -> Result<Vec<String>, Answer> {
    let mut notes = Vec::new();
    if !origin.from_host {
        let (kept, clamped) =
            caps::clamp_to_defaults(def.policy.caps.take(), &limits(state), before.and_then(|b| b.policy.caps.as_ref()));
        def.policy.caps = kept;
        notes.extend(clamped);
    }
    let Some(provider) = action_provider(state, &def.action).filter(|p| !p.is_empty() && !caps::reports_cost(p)) else {
        return Ok(notes);
    };
    if before.is_none_or(|b| agent_of(&b.action) != agent_of(&def.action)) {
        let why = format!(
            "this job's agent runs on {provider}, which doesn't report what a run costs: the spending caps can't stop it and only its \
             timeout limits it"
        );
        if !origin.from_host {
            return Err(refuse(
                StatusCode::FORBIDDEN,
                json!({ "error": format!("{why}. The operator confirms each such job: ask the operator to create it from the Jobs tab") }),
            ));
        }
        if !origin.confirm_no_cost {
            return Err(refuse(
                StatusCode::CONFLICT,
                json!({
                    "error": format!("{why}. Confirm it to create the job (confirm_no_cost: true)"),
                    "needs_confirmation": "no_cost",
                    "provider": provider,
                }),
            ));
        }
    }
    if def.policy.timeout_minutes.is_none() {
        def.policy.timeout_minutes = Some(NO_COST_TIMEOUT_MINUTES);
        notes.push(format!(
            "{provider} doesn't report what a run costs, so policy.timeout_minutes is {NO_COST_TIMEOUT_MINUTES}: only the timeout limits this job"
        ));
    }
    Ok(notes)
}

#[cfg(test)]
#[path = "job_caps_tests.rs"]
mod tests;
