// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Session metadata tracking for agent panes (Phase 1.4 — ultra-long-sessions).
//!
//! Tracks per-session stats as block metadata keys:
//!   `session:start_ts_ms`    — Unix ms when the first output line arrived
//!   `session:last_activity_ms` — Unix ms of most recent output line
//!   `session:line_count`     — total output lines emitted this session
//!   `session:token_estimate` — rough token count (chars / 4, cumulative)
//!
//! To avoid a `SetMeta` write on every output line (which can be very frequent),
//! this module debounces flushes to at most once per second using a local
//! `Instant`-based timestamp.  Accumulators live in `SessionStatsAccumulator`
//! which each controller instance owns privately.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::backend::obj::MetaMapType;
use crate::backend::storage::store::Store;

/// Keys used for session stats in block metadata.
pub const META_SESSION_START_TS_MS: &str = "session:start_ts_ms";
pub const META_SESSION_LAST_ACTIVITY_MS: &str = "session:last_activity_ms";
pub const META_SESSION_LINE_COUNT: &str = "session:line_count";
pub const META_SESSION_TOKEN_ESTIMATE: &str = "session:token_estimate";

/// Debounce interval: at most one Store write per second.
const FLUSH_DEBOUNCE: Duration = Duration::from_secs(1);

/// Returns the current Unix timestamp in milliseconds.
fn now_ms() -> i64 {
    agentmux_common::time::now_ms()
}

/// In-memory accumulator for session stats.  One per controller instance.
///
/// All fields are plain integers — no locking needed because each controller
/// calls `record_line` from its single async stdout-reader task only.
pub struct SessionStatsAccumulator {
    block_id: String,
    /// Unix ms when the first line was seen; 0 = not yet set.
    start_ts_ms: i64,
    /// Unix ms of the most-recently flushed line.
    last_activity_ms: i64,
    /// Total lines seen since session start.
    line_count: u64,
    /// Cumulative token estimate (chars / 4).
    token_estimate: u64,
    /// Wall-clock instant of the last flush; `None` = never flushed.
    last_flush: Option<Instant>,
}

impl SessionStatsAccumulator {
    /// Create a new accumulator for `block_id`.
    pub fn new(block_id: String) -> Self {
        Self {
            block_id,
            start_ts_ms: 0,
            last_activity_ms: 0,
            line_count: 0,
            token_estimate: 0,
            last_flush: None,
        }
    }

    /// Record one output line of `line_len` bytes.
    ///
    /// Updates in-memory counters.  Flushes to the Store if the debounce
    /// interval has elapsed *or* if this is the very first line (so the
    /// frontend sees `session:start_ts_ms` promptly).
    pub fn record_line(&mut self, line_len: usize, wstore: &Option<Arc<Store>>) {
        let ts = now_ms();
        let is_first = self.start_ts_ms == 0;

        if is_first {
            self.start_ts_ms = ts;
        }
        self.last_activity_ms = ts;
        self.line_count += 1;
        self.token_estimate += (line_len / 4) as u64;

        // Flush immediately on first line; otherwise debounce.
        let should_flush = is_first || match self.last_flush {
            None => true,
            Some(last) => last.elapsed() >= FLUSH_DEBOUNCE,
        };

        if should_flush {
            if let Some(ref store) = wstore {
                self.flush(store);
            }
        }
    }

    /// Force-flush all accumulated stats to the Store right now.
    ///
    /// Called by `record_line` when the debounce window has elapsed.
    fn flush(&mut self, wstore: &Arc<Store>) {
        let oref_str = format!("block:{}", self.block_id);
        let mut meta_update = MetaMapType::new();

        if self.start_ts_ms != 0 {
            meta_update.insert(
                META_SESSION_START_TS_MS.to_string(),
                serde_json::json!(self.start_ts_ms),
            );
        }
        meta_update.insert(
            META_SESSION_LAST_ACTIVITY_MS.to_string(),
            serde_json::json!(self.last_activity_ms),
        );
        meta_update.insert(
            META_SESSION_LINE_COUNT.to_string(),
            serde_json::json!(self.line_count),
        );
        meta_update.insert(
            META_SESSION_TOKEN_ESTIMATE.to_string(),
            serde_json::json!(self.token_estimate),
        );

        match crate::server::service::update_object_meta(wstore, &oref_str, &meta_update) {
            Ok(()) => {
                tracing::trace!(
                    block_id = %self.block_id,
                    line_count = self.line_count,
                    token_estimate = self.token_estimate,
                    "session stats flushed"
                );
            }
            Err(e) => {
                tracing::warn!(
                    block_id = %self.block_id,
                    error = %e,
                    "failed to flush session stats"
                );
            }
        }

        self.last_flush = Some(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_accumulator_first_line_sets_start_ts() {
        let mut acc = SessionStatsAccumulator::new("blk-1".to_string());
        // No wstore — flush is skipped but counters still update.
        acc.record_line(100, &None);
        assert_ne!(acc.start_ts_ms, 0);
        assert_eq!(acc.line_count, 1);
        assert_eq!(acc.token_estimate, 25); // 100 / 4
    }

    #[test]
    fn test_accumulator_multiple_lines() {
        let mut acc = SessionStatsAccumulator::new("blk-2".to_string());
        acc.record_line(40, &None);
        acc.record_line(80, &None);
        acc.record_line(120, &None);
        assert_eq!(acc.line_count, 3);
        // 40/4 + 80/4 + 120/4 = 10 + 20 + 30 = 60
        assert_eq!(acc.token_estimate, 60);
    }

    #[test]
    fn test_accumulator_start_ts_not_reset_on_second_line() {
        let mut acc = SessionStatsAccumulator::new("blk-3".to_string());
        acc.record_line(10, &None);
        let first_ts = acc.start_ts_ms;
        acc.record_line(10, &None);
        assert_eq!(acc.start_ts_ms, first_ts, "start_ts must not change after first line");
    }

    #[test]
    fn test_debounce_constants() {
        assert_eq!(FLUSH_DEBOUNCE, Duration::from_secs(1));
    }
}
