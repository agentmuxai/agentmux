// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Per-step timing for srv's boot path, logged once as a single summary line
//! when `AGENTMUXSRV-ESTART` is emitted.
//!
//! WHY. `docs/specs/SPEC_FAST_STARTUP_UPGRADE_OWNS_MIGRATIONS_AND_UPDATES_2026_09_15.md`
//! §3 inventories the boot path and finds several passes that run on *every*
//! boot and have never been individually timed — the registry `session_id`
//! catch-up backfill, the transcript backfill, the snapshot-source gap
//! repair, and the agent auto-seed. The splash's `backend` stage reports one
//! number covering all of them together, so there is currently no way to say
//! which of them (if any) is worth moving off the critical path. §6 of that
//! spec asks for measurement before relocation; this is the instrument.
//!
//! Deliberately `tracing`, not the `srv_stderr` splash protocol
//! (`agentmux_common::srv_stderr`) the per-migration rows use: the direction
//! of travel is *fewer* splash rows, not more
//! (`docs/reports/REPORT_SPLASH_MIGRATION_ROW_OVERFLOW_AND_SUMMARY_2026_09_15.md`),
//! and these numbers are for whoever is profiling a boot, not for the user
//! watching one. Read them with `muxlog srv grep boot_timing`.
//!
//! The global recorder is a process-lifetime singleton because the steps it
//! wraps are scattered through `bootstrap.rs`'s long boot functions; threading
//! a `&mut BootTimings` through all of them would be a much larger diff than
//! the measurement is worth. [`BootTimings`] itself is a plain value type with
//! no global state, which is what the tests below exercise.

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// One boot's worth of step timings.
pub struct BootTimings {
    started: Instant,
    steps: Vec<(&'static str, u64)>,
}

impl BootTimings {
    pub fn new() -> Self {
        Self { started: Instant::now(), steps: Vec::new() }
    }

    /// Run `f`, recording how long it took under `step`.
    pub fn time<T>(&mut self, step: &'static str, f: impl FnOnce() -> T) -> T {
        let t = Instant::now();
        let out = f();
        self.steps.push((step, t.elapsed().as_millis() as u64));
        out
    }

    pub fn steps(&self) -> &[(&'static str, u64)] {
        &self.steps
    }

    /// Sum of every recorded step. Steps may nest (a wrapped pass calling
    /// another wrapped pass), in which case this double-counts the inner one —
    /// which is why the summary reports it alongside the wall-clock total
    /// rather than deriving one from the other.
    pub fn accounted_ms(&self) -> u64 {
        self.steps.iter().map(|(_, ms)| ms).sum()
    }

    /// Wall-clock since [`BootTimings::new`] — i.e. since srv started timing,
    /// not since process start.
    pub fn total_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// `total=412ms accounted=350ms | open_stores=210ms registry_session_backfill=88ms ...`
    ///
    /// Steps are listed slowest-first: the point of reading this line is
    /// finding what to move off the boot path, and that is always the top of
    /// the list.
    pub fn summary(&self) -> String {
        let mut sorted: Vec<_> = self.steps.clone();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));
        let steps = sorted
            .iter()
            .map(|(step, ms)| format!("{step}={ms}ms"))
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "total={}ms accounted={}ms | {}",
            self.total_ms(),
            self.accounted_ms(),
            steps
        )
    }
}

impl Default for BootTimings {
    fn default() -> Self {
        Self::new()
    }
}

// ── Process-global recorder ──────────────────────────────────────────────────

fn recorder() -> &'static Mutex<Option<BootTimings>> {
    static RECORDER: OnceLock<Mutex<Option<BootTimings>>> = OnceLock::new();
    RECORDER.get_or_init(|| Mutex::new(None))
}

/// Start timing this boot. Called once from `main`, after logging is up.
pub fn init() {
    *recorder().lock().unwrap() = Some(BootTimings::new());
}

/// Run `f`, recording its duration under `step` if [`init`] has been called.
///
/// Never holds the recorder lock across `f`, so a timed step may itself
/// contain timed steps without deadlocking. Uninitialized (unit tests,
/// the `migrate` subcommand, anything that never calls [`init`]) it is a
/// transparent pass-through.
pub fn time<T>(step: &'static str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let out = f();
    let ms = t.elapsed().as_millis() as u64;
    if let Ok(mut guard) = recorder().lock() {
        if let Some(timings) = guard.as_mut() {
            timings.steps.push((step, ms));
        }
    }
    out
}

/// `time`, for an async step. Same no-lock-across-the-work property.
pub async fn time_async<T>(step: &'static str, fut: impl std::future::Future<Output = T>) -> T {
    let t = Instant::now();
    let out = fut.await;
    let ms = t.elapsed().as_millis() as u64;
    if let Ok(mut guard) = recorder().lock() {
        if let Some(timings) = guard.as_mut() {
            timings.steps.push((step, ms));
        }
    }
    out
}

/// Log the one summary line. Called once, where the boot path ends.
pub fn log_summary() {
    let Ok(guard) = recorder().lock() else { return };
    let Some(timings) = guard.as_ref() else { return };
    // The key is repeated in the MESSAGE, not just the target, on purpose:
    // `muxlog`'s grep matches `fields.message` only
    // (`backend/shellintegration/muxlog.mjs:157`), so the documented
    // `muxlog srv grep boot_timing` would filter this very line out if the
    // key lived in the target alone (codex P2 on PR #3267). The target is
    // kept as well, so `muxlog srv --target boot_timing` works too.
    tracing::info!(target: "boot_timing", "boot_timing: {}", timings.summary());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn time_returns_the_inner_value() {
        let mut t = BootTimings::new();
        assert_eq!(t.time("step", || 42), 42);
    }

    #[test]
    fn steps_are_recorded_in_call_order() {
        let mut t = BootTimings::new();
        t.time("first", || {});
        t.time("second", || {});
        let names: Vec<_> = t.steps().iter().map(|(n, _)| *n).collect();
        assert_eq!(names, vec!["first", "second"]);
    }

    #[test]
    fn a_steps_duration_reflects_real_elapsed_time() {
        let mut t = BootTimings::new();
        t.time("slow", || sleep(Duration::from_millis(12)));
        let (_, ms) = t.steps()[0];
        assert!(ms >= 10, "expected >=10ms for a 12ms sleep, got {ms}ms");
    }

    #[test]
    fn summary_lists_every_step_slowest_first() {
        let mut t = BootTimings::new();
        t.steps.push(("fast", 2));
        t.steps.push(("slowest", 300));
        t.steps.push(("middle", 40));
        let s = t.summary();
        let slowest = s.find("slowest=300ms").expect("slowest step must appear");
        let middle = s.find("middle=40ms").expect("middle step must appear");
        let fast = s.find("fast=2ms").expect("fast step must appear");
        assert!(slowest < middle && middle < fast, "steps must be ordered slowest-first: {s}");
    }

    #[test]
    fn summary_reports_total_and_accounted_separately() {
        // They are not derivable from each other: `accounted` double-counts
        // nested steps, and `total` includes boot-path time no step wraps.
        let mut t = BootTimings::new();
        t.steps.push(("a", 10));
        t.steps.push(("b", 20));
        let s = t.summary();
        assert!(s.contains("accounted=30ms"), "{s}");
        assert!(s.contains("total="), "{s}");
    }

    #[test]
    fn global_time_runs_the_closure_even_when_uninitialized() {
        // Every unit test in this crate runs without `init()`; a pass-through
        // that silently skipped the work would be catastrophic.
        let mut ran = false;
        time("never-initialized", || ran = true);
        assert!(ran);
    }

    #[test]
    fn nested_global_timing_does_not_deadlock() {
        // The lock must not be held across the closure — `open_stores` wraps
        // passes that are themselves wrapped.
        let out = time("outer", || time("inner", || 7));
        assert_eq!(out, 7);
    }
}
