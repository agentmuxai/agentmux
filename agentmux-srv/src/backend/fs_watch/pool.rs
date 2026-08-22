// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notify::{EventKind, RecursiveMode, Watcher};
use tokio::sync::{broadcast, mpsc};

use super::recovery::{HealthState, WatchBackend, FsWatchHealth, RETRY_BACKOFF, HEALTH_SWEEP_INTERVAL};

/// Broadcast channel capacity. Generous relative to real fs-event volume
/// (file-save bursts, not high-frequency streams) — a slow consumer would
/// need to fall behind by this many raw events before `RecvError::Lagged`,
/// at which point it should resync from scratch rather than assume it saw
/// everything (same "wake signal, not a guaranteed delivery log" contract
/// the existing three watchers already have via their own debounce logic).
const EVENT_CHANNEL_CAPACITY: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsWatchEventKind {
    Created,
    Modified,
    Removed,
    Other,
}

impl From<EventKind> for FsWatchEventKind {
    fn from(kind: EventKind) -> Self {
        match kind {
            EventKind::Create(_) => FsWatchEventKind::Created,
            EventKind::Modify(_) => FsWatchEventKind::Modified,
            EventKind::Remove(_) => FsWatchEventKind::Removed,
            _ => FsWatchEventKind::Other,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FsWatchEvent {
    pub path: PathBuf,
    pub kind: FsWatchEventKind,
}

/// Handle returned by `subscribe_file`/`subscribe_dir`. Holding this alive
/// is not required (unlike the raw `notify::Watcher`, which must be kept
/// alive by its owner) — the pool itself owns the underlying watcher.
/// `unsubscribe` takes this by value so a caller can't accidentally
/// unsubscribe the same handle twice.
#[derive(Debug, Clone)]
pub struct Subscription {
    id: u64,
    /// What the caller actually asked to watch (a file or a directory).
    pub path: PathBuf,
    /// What's actually registered with the OS watcher — the parent
    /// directory for a file subscription (see module doc: this is the
    /// baked-in atomic-rename-safety rule), or `path` itself for a
    /// directory subscription.
    watch_target: PathBuf,
    mode: RecursiveMode,
}

struct WatchEntry {
    refcount: usize,
    mode: RecursiveMode,
}

struct Inner {
    watcher: Option<Box<dyn Watcher + Send>>,
    targets: HashMap<PathBuf, WatchEntry>,
}

pub struct FsWatchPool {
    inner: Mutex<Inner>,
    health: HealthState,
    backend: WatchBackend,
    next_id: AtomicU64,
    tx: broadcast::Sender<FsWatchEvent>,
}

impl FsWatchPool {
    /// Construct the pool and start its background event-bridging and
    /// health-sweep tasks. Always succeeds — if even the `PollWatcher`
    /// fallback fails to construct (practically never happens; it's pure
    /// userspace polling), the pool still comes up with `watcher: None` and
    /// every subscribe call fails cleanly into `degraded_paths` rather than
    /// panicking or requiring every caller to handle an `Option<Arc<Self>>`
    /// at the top level. Live-update is additive everywhere it's used.
    pub fn new() -> Arc<Self> {
        let (raw_tx, mut raw_rx) = mpsc::unbounded_channel::<notify::Result<notify::Event>>();
        let (watcher, backend) = construct_watcher(raw_tx);

        let (broadcast_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);

        let this = Arc::new(Self {
            inner: Mutex::new(Inner { watcher, targets: HashMap::new() }),
            health: HealthState::default(),
            backend,
            next_id: AtomicU64::new(1),
            tx: broadcast_tx,
        });

        // Bridge notify's sync callback (already funneled into raw_rx by
        // construct_watcher's closure) into the broadcast stream. Mirrors
        // the sync-callback -> mpsc -> async-task shape all three existing
        // watchers already use, just shared once here instead of three
        // times.
        let bridge = this.clone();
        tokio::spawn(async move {
            while let Some(res) = raw_rx.recv().await {
                match res {
                    Ok(event) => {
                        let kind = FsWatchEventKind::from(event.kind);
                        if matches!(kind, FsWatchEventKind::Other) {
                            continue;
                        }
                        for path in event.paths {
                            let _ = bridge.tx.send(FsWatchEvent { path, kind });
                        }
                    }
                    Err(e) => {
                        // reagent P1 on #2722: sweep() now only re-arms
                        // targets already marked degraded — an explicit,
                        // non-silent notify error (e.g. a Windows
                        // ReadDirectoryChangesW buffer-overflow) has to be
                        // recorded here, or a watch that dies with a real
                        // reported error (not just the accepted "truly
                        // silent, never-erroring death" gap) would never be
                        // picked up by the sweep either. `notify::Error`
                        // carries the affected path(s) when it can attribute
                        // one; mark each degraded so the next sweep re-arms
                        // it. A path-less error (rare — not attributable to
                        // any specific target) can only be logged.
                        tracing::warn!(error = %e, paths = ?e.paths, "fs_watch: backend reported an error");
                        for path in &e.paths {
                            bridge.health.mark_degraded(path.clone(), e.to_string());
                        }
                    }
                }
            }
        });

        // Periodic self-healing sweep, scoped to already-degraded targets —
        // see `sweep()`'s own doc comment for the full rationale (a
        // healthy-looking target is deliberately left untouched, to avoid
        // both a handle leak and a real event-coverage gap notify's Windows
        // backend would otherwise cost on every tick).
        let sweeper = this.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(HEALTH_SWEEP_INTERVAL);
            loop {
                tick.tick().await;
                sweeper.sweep();
            }
        });

        this
    }

    /// Watch the file at `path` (specifically: its parent directory,
    /// non-recursively — see module doc). Returns a fresh `Subscription`
    /// even when the underlying OS watch is currently failing (tracked in
    /// `degraded_paths` instead) — a degraded subscription still becomes
    /// live automatically once retry/sweep succeeds, with no action needed
    /// from the caller.
    pub fn subscribe_file(self: &Arc<Self>, path: &Path) -> Subscription {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let target = canonical.parent().map(Path::to_path_buf).unwrap_or(canonical.clone());
        self.subscribe_target(canonical, target, RecursiveMode::NonRecursive)
    }

    /// Watch the directory at `path` directly (non-recursive — matching
    /// `media_file_watcher.rs`'s existing behavior; extension/content
    /// filtering is the caller's concern, not the pool's).
    pub fn subscribe_dir(self: &Arc<Self>, path: &Path) -> Subscription {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.subscribe_target(canonical.clone(), canonical, RecursiveMode::NonRecursive)
    }

    fn subscribe_target(self: &Arc<Self>, requested_path: PathBuf, target: PathBuf, mode: RecursiveMode) -> Subscription {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let is_new = {
            let mut inner = self.inner.lock().unwrap();
            let entry = inner.targets.entry(target.clone()).or_insert_with(|| WatchEntry { refcount: 0, mode });
            entry.refcount += 1;
            entry.refcount == 1
        };

        if is_new {
            self.start_watch(target.clone(), mode);
        }

        Subscription { id, path: requested_path, watch_target: target, mode }
    }

    /// Stop watching on behalf of `sub`. Idempotent-safe: unsubscribing the
    /// same handle twice would underflow the refcount, so this takes `self`
    /// by shared ref but `sub` by value specifically to discourage reuse —
    /// callers that need multiple independent subscriptions to the same
    /// path should call `subscribe_file`/`subscribe_dir` once per logical
    /// subscriber, matching how `editor_file_watcher.rs` already tracks one
    /// entry per `block_id` today.
    pub fn unsubscribe(&self, sub: Subscription) {
        let should_unwatch = {
            let mut inner = self.inner.lock().unwrap();
            let Some(entry) = inner.targets.get_mut(&sub.watch_target) else {
                return;
            };
            entry.refcount = entry.refcount.saturating_sub(1);
            let empty = entry.refcount == 0;
            if empty {
                inner.targets.remove(&sub.watch_target);
            }
            empty
        };

        if should_unwatch {
            let mut inner = self.inner.lock().unwrap();
            if let Some(w) = inner.watcher.as_mut() {
                let _ = w.unwatch(&sub.watch_target);
            }
            drop(inner);
            self.health.clear_degraded(&sub.watch_target);
        }
    }

    /// Raw change events for every currently-watched target, pool-wide.
    /// Each call returns an independent receiver — subscribe to this once
    /// per domain module (not once per file), and filter down to the paths
    /// that module actually cares about, same as today's per-watcher
    /// `Inner.watched_paths` filtering.
    pub fn events(&self) -> broadcast::Receiver<FsWatchEvent> {
        self.tx.subscribe()
    }

    pub fn health(&self) -> FsWatchHealth {
        let active_watches = self.inner.lock().unwrap().targets.len();
        FsWatchHealth {
            backend: self.backend,
            active_watches,
            degraded_paths: self.health.snapshot(),
        }
    }

    /// First attempt at a new target, plus a bounded retry-with-backoff if
    /// it fails — see `RETRY_BACKOFF`'s doc comment. Runs the retries on a
    /// spawned task so `subscribe_file`/`subscribe_dir` never blocks on it.
    fn start_watch(self: &Arc<Self>, target: PathBuf, mode: RecursiveMode) {
        let first_attempt = self.try_watch(&target, mode);
        if first_attempt.is_ok() {
            return;
        }
        let err = first_attempt.unwrap_err();
        tracing::warn!(path = %target.display(), error = %err, "fs_watch: initial watch failed, retrying with backoff");
        self.health.mark_degraded(target.clone(), err);

        let this = self.clone();
        tokio::spawn(async move {
            for delay in RETRY_BACKOFF {
                tokio::time::sleep(delay).await;
                // The subscription may have been torn down (refcount back to
                // zero) while we were waiting — don't resurrect a watch
                // nobody wants anymore.
                if !this.inner.lock().unwrap().targets.contains_key(&target) {
                    return;
                }
                match this.try_watch(&target, mode) {
                    Ok(()) => {
                        this.health.clear_degraded(&target);
                        tracing::info!(path = %target.display(), "fs_watch: watch recovered after retry");
                        return;
                    }
                    Err(e) => {
                        this.health.mark_degraded(target.clone(), e);
                    }
                }
            }
            tracing::warn!(
                path = %target.display(),
                "fs_watch: still degraded after all retries — will keep retrying via the periodic health sweep"
            );
        });
    }

    fn try_watch(&self, target: &Path, mode: RecursiveMode) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        let Some(watcher) = inner.watcher.as_mut() else {
            return Err("no fs watcher backend available".to_string());
        };
        watcher.watch(target, mode).map_err(|e| e.to_string())
    }

    /// Re-arm every currently-*degraded* target — the backstop that lets a
    /// path recover from a failed `watch()` attempt (at `start_watch` time
    /// or a prior sweep) without a dedicated retry loop running forever.
    ///
    /// **Deliberately does NOT touch a target that already looks healthy**
    /// (codex P2 on #2722, revising this fn's first version): the original
    /// fix here unconditionally called `unwatch()` then `watch()` on every
    /// subscribed target every tick, reasoning that this was needed to
    /// detect a watch that died *silently* (no error ever surfaced). Two
    /// problems with that, both real:
    /// 1. **The handle leak this fn exists to fix** — `notify` 7.0.0's
    ///    Windows backend (`add_watch` in the vendored `windows.rs`) opens a
    ///    brand-new `CreateFileW` handle + `CreateSemaphoreW` on *every*
    ///    `watch()` call, unconditionally, and silently drops the previous
    ///    entry's handles with no cleanup — see
    ///    `docs/status/STATUS_FS_WATCH_SWEEP_HANDLE_LEAK_2026_08_22.md`.
    /// 2. **A real coverage gap** — `unwatch()` then `watch()` leaves the
    ///    target genuinely unwatched for the gap between the two calls.
    ///    `native_memory_drift`'s slow-path reconciliation sweep backstops a
    ///    missed event there, but `config_watcher_fs`, `EditorFileWatcher`,
    ///    and `MediaFileWatcher` all rely solely on the broadcast stream —
    ///    a create/modify landing in that gap would go unnoticed until some
    ///    *later*, unrelated change on the same path happened to trigger a
    ///    refresh.
    ///
    /// Skipping healthy targets fixes both: nothing is ever double-watched
    /// (no leak) and a healthy watch is never touched (no gap). The
    /// **honestly-bounded trade-off**: a watch that fails *silently* — dies
    /// without `notify` or the OS ever reporting an error on it — is no
    /// longer self-healed by this sweep, only a watch that's already known
    /// degraded (an explicit prior failure). Accepted because a truly
    /// silent death has no confirmed occurrence in this codebase's history
    /// (the scenarios `HEALTH_SWEEP_INTERVAL`'s own doc comment lists —
    /// inotify instance-limit churn, flaky network mounts — are Linux-
    /// flavored concerns on a Windows-primary codebase), against a
    /// certain, systematic cost (leak or gap, every target, every tick) for
    /// every other watch the alternative would have imposed.
    ///
    /// **Race with concurrent `unsubscribe()` — closed by re-checking under
    /// one held lock** (reagent P1, second review round on #2722): the
    /// degraded-target list above is a snapshot; without re-verification, a
    /// concurrent `unsubscribe()` dropping the last subscriber for one of
    /// these targets between the snapshot and this loop's `watch()` call
    /// would remove it from `targets` and `unwatch()` it, while this sweep's
    /// in-flight `watch()` call still re-established a native watch for the
    /// now-untracked path — orphaning exactly the handle pair this fn exists
    /// to stop leaking, just via a race instead of per-tick re-watching.
    /// Each target's re-check + `unwatch()` + `watch()` now happens under a
    /// single `inner` lock acquisition (not `try_watch()`, which takes its
    /// own lock and would deadlock if called while this one is held) — a
    /// concurrent `unsubscribe()` either finishes its own removal entirely
    /// before this runs (this loop then sees the target is gone and skips
    /// it) or entirely after (its own `unwatch()` then correctly tears down
    /// the watch this loop just re-armed) — `Mutex` serializes the two, so
    /// no interleaving in between is possible.
    fn sweep(&self) {
        let targets: Vec<PathBuf> = {
            let inner = self.inner.lock().unwrap();
            inner
                .targets
                .keys()
                .filter(|p| self.health.is_degraded(p))
                .cloned()
                .collect()
        };
        for target in targets {
            self.rearm_if_still_subscribed(target);
        }
    }

    /// The per-target half of a sweep: re-verify `target` is still
    /// subscribed, then `unwatch()` + `watch()` it, all under one held
    /// `inner` lock (see `sweep()`'s doc comment for why the re-check and
    /// the single lock both matter). Split out from `sweep()`'s loop body
    /// so a test can call it directly with a `target` captured *before* a
    /// real `unsubscribe()` ran — deterministically reproducing "sweep
    /// already decided to process this target, then it was removed before
    /// the action ran" without needing to win a real thread-scheduling
    /// race.
    fn rearm_if_still_subscribed(&self, target: PathBuf) {
        let mut inner = self.inner.lock().unwrap();
        // Re-check: still subscribed? A concurrent unsubscribe() may have
        // removed it since the caller's snapshot — don't resurrect a watch
        // nobody wants anymore.
        let Some(mode) = inner.targets.get(&target).map(|e| e.mode) else {
            return;
        };
        let Some(w) = inner.watcher.as_mut() else {
            return;
        };
        // Best-effort: a never-established watch has nothing to unwatch —
        // the watch() call below (re)establishes it regardless of whether
        // this succeeds.
        let _ = w.unwatch(&target);
        let result = w.watch(&target, mode).map_err(|e| e.to_string());
        drop(inner);
        match result {
            Ok(()) => self.health.clear_degraded(&target),
            Err(e) => self.health.mark_degraded(target, e),
        }
    }
}

/// Build the watcher backend: prefer the native OS backend
/// (inotify/FSEvents/ReadDirectoryChangesW); fall back to `notify`'s
/// stat-based `PollWatcher` if the native backend can't even construct
/// (practically never on the platforms AgentMux ships for, but real on some
/// restricted/containerized environments). Returns `(None, Native)` only if
/// both fail to construct — recorded as `WatchBackend::Native` since that
/// was the attempted backend; every `subscribe_*` call will immediately
/// fail into `degraded_paths` until the process restarts, matching the
/// existing three watchers' "live-update becomes unavailable, nothing else
/// breaks" posture, just now with a health signal instead of silence.
fn construct_watcher(
    tx: mpsc::UnboundedSender<notify::Result<notify::Event>>,
) -> (Option<Box<dyn Watcher + Send>>, WatchBackend) {
    let tx_native = tx.clone();
    match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let _ = tx_native.send(res);
    }) {
        Ok(w) => return (Some(Box::new(w)), WatchBackend::Native),
        Err(e) => {
            tracing::warn!(error = %e, "fs_watch: native backend unavailable, falling back to polling");
        }
    }

    let poll_config = notify::Config::default().with_poll_interval(Duration::from_secs(2));
    match notify::PollWatcher::new(move |res: notify::Result<notify::Event>| { let _ = tx.send(res); }, poll_config) {
        Ok(w) => (Some(Box::new(w)), WatchBackend::Polling),
        Err(e) => {
            tracing::error!(error = %e, "fs_watch: polling fallback also failed to construct — live-update unavailable this session");
            (None, WatchBackend::Native)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration as StdDuration;

    #[tokio::test]
    async fn subscribe_unsubscribe_refcounting_does_not_panic() {
        let pool = FsWatchPool::new();
        let tmp = std::env::temp_dir().join("agentmux_fs_watch_pool_test");
        std::fs::create_dir_all(&tmp).unwrap();

        let sub1 = pool.subscribe_dir(&tmp);
        let sub2 = pool.subscribe_dir(&tmp);
        assert_eq!(pool.health().active_watches, 1, "two subscriptions to the same dir share one OS watch");

        pool.unsubscribe(sub1);
        assert_eq!(pool.health().active_watches, 1, "still watched — sub2 is still active");

        pool.unsubscribe(sub2);
        assert_eq!(pool.health().active_watches, 0, "last subscriber gone -> watch torn down");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn subscribe_file_watches_the_parent_directory() {
        let pool = FsWatchPool::new();
        let dir = std::env::temp_dir().join("agentmux_fs_watch_pool_file_test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("thing.md");
        std::fs::write(&file, "hello").unwrap();

        let sub = pool.subscribe_file(&file);
        assert_eq!(sub.watch_target, dir.canonicalize().unwrap());

        pool.unsubscribe(sub);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_change_event_is_observable_on_the_broadcast_stream() {
        let pool = FsWatchPool::new();
        let dir = std::env::temp_dir().join("agentmux_fs_watch_pool_event_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("watched.md");
        std::fs::write(&file, "v1").unwrap();

        let mut events = pool.events();
        let sub = pool.subscribe_file(&file);
        // Give the native backend a moment to actually register before we
        // write — otherwise this is a flaky "wrote before the watch was live"
        // race, not a real assertion about the pool's behavior.
        tokio::time::sleep(StdDuration::from_millis(200)).await;
        std::fs::write(&file, "v2").unwrap();

        let seen = tokio::time::timeout(StdDuration::from_secs(5), async {
            loop {
                match events.recv().await {
                    Ok(ev) if ev.path.file_name() == file.file_name() => return true,
                    Ok(_) => continue,
                    Err(_) => return false,
                }
            }
        })
        .await
        .unwrap_or(false);

        assert!(seen, "expected to observe a change event for the watched file");

        pool.unsubscribe(sub);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn health_reports_the_constructed_backend() {
        let pool = FsWatchPool::new();
        // On every platform this actually runs on in CI, the native backend
        // constructs successfully — this just pins that expectation rather
        // than the (currently untestable-without-platform-mocking) fallback
        // path, which recovery.rs's own unit tests cover at the bookkeeping
        // level instead.
        assert_eq!(pool.health().backend, WatchBackend::Native);
    }

    /// Regression test for
    /// `docs/status/STATUS_FS_WATCH_SWEEP_HANDLE_LEAK_2026_08_22.md`, healthy
    /// side: `sweep()` used to call `watch()` unconditionally on every
    /// subscribed target on every `HEALTH_SWEEP_INTERVAL` tick, leaking a
    /// File + Semaphore handle pair per call (see companion test below for
    /// why) — the fix (codex P2 on #2722) is to skip a target entirely once
    /// it's healthy, not just to `unwatch()` before re-`watch()`-ing it (that
    /// still leaked nothing, but cost every other watcher a real event-
    /// coverage gap). Simulates many sweep ticks on a healthy target and
    /// asserts both that handle count does not grow *and* that a healthy
    /// watch is never touched at all.
    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn sweep_leaves_a_healthy_target_untouched() {
        let pool = FsWatchPool::new();
        let tmp = std::env::temp_dir().join("agentmux_fs_watch_pool_sweep_healthy_test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let sub = pool.subscribe_dir(&tmp);
        // Let the initial watch settle before measuring.
        tokio::time::sleep(StdDuration::from_millis(100)).await;
        assert!(!pool.health.is_degraded(&sub.watch_target), "a freshly-established watch must be healthy");

        let pid = std::process::id();
        let before = crate::backend::sysinfo::process_handle_count(pid)
            .expect("own handle count must be queryable");

        const SWEEPS: u32 = 200;
        for _ in 0..SWEEPS {
            pool.sweep();
        }

        let after = crate::backend::sysinfo::process_handle_count(pid)
            .expect("own handle count must be queryable");
        let grew_by = after.saturating_sub(before);

        pool.unsubscribe(sub);
        let _ = std::fs::remove_dir_all(&tmp);

        assert!(
            grew_by < SWEEPS / 2,
            "handle count grew by {grew_by} over {SWEEPS} sweep() calls on a \
             healthy target (before={before}, after={after}) — a healthy \
             target must never be re-watched at all"
        );
    }

    /// Companion to the test above: exercises the actual `unwatch()` +
    /// `watch()` re-arm path that runs for a *degraded* target, proving that
    /// path itself doesn't leak either (the original bug — `notify` 7.0.0's
    /// Windows backend opens a new `CreateFileW` + `CreateSemaphoreW` on
    /// every `watch()` call, unconditionally, and its internal map's
    /// `insert()` silently drops the previous entry's handles with no
    /// cleanup). Forces the target into `degraded` state directly before
    /// every sweep so each call really re-arms it, then asserts handle
    /// count does not grow linearly with sweep count.
    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn sweep_does_not_leak_a_handle_pair_per_call_for_a_degraded_target() {
        let pool = FsWatchPool::new();
        let tmp = std::env::temp_dir().join("agentmux_fs_watch_pool_sweep_degraded_test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let sub = pool.subscribe_dir(&tmp);
        tokio::time::sleep(StdDuration::from_millis(100)).await;
        let target = sub.watch_target.clone();

        let pid = std::process::id();
        let before = crate::backend::sysinfo::process_handle_count(pid)
            .expect("own handle count must be queryable");

        const SWEEPS: u32 = 200;
        for _ in 0..SWEEPS {
            // Force degraded so this tick's sweep() actually re-arms the
            // watch (unwatch() + watch()), same as it would for a real
            // failure — clear_degraded() on success would otherwise make
            // only the first sweep do real work.
            pool.health.mark_degraded(target.clone(), "simulated for test".to_string());
            pool.sweep();
        }

        let after = crate::backend::sysinfo::process_handle_count(pid)
            .expect("own handle count must be queryable");
        let grew_by = after.saturating_sub(before);

        pool.unsubscribe(sub);
        let _ = std::fs::remove_dir_all(&tmp);

        assert!(
            grew_by < SWEEPS / 2,
            "handle count grew by {grew_by} over {SWEEPS} sweep() calls on a \
             degraded target (before={before}, after={after}) — the \
             unwatch()-before-watch() re-arm path must not leak a \
             File+Semaphore pair per call"
        );
    }

    /// Regression test for reagent's second-round P1 on #2722: `sweep()`
    /// snapshots the degraded-target list, then acts on each one later.
    /// Without re-verification, a concurrent `unsubscribe()` dropping the
    /// last subscriber for one of these targets in that window would remove
    /// it from `targets` (and `unwatch()` it), while this sweep's own
    /// in-flight `watch()` call still re-established a native watch for the
    /// now-untracked path — orphaning the handle pair forever, since nothing
    /// is left to `unwatch()` it again.
    ///
    /// **Deterministic, not a timing-based race** (an earlier version of
    /// this test tried to actually race `sweep()` against `unsubscribe()`
    /// on separate tasks — unreliable: `unsubscribe()`'s critical section is
    /// so much shorter than `spawn_blocking`'s own scheduling latency that
    /// it essentially always completed entirely before or after `sweep()`
    /// ran, never in the middle, so it couldn't reliably exercise the
    /// vulnerable window either way). Instead, `sweep()`'s per-target action
    /// is split into its own method
    /// ([`rearm_if_still_subscribed`](Self::rearm_if_still_subscribed)) so
    /// this test can call it directly with a target captured *before* a
    /// real `unsubscribe()` runs — deterministically reproducing "sweep
    /// already decided to process this target, then it was removed before
    /// the action ran" without needing to win any race at all.
    /// Repeatedly calling `rearm_if_still_subscribed()` on the SAME target
    /// does not by itself amplify a leak here — each call's own `unwatch()`
    /// cleans up the *previous* call's watch, so a tight loop on one path is
    /// self-cleaning regardless of whether the re-check exists (confirmed
    /// empirically writing this test — a single-shot before/after diff on
    /// one target is too small to reliably separate from handle-count
    /// noise, and looping the same target doesn't accumulate). The actual
    /// damage from a missing re-check is a *one-time* orphaned resurrection
    /// with no future subscriber left to ever `unwatch()` it again — so
    /// this amplifies by repeating the whole subscribe -> degrade ->
    /// unsubscribe -> single-stale-rearm sequence across many DISTINCT
    /// targets instead, each contributing at most one orphaned handle pair
    /// if the bug is present, summing to a clearly measurable total.
    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn sweep_does_not_resurrect_a_target_unsubscribed_after_being_snapshotted() {
        let pool = FsWatchPool::new();
        let base = std::env::temp_dir().join("agentmux_fs_watch_pool_sweep_race_test");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        let pid = std::process::id();
        let before = crate::backend::sysinfo::process_handle_count(pid)
            .expect("own handle count must be queryable");

        const ROUNDS: u32 = 200;
        for i in 0..ROUNDS {
            let dir = base.join(i.to_string());
            std::fs::create_dir_all(&dir).unwrap();

            let sub = pool.subscribe_dir(&dir);
            // Captured now, exactly as sweep()'s own snapshot phase would —
            // before the real unsubscribe() below runs.
            let target = sub.watch_target.clone();
            pool.health.mark_degraded(target.clone(), "simulated for test".to_string());

            // The real teardown — exactly what a concurrent unsubscribe()
            // does, landing (for this test) strictly *after* the target was
            // "snapshotted" above but strictly *before* the re-arm action
            // below.
            pool.unsubscribe(sub);

            // sweep()'s per-target action, called directly with the stale
            // (now-unsubscribed) target — the exact call sweep()'s loop
            // would have made had it reached this target after losing the
            // race.
            pool.rearm_if_still_subscribed(target);
        }

        let after = crate::backend::sysinfo::process_handle_count(pid)
            .expect("own handle count must be queryable");
        let grew_by = after.saturating_sub(before);

        let _ = std::fs::remove_dir_all(&base);

        assert!(
            grew_by < ROUNDS / 2,
            "handle count grew by {grew_by} over {ROUNDS} distinct \
             subscribe -> unsubscribe -> stale-rearm rounds (before={before}, \
             after={after}) — rearm_if_still_subscribed() must not \
             re-establish a watch for a target that was unsubscribed after \
             being snapshotted; doing so orphans the handle pair with \
             nothing left to ever unwatch() it"
        );
    }
}
