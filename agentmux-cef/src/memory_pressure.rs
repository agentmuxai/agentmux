// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Host memory-pressure detection — the foundation of
//! `docs/specs/SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.md` (§5.A / §5.D).
//!
//! The memory heartbeat (`memory_heartbeat.rs`) already samples system
//! commit-free every 20 s. This module turns that stream of numbers into a
//! debounced **pressure level** (Normal / Warn / Critical) with hysteresis, and
//! publishes it so the proactive-shedding responses (warm-pool drain, CDP
//! renderer purge, the low-memory banner — later PRs) and any operator tooling
//! can read a single, stable signal instead of re-deriving thresholds.
//!
//! Originally detection + observability only: a structured `mem_pressure` log
//! line on every transition and a published `PRESSURE_LEVEL` atom, with no
//! action taken on renderers, windows, or the pool. The first proactive
//! shedding response now reads it: `commands/window_pool.rs`'s
//! `spawn_pool_window` / `spawn_pane_pool_window` refuse to grow the warm
//! pool while pressure is non-Normal (issue #1936 /
//! `docs/specs/SPEC_MEMORY_COMMIT_ATTRIBUTION_CORRECTION_2026_07_02.md`
//! §B.5(b)) — refill suppression only, not active eviction of an
//! already-warm pool window (that needs its own saga-aware design).
//!
//! The classifier is pure and unit-tested; hysteresis prevents banner/log flap
//! when commit oscillates around a threshold (the same anti-flap discipline the
//! renderer spec's resume gate uses).
//!
//! Thresholds are **ratio-based** (free/total), not absolute MB — a prior
//! version thresholded on absolute free MB, which on a large-commit-limit
//! machine never fired until far past what the status bar's own ratio-based
//! gauge (`SystemStats.tsx::commitColor`) already showed as red, so the
//! shedding responses above engaged too late. See issue #2218.

use std::sync::atomic::{AtomicU8, Ordering};

/// Free-ratio thresholds (free / total) a `PressureTracker` classifies
/// against. Originally module-level consts shared by a single commit
/// tracker; made a field (`SPEC_RAM_PAGEFILE_PRESSURE_SPLIT_2026_08_07` §3)
/// so a second, independent RAM tracker can share the same classifier logic
/// without being forced to share the same numbers — today both trackers use
/// `::default()` (identical to the original consts), but they can diverge
/// later once there's real data to tune against, without another rewrite.
///
/// Reagent-adjacent finding, `docs/retro/retro-commit-restart-reclaim-2026-07-16.md`
/// §5.2: the commit tracker used to threshold on absolute free MB
/// (1024 / 512), which on a large-commit-limit machine (60-80 GB seen live)
/// never fired until far past the point the status bar already shows red —
/// the frontend's gauge (`frontend/app/statusbar/SystemStats.tsx::commitColor`)
/// has always been ratio-based (>95% used / >85% used). The defaults below
/// are that same threshold, expressed as free-ratio (`1 - used/total`) so the
/// two pipelines agree: used > 0.95 <=> free/total < 0.05, used > 0.85 <=>
/// free/total < 0.15.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PressureThresholds {
    pub warn_enter_free_ratio: f64,
    pub critical_enter_free_ratio: f64,
    /// Margin, in free-ratio points: to *leave* a pressure band, free ratio
    /// must recover this far past the enter threshold. Stops flap when the
    /// reading hovers at a boundary — same role as the previous fixed-MB
    /// `HYSTERESIS_MB`, just expressed in ratio terms now that the
    /// thresholds themselves are ratios.
    pub hysteresis_ratio: f64,
}

impl Default for PressureThresholds {
    fn default() -> Self {
        Self {
            warn_enter_free_ratio: 0.15,
            critical_enter_free_ratio: 0.05,
            hysteresis_ratio: 0.03,
        }
    }
}

/// Debounced system memory-pressure level. Ordered Normal < Warn < Critical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PressureLevel {
    /// Ample commit headroom.
    Normal = 0,
    /// Commit getting tight (< `WARN_ENTER_FREE_RATIO`) — proactive shedding territory.
    Warn = 1,
    /// Commit critically low (< `CRITICAL_ENTER_FREE_RATIO`) — an OOM is imminent.
    Critical = 2,
}

impl PressureLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            PressureLevel::Normal => "normal",
            PressureLevel::Warn => "warn",
            PressureLevel::Critical => "critical",
        }
    }
    fn from_u8(v: u8) -> PressureLevel {
        match v {
            1 => PressureLevel::Warn,
            2 => PressureLevel::Critical,
            _ => PressureLevel::Normal,
        }
    }
}

/// Latest published pressure level, readable lock-free by future shedding
/// responses + tooling. `Normal` until the first sample.
///
/// **Commit/page-file pressure only.** This atomic predates
/// `SPEC_RAM_PAGEFILE_PRESSURE_SPLIT_2026_08_07`'s second (RAM) tracker;
/// every existing reader (`commands/window_pool.rs`, `commands/pane_pool.rs`)
/// gates pane/window-pool shedding on it, which must keep tracking commit
/// pressure specifically (§6: RAM-only pressure with healthy commit isn't a
/// crash risk, so it must not drive shedding). Only `PressureTracker::observe`
/// (used by the commit tracker) publishes here — the RAM tracker uses
/// `observe_local`, which deliberately does not, so it can never clobber this
/// with a RAM-only transition.
static PRESSURE_LEVEL: AtomicU8 = AtomicU8::new(PressureLevel::Normal as u8);

/// The current debounced **commit/page-file** pressure level (last published
/// by a tracker's `observe()`, not `observe_local()`). Read by proactive-
/// shedding responses — currently the warm-pool refill guard
/// (`commands/window_pool.rs::spawn_pool_window` / `spawn_pane_pool_window`)
/// — and available for future CDP purge / banner consumers.
pub fn current_level() -> PressureLevel {
    PressureLevel::from_u8(PRESSURE_LEVEL.load(Ordering::Relaxed))
}

/// Debounced pressure classifier. One instance per tracked metric lives in
/// the heartbeat thread (today: one for commit/page-file, one for physical
/// RAM — `SPEC_RAM_PAGEFILE_PRESSURE_SPLIT_2026_08_07` §3); `observe()` is
/// fed each sample and returns `Some(new)` only on a level change (so
/// callers log/act once per transition, not every tick).
#[derive(Debug, Clone, Copy)]
pub struct PressureTracker {
    level: PressureLevel,
    thresholds: PressureThresholds,
}

impl Default for PressureTracker {
    fn default() -> Self {
        Self { level: PressureLevel::Normal, thresholds: PressureThresholds::default() }
    }
}

impl PressureTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct with non-default thresholds. Not used yet — every tracker
    /// today starts on `PressureThresholds::default()` (§3's "start with the
    /// same ratios" call) — but kept available for the RAM tracker to diverge
    /// from the commit tracker once there's real data to tune against.
    #[allow(dead_code)]
    pub fn with_thresholds(thresholds: PressureThresholds) -> Self {
        Self { level: PressureLevel::Normal, thresholds }
    }

    #[allow(dead_code)] // read by tests + future shedding responses
    pub fn level(&self) -> PressureLevel {
        self.level
    }

    /// Feed a free/total reading (MB). Returns `Some(level)` iff the
    /// debounced level changed, and publishes the new level to the shared
    /// `PRESSURE_LEVEL` atomic that `current_level()` / the pane/window-pool
    /// shedding responses read. `total_mb == 0` (not yet sampled) is treated
    /// as "unknown — stay Normal" rather than dividing by zero.
    ///
    /// Use this for the commit/page-file tracker only. A second tracker
    /// (e.g. the RAM tracker) must use `observe_local` instead — publishing
    /// here from more than one tracker makes `current_level()` ambiguous
    /// (whichever tracker transitioned most recently wins), silently
    /// defeating the shedding response the commit signal exists to drive.
    pub fn observe(&mut self, free_mb: u64, total_mb: u64) -> Option<PressureLevel> {
        self.observe_impl(free_mb, total_mb, true)
    }

    /// Like `observe`, but does not publish to the shared `PRESSURE_LEVEL`
    /// atomic — only this tracker instance's own `level()` changes. Use for
    /// any tracker whose level isn't what `current_level()`'s callers mean;
    /// today that's the RAM tracker (`SPEC_RAM_PAGEFILE_PRESSURE_SPLIT_2026_08_07`
    /// §6: RAM pressure alone doesn't drive pane-pool shedding, so it must
    /// never overwrite the commit tracker's published level).
    pub fn observe_local(&mut self, free_mb: u64, total_mb: u64) -> Option<PressureLevel> {
        self.observe_impl(free_mb, total_mb, false)
    }

    fn observe_impl(&mut self, free_mb: u64, total_mb: u64, publish: bool) -> Option<PressureLevel> {
        let next = classify(free_mb, total_mb, self.level, &self.thresholds);
        if next != self.level {
            self.level = next;
            if publish {
                PRESSURE_LEVEL.store(next as u8, Ordering::Relaxed);
            }
            Some(next)
        } else {
            None
        }
    }
}

/// Pure classifier with hysteresis: the *enter* thresholds are strict, but
/// *leaving* a band requires recovering `hysteresis_ratio` past the enter
/// threshold, so a reading parked at a boundary doesn't oscillate.
fn classify(
    free_mb: u64,
    total_mb: u64,
    current: PressureLevel,
    t: &PressureThresholds,
) -> PressureLevel {
    if total_mb == 0 {
        return PressureLevel::Normal;
    }
    let free_ratio = free_mb as f64 / total_mb as f64;
    match current {
        PressureLevel::Normal => {
            if free_ratio < t.critical_enter_free_ratio {
                PressureLevel::Critical
            } else if free_ratio < t.warn_enter_free_ratio {
                PressureLevel::Warn
            } else {
                PressureLevel::Normal
            }
        }
        PressureLevel::Warn => {
            if free_ratio < t.critical_enter_free_ratio {
                PressureLevel::Critical
            } else if free_ratio >= t.warn_enter_free_ratio + t.hysteresis_ratio {
                PressureLevel::Normal
            } else {
                PressureLevel::Warn
            }
        }
        PressureLevel::Critical => {
            if free_ratio >= t.warn_enter_free_ratio + t.hysteresis_ratio {
                PressureLevel::Normal
            } else if free_ratio >= t.critical_enter_free_ratio + t.hysteresis_ratio {
                PressureLevel::Warn
            } else {
                PressureLevel::Critical
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes every test in this module. `observe()` writes to the
    /// process-global `PRESSURE_LEVEL` atomic (by design — that's what makes
    /// it readable by `current_level()`), so parallel test threads racing
    /// unlocked writes/reads against the same static would be flaky —
    /// same rationale as `agentmux_common::TEST_ENV_LOCK`.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Acquire `TEST_LOCK`, tolerating poisoning. A poisoned lock here only
    /// ever means some *other* test in this module panicked while holding
    /// it — that failure is already reported on its own test thread; letting
    /// poison propagate here would additionally fail every test that runs
    /// after it with an unrelated `PoisonError`, masking which test actually
    /// broke.
    fn lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Fixed total used across every test below so the free-MB values read
    /// naturally as ratios (total = 10_000 MB, so e.g. free=1500 is exactly
    /// the 0.15 WARN_ENTER_FREE_RATIO boundary).
    const TOTAL: u64 = 10_000;

    #[test]
    fn starts_normal_and_classifies_each_band_from_normal() {
        let _guard = lock();
        let mut t = PressureTracker::new();
        assert_eq!(t.level(), PressureLevel::Normal);
        // Ample headroom (ratio 0.8) → no transition.
        assert_eq!(t.observe(8000, TOTAL), None);
        // Drop below WARN ratio (0.15) → Warn.
        assert_eq!(t.observe(1000, TOTAL), Some(PressureLevel::Warn));
        // Drop below CRITICAL ratio (0.05) → Critical.
        assert_eq!(t.observe(400, TOTAL), Some(PressureLevel::Critical));
    }

    #[test]
    fn only_emits_on_change() {
        let _guard = lock();
        let mut t = PressureTracker::new();
        assert_eq!(t.observe(1000, TOTAL), Some(PressureLevel::Warn));
        // Still in the Warn band → no repeat emission.
        assert_eq!(t.observe(900, TOTAL), None);
        assert_eq!(t.observe(1100, TOTAL), None); // below the warn-exit hysteresis ceiling
    }

    #[test]
    fn hysteresis_prevents_flap_at_the_warn_boundary() {
        let _guard = lock();
        let mut t = PressureTracker::new();
        assert_eq!(t.observe(1499, TOTAL), Some(PressureLevel::Warn)); // enter Warn (ratio <0.15)
        // Hovering just above the enter threshold but below enter+hysteresis
        // (ratio 0.15..0.18, free 1500..1799) must NOT leave Warn.
        assert_eq!(t.observe(1500, TOTAL), None);
        assert_eq!(t.observe(1799, TOTAL), None);
        // Only at enter+hysteresis (ratio 0.18, free 1800) does it return to Normal.
        assert_eq!(t.observe(1800, TOTAL), Some(PressureLevel::Normal));
    }

    #[test]
    fn critical_steps_down_through_warn_with_hysteresis() {
        let _guard = lock();
        let mut t = PressureTracker::new();
        let _ = t.observe(400, TOTAL); // Normal → Critical (ratio 0.04)
        assert_eq!(t.level(), PressureLevel::Critical);
        // Recover past CRITICAL_ENTER+HYST (ratio 0.08, free 800) but not
        // WARN_ENTER+HYST (ratio 0.18, free 1800) → Warn.
        assert_eq!(t.observe(800, TOTAL), Some(PressureLevel::Warn));
        // Recover fully → Normal.
        assert_eq!(t.observe(1900, TOTAL), Some(PressureLevel::Normal));
    }

    #[test]
    fn normal_can_jump_straight_to_critical_and_back() {
        let _guard = lock();
        let mut t = PressureTracker::new();
        assert_eq!(t.observe(100, TOTAL), Some(PressureLevel::Critical));
        assert_eq!(t.observe(5000, TOTAL), Some(PressureLevel::Normal));
    }

    #[test]
    fn ratio_thresholds_match_frontend_commit_color() {
        let _guard = lock();
        // frontend/app/statusbar/SystemStats.tsx::commitColor: used/total > 0.95
        // -> red, > 0.85 -> amber. Expressed as free-ratio here: < 0.05 -> Critical,
        // < 0.15 -> Warn. Locks the two independently-computed pipelines together
        // so they can't silently drift apart again (the whole point of this fix).
        let mut t = PressureTracker::new();
        // Free ratio 0.1499 (< 0.15, i.e. used > 0.85) -> Warn.
        assert_eq!(t.observe(1499, TOTAL), Some(PressureLevel::Warn));
        // Free ratio 0.0499 (< 0.05, i.e. used > 0.95) -> Critical.
        assert_eq!(t.observe(499, TOTAL), Some(PressureLevel::Critical));

        // Exactly at the boundary (free ratio == 0.05, used == 0.95) must NOT
        // be Critical -- commitColor uses a strict `>`, matched here by a
        // strict `<` on the free-ratio classify condition.
        let mut t2 = PressureTracker::new();
        assert_eq!(t2.observe(500, TOTAL), Some(PressureLevel::Warn)); // ratio 0.05 -> not Critical, still < 0.15 -> Warn
    }

    #[test]
    fn zero_total_does_not_panic_or_misclassify() {
        let _guard = lock();
        let mut t = PressureTracker::new();
        // Heartbeat hasn't sampled total yet -- must degrade to Normal, not
        // divide by zero / panic.
        assert_eq!(t.observe(0, 0), None);
        assert_eq!(t.level(), PressureLevel::Normal);
        assert_eq!(t.observe(100_000, 0), None);
        assert_eq!(t.level(), PressureLevel::Normal);
    }

    #[test]
    fn observe_local_tracks_level_without_publishing_to_the_shared_atomic() {
        let _guard = lock();
        // `PRESSURE_LEVEL` is a process-global static that outlives any one
        // test — other tests in this module intentionally leave it in Warn
        // or Critical when they finish, and test execution order is not
        // guaranteed. So establish a KNOWN baseline via a real, guaranteed
        // transition (Critical, since a fresh tracker always starts at
        // Normal and 100/TOTAL is always Critical regardless of what any
        // earlier test left the global at) rather than assuming it starts
        // at Normal — that assumption is exactly what made this test flaky
        // depending on run order before this fix.
        let mut commit = PressureTracker::new();
        assert_eq!(commit.observe(100, TOTAL), Some(PressureLevel::Critical));
        assert_eq!(current_level(), PressureLevel::Critical);
        // Now step it back down to Normal — another guaranteed transition —
        // as the clean baseline the rest of this test builds on.
        assert_eq!(commit.observe(8000, TOTAL), Some(PressureLevel::Normal));
        assert_eq!(current_level(), PressureLevel::Normal);

        // A second, independent tracker (standing in for the RAM tracker)
        // transitions all the way to Critical via observe_local...
        let mut ram = PressureTracker::new();
        assert_eq!(ram.observe_local(100, TOTAL), Some(PressureLevel::Critical));
        assert_eq!(ram.level(), PressureLevel::Critical); // its own state DID change...

        // ...but the shared atomic every pane/window-pool shedding check
        // reads must be untouched by that — this is the exact bug reagent
        // caught: two trackers publishing to one atomic makes current_level()
        // ambiguous. observe_local must never clobber it.
        assert_eq!(current_level(), PressureLevel::Normal);

        // The real commit tracker transitioning still publishes normally,
        // proving observe_local's non-publishing is specific to that call,
        // not a side effect that broke publishing generally.
        assert_eq!(commit.observe(100, TOTAL), Some(PressureLevel::Critical));
        assert_eq!(current_level(), PressureLevel::Critical);
    }
}
