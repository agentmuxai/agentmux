// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Keeps the cloud subscriber's loop alive. The loop ends only by a panic, and
//! it can stall in a call that never returns (on 2026-11-11 two hosts stopped
//! receiving cloud messages for hours while their status said connected). The
//! loop stamps [`beat`] every time round; a stamp that stops for [`STALL_AFTER`]
//! means it is stuck, and either way a new loop is started.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use agentmux_common::time::now_ms;

use crate::muxbus::delivery_status;

/// When the loop last made progress (ms since the epoch), stamped by [`beat`].
pub(super) static LOOP_BEAT: AtomicI64 = AtomicI64::new(0);

/// How often the supervisor looks at the loop.
pub(super) const WATCH_EVERY: Duration = Duration::from_secs(30);
/// How long the loop may go without a stamp before it counts as stalled. Every
/// wait inside it is bounded by a minute or two, so a healthy loop stamps far
/// more often than this.
pub(super) const STALL_AFTER: Duration = Duration::from_secs(10 * 60);
/// The pause before a replacement loop starts.
const RESTART_DELAY: Duration = Duration::from_secs(5);

/// The loop is making progress.
pub(super) fn beat() {
    LOOP_BEAT.store(now_ms(), Ordering::Relaxed);
}

/// Runs the loop `start` makes, and starts another when it ends, panics, or
/// stops stamping `beat`. Never returns.
pub(super) async fn supervise<F>(start: F, beat: &AtomicI64, watch_every: Duration, stall_after: Duration, restart_delay: Duration)
where
    F: Fn() -> tokio::task::JoinHandle<()>,
{
    loop {
        beat.store(now_ms(), Ordering::Relaxed);
        let mut task = start();
        let why = loop {
            tokio::select! {
                done = &mut task => {
                    break match done {
                        Ok(()) => "ended".to_string(),
                        Err(e) if e.is_panic() => "panicked".to_string(),
                        Err(_) => "was cancelled".to_string(),
                    };
                }
                _ = tokio::time::sleep(watch_every) => {
                    let quiet_ms = now_ms() - beat.load(Ordering::Relaxed);
                    if quiet_ms > stall_after.as_millis() as i64 {
                        task.abort();
                        let _ = (&mut task).await;
                        break format!("stalled for {}s", quiet_ms / 1000);
                    }
                }
            }
        };
        tracing::error!("cloud_subscriber: its loop {why}; starting a new one");
        // Not "connected": whatever the dead loop last said no longer holds.
        delivery_status::set_link(delivery_status::Link::Unreachable(format!("the cloud connection's task {why}")));
        tokio::time::sleep(restart_delay).await;
    }
}

/// [`supervise`] with the production timings.
pub(super) async fn supervise_loop<F>(start: F)
where
    F: Fn() -> tokio::task::JoinHandle<()>,
{
    supervise(start, &LOOP_BEAT, WATCH_EVERY, STALL_AFTER, RESTART_DELAY).await
}
