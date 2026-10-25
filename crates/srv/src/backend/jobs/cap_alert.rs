// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The alert for a job's first cap skip of the day (docs/specs/
//! SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.3): one note to the owning
//! leader and one notification to the operator. No Kanban task: a skip is the
//! cap doing its work, not a failure. The job isn't paused and its runs
//! resume the next day.

use super::caps::Refusal;
use super::RunLedger;
use crate::backend::reactive::sanitize::quote_in_system_note;
use crate::backend::storage::cron::CronJob;

/// The notification's group: one per job, apart from its failure incident's.
pub fn alert_key(job_id: &str) -> String {
    format!("job:{job_id}:cap")
}

/// The `[AgentMux]` note to the owning leader. The job's name is quoted: an
/// agent named it.
pub fn note_text(job_name: &str, refusal: Refusal) -> String {
    format!(
        "[AgentMux] Job \"{}\" skipped a run: {}. It isn't paused and runs again tomorrow (the day ends at the \
         operator's local midnight). Only the operator raises a cap, in Settings → Jobs.",
        quote_in_system_note(job_name),
        refusal.words()
    )
}

impl RunLedger {
    /// Raise the once-a-day alert for `job`, skipped for `refusal`.
    pub(super) fn alert_cap(&self, job: &CronJob, refusal: Refusal) {
        #[cfg(test)]
        self.cap_alerts.lock().unwrap().push((job.id.clone(), refusal.reason().to_string()));
        #[cfg(not(test))]
        {
            use super::incidents::owning_leader;
            if let Some(leader) = owning_leader(self.mstore.as_ref(), job) {
                let note = note_text(&job.name, refusal);
                if let Err(e) = crate::backend::reactive::get_global_handler().deliver_system_note(&leader.uid, &note) {
                    tracing::info!(job_id = %job.id, error = %e, "jobs: the owning leader didn't get the cap note");
                }
            }
            match crate::backend::notify::router::get(&self.broker) {
                Some(router) => {
                    let title = format!("{} ({})", job.name, refusal.label());
                    router.job_failed(&alert_key(&job.id), &title, format!("Skipped a run: {}.", refusal.words()));
                }
                None => tracing::info!(job_id = %job.id, "jobs: no notification router yet; the cap alert is not shown"),
            }
        }
    }
}
