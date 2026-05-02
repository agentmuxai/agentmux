// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase 2 — top-level window creation watchdog.
//
// Wakes at the in-flight creation's deadline (or every 5s when idle) and
// dispatches `TopLevelTimeoutTick` to the host reducer. The reducer either:
//   - Finds the in-flight slot empty or under-deadline → no-op; or
//   - Finds it past-deadline → evicts as `TimedOut`, archives to history,
//     auto-emits `StartNextTopLevelIfIdle` to advance the queue.
//
// The wedged renderer/browser is leaked (we can't safely tear it down — it's
// stuck inside CEF). Recoverable across restarts. Operator-visible via
// `--diag windows` (Phase 3).
//
// See `docs/specs/SPEC_HOST_WINDOW_CREATION_RUNNER_2026-05-02.md` §"Watchdog".

use std::sync::Arc;
use std::time::{Duration, Instant};

const IDLE_POLL: Duration = Duration::from_secs(5);
const SLOP: Duration = Duration::from_millis(100);

/// Spawn the watchdog tokio task. Lives for the lifetime of the host
/// process; tokio runtime shutdown cancels it.
pub fn spawn_watchdog(state: Arc<crate::state::AppState>) {
    tokio::spawn(async move {
        tracing::info!("[window-create-watchdog] started");
        loop {
            // Read the deadline of any current in-flight request, holding
            // the reducer mutex only briefly. Snapshot-and-drop.
            let next_wake_at = {
                let host = state.host_state.lock();
                host.in_flight_top_level_creation
                    .as_ref()
                    .map(|c| c.deadline)
            };

            match next_wake_at {
                Some(deadline) => {
                    let now = Instant::now();
                    let sleep_dur = deadline.saturating_duration_since(now) + SLOP;
                    tokio::time::sleep(sleep_dur).await;
                }
                None => {
                    // No in-flight request; poll periodically in case one
                    // starts while we're sleeping. Cheap.
                    tokio::time::sleep(IDLE_POLL).await;
                }
            }

            // Tick the reducer. If there's no in-flight or it's not past
            // deadline, this is a no-op. Otherwise the reducer evicts the
            // slot and auto-advances.
            let out = state.host_dispatch(crate::reducer::HostCommand::TopLevelTimeoutTick {
                now: Instant::now(),
            });

            // Visible alert when we actually evicted something.
            for ev in &out.events {
                if let crate::reducer::HostEvent::TopLevelCreationTimedOut {
                    creation_id,
                    label,
                    last_phase,
                    elapsed_ms,
                    ..
                } = ev
                {
                    tracing::error!(
                        creation_id = %creation_id,
                        label = %label,
                        last_phase = ?last_phase,
                        elapsed_ms = %elapsed_ms,
                        "[window-create-watchdog] creation timed out — wedged init evicted"
                    );
                }
            }
        }
    });
}
