// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Persistent cron scheduler for AgentMux.
//!
//! Backs each enabled `CronJob` with a tokio task that sleeps until the
//! next scheduled fire time (computed from the 5-field UTC cron expression),
//! delivers the prompt as `/agentmux/reactive/inject` would — in process
//! since identity M4c-3 (see [`FireDelivery`]) — and records the fire in the
//! DB.
//!
//! On startup, runs one catch-up fire for any job whose next scheduled time
//! after `last_fired` is already in the past (FIRE_ONCE_NOW misfire policy —
//! never replay all missed fires, never cause a cron storm).
//!
//! See `docs/specs/SPEC_CRON_LOOP_ROBUSTNESS_2026_06_25.md §3.2`.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use agentmux_common::api_types::InjectRequest;
use chrono::{DateTime, Utc};
use cron::Schedule;

use crate::backend::storage::store::Store;
use crate::backend::storage::cron::CronJob;
use crate::backend::mps::{Broker, MuxEvent, EVENT_CRON_CHANGED};
use crate::backend::reactive::types::InjectionRequest;

/// Abort handles for every currently scheduled cron task.
type HandleMap = Mutex<HashMap<String, tokio::task::AbortHandle>>;

/// In-process delivery of a fire: the server's shared inject path
/// (`server::reactive::deliver`, on the instance-key tier), given the job's
/// request and its creator's UID. Identity M4c-3
/// (SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md §6.5.9): the
/// scheduler is built before `AppState`, so the server installs this once
/// `AppState` exists ([`CronScheduler::install_delivery`]), as it installs
/// agent-turn delivery. Until then a fire POSTs to the route, as before.
pub type FireDelivery = Arc<
    dyn Fn(
            InjectionRequest,
            String,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = serde_json::Value> + Send>>
        + Send
        + Sync,
>;

pub struct CronScheduler {
    handles: HandleMap,
    shared_store: Option<Arc<Store>>,
    http_client: reqwest::Client,
    local_url: String,
    auth_key: String,
    broker: Arc<Broker>,
    delivery: std::sync::OnceLock<FireDelivery>,
}

impl CronScheduler {
    pub fn new(
        shared_store: Option<Arc<Store>>,
        http_client: reqwest::Client,
        local_url: String,
        auth_key: String,
        broker: Arc<Broker>,
    ) -> Arc<Self> {
        Arc::new(Self {
            handles: Mutex::new(HashMap::new()),
            shared_store,
            http_client,
            local_url,
            auth_key,
            broker,
            delivery: std::sync::OnceLock::new(),
        })
    }

    /// Deliver fires in process from now on (see [`FireDelivery`]). Once;
    /// a second install is ignored.
    pub fn install_delivery(&self, delivery: FireDelivery) {
        let _ = self.delivery.set(delivery);
    }

    fn publish_changed(&self) {
        self.broker.publish(MuxEvent {
            event: EVENT_CRON_CHANGED.to_string(),
            scopes: vec![],
            sender: String::new(),
            persist: 0,
            data: None,
        });
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

        let now_dt = Utc::now();
        let count = jobs.len();

        for job in jobs {
            // FIRE_ONCE_NOW: if the job missed its window, fire once immediately
            // as catch-up. did_catchup is passed to schedule_job so the live
            // task's fires counter is seeded at fire_count+1 and the catch-up
            // fire counts toward max_fires.
            let did_catchup = should_catchup(&job, now_dt);
            if did_catchup {
                let sched = self.clone();
                let job_id = job.id.clone();
                let job_prompt = job.prompt.clone();
                let job_target = job.target.clone();
                let job_target_uid = job.target_uid.clone();
                let job_created_by_uid = job.created_by_uid.clone();
                tokio::spawn(async move {
                    sched
                        .fire(&job_id, &job_prompt, &job_target, &job_target_uid, &job_created_by_uid)
                        .await;
                });
            }

            let initial_fires = job.fire_count + if did_catchup { 1 } else { 0 };
            self.schedule_job_with_fires(&job, initial_fires);
        }

        tracing::info!(count, "cron: scheduled {} job(s) from DB", count);
    }

    /// Schedule a single job (or reschedule it after a DB change). Replaces
    /// any existing task for the same `job.id`. Call sites that don't need to
    /// account for a simultaneous catch-up fire should pass `job.fire_count`.
    pub fn schedule_job(self: &Arc<Self>, job: &CronJob) {
        self.schedule_job_with_fires(job, job.fire_count);
    }

    fn schedule_job_with_fires(self: &Arc<Self>, job: &CronJob, initial_fires: i64) {
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
        let job_target_uid = job.target_uid.clone();
        let job_created_by_uid = job.created_by_uid.clone();
        let job_max_fires = job.max_fires;
        let job_created_at = job.created_at;
        let job_max_age_secs = job.max_age_secs;

        let handle = tokio::spawn(async move {
            // `initial_fires` is seeded from the persisted fire_count (plus 1
            // if a catch-up fire was also dispatched at startup) so max_fires
            // is enforced across restarts, not per process run.
            let mut fires: i64 = initial_fires;
            loop {
                // Guard at top of loop: if fires was seeded at or above max_fires
                // (e.g. a catch-up fire brought the persisted count to the cap),
                // don't fire again before sleeping — this prevents one extra fire
                // on restart when the catch-up itself hits the limit. Age is
                // checked the same way (Phase 0, SPEC_AGENT_POLLING_AND_WAKEUP_
                // HARDENING_2026_08_04.md) — a job created before max_age_secs
                // existed has max_age_secs = None and is never expired by this.
                if let Some(max) = job_max_fires {
                    if fires >= max {
                        sched.disable_job(&job_id, "max_fires reached");
                        break;
                    }
                }
                if is_expired_by_age(job_created_at, job_max_age_secs, Utc::now().timestamp()) {
                    sched.disable_job(&job_id, "max_age_secs reached");
                    break;
                }
                let next = match schedule.upcoming(Utc).next() {
                    Some(t) => t,
                    None => break,
                };
                // Reviewer-caught gap (P1, PR #2418): checking age only in "now"
                // terms at the top of the loop lets one extra fire slip through
                // when the cron interval is longer than the remaining time to
                // expiry — the loop would sleep straight past the expiry bound
                // and fire anyway, only catching it on the *next* iteration.
                // Check whether the fire we're about to sleep for would itself
                // land at/after expiry, and skip it (disable now, don't sleep)
                // if so — the "hard expiry bound, regardless of fire count"
                // guarantee has to hold at fire time, not just at loop-top time.
                if is_expired_by_age(job_created_at, job_max_age_secs, next.timestamp()) {
                    sched.disable_job(&job_id, "max_age_secs would be reached before the next scheduled fire");
                    break;
                }
                let delay = (next - Utc::now()).to_std().unwrap_or_default();
                tokio::time::sleep(delay).await;
                sched
                    .fire(&job_id, &job_prompt, &job_target, &job_target_uid, &job_created_by_uid)
                    .await;
                fires += 1;
                // Post-fire check: enforce max_fires. The top-of-loop guard
                // handles the restart/seeded-at-cap case; this handles the
                // normal live-run case.
                if let Some(max) = job_max_fires {
                    if fires >= max {
                        sched.disable_job(&job_id, "max_fires reached");
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

    /// Disable a job in the DB (audit trail preserved, unlike delete), drop
    /// its live task handle, and notify listeners. Shared by every
    /// self-disabling reason a scheduled job's own loop hits (max_fires,
    /// max_age_secs) — extracted so those call sites don't each repeat the
    /// same three-step disable sequence.
    fn disable_job(&self, job_id: &str, reason: &str) {
        tracing::info!(id = job_id, reason, "cron: disabling job");
        if let Some(store) = &self.shared_store {
            let _ = store.cron_set_enabled(job_id, false);
        }
        self.handles.lock().unwrap().remove(job_id);
        self.publish_changed();
    }

    /// Fire a cron job: deliver the prompt and record the fire in DB.
    ///
    /// Identity M3 (spec §5.4): a job that captured its target's UID at
    /// creation fires BY UID — a job firing at 03:00 must never resolve a
    /// name. A job with no UID (created before M3, or by a caller that did
    /// not carry one) fires by name as before, counted.
    ///
    /// Identity M4c-3 (§6.5.9): delivered in process ([`FireDelivery`]), with
    /// the creator's UID as attribution on this instance only — a forwarded
    /// hop never carries it. **The sender name stays `"cron"`**: showing the
    /// creator's name unsigned would make every fire of an agent-created job
    /// look forged (its key is on file under that name) and echo each fire
    /// into the creator's pane. A job with no captured creator UID fires as
    /// before, counted.
    pub(crate) async fn fire(&self, id: &str, prompt: &str, target: &str, target_uid: &str, created_by_uid: &str) {
        if created_by_uid.is_empty() {
            crate::backend::agent_resolve::record_uid_fallback("m4c.cron_fire_unattributed");
        }
        let target_agent = fire_target(target, target_uid).to_string();
        let body = match self.delivery.get() {
            Some(deliver) => {
                let req = InjectionRequest {
                    target_agent,
                    message: prompt.to_string(),
                    source_agent: Some("cron".to_string()),
                    ..Default::default()
                };
                Some(deliver(req, created_by_uid.to_string()).await)
            }
            None => {
                if self.local_url.is_empty() || self.auth_key.is_empty() {
                    tracing::warn!(id, "cron: no local_url/auth_key — skipping fire");
                    return;
                }
                crate::backend::agent_resolve::record_uid_fallback("m4c.cron_fire_http");
                self.post_fire(id, target_agent, prompt).await
            }
        };
        if let Some(body) = body {
            // Inject answers `success: false` when the target is not
            // reachable — since M3 that includes a UID nobody is registered
            // under. A fire that reached no one is a failure, and a firing
            // job has nobody to tell but the log (§5.4).
            if body.get("success").and_then(|v| v.as_bool()) == Some(false) {
                tracing::warn!(
                    id,
                    target,
                    target_uid,
                    error = body.get("error").and_then(|v| v.as_str()).unwrap_or(""),
                    "cron: fire was not delivered"
                );
            } else {
                tracing::debug!(id, target, "cron: fired");
            }
        }

        if let Some(store) = &self.shared_store {
            let now = Utc::now().timestamp();
            if let Err(e) = store.cron_record_fire(id, now) {
                tracing::warn!(id, error = %e, "cron: failed to record fire in DB");
            }
        }
        self.publish_changed();
    }

    /// The pre-M4c-3 fire: POST to the local inject route with the instance
    /// key. Only until [`Self::install_delivery`] runs. `None` when the POST
    /// itself failed (logged here).
    async fn post_fire(&self, id: &str, target_agent: String, prompt: &str) -> Option<serde_json::Value> {
        let url = format!("{}/agentmux/reactive/inject", self.local_url.trim_end_matches('/'));
        let req = InjectRequest {
            target_agent,
            message: prompt.to_string(),
            source_agent: Some("cron".to_string()),
            ..Default::default()
        };
        match self.http_client.post(&url).header("X-AuthKey", &self.auth_key).json(&req).send().await {
            Ok(r) if r.status().is_success() => Some(r.json().await.unwrap_or_default()),
            Ok(r) => {
                tracing::warn!(id, status = %r.status(), "cron: inject returned non-2xx");
                None
            }
            Err(e) => {
                tracing::warn!(id, error = %e, "cron: inject request failed");
                None
            }
        }
    }
}

/// Determine if a job needs a catch-up fire on startup.
///
/// True when there is a scheduled fire time that falls strictly between
/// `last_fired` and `now` — i.e., the first occurrence of the cron expression
/// after `last_fired` is already in the past. This is correct regardless of
/// schedule granularity: a daily job that ran at 09:00 and is restarted at
/// 09:05 yields a next-after-last of TOMORROW 09:00, which is NOT < now, so
/// no spurious catch-up fires.
///
/// Jobs that have never fired are skipped (they'll fire at the next naturally
/// scheduled time without any missed-window concept).
fn should_catchup(job: &CronJob, now_dt: DateTime<Utc>) -> bool {
    let Some(last) = job.last_fired else { return false; };
    let last_dt = match DateTime::from_timestamp(last, 0) {
        Some(dt) => dt,
        None => return false,
    };
    let Ok(schedule) = Schedule::from_str(&format!("0 {}", job.expression)) else {
        return false;
    };
    // First scheduled time after last_fired — if it's already past, a fire was missed.
    match schedule.after(&last_dt).next() {
        Some(next_after_last) => next_after_last < now_dt,
        None => false,
    }
}

/// True when a job's hard age-expiry bound (`max_age_secs`, seconds since
/// `created_at`) has been reached. `None` = no bound, never expires by age
/// (the default for every job created before this field existed, and for
/// any job that explicitly opts out). See
/// docs/specs/SPEC_AGENT_POLLING_AND_WAKEUP_HARDENING_2026_08_04.md Phase 0.
fn is_expired_by_age(created_at: i64, max_age_secs: Option<i64>, now: i64) -> bool {
    match max_age_secs {
        Some(max_age) => now - created_at >= max_age,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_expired_by_age_none_never_expires() {
        assert!(!is_expired_by_age(1_000, None, 1_000_000_000));
    }

    #[test]
    fn is_expired_by_age_before_bound_is_not_expired() {
        assert!(!is_expired_by_age(1_000, Some(3_600), 1_000 + 3_599));
    }

    #[test]
    fn is_expired_by_age_at_bound_is_expired() {
        // >= at the boundary, matching the max_fires check's own >= semantics.
        assert!(is_expired_by_age(1_000, Some(3_600), 1_000 + 3_600));
    }

    #[test]
    fn is_expired_by_age_past_bound_is_expired() {
        assert!(is_expired_by_age(1_000, Some(3_600), 1_000 + 10_000));
    }

    /// Regression test for PR #2418's P1 review finding: the scheduler now
    /// calls this same function with the *next scheduled fire's* timestamp,
    /// not just "now", to decide whether to skip a fire that would land past
    /// expiry rather than sleeping through the bound and firing anyway. A
    /// job created 1000s ago with a 3600s bound, checked against a `next`
    /// fire time far past that bound (e.g. a daily cron job's next fire is
    /// hours away), must report expired — this is exactly the "would this
    /// fire itself violate the hard expiry bound" check the loop performs
    /// before deciding whether to sleep at all.
    #[test]
    fn is_expired_by_age_catches_a_next_fire_time_past_the_bound() {
        let created_at = 1_000;
        let max_age_secs = Some(3_600);
        let next_fire_far_in_the_future = created_at + 86_400; // 24h away
        assert!(is_expired_by_age(created_at, max_age_secs, next_fire_far_in_the_future));
    }
}

/// What a cron fire addresses: the UID when one was captured at creation,
/// else the name (counted, spec §9.2 — `cron.fire_by_name` reaching zero is
/// part of M5's exit criterion).
fn fire_target<'a>(target: &'a str, target_uid: &'a str) -> &'a str {
    let uid = target_uid.trim();
    if uid.is_empty() {
        crate::backend::agent_resolve::record_uid_fallback("cron.fire_by_name");
        target
    } else {
        uid
    }
}

#[cfg(test)]
mod identity_m3_tests {
    use super::fire_target;

    #[test]
    fn fires_by_uid_when_captured_and_by_name_otherwise() {
        assert_eq!(fire_target("AgentY", "4f3c-a91"), "4f3c-a91");
        assert_eq!(fire_target("AgentY", ""), "AgentY");
        assert_eq!(fire_target("AgentY", "   "), "AgentY");
    }
}

#[cfg(test)]
mod identity_m4c3_tests {
    use super::*;

    /// Identity M4c-3 (spec §6.5.9): with delivery installed, a fire goes in
    /// process — by target UID, as `"cron"`, with the creator's UID beside
    /// it — and never over HTTP.
    #[tokio::test]
    async fn a_fire_is_delivered_in_process_as_cron_with_the_creators_uid() {
        let sched = CronScheduler::new(
            None,
            reqwest::Client::new(),
            // Unreachable: a fire that fell back to HTTP could not succeed.
            "http://127.0.0.1:1".to_string(),
            "test".to_string(),
            Arc::new(Broker::new()),
        );
        let seen: Arc<Mutex<Vec<(InjectionRequest, String)>>> = Arc::default();
        let sink = seen.clone();
        sched.install_delivery(Arc::new(move |req, creator_uid| {
            sink.lock().unwrap().push((req, creator_uid));
            Box::pin(async { serde_json::json!({"success": true}) })
        }));

        sched.fire("job-1", "check in", "AgentY", "uid-target", "uid-creator").await;

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        let (req, creator_uid) = &seen[0];
        assert_eq!(req.target_agent, "uid-target");
        assert_eq!(req.message, "check in");
        assert_eq!(req.source_agent.as_deref(), Some("cron"), "the sender name stays cron");
        assert_eq!(creator_uid, "uid-creator");
        assert_eq!(req.audit_source_uid, "", "the delivery path sets it, on this instance only");
    }
}
