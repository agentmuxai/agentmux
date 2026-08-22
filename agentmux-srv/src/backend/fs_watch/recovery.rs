// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Health tracking + retry policy for `FsWatchPool`. Split out of `pool.rs`
//! so the recovery bookkeeping (what's degraded, which backend is active)
//! is testable independent of the notify/tokio wiring.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

/// Which `notify` backend is actually driving watches right now. `Polling`
/// only happens if the native backend failed to construct at all (rare —
/// e.g. inotify unavailable) — see `SPEC_SHARED_FS_WATCHER_FRAMEWORK_2026_08_07.md`
/// §4 point 2. Once picked at pool construction, this doesn't change for the
/// pool's lifetime; per-path failures are tracked in `degraded_paths`
/// instead, independent of which backend is in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchBackend {
    Native,
    Polling,
}

/// Snapshot of the pool's current health, for diagnostics (`muxlog`,
/// eventually a UI surface — not built here, see spec §6 non-goals).
#[derive(Debug, Clone)]
pub struct FsWatchHealth {
    pub backend: WatchBackend,
    pub active_watches: usize,
    /// Paths that failed to (re-)establish a watch on their last attempt,
    /// paired with the error. Retried in the background per
    /// `RETRY_BACKOFF`/the periodic sweep; a path leaves this list the
    /// moment any attempt succeeds.
    pub degraded_paths: Vec<(PathBuf, String)>,
}

/// Backoff schedule for retrying a failed `watch()` call, per spec §4 point
/// 1 — bounded at 3 attempts over ~4.2s total, not indefinite: a path that's
/// still failing after this moves into `degraded_paths` and is picked up by
/// the periodic health sweep (`pool.rs`'s `HEALTH_SWEEP_INTERVAL`) instead of
/// spinning a dedicated retry loop forever.
pub const RETRY_BACKOFF: [Duration; 3] = [
    Duration::from_millis(200),
    Duration::from_millis(800),
    Duration::from_millis(3200),
];

/// How often the background sweep re-arms every currently-*degraded*
/// path, self-healing a watch that failed to establish (or a prior sweep
/// found dead) without needing a dedicated retry loop running forever.
/// **Deliberately scoped to degraded targets only, not every subscribed
/// path** — an earlier version of this sweep re-issued `watch()`
/// unconditionally on everything, every tick, to also catch a *silently*
/// dead healthy-looking watch (inotify instance-limit churn, a watched
/// directory deleted and recreated at a new inode, a flaky network mount).
/// That cost more than it bought: `notify` 7.0.0's Windows backend leaks a
/// File + Semaphore handle pair on every redundant `watch()` call (no
/// cleanup of the previous entry), and even with an `unwatch()` first to
/// avoid the leak, the gap between `unwatch()` and `watch()` drops event
/// coverage for consumers with no reconciliation backstop of their own
/// (`config_watcher_fs`, `EditorFileWatcher`, `MediaFileWatcher`). See
/// `pool.rs`'s `sweep()` and
/// `docs/status/STATUS_FS_WATCH_SWEEP_HANDLE_LEAK_2026_08_22.md` for the
/// full history and the accepted trade-off (a truly silent death of an
/// already-healthy watch is no longer self-healed by this sweep).
pub const HEALTH_SWEEP_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(super) struct HealthState {
    degraded: Mutex<HashMap<PathBuf, String>>,
}

impl HealthState {
    pub(super) fn mark_degraded(&self, path: PathBuf, err: String) {
        self.degraded.lock().unwrap().insert(path, err);
    }

    pub(super) fn clear_degraded(&self, path: &std::path::Path) {
        self.degraded.lock().unwrap().remove(path);
    }

    pub(super) fn is_degraded(&self, path: &std::path::Path) -> bool {
        self.degraded.lock().unwrap().contains_key(path)
    }

    pub(super) fn snapshot(&self) -> Vec<(PathBuf, String)> {
        self.degraded
            .lock()
            .unwrap()
            .iter()
            .map(|(p, e)| (p.clone(), e.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_then_clear_round_trips() {
        let state = HealthState::default();
        let path = PathBuf::from("/tmp/watched-thing");
        state.mark_degraded(path.clone(), "permission denied".to_string());
        assert_eq!(state.snapshot(), vec![(path.clone(), "permission denied".to_string())]);

        state.clear_degraded(&path);
        assert!(state.snapshot().is_empty());
    }

    #[test]
    fn clearing_an_unknown_path_is_a_no_op() {
        let state = HealthState::default();
        state.clear_degraded(std::path::Path::new("/never/watched"));
        assert!(state.snapshot().is_empty());
    }

    #[test]
    fn retry_backoff_is_bounded_and_increasing() {
        assert_eq!(RETRY_BACKOFF.len(), 3);
        assert!(RETRY_BACKOFF[0] < RETRY_BACKOFF[1]);
        assert!(RETRY_BACKOFF[1] < RETRY_BACKOFF[2]);
    }
}
