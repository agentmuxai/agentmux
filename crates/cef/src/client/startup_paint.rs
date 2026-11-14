// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The restart splash's "Window paint" row, reported once per host process.
//!
//! Every arm of a top-level window's paint gate used to report a new stage
//! begin, so a main page that kept failing to load (a dev frontend server that
//! was down) added one row per load retry, and the splash showed a growing
//! list of counters (operator screenshot, 2026-11-10). One begin per process
//! is all the splash needs; later arms and retries are the same wait.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

const STAGE: &str = "paint";

static BEGAN_AT: OnceLock<Instant> = OnceLock::new();
/// The first window's paint gate resolved, so the splash is being dismissed: the row is final.
static FINISHED: AtomicBool = AtomicBool::new(false);

/// Report the "paint" stage begin the first time a window arms its paint
/// gate in this process, and never again.
pub(super) fn report_begin_once() {
    if BEGAN_AT.set(Instant::now()).is_ok() {
        crate::launcher_ipc::report_startup_stage_begin(STAGE, "Window paint");
    }
}

/// A paint gate resolved: the splash is on its way out, so the row is final.
pub(super) fn mark_finished() {
    FINISHED.store(true, Ordering::Relaxed);
}

/// How long the "Window paint" row says the wait took, when the first paint
/// gate resolves: the whole wait since the first gate armed in this process,
/// not just since the latest arm. A page that recovers after a minute of load
/// retries re-arms the gate on each try, and the latest arm alone is a few
/// milliseconds; the rest would land in the splash's "other" time.
pub(super) fn final_elapsed_ms(armed_ms: u64) -> u64 {
    let since_begin = BEGAN_AT.get().map(|began| began.elapsed().as_millis() as u64);
    pick_final_elapsed(since_begin, FINISHED.load(Ordering::Relaxed), armed_ms)
}

/// The whole wait while the row is still open; after it was finalized (a later
/// window's gate resolving in a long-running process) that gate's own time.
fn pick_final_elapsed(since_begin_ms: Option<u64>, finished: bool, armed_ms: u64) -> u64 {
    match since_begin_ms {
        Some(ms) if !finished => ms.max(armed_ms),
        _ => armed_ms,
    }
}

/// The main page failed to load and will be retried: say why on the paint row
/// instead of leaving it counting up. A later successful reveal ends the row
/// again, as ok. Does nothing before the first paint gate was armed, after a
/// gate resolved, or for a window that carries its own label in its URL (a
/// tear-off, floating or pooled window is not the startup window).
pub(super) fn report_load_retry(failed_url: &str, error_text: &str) {
    let Some(began) = BEGAN_AT.get() else { return };
    if FINISHED.load(Ordering::Relaxed) || failed_url.contains("windowLabel=") {
        return;
    }
    crate::launcher_ipc::report_startup_stage_end(STAGE, began.elapsed().as_millis() as u64, "warn", Some(retry_detail(error_text)));
}

fn retry_detail(error_text: &str) -> String {
    let why = error_text.trim();
    if why.is_empty() {
        "Can't reach the app page, retrying".to_string()
    } else {
        format!("Can't reach the app page: {why}, retrying")
    }
}

#[cfg(test)]
mod tests {
    use super::{pick_final_elapsed, retry_detail};

    #[test]
    fn the_retry_detail_names_the_error() {
        assert_eq!(retry_detail("ERR_CONNECTION_REFUSED"), "Can't reach the app page: ERR_CONNECTION_REFUSED, retrying");
        assert_eq!(retry_detail("  "), "Can't reach the app page, retrying");
    }

    #[test]
    fn the_final_duration_covers_the_whole_wait() {
        // Retries re-armed the gate 40 ms ago, 61 s after the first arm.
        assert_eq!(pick_final_elapsed(Some(61_000), false, 40), 61_000);
        // No first arm recorded: that gate's own time.
        assert_eq!(pick_final_elapsed(None, false, 40), 40);
        // The row was already final: a later window's gate keeps its own time.
        assert_eq!(pick_final_elapsed(Some(61_000), true, 40), 40);
        // Never shorter than the arm it ends.
        assert_eq!(pick_final_elapsed(Some(10), false, 40), 40);
    }
}
