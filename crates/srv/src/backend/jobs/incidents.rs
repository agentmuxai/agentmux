// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! When a job's run fails: one incident per job, not one per run
//! (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §3.4).
//!
//! After each change to a job's runs the ledger nudges [`Incidents`], and a
//! sweep looks at every job every [`SWEEP_MS`]. The keeper walks the job's
//! decisive runs in the order they ended, from where it last stopped (its
//! cursor, `storage::job_incidents`), judging each with the Jobs tab's own
//! rule (`verdict::run_failed`):
//!
//! - **A failure with no incident open** opens one, files a Kanban task for
//!   the job's owning leader (`kind: job-failure`, no review), sends that
//!   leader an `[AgentMux]` note and the operator one notification
//!   (`NotifyKind::JobFailed`).
//! - **Another failure while it is open** is counted, and adds a line to the
//!   task's history (fewer as a streak grows, [`worth_a_line`]), and nothing
//!   else.
//! - **The next success** recovers it: the task gets a "recovered" line and
//!   stays open for its owner (§10, decision 6); the notification is taken
//!   down.
//! - **A task its owner finished or cancelled** while the job still failed
//!   ends the incident; the job's next failure opens a new one, and the next
//!   success still takes its notification down.
//! - **A late report that changes a run's outcome** moves the cursor back to
//!   it (`job_run_report`), so it is judged again: a success reported failed
//!   reopens the incident it recovered, or opens one.
//!
//! Only the instance that has the cron job (its shared store lists it) moves
//! an incident: the owner is resolved from the job, and the notification is
//! on that instance. Another channel that followed a run (#1988) only says
//! the job changed. Skips (paused, Pause all) are never decisive, so they
//! neither fail nor recover a job; a cap skip has its own once-a-day alert
//! (`cap_alert.rs`), and a run stopped at its cap (`over_budget`) is a failure
//! like any other. Cron jobs only: a Drone graph is run by hand, from a pane that
//! shows its failure already.

use std::collections::BTreeSet;
use std::sync::{Arc, Weak};

use agentmux_common::time::now_ms;
use serde_json::json;

use super::verdict::run_failed;
use crate::backend::mps::Broker;
use crate::backend::reactive::sanitize::quote_in_system_note;
use crate::backend::storage::cron::CronJob;
use crate::backend::storage::job_incidents::{IncidentCursor, JobIncident};
use crate::backend::storage::job_runs::{infra, reason, state, JobRun};
use crate::backend::storage::store::Store;
use crate::backend::storage::work_board::{requested, NewCard, WorkEvent};
use crate::backend::storage::work_queue::{work_state, WorkItem};

/// The task's `kind`, and who files it (`created_by`, `filed_by`, the
/// history's actor).
pub const TASK_KIND: &str = "job-failure";
pub const FILED_BY: &str = "jobs";

/// The most runs one look at a job walks; the rest wait for the next.
pub const PASS_MAX: i64 = 500;

/// How often every job is looked at even without a nudge: a run another
/// channel closed, or reported, reaches the owner this way at the latest.
pub const SWEEP_MS: u64 = 5 * 60 * 1000;

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

/// What one decisive run did to the job's incident.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// The first failure: an incident, its task, and the alert.
    Opened { incident: JobIncident, leader: Option<Leader>, note: String, notify_body: String },
    /// Another failure of the open incident: `count` in a row.
    FailedAgain { count: i64 },
    /// The job succeeded again after failing.
    Recovered,
    /// A run judged a success, which recovered `incident`, was reported
    /// failed later: the incident is open again, its notification back up.
    Reopened { incident: JobIncident, count: i64 },
}

/// What one walk over a job's runs did: the steps it took, whether it
/// stopped at [`PASS_MAX`] with more to take, and the error it stopped at.
#[derive(Default)]
struct Walked {
    steps: Vec<Step>,
    more: bool,
    error: Option<crate::backend::storage::StoreError>,
}

/// The incident keeper: one per instance, beside the run ledger.
pub struct Incidents {
    /// The identity store: the ledger, its incidents and the work queue.
    store: Arc<Store>,
    /// The channel's object store: agent definitions, to resolve the owning
    /// leader and name it.
    mstore: Option<Arc<Store>>,
    /// Where jobs are (`db_jobs`, the shared store): which jobs this
    /// instance has, and their owners.
    jobs: Option<Arc<Store>>,
    broker: Arc<Broker>,
    queue: std::sync::OnceLock<tokio::sync::mpsc::UnboundedSender<String>>,
    /// Tests: applying this run fails, as a store error would.
    #[cfg(test)]
    fail_on: std::sync::Mutex<Option<String>>,
    /// Tests: run once when this run is applied, after the walk read it.
    #[cfg(test)]
    on_apply: std::sync::Mutex<Option<(String, Box<dyn FnOnce() + Send>)>>,
    /// Tests: the notification router, once there is one: what it was sent.
    #[cfg(test)]
    router: std::sync::Mutex<Option<Vec<(String, String, String)>>>,
}

impl Incidents {
    /// A keeper that acts only when [`Self::assess`] is called.
    pub fn new(store: Arc<Store>, mstore: Option<Arc<Store>>, jobs: Option<Arc<Store>>, broker: Arc<Broker>) -> Arc<Self> {
        Arc::new(Incidents {
            store,
            mstore,
            jobs,
            broker,
            queue: std::sync::OnceLock::new(),
            #[cfg(test)]
            fail_on: std::sync::Mutex::new(None),
            #[cfg(test)]
            on_apply: std::sync::Mutex::new(None),
            #[cfg(test)]
            router: std::sync::Mutex::new(None),
        })
    }

    /// Look at each nudged job, and every job each [`SWEEP_MS`], on a task
    /// of its own, one at a time, off the caller's thread: a nudge comes from
    /// inside the ledger's writes. Needs a tokio runtime; without one,
    /// nudges do nothing.
    pub fn start(self: &Arc<Self>) {
        let Ok(rt) = tokio::runtime::Handle::try_current() else { return };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        if self.queue.set(tx).is_err() {
            return;
        }
        let weak: Weak<Incidents> = Arc::downgrade(self);
        rt.spawn(async move {
            let mut sweep = tokio::time::interval(std::time::Duration::from_millis(SWEEP_MS));
            sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                let mut due = BTreeSet::new();
                tokio::select! {
                    got = rx.recv() => match got {
                        Some(job_id) => {
                            due.insert(job_id);
                        }
                        None => return,
                    },
                    _ = sweep.tick() => {
                        let Some(me) = weak.upgrade() else { return };
                        due.extend(tokio::task::spawn_blocking(move || me.owned_jobs()).await.unwrap_or_default());
                    }
                }
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

    /// The cron jobs this instance has.
    fn owned_jobs(&self) -> Vec<String> {
        self.jobs.as_ref().and_then(|s| s.cron_list().ok()).unwrap_or_default().into_iter().map(|j| j.id).collect()
    }

    /// Look at job `job_id` now: walk its decisive runs since the last look,
    /// and act on each. Blocking. A job this instance doesn't have is left
    /// to the one that has it.
    pub fn assess(&self, job_id: &str) -> Vec<Step> {
        let Some(job) = self.jobs.as_ref().and_then(|s| s.cron_get(job_id).ok().flatten()) else {
            return Vec::new();
        };
        let walked = self.walk(&job);
        for step in &walked.steps {
            match step {
                Step::Opened { incident, leader: Some(leader), note, .. } => self.note_leader(incident, leader, note),
                Step::Opened { .. } | Step::FailedAgain { .. } | Step::Reopened { .. } => {}
                Step::Recovered => {
                    if let Some(router) = crate::backend::notify::router::get(&self.broker) {
                        router.resolve(&notify_key(job_id), crate::backend::notify::policy::Family::Job);
                    }
                }
            }
        }
        // An incident opened, or reopened, while no router existed (srv
        // starting, no window yet) is notified at a later look.
        self.notify_pending(job_id);
        if let Some(e) = &walked.error {
            // The run it stopped at is taken again at the next look.
            tracing::warn!(job_id, error = %e, "jobs: couldn't judge an incident");
        }
        if !walked.steps.is_empty() {
            self.changed(job_id);
        }
        if walked.more {
            self.nudge(job_id);
        }
        walked.steps
    }

    /// Apply the job's decisive runs after its cursor, one at a time, moving
    /// the cursor past each only once it is applied: an error stops the walk
    /// at that run, for the next look to take again, and the steps already
    /// taken are kept. Applying a run twice changes nothing
    /// (`db_job_incident_runs`), so two keepers, or a report that moved the
    /// cursor back, are safe. With no cursor yet, only the newest run
    /// counts, so a job's history from before this build opens nothing.
    fn walk(&self, job: &CronJob) -> Walked {
        let mut walked = Walked::default();
        let now = now_ms();
        let started = self.end_if_task_closed(&job.id, now).and_then(|_| {
            let cursor = self.store.job_incident_cursor(&job.id)?;
            let after = cursor.as_ref().map(|c| (c.ended_at_ms, c.run_id.as_str()));
            let runs = self.store.job_runs_decisive_after(&job.id, after, PASS_MAX)?;
            Ok((cursor, runs))
        });
        let (mut cursor, runs) = match started {
            Ok(got) => got,
            Err(e) => {
                walked.error = Some(e);
                return walked;
            }
        };
        walked.more = runs.len() as i64 >= PASS_MAX;
        for run in runs {
            if run.job_source == super::SOURCE_CRON {
                match self.apply(job, &run, now) {
                    Ok(step) => walked.steps.extend(step),
                    Err(e) => {
                        walked.error = Some(e);
                        walked.more = false;
                        return walked;
                    }
                }
            }
            let next = IncidentCursor { ended_at_ms: run.ended_at_ms.unwrap_or_default(), run_id: run.id.clone() };
            match self.store.job_incident_cursor_advance(&job.id, cursor.as_ref(), &next, &run.outcome) {
                Ok(true) => cursor = Some(next),
                // Another keeper moved it, a report moved it back, or a
                // report changed this run while it was applied: the next
                // look reads it again.
                Ok(false) => {
                    walked.more = false;
                    return walked;
                }
                Err(e) => {
                    walked.error = Some(e);
                    walked.more = false;
                    return walked;
                }
            }
        }
        walked
    }

    /// End the job's open incident when its task was finished or cancelled.
    /// `true` when it did.
    fn end_if_task_closed(&self, job_id: &str, now: i64) -> Result<bool, crate::backend::storage::StoreError> {
        match self.store.job_incident_open_for(job_id)? {
            Some(open) if self.task_closed(&open.work_id) => self.store.job_incident_end_task_closed(&open.id, now),
            _ => Ok(false),
        }
    }

    /// What one decisive run does to the job's incident.
    fn apply(&self, job: &CronJob, run: &JobRun, now: i64) -> Result<Option<Step>, crate::backend::storage::StoreError> {
        #[cfg(test)]
        {
            if self.fail_on.lock().unwrap().as_deref() == Some(run.id.as_str()) {
                return Err(crate::backend::storage::StoreError::NotFound);
            }
            let hook = {
                let mut slot = self.on_apply.lock().unwrap();
                if slot.as_ref().is_some_and(|(id, _)| *id == run.id) { slot.take() } else { None }
            };
            if let Some((_, f)) = hook {
                f();
            }
        }
        if !run_failed(run) {
            let recovered = self.store.job_incidents_recover(&job.id, &run.id, now)?;
            if recovered.is_empty() {
                return Ok(None);
            }
            let at = run.ended_at_ms.unwrap_or(now);
            for inc in &recovered {
                let streak = if inc.failures == 1 { "1 failure".to_string() } else { format!("{} failures in a row", inc.failures) };
                self.history(&inc.work_id, &format!("recovered at {}: the job's last run succeeded, after {streak}", clock(at)));
            }
            return Ok(Some(Step::Recovered));
        }
        if self.store.job_incident_run_counted(&run.id)? {
            return Ok(None);
        }
        // Judged a success before, it recovered an incident: a report says it
        // failed after all, so that incident goes on, unless its task was
        // finished or cancelled since: then this failure opens a new one,
        // with a task someone can act on.
        if let Some(inc) = self.store.job_incident_recovered_by(&job.id, &run.id)? {
            if !self.task_closed(&inc.work_id) {
                if let Some(count) = self.store.job_incident_reopen(&inc.id, &run.id, now)? {
                    self.link_run(&run.id, &inc);
                    let line = format!("failed after all: the run of {} reported it failed, {count} in a row", clock(failed_at(run)));
                    self.history(&inc.work_id, &line);
                    return Ok(Some(Step::Reopened { incident: inc, count }));
                }
            }
        }
        self.end_if_task_closed(&job.id, now)?;
        let Some(inc) = self.store.job_incident_open_for(&job.id)? else { return self.open(job, run, now) };
        let Some(count) = self.store.job_incident_fail_again(&inc.id, &inc.last_run_id, &run.id, now)? else {
            return Ok(None);
        };
        self.link_run(&run.id, &inc);
        if worth_a_line(count) {
            let line = format!("failed again at {} ({}), {count} in a row", clock(failed_at(run)), failure_words(run));
            self.history(&inc.work_id, &line);
        }
        Ok(Some(Step::FailedAgain { count }))
    }

    fn open(&self, job: &CronJob, run: &JobRun, now: i64) -> Result<Option<Step>, crate::backend::storage::StoreError> {
        let leader = self.leader(job);
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
            notified: false,
            ..Default::default()
        };
        if !self.store.job_incident_open(&incident)? {
            return Ok(None);
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
        Ok(Some(Step::Opened { incident, leader, note, notify_body }))
    }

    /// The owning leader of `job` (§3.4): its owner (the cron job's
    /// creator), that owner's lead when it has a seat with one or is a
    /// spawned worker, else the owner itself.
    fn leader(&self, job: &CronJob) -> Option<Leader> {
        owning_leader(self.mstore.as_ref(), job)
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

    /// The `[AgentMux]` note to the owning leader.
    fn note_leader(&self, incident: &JobIncident, leader: &Leader, note: &str) {
        if let Err(e) = crate::backend::reactive::get_global_handler().deliver_system_note(&leader.uid, note) {
            tracing::info!(job_id = %incident.job_id, error = %e, "jobs: the owning leader didn't get the incident note");
        }
    }

    /// Send the operator the open incident's notification, if it hasn't
    /// reached a router yet, and mark it sent only once one took it.
    fn notify_pending(&self, job_id: &str) {
        let Ok(Some(inc)) = self.store.job_incident_open_for(job_id) else { return };
        if inc.notified {
            return;
        }
        let Some(run) = self.store.job_run_get(&inc.last_run_id).ok().flatten() else { return };
        if !self.notify(&notify_key(job_id), &inc.job_name, notify_body(&run, inc.task_num)) {
            tracing::info!(job_id, "jobs: no notification router yet; the incident is notified at a later look");
            return;
        }
        if let Err(e) = self.store.job_incident_mark_notified(&inc.id) {
            tracing::warn!(job_id, error = %e, "jobs: couldn't mark an incident notified");
        }
    }

    /// Hand a `JobFailed` notification to this instance's router. `false`
    /// when there is none yet.
    fn notify(&self, key: &str, job_name: &str, body: String) -> bool {
        #[cfg(test)]
        {
            let mut router = self.router.lock().unwrap();
            return match router.as_mut() {
                Some(sent) => {
                    sent.push((key.to_string(), job_name.to_string(), body));
                    true
                }
                None => false,
            };
        }
        #[cfg(not(test))]
        match crate::backend::notify::router::get(&self.broker) {
            Some(router) => {
                router.job_failed(key, job_name, body);
                true
            }
            None => false,
        }
    }

    /// Tell the Jobs tab and the Kanban.
    fn changed(&self, job_id: &str) {
        let event = crate::backend::mps_events::JobsChangedEvent { job_id: job_id.to_string() };
        self.broker.publish_event::<crate::backend::mps_events::JobsChanged>(&event, vec![], 0);
        self.broker.publish_event::<crate::backend::mps_events::WorkQueueChanged>(&(), vec![], 0);
    }
}

/// The owning leader of `job` (§3.4): its owner (the cron job's creator),
/// that owner's lead when it has a seat with one or is a spawned worker, else
/// the owner itself. `None` when the job has no owner, or none resolves.
pub fn owning_leader(mstore: Option<&Arc<Store>>, job: &CronJob) -> Option<Leader> {
    let owner = job.owner_uid();
    if owner.is_empty() {
        return None;
    }
    let mstore = mstore?;
    let name_of = |uid: &str| mstore.agent_def_get(uid).ok().flatten().map(|d| d.name);
    let lead = crate::backend::agent_seat::effective_seat_by_id(mstore, owner).and_then(|s| s.lead);
    let uid = lead.filter(|l| name_of(l).is_some()).unwrap_or_else(|| owner.to_string());
    let name = name_of(&uid)?;
    Some(Leader { uid, name })
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
        lines.push(format!("- Transcript: the agent pane {block} (History)"));
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
