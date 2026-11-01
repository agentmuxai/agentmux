// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where cloud job events enter this AgentMux (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §4.2).
//!
//! MuxBus's subscriber hands each `job_event` frame here. The frame is never offered to an agent: it goes to the run
//! ledger if its reporter is pinned, its signature holds and an approved cloud job names it, and is counted and
//! dropped otherwise (`cloud_events`).

use std::sync::{Arc, OnceLock};

use agentmux_common::time::now_ms;

use super::cloud_events::{Applied, CloudJob, CloudJobs, Dropped, SignedJobEvent};
use crate::backend::job_templates;
use super::reporters::{Reporters, REPORTERS_FILE};
use super::RunLedger;

/// No cloud job is known while job templates are not running: every event is dropped as "unknown job".
struct NoneApproved;

impl CloudJobs for NoneApproved {
    fn cloud_job(&self, _job_id: &str) -> Option<CloudJob> {
        None
    }
}

struct Hub {
    ledger: Arc<RunLedger>,
    reporters: Arc<Reporters>,
    jobs: Box<dyn CloudJobs + Send + Sync>,
}

static HUB: OnceLock<Hub> = OnceLock::new();

/// Start taking cloud job events into `ledger`. The reporters are the job templates' own, so approving a cloud
/// template pins its reporter where events are checked; they are read from the data folder if templates are not
/// running.
pub fn install(ledger: Arc<RunLedger>) {
    let reporters = job_templates::service()
        .map(|s| s.reporters().clone())
        .unwrap_or_else(|| Arc::new(Reporters::open(&crate::backend::base::get_mux_data_dir().join(REPORTERS_FILE))));
    let jobs: Box<dyn CloudJobs + Send + Sync> = match job_templates::service() {
        Some(templates) => Box::new(templates.clone()),
        None => Box::new(NoneApproved),
    };
    let _ = HUB.set(Hub { ledger, reporters, jobs });
}

/// The reporters pinned on this computer, once the hub is installed.
pub fn reporters() -> Option<&'static Reporters> {
    HUB.get().map(|h| &*h.reporters)
}

/// One event off MuxBus. `None` before the hub is installed (the event is not kept: the relay holds it).
pub fn ingest(event: &SignedJobEvent) -> Option<Result<Applied, Dropped>> {
    let hub = HUB.get()?;
    Some(hub.ledger.ingest_cloud_event(event, &hub.reporters, hub.jobs.as_ref(), now_ms()))
}
