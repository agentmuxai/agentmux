// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Credential Broker — Phase A of the auth-architecture consolidation.
//!
//! See `docs/specs/REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07_21.md`
//! for the full rationale. AgentMux runs three independent OAuth/credential
//! systems today (provider-CLI identity, MuxBus cloud login, an unused
//! Armory service-account scaffold) with no shared refresh or storage model.
//! This module is the consolidation point: a single, generic,
//! single-flight-guarded, proactively-scheduled refresher (`scheduler`),
//! with each credential system registering its own load/refresh/save
//! behavior against its own real backing store.
//!
//! **Phase A scope:** MuxBus is the only registered source (it's the only
//! system that already does active token refresh today, so it's the
//! smallest, lowest-risk slice to prove the broker out on). CLI-provider
//! credentials and the Armory service-account scaffold are future backends
//! (Phase C+), not touched here.

pub mod scheduler;

pub use scheduler::RefreshScheduler;

use std::sync::{Arc, OnceLock};

static GLOBAL_SCHEDULER: OnceLock<Arc<RefreshScheduler>> = OnceLock::new();

/// Initialize the global broker scheduler and start its background sweep.
/// Idempotent — the sweep loop is spawned exactly once no matter how many
/// times this is called; later calls just return the existing scheduler.
pub fn init_global(sweep_interval: std::time::Duration) -> Arc<RefreshScheduler> {
    GLOBAL_SCHEDULER
        .get_or_init(|| {
            let scheduler = Arc::new(RefreshScheduler::new());
            tokio::spawn(scheduler.clone().run_sweep_loop(sweep_interval));
            scheduler
        })
        .clone()
}

/// The global scheduler, if `init_global` has already run.
pub fn get_global() -> Option<Arc<RefreshScheduler>> {
    GLOBAL_SCHEDULER.get().cloned()
}
