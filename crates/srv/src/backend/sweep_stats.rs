// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the orphan reaper (`sagas/orphan_reaper.rs`) found on its last sweep, for the system report
//! (`sysinfo.rs`). They live in the core so the report does not reach up into `sagas/`; the reaper writes
//! them and re-exports the reader under its old path.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Diagnostic-only counters updated at the end of every `sweep()` — for the
/// periodic `mem_attribution` log (`sysinfo.rs`) to correlate commit/handle
/// growth against reaper activity. A `reaper_candidates`/`reaper_first_seen`
/// that climbs sweep over sweep without a matching rise in
/// `reaper_reaped_cumulative` would mean the reaper keeps finding (but not
/// managing to close) the same growing set of blocks — worth seeing
/// directly rather than inferred from commit numbers alone.
static LAST_CANDIDATES: AtomicUsize = AtomicUsize::new(0);
static LAST_FIRST_SEEN_LEN: AtomicUsize = AtomicUsize::new(0);
static REAPED_CUMULATIVE: AtomicU64 = AtomicU64::new(0);

/// `(candidates found this sweep, first_seen entries carried across sweeps,
/// cumulative blocks reaped since process start)`.
#[cfg(any(windows, test, feature = "test-support"))]
pub fn last_sweep_stats() -> (usize, usize, u64) {
    (
        LAST_CANDIDATES.load(Ordering::Relaxed),
        LAST_FIRST_SEEN_LEN.load(Ordering::Relaxed),
        REAPED_CUMULATIVE.load(Ordering::Relaxed),
    )
}

/// One sweep's result: the candidates it found, the `first_seen` entries it carries into the next sweep, and
/// how many blocks it reaped (added to the running total).
pub fn record(candidates: usize, first_seen: usize, reaped: usize) {
    LAST_CANDIDATES.store(candidates, Ordering::Relaxed);
    LAST_FIRST_SEEN_LEN.store(first_seen, Ordering::Relaxed);
    if reaped > 0 {
        REAPED_CUMULATIVE.fetch_add(reaped as u64, Ordering::Relaxed);
    }
}
