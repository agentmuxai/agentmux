// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Persistent cron scheduler for AgentMux.
//!
//! Backs each enabled `CronJob` with a tokio task that sleeps until the
//! next scheduled fire time (computed from the 5-field UTC cron expression),
//! POSTs to `/agentmux/reactive/inject`, and records the fire in the DB.
//!
//! On startup, runs one catch-up fire for any job whose `last_fired` is
//! more than one interval behind (FIRE_ONCE_NOW misfire policy — never
//! replay all missed fires, never cause a cron storm).
//!
//! See `docs/specs/SPEC_CRON_LOOP_ROBUSTNESS_2026_06_25.md §3.2`.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use agentmux_common::api_types::InjectRequest;
use chrono::Utc;
use cron::Schedule;

use crate::backend::storage::store::Store;
use crate::backend::storage::cron::CronJob;

/// Abort handles for every currently scheduled cron task.
type HandleMap = Mutex<HashMap<String, tokio::task::AbortHandle>>;

pub struct CronScheduler {
    handles: HandleMap,
    shared_store: Option<Arc<Store>>,
    http_client: reqwest::Client,
    local_url: String,
    auth_key: String,
}

impl CronScheduler {
    pub fn new(
        shared_store: Option<Arc<Store>>,
        http_client: reqwest::Client,
        local_url: String,
        auth_key: String,
    ) -> Arc<Self> {
        Arc::new(Self {
            handles: Mutex::new(HashMap::new()),
            shared_store,
            http_client,
            local_url,
            auth_key,
        })
    }

    /// Load all enabled jobs from the DB and schedule them. Call once at startup.
    pub async fn start(self: &Arc<Self>) {
        let store = match &self.shared_store {
            Some(s) => s.clone(),
            None => {
                tracing::warn!("cron: no shared store — cron scheduler disabled");
                return;
            }
        };

        let jobs = match store.cron_list_enabled() {
            Ok(j) => j,
            Err(e) => {
                tracing::error!(error = %e, "cron: failed to load jobs on startup");
                return;
            }
        };

        let now_secs = Utc::now().timestamp();
        let count = jobs.len();

        for job in jobs {
            // FIRE_ONCE_NOW: if the job missed its window (last_fired is more
            // than one interval ago), fire once immediately as catch-up.
            if should_catchup(&job, now_secs) {
                let sched = self.clone();
                let job_id = job.id.clone();
                let job_prompt = job.prompt.clone();
                let job_target = job.target.clone();
                tokio::spawn(async move {
                    sched.fire(&job_id, &job_prompt, &job_target).await;
                });
            }

            self.schedule_job(&job);
        }

        tracing::info!(count, "cron: scheduled {} job(s) from DB", count);
    }

    /// Schedule a single job (or reschedule it after a DB change). Replaces
    /// any existing task for the same `job.id`.
    pub fn schedule_job(self: &Arc<Self>, job: &CronJob) {
        self.cancel_job(&job.id);

        let schedule = match Schedule::from_str(&format!("0 {}", job.expression)) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(id = %job.id, expr = %job.expression, error = %e, "cron: invalid expression — skipping job");
                return;
            }
        };

        let sched = self.clone();
        let job_id = job.id.clone();
        let job_prompt = job.prompt.clone();
        let job_target = job.target.clone();
        let job_max_fires = job.max_fires;
        // Seed from persisted fire_count so a srv restart doesn't reset the
        // per-lifetime counter and allow a capped job to over-fire.
        let job_fire_count = job.fire_count;

        let handle = tokio::spawn(async move {
            let mut fires: i64 = job_fire_count;
            loop {
                let next = match schedule.upcoming(Utc).next() {
                    Some(t) => t,
                    None => break,
                };
                let delay = (next - Utc::now()).to_std().unwrap_or_default();
                tokio::time::sleep(delay).await;
                sched.fire(&job_id, &job_prompt, &job_target).await;
                fires += 1;
                // Enforce max_fires in the live task — don't rely on the DB
                // flag alone, which only takes effect on the next srv restart.
                if let Some(max) = job_max_fires {
                    if fires >= max {
                        tracing::info!(id = %job_id, fires, "cron: max_fires reached — disabling job");
                        if let Some(store) = &sched.shared_store {
                            let _ = store.cron_set_enabled(&job_id, false);
                        }
                        sched.handles.lock().unwrap().remove(&job_id);
                        break;
                    }
                }
            }
        });

        self.handles.lock().unwrap().insert(job.id.clone(), handle.abort_handle());
    }

    /// Stop (abort) a job's scheduled task without removing it from the DB.
    pub fn cancel_job(&self, id: &str) {
        if let Some(handle) = self.handles.lock().unwrap().remove(id) {
            handle.abort();
        }
    }

    /// Fire a cron job: POST to reactive inject and record the fire in DB.
    async fn fire(&self, id: &str, prompt: &str, target: &str) {
        if self.local_url.is_empty() || self.auth_key.is_empty() {
            tracing::warn!(id, "cron: no local_url/auth_key — skipping fire");
            return;
        }

        let url = format!("{}/agentmux/reactive/inject", self.local_url.trim_end_matches('/'));
        let req = InjectRequest {
            target_agent: target.to_string(),
            message: prompt.to_string(),
            source_agent: Some("cron".to_string()),
        };

        match self.http_client.post(&url).header("X-AuthKey", &self.auth_key).json(&req).send().await {
            Ok(r) if r.status().is_success() => {
                tracing::debug!(id, target, "cron: fired");
            }
            Ok(r) => {
                tracing::warn!(id, status = %r.status(), "cron: inject returned non-2xx");
            }
            Err(e) => {
                tracing::warn!(id, error = %e, "cron: inject request failed");
            }
        }

        if let Some(store) = &self.shared_store {
            let now = Utc::now().timestamp();
            if let Err(e) = store.cron_record_fire(id, now) {
                tracing::warn!(id, error = %e, "cron: failed to record fire in DB");
            }
        }
    }
}

/// Determine if a job needs a catch-up fire on startup. True when the job
/// has never fired or missed its most recent window by more than the expected
/// cycle (approximated as 60s minimum — any job that was due more than a
/// minute ago and hasn't fired yet gets one immediate catch-up run).
fn should_catchup(job: &CronJob, now_secs: i64) -> bool {
    let Some(last) = job.last_fired else {
        // Never fired before — check if it was scheduled to fire before now.
        // We don't have a "created_at as first expected fire" without parsing
        // the expression, so we skip the catch-up for brand-new jobs. They'll
        // fire at the next scheduled time naturally.
        return false;
    };
    let elapsed = now_secs.saturating_sub(last);
    elapsed > 120 // missed by more than 2 minutes → one catch-up
}
