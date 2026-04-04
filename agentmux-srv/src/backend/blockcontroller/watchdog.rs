// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Agent process watchdog: kills agent panes that exceed max-runtime or idle-output limits.
//!
//! Runs every 60 seconds and inspects every running ShellController that has
//! `is_agent_pane = true`. Two independent kill conditions:
//!
//!   A) `term:agentmaxruntimehours` — wall-clock runtime since spawn (0 = disabled)
//!   B) `term:agentidletimeoutmins` — minutes since last PTY byte (0 = disabled)
//!
//! Both limits default to 0 (disabled) so the watchdog is opt-in.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::interval;

use crate::backend::wconfig::ConfigWatcher;
use super::{get_all_controllers, STATUS_RUNNING};

/// Check interval for the watchdog loop.
const WATCHDOG_INTERVAL_SECS: u64 = 60;

/// Run the agent watchdog loop. Never returns.
pub async fn run_watchdog_loop(config: Arc<ConfigWatcher>) {
    let mut ticker = interval(Duration::from_secs(WATCHDOG_INTERVAL_SECS));
    loop {
        ticker.tick().await;
        let settings = config.get_settings();
        let max_runtime_hours = settings.term_agent_max_runtime_hours;
        let idle_timeout_mins = settings.term_agent_idle_timeout_mins;

        // Skip entire scan if both limits are disabled.
        if max_runtime_hours <= 0.0 && idle_timeout_mins <= 0.0 {
            continue;
        }

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        for (block_id, ctrl) in get_all_controllers() {
            let status = ctrl.get_runtime_status();
            if status.shellprocstatus != STATUS_RUNNING {
                continue;
            }
            if !status.is_agent_pane {
                continue;
            }

            // ── Condition A: max runtime ──────────────────────────────────
            if max_runtime_hours > 0.0 {
                if let Some(spawn_ms) = status.spawn_ts_ms {
                    let elapsed_secs = ((now_ms - spawn_ms).max(0) as u64) / 1000;
                    let limit_secs = (max_runtime_hours * 3600.0) as u64;
                    if elapsed_secs >= limit_secs {
                        tracing::warn!(
                            block_id = %block_id,
                            elapsed_hours = elapsed_secs / 3600,
                            limit_hours = %max_runtime_hours,
                            "watchdog: agent pane exceeded max-runtime, stopping"
                        );
                        let _ = ctrl.stop(true, super::STATUS_DONE);
                        continue;
                    }
                }
            }

            // ── Condition B: idle output timeout ─────────────────────────
            if idle_timeout_mins > 0.0 {
                if let Some(shell_ctrl) = ctrl.as_any().downcast_ref::<super::shell::ShellController>() {
                    if let Some(idle_secs) = shell_ctrl.last_output_secs_ago() {
                        let limit_secs = (idle_timeout_mins * 60.0) as u64;
                        if idle_secs >= limit_secs {
                            tracing::warn!(
                                block_id = %block_id,
                                idle_mins = idle_secs / 60,
                                limit_mins = %idle_timeout_mins,
                                "watchdog: agent pane exceeded idle-output timeout, stopping"
                            );
                            let _ = ctrl.stop(true, super::STATUS_DONE);
                        }
                    }
                }
            }
        }
    }
}
