// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wall-clock "now" helpers.
//!
//! The three-line `SystemTime::now().duration_since(UNIX_EPOCH)` body was
//! re-implemented in 26 files across four crates before this module
//! existed, with the only variation being the return type and whether a
//! pre-epoch clock maps to `0` or `unwrap_or_default()` — which are the
//! same thing (`docs/reports/REPORT_DRY_AND_MODULARITY_AUDIT_2026_09_06.md`
//! §2.2).
//!
//! Two integer widths are offered rather than one, because callers were
//! split 16/8 between `i64` (SQLite column types, JSON timestamps) and
//! `u64` (durations, comparisons against other `u64`s). Forcing one width
//! would push a cast into every caller of the other; offering both keeps
//! every existing call site's type unchanged.
//!
//! A clock set before the Unix epoch yields `0`, never a panic — these are
//! used on hot paths (every persisted event, every stats sample) where a
//! panic on a misconfigured clock would be far worse than a zero timestamp.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn since_epoch() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
}

/// Milliseconds since the Unix epoch, as `i64`.
pub fn now_ms() -> i64 {
    since_epoch().as_millis() as i64
}

/// Milliseconds since the Unix epoch, as `u64`.
pub fn now_ms_u64() -> u64 {
    since_epoch().as_millis() as u64
}

/// Seconds since the Unix epoch, as `i64`.
pub fn now_secs() -> i64 {
    since_epoch().as_secs() as i64
}

/// Seconds since the Unix epoch, as `u64`.
pub fn now_secs_u64() -> u64 {
    since_epoch().as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sanity bounds rather than an exact value: after 2020-01-01 (so the
    /// epoch math isn't off by a factor of 1000) and the two widths agree.
    #[test]
    fn now_is_after_2020_and_widths_agree() {
        const JAN_1_2020_SECS: i64 = 1_577_836_800;
        let s = now_secs();
        let ms = now_ms();
        assert!(s > JAN_1_2020_SECS, "now_secs() = {s}");
        assert!(ms > JAN_1_2020_SECS * 1000, "now_ms() = {ms}");
        // ms and secs sampled back-to-back must agree to within a second.
        assert!((ms / 1000 - s).abs() <= 1);
        assert!(now_ms_u64() as i64 - now_ms() <= 1000);
        assert_eq!(now_secs_u64() as i64 - now_secs() <= 1, true);
    }
}
