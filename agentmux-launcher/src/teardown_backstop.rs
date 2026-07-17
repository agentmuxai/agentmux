// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11 Phase 2 — the armed J0
//! teardown state machine.
//!
//! ```text
//! Disarmed ──(PoolDrained / OrphanInstance drift)──▶ Armed(t0)
//! Armed    ──(host exits — supervisor disarms)─────▶ Disarmed
//! Armed    ──(any WindowOpened event)──────────────▶ Disarmed
//! Armed    ──(t > t0+GRACE and ≥2 unanswered probes)▶ Teardown
//! ```
//!
//! Consumes Phase 1's `ui_liveness` telemetry: the *decision* to tear down
//! requires BOTH the grace period elapsing since arming AND the UI-thread
//! prober reporting ≥ N consecutive unanswered probes — a wedged UI thread
//! cannot answer, and transport failures never count as misses (the prober
//! retracts probes whose SEND failed, so silence here is genuinely
//! UI-thread evidence, not pipe evidence).
//!
//! Arm/disarm are driven from `ipc::server`'s post-reducer event hook
//! (validated events only — same layering as the Phase 1 transport-side
//! `ReportUiThreadAlive` intercept: this is process-supervision state about
//! the host, not domain state the reducer owns). The supervisor's select
//! loop polls `should_teardown` on a low-rate tick and, on `true`,
//! `TerminateJobObject(J0)`s the tree — the one deliberate exception to
//! "never kill what a saga can reconcile": zero user windows remain and the
//! UI thread is provably dead, so there is nothing left to reconcile.
//!
//! False-positive guards (spec §Phase 2), and where each lives:
//! - Startup: nothing arms until a drain/orphan event, which requires a
//!   registered host that already opened (and closed) a user window.
//! - Crash-restart gap: the supervisor disarms on every host exit; the
//!   machine can only re-arm from a fresh drain report by the NEW host.
//! - Crash-reproject: reproject re-opens windows within the grace; the
//!   mirror's `WindowOpened` event disarms.
//! - Probe transport failure: `ui_liveness::retract_probe` keeps failed
//!   sends out of the consecutive-miss count entirely.
//!
//! Like `ui_liveness`, all logic lives on the struct (unit-testable with an
//! injected clock); the module-level functions delegate to one
//! process-global instance.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Grace period between arming and the earliest possible teardown. An
/// order of magnitude above the slowest healthy quit observed
/// (multi-window with pool sweep: ~2-5s), far below "user annoyed".
pub const TEARDOWN_GRACE: Duration = Duration::from_secs(30);

/// Consecutive unanswered UI-thread probes required (on top of the grace)
/// before the host counts as wedged. At the Phase-1 probe rate (60s) this
/// bounds worst-case wedge→teardown latency to roughly GRACE + 2 probe
/// intervals — the spec's stated verification envelope.
pub const TEARDOWN_REQUIRED_MISSES: u32 = 2;

#[derive(Debug, Default)]
pub struct TeardownBackstop {
    /// `Some(t0)` = armed since `t0`. Re-arming while armed keeps the
    /// ORIGINAL `t0` — a second drain report means the host has been
    /// failing to exit since the first one, not that the clock restarts.
    armed_since: Option<Instant>,
}

impl TeardownBackstop {
    /// Arm the machine. Returns `true` when this call newly armed it
    /// (callers log the transition); `false` when it was already armed
    /// (the original arm time is kept).
    pub fn arm(&mut self, now: Instant) -> bool {
        if self.armed_since.is_some() {
            return false;
        }
        self.armed_since = Some(now);
        true
    }

    /// Disarm. Returns `true` when this call newly disarmed it.
    pub fn disarm(&mut self) -> bool {
        self.armed_since.take().is_some()
    }

    pub fn is_armed(&self) -> bool {
        self.armed_since.is_some()
    }

    /// The teardown decision: armed for longer than `grace`, AND the
    /// UI-thread prober has accumulated at least `required_misses`
    /// consecutive unanswered probes. Pure read — the caller executes.
    pub fn should_teardown(
        &self,
        now: Instant,
        grace: Duration,
        consecutive_misses: u32,
        required_misses: u32,
    ) -> bool {
        match self.armed_since {
            Some(t0) => {
                now.saturating_duration_since(t0) > grace && consecutive_misses >= required_misses
            }
            None => false,
        }
    }
}

fn cell() -> &'static Mutex<TeardownBackstop> {
    static S: OnceLock<Mutex<TeardownBackstop>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(TeardownBackstop::default()))
}

/// See [`TeardownBackstop::arm`]. Process-global instance.
pub fn arm() -> bool {
    cell().lock().unwrap().arm(Instant::now())
}

/// See [`TeardownBackstop::disarm`]. Process-global instance.
pub fn disarm() -> bool {
    cell().lock().unwrap().disarm()
}

/// See [`TeardownBackstop::should_teardown`]. Process-global instance,
/// evaluated against the spec constants and the live probe-miss count.
pub fn should_teardown(consecutive_misses: u32) -> bool {
    cell().lock().unwrap().should_teardown(
        Instant::now(),
        TEARDOWN_GRACE,
        consecutive_misses,
        TEARDOWN_REQUIRED_MISSES,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(base: Instant, secs: u64) -> Instant {
        base + Duration::from_secs(secs)
    }

    #[test]
    fn disarmed_never_tears_down() {
        let b = TeardownBackstop::default();
        let now = Instant::now();
        assert!(!b.should_teardown(now, TEARDOWN_GRACE, 99, TEARDOWN_REQUIRED_MISSES));
    }

    #[test]
    fn armed_within_grace_never_tears_down_even_with_misses() {
        let mut b = TeardownBackstop::default();
        let base = Instant::now();
        assert!(b.arm(base), "first arm is a transition");
        assert!(!b.should_teardown(t(base, 29), TEARDOWN_GRACE, 99, TEARDOWN_REQUIRED_MISSES));
    }

    #[test]
    fn armed_past_grace_without_misses_never_tears_down() {
        // A host answering probes is alive-but-slow — the host-side quit
        // watchdog owns that case, not this backstop.
        let mut b = TeardownBackstop::default();
        let base = Instant::now();
        b.arm(base);
        assert!(!b.should_teardown(t(base, 300), TEARDOWN_GRACE, 1, TEARDOWN_REQUIRED_MISSES));
    }

    #[test]
    fn armed_past_grace_with_required_misses_tears_down() {
        let mut b = TeardownBackstop::default();
        let base = Instant::now();
        b.arm(base);
        assert!(b.should_teardown(t(base, 31), TEARDOWN_GRACE, 2, TEARDOWN_REQUIRED_MISSES));
    }

    #[test]
    fn disarm_cancels_a_pending_teardown() {
        let mut b = TeardownBackstop::default();
        let base = Instant::now();
        b.arm(base);
        assert!(b.disarm(), "disarm of an armed machine is a transition");
        assert!(!b.disarm(), "second disarm is a no-op");
        assert!(!b.should_teardown(t(base, 300), TEARDOWN_GRACE, 99, TEARDOWN_REQUIRED_MISSES));
    }

    #[test]
    fn rearming_keeps_the_original_arm_time() {
        let mut b = TeardownBackstop::default();
        let base = Instant::now();
        b.arm(base);
        // A second arm 29s in must NOT restart the grace clock…
        assert!(!b.arm(t(base, 29)), "re-arm while armed is not a transition");
        // …so at t=31 the ORIGINAL t0 has aged past the 30s grace.
        assert!(b.should_teardown(t(base, 31), TEARDOWN_GRACE, 2, TEARDOWN_REQUIRED_MISSES));
    }
}
