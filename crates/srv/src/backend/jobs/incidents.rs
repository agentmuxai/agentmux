// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! When a job's run fails: one incident per job, not one per run
//! (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §3.4).
//!
//! After each change to a job's runs the ledger nudges [`Incidents`], which
//! reads the job's newest decisive run (`Store::job_run_last_decisive`) and
//! judges it with the Jobs tab's own rule (`verdict::run_failed`):
//!
//! - **A failure with no incident open** opens one (`storage::job_incidents`),
//!   files a Kanban task for the job's owning leader (`kind: job-failure`, no
//!   review), sends that leader an `[AgentMux]` note and the operator one
//!   notification (`NotifyKind::JobFailed`).
//! - **Another failure while it is open** is counted, and adds a line to the
//!   task's history (fewer as a streak grows, [`worth_a_line`]), and nothing
//!   else.
//! - **The next success** ends it: the task gets a "recovered" line and stays
//!   open for its owner (§10, decision 6); the notification is taken down.
//! - **A task its owner finished or cancelled** while the job still failed
//!   ends the incident; the job's next failure opens a new one.
//!
//! Skips (paused, Pause all) are never decisive, so they neither fail nor
//! recover a job. Caps (§5.3) don't exist yet; their once-a-day alert for a
//! cap skip belongs beside [`Incidents::assess`] when they do. Cron jobs only:
//! a Drone graph is run by hand, from a pane that shows its failure already.

use std::collections::BTreeSet;
use std::sync::{Arc, Weak};

use agentmux_common::time::now_ms;
use serde_json::json;

use super::verdict::run_failed;
use crate::backend::mps::Broker;
use crate::backend::reactive::sanitize::quote_in_system_note;
use crate::backend::storage::job_incidents::{ended, JobIncident};
use crate::backend::storage::job_runs::{infra, reason, state, JobRun};
use crate::backend::storage::store::Store;
use crate::backend::storage::work_board::{requested, NewCard, WorkEvent};
use crate::backend::storage::work_queue::{work_state, WorkItem};

/// The task's `kind`, and who files it (`created_by`, `filed_by`, the
/// history's actor).
pub const TASK_KIND: &str = "job-failure";
pub const FILED_BY: &str = "jobs";

/// The notification's group: one per job.
pub fn notify_key(job_id: &str) -> String {
    format!("job:{job_id}")
}

/// Who an incident's alerts go to (§3.4): the owning leader, resolved when
/// the incident opens. `None` when no agent resolves: the operator alone is
/// told, and the task waits in the backlog for anyone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leader {
    pub uid: String,
    pub name: String,
}

/// What one look at a job did.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Nothing new: no decisive run, a success with no incident open, or a
    /// failure already counted.
    Nothing,
    /// The first failure: an incident, its task, and the alert.
    Opened { incident: JobIncident, leader: Option<Leader>, note: String, notify_body: String },
    /// Another failure of the open incident: `count` in a row.
    FailedAgain { count: i64 },
    /// The job succeeded again.
    Recovered,
}

/// The incident keeper: one per instance, beside the run ledger.
pub struct Incidents {
    /// The identity store: the ledger, its incidents and the work queue.
    store: Arc<Store>,
    /// The channel's object store: agent definitions, to resolve the owning
    /// leader and name it.
    mstore: Option<Arc<Store>>,
    /// Where cron jobs are (`db_cron_jobs`, the shared store): a job's owner.
    jobs: Option<Arc<Store>>,
    broker: Arc<Broker>,
    queue: std::sync::OnceLock<tokio::sync::mpsc::UnboundedSender<String>>,
}

impl Incidents {
    /// A keeper that acts only when [`Self::assess`] is called.
    pub fn new(store: Arc<Store>, mstore: Option<Arc<Store>>, jobs: Option<Arc<Store>>, broker: Arc<Broker>) -> Arc<Self> {
        Arc::new(Incidents { store, mstore, jobs, broker, queue: std::sync::OnceLock::new() })
    }

    /// Look at each nudged job on a task of its own, one at a time, off the
    /// caller's thread: a nudge comes from inside the ledger's writes. Needs a
    /// tokio runtime; without one, nudges do nothing.
    pub fn start(self: &Arc<Self>) {
        let Ok(rt) = tokio::runtime::Handle::try_current() else { return };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        if self.queue.set(tx).is_err() {
            return;
        }
        let weak: Weak<Incidents> = Arc::downgrade(self);
        rt.spawn(async move {
            while let Some(first) = rx.recv().await {
                let mut due = BTreeSet::from([first]);
                while let Ok(more) = rx.try_recv() {
                    due.insert(more);
                }
                for job_id in due {
                    let Some(me) = weak.upgrade() else { return };
                    let _ = tokio::task::spawn_blocking(move || me.assess(&job_id)).await;
                }
            }
        });
    }

    /// Job `job_id`'s runs changed: look at it soon.
    pub fn nudge(&self, job_id: &str) {
        if let Some(tx) = self.queue.get() {
            let _ = tx.send(job_id.to_string());
        }
    }

    /// Look at job `job_id` now, and act on what changed. Blocking.
    pub fn assess(&self, job_id: &str) -> Step {
        let step = self.judge(job_id).unwrap_or_else(|e| {
            tracing::warn!(job_id, error = %e, "jobs: couldn't judge an incident");
            Step::Nothing
        });
        match &step {
            Step::Nothing => {}
            Step::Opened { incident, leader, note, notify_body } => {
                self.alert(incident, leader.as_ref(), note, notify_body);
                self.changed(job_id);
            }
            Step::FailedAgain { .. } => self.changed(job_id),
            Step::Recovered => {
                if let Some(router) = crate::backend::notify::router::get(&self.broker) {
                    router.resolve(&notify_key(job_id), crate::backend::notify::policy::Family::Job);
                }
                self.changed(job_id);
            }
        }
        step
    }

    fn judge(&self, job_id: &str) -> Result<Step, crate::backend::storage::StoreError> {
        let Some(run) = self.store.job_run_last_decisive(job_id)? else { return Ok(Step::Nothing) };
        if run.job_source != super::SOURCE_CRON {
            return Ok(Step::Nothing);
        }
        let now = now_ms();
        let mut open = self.store.job_incident_open_for(job_id)?;
        if let Some(inc) = &open {
            if self.task_closed(&inc.work_id) && self.store.job_incident_end(&inc.id, ended::TASK_CLOSED, "", now)? {
                open = None;
            }
        }
        if !run_failed(&run) {
            let Some(inc) = open else { return Ok(Step::Nothing) };
            if !self.store.job_incident_end(&inc.id, ended::RECOVERED, &run.id, now)? {
                return Ok(Step::Nothing);
            }
            let at = run.ended_at_ms.unwrap_or(now);
            let streak = if inc.failures == 1 { "1 failure".to_string() } else { format!("{} failures in a row", inc.failures) };
            self.history(&inc.work_id, &format!("recovered at {}: the job's last run succeeded, after {streak}", clock(at)));
            return Ok(Step::Recovered);
        }
        if let Some(inc) = open {
            if inc.last_run_id == run.id {
                return Ok(Step::Nothing);
            }
            let Some(count) = self.store.job_incident_fail_again(&inc.id, &inc.last_run_id, &run.id, now)? else {
                return Ok(Step::Nothing);
            };
            self.link_run(&run.id, &inc);
            if worth_a_line(count) {
                let line = format!("failed again at {} ({}), {count} in a row", clock(failed_at(&run)), failure_words(&run));
                self.history(&inc.work_id, &line);
            }
            return Ok(Step::FailedAgain { count });
        }
        // A failure an ended incident already counted (its task was closed
        // while the job still failed) opens nothing until the next failure.
        if self.store.job_incident_latest(job_id)?.is_some_and(|last| last.last_run_id == run.id) {
            return Ok(Step::Nothing);
        }
        self.open(&run, now)
    }

    fn open(&self, run: &JobRun, now: i64) -> Result<Step, crate::backend::storage::StoreError> {
        // Only an instance that has the job opens its incident, so the owner
        // is resolved from the job itself. Another channel that followed the
        // run (#1988) leaves it to the one that fired it.
        let Some(job) = self.jobs.as_ref().and_then(|s| s.cron_get(&run.job_id).ok().flatten()) else {
            return Ok(Step::Nothing);
        };
        let leader = self.leader(&job);
        let mut incident = JobIncident {
            id: uuid::Uuid::new_v4().to_string(),
            job_id: run.job_id.clone(),
            job_source: run.job_source.clone(),
            job_name: run.job_name.clone(),
            owner_uid: leader.as_ref().map(|l| l.uid.clone()).unwrap_or_default(),
            first_run_id: run.id.clone(),
            last_run_id: run.id.clone(),
            failures: 1,
            opened_at_ms: now,
            last_failed_at_ms: now,
            ..Default::default()
        };
        if !self.store.job_incident_open(&incident)? {
            // Another instance opened it first: it counts this failure.
            return Ok(Step::Nothing);
        }
        match self.file_task(run, leader.as_ref(), now) {
            Ok((work_id, num)) => {
                self.store.job_incident_set_task(&incident.id, &work_id, num)?;
                incident.work_id = work_id;
                incident.task_num = Some(num);
                self.link_run(&run.id, &incident);
            }
            Err(e) => tracing::warn!(job_id = %run.job_id, error = %e, "jobs: couldn't file an incident's task"),
        }
        let note = note_text(run, incident.task_num);
        let notify_body = notify_body(run, incident.task_num);
        Ok(Step::Opened { incident, leader, note, notify_body })
    }

    /// The owning leader of `job` (§3.4): its owner (the cron job's
    /// creator), that owner's lead when it has a seat with one or is a
    /// spawned worker, else the owner itself.
    fn leader(&self, job: &crate::backend::storage::cron::CronJob) -> Option<Leader> {
        let owner = job.created_by_uid.trim();
        if owner.is_empty() {
            return None;
        }
        let mstore = self.mstore.as_ref()?;
        let name_of = |uid: &str| mstore.agent_def_get(uid).ok().flatten().map(|d| d.name);
        let lead = crate::backend::agent_seat::effective_seat_by_id(mstore, owner).and_then(|s| s.lead);
        let uid = lead.filter(|l| name_of(l).is_some()).unwrap_or_else(|| owner.to_string());
        let name = name_of(&uid)?;
        Some(Leader { uid, name })
    }

    /// File the incident's task, as `kanban.act` `create` does: the queue
    /// row, its card, and its first history line.
    fn file_task(&self, run: &JobRun, leader: Option<&Leader>, now: i64) -> Result<(String, i64), crate::backend::storage::StoreError> {
        let item = WorkItem {
            id: uuid::Uuid::new_v4().to_string(),
            title: task_title(run),
            payload: task_body(run),
            kind: TASK_KIND.to_string(),
            target_agent: leader.map(|l| l.name.clone()).unwrap_or_default(),
            target_group: String::new(),
            priority: 0,
            state: String::new(),
            claimed_by: String::new(),
            target_agent_uid: leader.map(|l| l.uid.clone()).unwrap_or_default(),
            claimed_by_uid: String::new(),
            claim_expires: None,
            attempts: 0,
            max_attempts: 3,
            created_by: FILED_BY.to_string(),
            created_by_uid: String::new(),
            created_at: now,
            updated_at: now,
            not_before: None,
            result: String::new(),
        };
        self.store.work_queue_enqueue(&item)?;
        let card = NewCard {
            review_required: false,
            requested_by: FILED_BY.to_string(),
            requested_by_kind: requested::OTHER.to_string(),
            filed_by: FILED_BY.to_string(),
            ..Default::default()
        };
        let num = self.store.work_card_create(&item.id, &card, now)?;
        let event = WorkEvent {
            work_id: item.id.clone(),
            at: now,
            actor: FILED_BY.to_string(),
            actor_kind: "system".to_string(),
            kind: "created".to_string(),
            to_state: work_state::OPEN.to_string(),
            ..Default::default()
        };
        self.store.work_event_add(&event)?;
        Ok((item.id, num))
    }

    /// Whether the incident's task was finished or cancelled (or is gone).
    fn task_closed(&self, work_id: &str) -> bool {
        if work_id.is_empty() {
            return false;
        }
        match self.store.work_queue_get(work_id) {
            Ok(Some(t)) => matches!(t.state.as_str(), work_state::DONE | work_state::CANCELLED | work_state::FAILED),
            Ok(None) => true,
            Err(_) => false,
        }
    }

    /// A line in the task's history, from `jobs`.
    fn history(&self, work_id: &str, body: &str) {
        if work_id.is_empty() {
            return;
        }
        let event = WorkEvent {
            work_id: work_id.to_string(),
            at: now_ms(),
            actor: FILED_BY.to_string(),
            actor_kind: "system".to_string(),
            kind: "updated".to_string(),
            body: body.to_string(),
            ..Default::default()
        };
        if let Err(e) = self.store.work_event_add(&event) {
            tracing::warn!(work_id, error = %e, "jobs: couldn't add to an incident's task");
        }
    }

    /// The run's `links.kanban_task`: the incident's task.
    fn link_run(&self, run_id: &str, incident: &JobIncident) {
        let Some(num) = incident.task_num else { return };
        let link = json!({ "kanban_task": { "id": incident.work_id, "num": num } });
        if let Err(e) = self.store.job_run_add_links(run_id, &link) {
            tracing::warn!(run_id, error = %e, "jobs: couldn't link a run to its incident");
        }
    }

    /// The note to the owning leader and the operator's notification.
    fn alert(&self, incident: &JobIncident, leader: Option<&Leader>, note: &str, notify_body: &str) {
        if let Some(leader) = leader {
            if let Err(e) = crate::backend::reactive::get_global_handler().deliver_system_note(&leader.uid, note) {
                tracing::info!(job_id = %incident.job_id, error = %e, "jobs: the owning leader didn't get the incident note");
            }
        }
        match crate::backend::notify::router::get(&self.broker) {
            Some(router) => router.job_failed(&notify_key(&incident.job_id), &incident.job_name, notify_body.to_string()),
            None => tracing::info!(job_id = %incident.job_id, "jobs: no notification router yet; the Jobs tab shows the failure"),
        }
    }

    /// Tell the Jobs tab and the Kanban.
    fn changed(&self, job_id: &str) {
        let event = crate::backend::mps_events::JobsChangedEvent { job_id: job_id.to_string() };
        self.broker.publish_event::<crate::backend::mps_events::JobsChanged>(&event, vec![], 0);
        self.broker.publish_event::<crate::backend::mps_events::WorkQueueChanged>(&(), vec![], 0);
    }
}

/// Whether the `count`th failure in a row gets a line in the task's history:
/// the first five, then 10, 20, 50, 100, 200, 500… A job failing every
/// minute would otherwise write 1,440 lines a day; the incident keeps the
/// exact count, and the Jobs tab every run.
pub fn worth_a_line(count: i64) -> bool {
    if count <= 5 {
        return true;
    }
    let mut n = count;
    while n % 10 == 0 {
        n /= 10;
    }
    matches!(n, 1 | 2 | 5)
}

/// When the run failed: when it ended, else when it started.
fn failed_at(run: &JobRun) -> i64 {
    run.ended_at_ms.unwrap_or(run.started_at_ms)
}

/// `HH:MM UTC`, as a fire's brief line says times.
fn clock(at_ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(at_ms).map(|t| t.format("%H:%M UTC").to_string()).unwrap_or_default()
}

/// How the run failed, in a few words of AgentMux's own (never the run's
/// summary): for titles, notes and the notification.
pub fn failure_words(run: &JobRun) -> String {
    if run.state == state::MISSED || run.state == "lost" || (run.state == state::PENDING && run.reason == reason::EXPIRED) {
        return match run.reason.as_str() {
            reason::TARGET_STOPPED => "missed: its agent wasn't running".to_string(),
            reason::EXPIRED => "missed: its agent didn't come back before the next run".to_string(),
            _ if run.state == "lost" => "lost: it stopped reporting".to_string(),
            _ => "missed".to_string(),
        };
    }
    match run.infra.as_str() {
        infra::ERROR => return "it couldn't be delivered or ran into an error".to_string(),
        "timeout" => return "it timed out".to_string(),
        "over_budget" => return "it reached its spending cap".to_string(),
        infra::INTERRUPTED => return "it was interrupted".to_string(),
        _ => {}
    }
    if run.outcome == "failed" {
        return "its agent reported it failed".to_string();
    }
    "failed".to_string()
}

/// The task's title: the job's name and how it failed.
pub fn task_title(run: &JobRun) -> String {
    format!("Job failed: {} ({})", one_line(&run.job_name), failure_words(run))
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn when(at_ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(at_ms).map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string()).unwrap_or_default()
}

/// The task's body: the failing run's state, reason, infrastructure and
/// outcome, its summary (the agent's words, as data), its links, and where
/// the job's history is.
pub fn task_body(run: &JobRun) -> String {
    let mut lines = vec![format!("The job \"{}\" failed: {}.", one_line(&run.job_name), failure_words(run)), String::new()];
    lines.push(format!("Run {}", run.id));
    if let Some(slot) = run.scheduled_for_ms {
        lines.push(format!("- Due: {}", when(slot)));
    }
    lines.push(format!("- Started: {}", when(run.started_at_ms)));
    let reason = if run.reason.is_empty() { String::new() } else { format!(" ({})", run.reason) };
    lines.push(format!("- State: {}{reason}", run.state));
    if !run.infra.is_empty() {
        lines.push(format!("- Infrastructure: {}", run.infra));
    }
    lines.push(format!("- Outcome: {}", run.outcome));
    if !run.summary.is_empty() {
        lines.push(format!("- Summary, as the run reported it: {}", one_line(&run.summary)));
    }
    if let Some(block) = run.links.pointer("/transcript/block_id").and_then(|v| v.as_str()) {
        lines.push(format!("- Transcript: the agent pane {block} (Warden > History)"));
    }
    if let Some(reported) = run.links.get("reported").and_then(|v| v.as_array()) {
        for link in reported.iter().take(5) {
            let url = link.as_str().or_else(|| link.get("url").and_then(|u| u.as_str())).unwrap_or("");
            if url.starts_with("https://") {
                lines.push(format!("- Link: {url}"));
            }
        }
    }
    lines.push(String::new());
    lines.push(format!(
        "Every run of the job is in Warden > Jobs > {}. While the job keeps failing, each failure is added to this \
         task's history; its next success adds \"recovered\", and the task stays open for you to finish.",
        one_line(&run.job_name)
    ));
    lines.join("\n")
}

/// The `[AgentMux]` note to the owning leader (§3.4). The job's name is
/// quoted: an agent named it.
pub fn note_text(run: &JobRun, task_num: Option<i64>) -> String {
    let task = task_num.map(|n| format!(" Kanban #{n} is yours to look into.")).unwrap_or_default();
    format!(
        "[AgentMux] Job \"{}\" failed at {} ({}).{task} Its runs are in Warden > Jobs.",
        quote_in_system_note(&run.job_name),
        clock(failed_at(run)),
        failure_words(run)
    )
}

/// The notification's body: fixed text and the reason, never the summary.
pub fn notify_body(run: &JobRun, task_num: Option<i64>) -> String {
    let words = failure_words(run);
    let mut first = words.chars();
    let words = first.next().map(|c| c.to_uppercase().chain(first).collect::<String>()).unwrap_or_default();
    match task_num {
        Some(n) => format!("{words}. Kanban #{n}."),
        None => format!("{words}."),
    }
}

#[cfg(test)]
#[path = "incidents_tests.rs"]
mod tests;
