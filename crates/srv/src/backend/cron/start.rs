// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Starting a run once policy lets it (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md
//! §5.1): the job's action (a message to an agent, a task filed, a worker
//! spawned), and the waiting runs that start when a place frees up.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use super::{CronScheduler, FireOutcome, Fired};
use crate::backend::jobs;
use crate::backend::storage::cron::CronJob;

/// What a `task` or `spawn` action did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionOutcome {
    /// The task was filed: its queue id and card number.
    Filed { work_id: String, num: i64 },
    /// The worker is open. Its brief is delivered to it as any message
    /// is, and the run ends when the worker's first turn does.
    Spawned { block_id: String, def_id: String, slug: String, brief: String, keep: bool },
    Failed(String),
}

/// Runs a job's `task` or `spawn` action (the server installs it: both need
/// its stores and the spawn path).
pub type ActionRunner = Arc<dyn Fn(CronJob, String) -> Pin<Box<dyn Future<Output = ActionOutcome> + Send>> + Send + Sync>;

impl CronScheduler {
    /// Run `task` and `spawn` actions through `runner` from now on. Once.
    pub fn install_actions(&self, runner: ActionRunner) {
        let _ = self.actions.set(runner);
    }

    /// Carry out an admitted run: the job's action, then, for an action that
    /// starts an agent, the delivery of its brief. `scheduled` runs count as
    /// the job's fires; `late_for` is the slot a late start was due.
    pub(crate) async fn start_run(&self, job: &CronJob, run_id: Option<String>, scheduled: bool, late_for: Option<i64>) -> Fired {
        let delivery_started = agentmux_common::time::now_ms();
        let mut to_deliver = job.clone();
        let mut worker_started = false;
        if matches!(job.action_kind.as_str(), "task" | "spawn") {
            let acted = match self.actions.get() {
                Some(runner) => runner(job.clone(), run_id.clone().unwrap_or_default()).await,
                None => ActionOutcome::Failed("this build can't run the job's action here".to_string()),
            };
            match acted {
                ActionOutcome::Filed { work_id, num } => {
                    if let (Some(runs), Some(run_id)) = (self.runs(), &run_id) {
                        let (runs, run_id, name) = (runs.clone(), run_id.clone(), job.name.clone());
                        let _ = tokio::task::spawn_blocking(move || runs.task_filed(&run_id, &name, &work_id, num)).await;
                    }
                    let outcome = FireOutcome::Delivered { block_id: None };
                    if scheduled {
                        self.record_result(&job.id, &outcome);
                    }
                    self.publish_changed();
                    return Fired { run_id, outcome: Some(outcome), waiting: None };
                }
                ActionOutcome::Failed(error) => return self.finish_undelivered(job, run_id, FireOutcome::Failed { error }, scheduled, delivery_started),
                ActionOutcome::Spawned { block_id, def_id, slug, brief, keep } => {
                    if let (Some(runs), Some(run_id)) = (self.runs(), &run_id) {
                        let links = serde_json::json!({ "worker": { "block_id": block_id, "def_id": def_id, "keep": keep } });
                        let _ = runs.store().job_run_add_links(run_id, &links);
                    }
                    (to_deliver.target, to_deliver.target_uid, to_deliver.prompt) = (slug, def_id, brief);
                    worker_started = true;
                }
            }
        }
        let Some(mut outcome) = self.deliver(&to_deliver, run_id.as_deref(), late_for).await else {
            let failed = FireOutcome::Failed { error: "no way to deliver".to_string() };
            if let (Some(runs), Some(run_id)) = (self.runs(), &run_id) {
                runs.record_delivery(run_id, &to_deliver, &failed, false, delivery_started);
            }
            return Fired { run_id, outcome: None, waiting: None };
        };
        if worker_started && !outcome.delivered() {
            // The worker opened but took no brief: nothing will run, and
            // waiting for it to come back is no use.
            outcome = FireOutcome::Failed { error: format!("the spawned worker didn't take its brief ({:?})", outcome.miss_reason()) };
        }
        if let (Some(runs), Some(run_id)) = (self.runs(), &run_id) {
            let (runs, run_id, job, o) = (runs.clone(), run_id.clone(), to_deliver.clone(), outcome.clone());
            let _ = tokio::task::spawn_blocking(move || runs.record_delivery(&run_id, &job, &o, scheduled, delivery_started)).await;
        }
        // Run now (manual) is in the ledger only: it neither counts as a
        // fire nor as a miss.
        if scheduled {
            self.record_result(&job.id, &outcome);
        }
        self.publish_changed();
        Fired { run_id, outcome: Some(outcome), waiting: None }
    }

    /// An action that failed before anything was delivered.
    fn finish_undelivered(&self, job: &CronJob, run_id: Option<String>, outcome: FireOutcome, scheduled: bool, started_ms: i64) -> Fired {
        if let (Some(runs), Some(run_id)) = (self.runs(), &run_id) {
            runs.record_delivery(run_id, job, &outcome, scheduled, started_ms);
        }
        if scheduled {
            self.record_result(&job.id, &outcome);
        }
        self.publish_changed();
        Fired { run_id, outcome: Some(outcome), waiting: None }
    }

    /// One pass over the waiting runs and the running ones: stop what ran
    /// past its timeout, then start the waiting runs policy now lets start.
    /// Returns how many started.
    pub(crate) async fn run_policy_pass(&self) -> usize {
        let Some(runs) = self.runs().cloned() else { return 0 };
        let started = {
            let runs = runs.clone();
            tokio::task::spawn_blocking(move || {
                runs.enforce_timeouts();
                runs.start_waiting()
            })
            .await
            .unwrap_or_default()
        };
        let n = started.len();
        for (run, job) in started {
            let scheduled = run.trigger != jobs::TRIGGER_MANUAL;
            let fired = self.start_run(&job, Some(run.id.clone()), scheduled, run.scheduled_for_ms).await;
            if matches!(fired.outcome, Some(FireOutcome::Delivered { .. })) {
                self.stop_at_max_fires(&job.id);
            }
        }
        n
    }
}
