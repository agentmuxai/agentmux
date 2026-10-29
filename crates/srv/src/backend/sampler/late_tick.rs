// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The sampler's own lateness, as a log line. A chart draws a break after 3 s without a sample, and the
//! sampler runs on the same runtime as everything else, so a tick that comes late is that runtime (or the
//! machine) not running it. Without a record, whether chart gaps happen, and how often, is a guess.
//! docs/analysis/ANALYSIS_SRV_HTTP_STALL_IO_DRIVER_STARVATION_2026_09_26.md section 9.

/// A tick later than this is logged: close enough to the chart's 3 s break to show it coming.
pub const LATE_TICK_WARN_MS: u64 = 2_000;
/// At most one line per this long; the ones in between are counted into the next.
pub const LATE_TICK_LOG_EVERY_MS: u64 = 60_000;

/// What to say about one late tick.
#[derive(Debug, PartialEq, Eq)]
pub struct LateTick {
    pub late_ms: u64,
    /// Late ticks not logged since the last line.
    pub also: u32,
}

#[derive(Default)]
pub struct LateTickLog {
    last_logged_ms: Option<u64>,
    held: u32,
}

impl LateTickLog {
    /// A tick planned for `due_ms` that ran at `now_ms` (the same clock): `Some` when it is late enough to
    /// log and the last line was long enough ago.
    pub fn check(&mut self, due_ms: u64, now_ms: u64) -> Option<LateTick> {
        let late_ms = now_ms.saturating_sub(due_ms);
        if late_ms < LATE_TICK_WARN_MS {
            return None;
        }
        if self.last_logged_ms.is_some_and(|t| now_ms.saturating_sub(t) < LATE_TICK_LOG_EVERY_MS) {
            self.held += 1;
            return None;
        }
        self.last_logged_ms = Some(now_ms);
        Some(LateTick { late_ms, also: std::mem::take(&mut self.held) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tick_on_time_or_a_little_late_says_nothing() {
        let mut log = LateTickLog::default();
        assert_eq!(log.check(10_000, 10_000), None);
        assert_eq!(log.check(10_000, 10_000 + LATE_TICK_WARN_MS - 1), None);
        assert_eq!(log.check(10_000, 9_000), None, "early is not late");
    }

    #[test]
    fn a_late_tick_is_logged_and_the_next_ones_are_held_then_counted() {
        let mut log = LateTickLog::default();
        assert_eq!(log.check(10_000, 13_000), Some(LateTick { late_ms: 3_000, also: 0 }));
        assert_eq!(log.check(20_000, 24_000), None);
        assert_eq!(log.check(30_000, 35_000), None);
        let after = 13_000 + LATE_TICK_LOG_EVERY_MS;
        assert_eq!(log.check(after - 2_500, after), Some(LateTick { late_ms: 2_500, also: 2 }));
    }
}
