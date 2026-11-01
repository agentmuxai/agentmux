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
use super::reporters::{Reporters, REPORTERS_FILE};
use super::RunLedger;

/// Cloud jobs that are approved on this computer. None are yet: every event is dropped as "unknown job" until
/// cloud job manifests can be approved.
struct NoneApproved;

impl CloudJobs for NoneApproved {
    fn cloud_job(&self, _job_id: &str) -> Option<CloudJob> {
        None
    }
}

struct Hub {
    ledger: Arc<RunLedger>,
    reporters: Reporters,
    jobs: Box<dyn CloudJobs + Send + Sync>,
}

static HUB: OnceLock<Hub> = OnceLock::new();

/// Start taking cloud job events into `ledger`. The reporters the operator pinned are read from the job templates'
/// data folder.
pub fn install(ledger: Arc<RunLedger>) {
    let reporters = Reporters::open(&crate::backend::base::get_mux_data_dir().join(REPORTERS_FILE));
    let _ = HUB.set(Hub { ledger, reporters, jobs: Box::new(NoneApproved) });
}

/// The reporters pinned on this computer, once the hub is installed.
pub fn reporters() -> Option<&'static Reporters> {
    HUB.get().map(|h| &h.reporters)
}

/// One event off MuxBus. `None` before the hub is installed (the event is not kept: the relay holds it).
pub fn ingest(event: &SignedJobEvent) -> Option<Result<Applied, Dropped>> {
    let hub = HUB.get()?;
    Some(hub.ledger.ingest_cloud_event(event, &hub.reporters, hub.jobs.as_ref(), now_ms()))
}
