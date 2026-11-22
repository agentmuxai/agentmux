// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! How often a frame may be made. A screen that does not change makes none; one that does is capped at a steady rate, except just after
//! the person typed or clicked: then the next pictures come at once and quickly for a short while, so the typing is seen as it happens.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// How long after the person's last key, click, wheel or paste frames may come quickly.
pub const BURST_WINDOW: Duration = Duration::from_millis(300);
/// The shortest gap between two frames inside that window.
pub const BURST_GAP: Duration = Duration::from_millis(16);

/// The gap to leave between two frames: `steady` normally, `BURST_GAP` while the person's input is recent.
pub fn gap(since_input: Option<Duration>, steady: Duration) -> Duration {
    match since_input {
        Some(d) if d < BURST_WINDOW => BURST_GAP.min(steady),
        _ => steady,
    }
}

fn base() -> Instant {
    static BASE: OnceLock<Instant> = OnceLock::new();
    *BASE.get_or_init(Instant::now)
}

/// When the person last sent input, shared between the thread that sends it to the screen and the thread that makes frames.
#[derive(Clone, Default)]
pub struct Activity(Arc<AtomicU64>);

impl Activity {
    /// The person just sent input.
    pub fn touch(&self) {
        // One more than the time since the first look, so that zero can say "never".
        self.0.store(base().elapsed().as_millis() as u64 + 1, Ordering::Relaxed);
    }

    /// How long ago the person last sent input; none when they have not.
    pub fn since(&self) -> Option<Duration> {
        match self.0.load(Ordering::Relaxed) {
            0 => None,
            at => Some(base().elapsed().saturating_sub(Duration::from_millis(at - 1))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_come_quickly_only_for_a_short_while_after_input() {
        let steady = Duration::from_millis(66);
        assert_eq!(gap(None, steady), steady, "no input yet");
        assert_eq!(gap(Some(Duration::from_millis(0)), steady), BURST_GAP);
        assert_eq!(gap(Some(Duration::from_millis(299)), steady), BURST_GAP);
        assert_eq!(gap(Some(Duration::from_millis(300)), steady), steady, "the window is over");
        assert_eq!(gap(Some(Duration::from_secs(10)), steady), steady);
    }

    #[test]
    fn the_burst_never_slows_a_steady_rate_that_is_already_faster() {
        assert_eq!(gap(Some(Duration::ZERO), Duration::from_millis(5)), Duration::from_millis(5));
    }

    #[test]
    fn activity_says_never_then_how_long_ago() {
        let a = Activity::default();
        assert_eq!(a.since(), None);
        a.touch();
        let seen = a.since().expect("touched");
        assert!(seen < Duration::from_secs(1), "{seen:?}");
        let other = a.clone();
        std::thread::sleep(Duration::from_millis(20));
        assert!(other.since().unwrap() >= Duration::from_millis(15), "a clone shares the time");
    }
}
