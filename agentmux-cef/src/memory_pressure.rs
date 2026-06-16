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
//! This PR is **detection + observability only**: a structured `mem_pressure`
//! log line on every transition and a published `PRESSURE_LEVEL` atom. It
//! deliberately takes **no** action on renderers, windows, or the pool — those
//! responses need runtime verification and land in their own slices.
//!
//! The classifier is pure and unit-tested; hysteresis prevents banner/log flap
//! when commit oscillates around a threshold (the same anti-flap discipline the
//! renderer spec's resume gate uses).

use std::sync::atomic::{AtomicU8, Ordering};

/// Commit-free **enter** thresholds, in MB. Aligned with the spec's
/// `WARN_FLOOR` (1 GB) and `RESUME_FLOOR` (512 MB) so the host's pressure view
/// and the launcher's relaunch gate agree on what "low" means (SPEC §6).
const WARN_ENTER_MB: u64 = 1024;
const CRITICAL_ENTER_MB: u64 = 512;
/// Hysteresis margin, in MB: to *leave* a pressure band, commit-free must
/// recover this far past the enter threshold. Stops flap when commit hovers at
/// a boundary.
const HYSTERESIS_MB: u64 = 256;

/// Debounced system memory-pressure level. Ordered Normal < Warn < Critical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PressureLevel {
    /// Ample commit headroom.
    Normal = 0,
    /// Commit getting tight (< `WARN_ENTER_MB`) — proactive shedding territory.
    Warn = 1,
    /// Commit critically low (< `CRITICAL_ENTER_MB`) — an OOM is imminent.
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
static PRESSURE_LEVEL: AtomicU8 = AtomicU8::new(PressureLevel::Normal as u8);

/// The current debounced pressure level (last published by the tracker). The
/// proactive-shedding responses (warm-pool drain, CDP purge, banner — later
/// PRs) read this; unused in this detection-only slice.
#[allow(dead_code)]
pub fn current_level() -> PressureLevel {
    PressureLevel::from_u8(PRESSURE_LEVEL.load(Ordering::Relaxed))
}

/// Debounced pressure classifier. One instance lives in the heartbeat thread;
/// `observe()` is fed each commit-free sample and returns `Some(new)` only on a
/// level change (so callers log/act once per transition, not every tick).
#[derive(Debug, Clone, Copy)]
pub struct PressureTracker {
    level: PressureLevel,
}

impl Default for PressureTracker {
    fn default() -> Self {
        Self { level: PressureLevel::Normal }
    }
}

impl PressureTracker {
    pub fn new() -> Self {
        Self::default()
    }

    #[allow(dead_code)] // read by tests + future shedding responses
    pub fn level(&self) -> PressureLevel {
        self.level
    }

    /// Feed a commit-free reading (MB). Returns `Some(level)` iff the debounced
    /// level changed, and publishes the new level to `PRESSURE_LEVEL`.
    pub fn observe(&mut self, commit_free_mb: u64) -> Option<PressureLevel> {
        let next = classify(commit_free_mb, self.level);
        if next != self.level {
            self.level = next;
            PRESSURE_LEVEL.store(next as u8, Ordering::Relaxed);
            Some(next)
        } else {
            None
        }
    }
}

/// Pure classifier with hysteresis: the *enter* thresholds are strict, but
/// *leaving* a band requires recovering `HYSTERESIS_MB` past the enter
/// threshold, so a reading parked at a boundary doesn't oscillate.
fn classify(free_mb: u64, current: PressureLevel) -> PressureLevel {
    match current {
        PressureLevel::Normal => {
            if free_mb < CRITICAL_ENTER_MB {
                PressureLevel::Critical
            } else if free_mb < WARN_ENTER_MB {
                PressureLevel::Warn
            } else {
                PressureLevel::Normal
            }
        }
        PressureLevel::Warn => {
            if free_mb < CRITICAL_ENTER_MB {
                PressureLevel::Critical
            } else if free_mb >= WARN_ENTER_MB + HYSTERESIS_MB {
                PressureLevel::Normal
            } else {
                PressureLevel::Warn
            }
        }
        PressureLevel::Critical => {
            if free_mb >= WARN_ENTER_MB + HYSTERESIS_MB {
                PressureLevel::Normal
            } else if free_mb >= CRITICAL_ENTER_MB + HYSTERESIS_MB {
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

    #[test]
    fn starts_normal_and_classifies_each_band_from_normal() {
        let mut t = PressureTracker::new();
        assert_eq!(t.level(), PressureLevel::Normal);
        // Ample headroom → no transition.
        assert_eq!(t.observe(8192), None);
        // Drop below WARN → Warn.
        assert_eq!(t.observe(1000), Some(PressureLevel::Warn));
        // Drop below CRITICAL → Critical.
        assert_eq!(t.observe(400), Some(PressureLevel::Critical));
    }

    #[test]
    fn only_emits_on_change() {
        let mut t = PressureTracker::new();
        assert_eq!(t.observe(1000), Some(PressureLevel::Warn));
        // Still in the Warn band → no repeat emission.
        assert_eq!(t.observe(900), None);
        assert_eq!(t.observe(1100), None); // below the warn-exit hysteresis ceiling
    }

    #[test]
    fn hysteresis_prevents_flap_at_the_warn_boundary() {
        let mut t = PressureTracker::new();
        assert_eq!(t.observe(1000), Some(PressureLevel::Warn)); // enter Warn (<1024)
        // Hovering just above the enter threshold but below enter+hysteresis
        // (1024..1280) must NOT leave Warn.
        assert_eq!(t.observe(1024), None);
        assert_eq!(t.observe(1279), None);
        // Only at enter+hysteresis (1280) does it return to Normal.
        assert_eq!(t.observe(1280), Some(PressureLevel::Normal));
    }

    #[test]
    fn critical_steps_down_through_warn_with_hysteresis() {
        let mut t = PressureTracker::new();
        let _ = t.observe(400); // Normal → Critical
        assert_eq!(t.level(), PressureLevel::Critical);
        // Recover past CRITICAL_ENTER+HYST (768) but not WARN_ENTER+HYST (1280)
        // → Warn.
        assert_eq!(t.observe(800), Some(PressureLevel::Warn));
        // Recover fully → Normal.
        assert_eq!(t.observe(1300), Some(PressureLevel::Normal));
    }

    #[test]
    fn normal_can_jump_straight_to_critical_and_back() {
        let mut t = PressureTracker::new();
        assert_eq!(t.observe(100), Some(PressureLevel::Critical));
        assert_eq!(t.observe(5000), Some(PressureLevel::Normal));
    }
}
