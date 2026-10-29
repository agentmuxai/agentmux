// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Runs the old-version cleanup (`old_versions.rs`) once a day.
//!
//! Every srv looks at its own channel's version folders and logs what it found. It removes them
//! only on a host whose settings say `oldversions:clean: true`; until then the plan is the report
//! (the log line, and `GET /agentmux/old-versions`). Like the build-cache cleanup it runs only in
//! an srv started by its launcher on the real home, never in a dev, headless or test instance.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::backend::old_versions::{self as ov, Policy};
use crate::backend::wconfig::ConfigState;

/// The first pass waits this long after start, so it never competes with startup.
const FIRST_AFTER: Duration = Duration::from_secs(20 * 60);
const EVERY: Duration = Duration::from_secs(24 * 3600);

pub fn policy(config: &ConfigState) -> Policy {
    Policy::from_extra(&config.get_settings().extra)
}

fn pass(config: &ConfigState) {
    let Some(layout) = ov::layout_from_env() else {
        tracing::debug!("old versions: this srv has no per-version folders");
        return;
    };
    let policy = policy(config);
    let run = ov::run(&layout, &policy, !policy.clean, None, SystemTime::now());
    if !run.scan_ok {
        tracing::warn!("old versions: the running processes could not be read; nothing is removed");
        return;
    }
    let plan = &run.plan;
    match &run.report {
        Some(report) => {
            for (dir, bytes) in &report.removed {
                tracing::info!(dir = %dir.display(), freed = %crate::backend::build_cache::human(*bytes), "old versions: removed");
            }
            for (dir, why) in &report.skipped {
                tracing::info!(dir = %dir.display(), why = %why, "old versions: left alone");
            }
        }
        None if !plan.removable.is_empty() => {
            for c in &plan.removable {
                tracing::info!(dir = %c.dir.display(), kind = ?c.kind, size = %crate::backend::build_cache::human(c.bytes), "old versions: could be removed");
            }
            tracing::info!(
                folders = plan.removable.len(),
                reclaimable = %crate::backend::build_cache::human(plan.reclaimable_bytes()),
                "old versions: a dry run (set oldversions:clean to remove them)"
            );
        }
        None => {}
    }
}

/// Starts the schedule. Safe to call once at boot.
pub fn install(config: Arc<ConfigState>) {
    if !crate::backend::build_cache_service::this_srv_may_clean() {
        tracing::debug!("old versions: not an srv that cleans");
        return;
    }
    tokio::spawn(async move {
        tokio::time::sleep(FIRST_AFTER).await;
        let mut tick = tokio::time::interval(EVERY);
        loop {
            tick.tick().await;
            let cfg = config.clone();
            let _ = tokio::task::spawn_blocking(move || pass(&cfg)).await;
        }
    });
}
