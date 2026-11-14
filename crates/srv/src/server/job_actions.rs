// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The job actions that need the server: `task` files a task on the Kanban
//! as `kanban.act` does, `spawn` opens a worker through the spawn path
//! (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.1 "Actions").
//! `drone` starts a Drone run. `message` is the scheduler's own delivery.

use std::sync::Arc;

use agentmux_common::time::now_ms;

use crate::backend::cron::ActionOutcome;
use crate::backend::jobs;
use crate::backend::jobs::definition::Action;
use crate::backend::reactive::sanitize::{neutralize_markers, sanitize_message};
use crate::backend::storage::cron::CronJob;
use crate::backend::storage::store::Store;
use crate::backend::storage::work_board::{requested, NewCard, WorkEvent};
use crate::backend::storage::work_queue::{work_state, WorkItem};

use super::AppState;

/// Who files a job's task and who its history says did.
const FILED_BY: &str = "jobs";

/// The task's `kind`.
const TASK_KIND: &str = "job";

/// Run `task`, `spawn` and `drone` actions in this server from now on.
pub(crate) fn install(state: &AppState) {
    let st = state.clone();
    state.cron_scheduler.install_actions(Arc::new(move |job, run_id| {
        let st = st.clone();
        Box::pin(async move { run_action(&st, job, &run_id).await })
    }));
}

/// Carry out `job`'s action for run `run_id`.
pub(crate) async fn run_action(state: &AppState, job: CronJob, run_id: &str) -> ActionOutcome {
    match serde_json::from_value::<Action>(job.action.clone()) {
        Ok(Action::Task { title, payload, target }) => {
            let store = super::cron::work_store(state);
            let target = target_of(state, &target);
            match file_task(&store, &job, run_id, &title, &payload, target) {
                Ok((work_id, num)) => ActionOutcome::Filed { work_id, num },
                Err(e) => ActionOutcome::Failed(format!("couldn't file the task: {e}")),
            }
        }
        Ok(Action::Spawn { leader, brief, model, account, keep_worker }) => {
            // A spawn spends until someone stops it: it runs only for a job
            // the operator created (their creating it is the approval).
            if !operator_made(&job) {
                return ActionOutcome::Failed("a spawn job runs only when the operator created it".to_string());
            }
            let (model, account) = (Some(model).filter(|m| !m.is_empty()), Some(account).filter(|a| !a.is_empty()));
            let leader_name = state.mstore.agent_def_get(&leader).ok().flatten().map(|d| d.name).unwrap_or_default();
            let brief_text = worker_brief(&leader_name, &brief);
            match super::app_api::agent_spawn::spawn_for_job(state, &leader, &brief_text, model, account).await {
                Ok(w) => ActionOutcome::Spawned { block_id: w.block_id, def_id: w.agent_id, slug: w.message_to, brief: brief_text, keep: keep_worker },
                Err(e) => ActionOutcome::Failed(e),
            }
        }
        Ok(Action::Drone { drone_id }) => {
            // A drone run can spend (its Agent blocks) and a timer repeats it. Until
            // an agent's drone job gets a notice with a veto window and run limits,
            // only the person's runs (a temporary gate, not a rule).
            if !operator_made(&job) {
                return ActionOutcome::Failed("a drone job made by an agent doesn't run yet (temporary: its Agent blocks spend, and agent-made drone jobs get a notice with a veto window and run limits in the next change). Ask the person to create it in the Jobs tab".to_string());
            }
            if run_id.is_empty() {
                return ActionOutcome::Failed("a drone job runs only with a ledger row to follow it".to_string());
            }
            let trigger = match drone_trigger(state, &job, run_id) {
                Ok(trigger) => trigger,
                Err(e) => return ActionOutcome::Failed(e),
            };
            let cap = state.cron_scheduler.runs().map(|l| l.run_cap_usd(&job));
            match super::drone_handlers::start_job_drone_run(state, &drone_id, trigger, (run_id, cap)).await {
                Ok(_) => ActionOutcome::Drone,
                Err(e) => ActionOutcome::Failed(e),
            }
        }
        Ok(other) => ActionOutcome::Failed(format!("action kind {} has nothing to run here", other.kind())),
        Err(e) => ActionOutcome::Failed(format!("the job's action doesn't read: {e}")),
    }
}

/// What a job hands the drone it runs, read as `{{trigger.*}}`: how the run
/// started (`schedule`, `manual`, `retry`, `event`), the job, and the slot it was due.
///
/// For an event's run it is the event's own fields (`jobs::RunLedger::event_input`) with those keys set on top, so a
/// source's field never stands in for `kind`, `job`, `run_id` or `fired_at_ms`. The event's data is held in memory from the
/// fire to here: a run that waited through a restart has lost it, and fails saying so, not running on an empty input.
fn drone_trigger(state: &AppState, job: &CronJob, run_id: &str) -> Result<serde_json::Value, String> {
    let ledger = state.cron_scheduler.runs();
    let run = ledger.and_then(|l| l.store().job_run_get(run_id).ok().flatten());
    let kind = run.as_ref().map_or(jobs::TRIGGER_MANUAL, |r| r.trigger.as_str());
    let mut fields = serde_json::Map::new();
    // An event's run, and a retry of it, read the event: never start one on an empty input.
    let of_event = kind == jobs::TRIGGER_EVENT || run.as_ref().is_some_and(|r| r.links["from_event"] == serde_json::json!(true));
    if of_event {
        match ledger.and_then(|l| l.event_input(run_id)) {
            Some(serde_json::Value::Object(input)) => fields = input,
            Some(other) => {
                fields.insert("value".to_string(), other);
            }
            None => return Err("the event's data was lost when AgentMux restarted, so this run didn't start".to_string()),
        }
    }
    fields.insert("kind".to_string(), serde_json::json!(kind));
    fields.insert("job".to_string(), serde_json::json!({ "id": job.id, "name": job.name }));
    fields.insert("run_id".to_string(), serde_json::json!(run_id));
    fields.insert("scheduled_for_ms".to_string(), serde_json::json!(run.as_ref().and_then(|r| r.scheduled_for_ms)));
    fields.insert("fired_at_ms".to_string(), serde_json::json!(now_ms()));
    Ok(serde_json::Value::Object(fields))
}

/// Whether the operator created `job`: it carries no agent's UID.
pub(crate) fn operator_made(job: &CronJob) -> bool {
    job.created_by == jobs::BY_OPERATOR && job.created_by_uid.is_empty()
}

/// The agent a task is for, `(name, uid)`; none for the backlog.
pub(crate) fn target_of(state: &AppState, target: &str) -> (String, String) {
    let target = target.trim();
    if target.is_empty() {
        return (String::new(), String::new());
    }
    match state.mstore.agent_def_get(target) {
        Ok(Some(def)) => (def.slug, target.to_string()),
        _ => (target.to_string(), String::new()),
    }
}

/// The worker's first message: who it reports to, then the brief. As the
/// `Agents` tool sends it, naming the tool exactly.
pub(crate) fn worker_brief(leader: &str, brief: &str) -> String {
    let brief = neutralize_markers(&sanitize_message(brief));
    format!(
        "You are a worker of {leader}, started to do the task below. Report to {leader} with the agentmux \
         MCP server's Message tool (mcp__agentmux__Message in Claude Code; action \"send\", to \"{leader}\"), \
         never a built-in SendMessage, when you finish, get stuck, or need a decision.\n\n{brief}"
    )
}

/// File the job's task, as `kanban.act` `create` does: the queue row, its
/// card (no review: the task isn't the operator's to accept), and its first
/// history line, which names the job and the run. Returns its id and number.
pub(crate) fn file_task(
    store: &Store,
    job: &CronJob,
    run_id: &str,
    title: &str,
    payload: &str,
    target: (String, String),
) -> Result<(String, i64), crate::backend::storage::StoreError> {
    let first_line = format!("Filed by the job \"{}\" (job {}, run {run_id}).", crate::server::work_tasks::clean(&job.name), job.id);
    file_task_as(store, FILED_BY, TASK_KIND, title, payload, target, &first_line)
}

/// File a task for an automation, as `kanban.act` `create` does: the queue
/// row, its card (no review: the task isn't the operator's to accept) and its
/// first history line. `filed_by` is who the history says did it, `kind` the
/// task's tag, and `first_line` the history's first line. Returns its id and number.
pub(crate) fn file_task_as(
    store: &Store,
    filed_by: &str,
    kind: &str,
    title: &str,
    payload: &str,
    (target, target_uid): (String, String),
    first_line: &str,
) -> Result<(String, i64), crate::backend::storage::StoreError> {
    let now = now_ms();
    let item = WorkItem {
        id: uuid::Uuid::new_v4().to_string(),
        title: crate::server::work_tasks::clean(title),
        payload: crate::server::work_tasks::clean(payload),
        kind: kind.to_string(),
        target_agent: target,
        target_group: String::new(),
        priority: 0,
        state: String::new(),
        claimed_by: String::new(),
        target_agent_uid: target_uid,
        claimed_by_uid: String::new(),
        claim_expires: None,
        attempts: 0,
        max_attempts: 3,
        created_by: filed_by.to_string(),
        created_by_uid: String::new(),
        created_at: now,
        updated_at: now,
        not_before: None,
        result: String::new(),
    };
    store.work_queue_enqueue(&item)?;
    let card = NewCard {
        review_required: false,
        requested_by: filed_by.to_string(),
        requested_by_kind: requested::OTHER.to_string(),
        filed_by: filed_by.to_string(),
        ..Default::default()
    };
    let num = store.work_card_create(&item.id, &card, now)?;
    let event = WorkEvent {
        work_id: item.id.clone(),
        at: now,
        actor: filed_by.to_string(),
        actor_kind: "system".to_string(),
        kind: "created".to_string(),
        to_state: work_state::OPEN.to_string(),
        body: first_line.to_string(),
        ..Default::default()
    };
    store.work_event_add(&event)?;
    Ok((item.id, num))
}
