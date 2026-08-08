// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Filesystem watcher for files open in editor/preview panes — detects
//! external changes (an agent's `Edit`/`Write` tool, another process, a
//! second AgentMux window) and pushes a per-block "this file changed on
//! disk" wake signal so panes can refresh instead of silently going stale.
//!
//! Migrated onto the shared `fs_watch::FsWatchPool`
//! (SPEC_SHARED_FS_WATCHER_FRAMEWORK_2026_08_07.md) — this module keeps only
//! its own domain-specific bookkeeping (which block ids have which paths
//! open, per-path debounce, and the `EVENT_EDITOR_FILE_CHANGED` publish
//! shape). The actual `notify` construction, OS-level watch/unwatch, and
//! recovery-on-failure now live in the pool, shared with every other
//! watcher that migrates onto it (`config_watcher_fs.rs` already has;
//! `media_file_watcher.rs`, below in the same PR, is the directory-mode
//! sibling migrating alongside this one).
//!
//! Spec: docs/specs/SPEC_EDITOR_LIVE_FILE_RELOAD_2026_07_18.md

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use tokio::sync::broadcast;

use super::fs_watch::{FsWatchPool, Subscription};
use super::wps::{Broker, WaveEvent};

/// WPS event fired when a file open in at least one editor tab changes on
/// disk. Scoped per-block (`block:<id>`) via `WaveEvent::scopes`, matching
/// `EVENT_CONTROLLER_STATUS`/`EVENT_BLOCK_ACTIVITY`'s existing pattern —
/// never a global broadcast, so panes on unrelated files aren't notified.
/// Payload is deliberately just the path (a wake signal, not content); the
/// frontend re-fetches via the existing `readeditorfile` RPC.
pub const EVENT_EDITOR_FILE_CHANGED: &str = "editor:file_changed";

const DEBOUNCE: Duration = Duration::from_millis(300);

struct Inner {
    /// Canonicalized file path -> block ids with a tab open on it.
    watched_paths: HashMap<PathBuf, std::collections::HashSet<String>>,
    /// One pool subscription per watched path — held so it can be handed
    /// back to `pool.unsubscribe` once the last block id for that path
    /// unsubscribes. The pool does its own OS-level refcounting internally;
    /// this is a *different* refcount (by block id, which the pool has no
    /// concept of), so both layers of bookkeeping are genuinely needed.
    pool_subs: HashMap<PathBuf, Subscription>,
}

/// Per-path debounce generation counters. A new fs event for a path bumps
/// its counter; the delayed publish task only fires if its captured
/// generation is still current when its sleep completes, so a burst of
/// writes (e.g. an editor's multi-syscall save) collapses into one event.
type DebounceGens = Mutex<HashMap<PathBuf, Arc<AtomicU64>>>;

pub struct EditorFileWatcher {
    pool: Arc<FsWatchPool>,
    inner: Mutex<Inner>,
    debounce_gens: DebounceGens,
    broker: Arc<Broker>,
}

impl EditorFileWatcher {
    /// Construct and start the watcher, subscribing to `pool`'s shared
    /// broadcast stream. Unlike the pre-migration version, this always
    /// succeeds — `FsWatchPool` itself absorbs the "can the OS backend even
    /// construct" failure mode (see its own `health()`), so there's no
    /// `Option` for callers to check anymore.
    pub fn new(pool: Arc<FsWatchPool>, broker: Arc<Broker>) -> Arc<Self> {
        let this = Arc::new(Self {
            pool: pool.clone(),
            inner: Mutex::new(Inner { watched_paths: HashMap::new(), pool_subs: HashMap::new() }),
            debounce_gens: Mutex::new(HashMap::new()),
            broker,
        });

        let mut events = pool.events();
        let worker = this.clone();
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(ev) => worker.handle_fs_event(ev.path),
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "editor file watcher lagged behind the fs_watch broadcast stream");
                    }
                }
            }
        });

        this
    }

    /// Start watching `path` on behalf of `block_id`. Idempotent — calling
    /// again for the same (path, block_id) is a no-op. Called from the
    /// `watcheditorfile` RPC handler when a tab finishes loading a file.
    pub fn watch_path(&self, path: &Path, block_id: &str) {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let mut inner = self.inner.lock().unwrap();
        let block_ids = inner.watched_paths.entry(canonical.clone()).or_default();
        let is_new_path = block_ids.is_empty();
        let newly_inserted = block_ids.insert(block_id.to_string());
        if !is_new_path {
            if newly_inserted {
                tracing::debug!(path = %canonical.display(), block_id, "editor file watch: added subscriber");
            }
            return;
        }

        let sub = self.pool.subscribe_file(&canonical);
        inner.pool_subs.insert(canonical.clone(), sub);
        tracing::debug!(path = %canonical.display(), block_id, "editor file watch: started");
    }

    /// Stop watching `path` on behalf of `block_id`. No-op if that pairing
    /// wasn't watched. Called on tab close / pane dispose.
    pub fn unwatch_path(&self, path: &Path, block_id: &str) {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let mut inner = self.inner.lock().unwrap();
        let Some(block_ids) = inner.watched_paths.get_mut(&canonical) else {
            return;
        };
        block_ids.remove(block_id);
        if !block_ids.is_empty() {
            return;
        }
        inner.watched_paths.remove(&canonical);
        if let Some(sub) = inner.pool_subs.remove(&canonical) {
            self.pool.unsubscribe(sub);
        }
        drop(inner);

        // Prune the debounce-generation entry now that nothing watches this
        // path — otherwise every distinct path ever externally modified
        // during the process's lifetime accumulates here forever, even
        // after its last watcher unsubscribed. Any debounce task already in
        // flight still holds its own Arc clone of the counter, so this is
        // safe to drop from the map immediately; a future re-watch of the
        // same path just mints a fresh counter in handle_fs_event.
        self.debounce_gens.lock().unwrap().remove(&canonical);

        tracing::debug!(path = %canonical.display(), block_id, "editor file watch: stopped");
    }

    fn handle_fs_event(self: &Arc<Self>, changed_path: PathBuf) {
        let canonical = changed_path.canonicalize().unwrap_or_else(|_| changed_path.clone());
        let matched = {
            let inner = self.inner.lock().unwrap();
            // `notify` events aren't always pre-canonicalized (symlinked
            // ancestors, `\\?\` UNC prefixing on Windows) — fall back to a
            // raw-path match so we don't silently miss a watched file.
            if inner.watched_paths.contains_key(&canonical) {
                Some(canonical.clone())
            } else if inner.watched_paths.contains_key(&changed_path) {
                Some(changed_path.clone())
            } else {
                None
            }
        };
        let Some(path) = matched else { return };

        let gen_counter = {
            let mut gens = self.debounce_gens.lock().unwrap();
            gens.entry(path.clone())
                .or_insert_with(|| Arc::new(AtomicU64::new(0)))
                .clone()
        };
        let my_gen = gen_counter.fetch_add(1, Ordering::SeqCst) + 1;

        let this = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(DEBOUNCE).await;
            if gen_counter.load(Ordering::SeqCst) != my_gen {
                return; // superseded by a later event for this path
            }
            let block_ids: Vec<String> = {
                let inner = this.inner.lock().unwrap();
                inner
                    .watched_paths
                    .get(&path)
                    .map(|s| s.iter().cloned().collect())
                    .unwrap_or_default()
            };
            if block_ids.is_empty() {
                return; // last watcher unsubscribed while we were debouncing
            }
            publish_editor_file_changed(&this.broker, &path, &block_ids);
        });
    }
}

/// Publish `EVENT_EDITOR_FILE_CHANGED`, scoped to every block that has a tab
/// open on `path`. Mirrors `publish_block_activity`'s per-block scoping
/// (`agentmux-srv/src/backend/wps.rs`) — not a global broadcast.
fn publish_editor_file_changed(broker: &Broker, path: &Path, block_ids: &[String]) {
    broker.publish(WaveEvent {
        event: EVENT_EDITOR_FILE_CHANGED.to_string(),
        scopes: block_ids.iter().map(|id| format!("block:{id}")).collect(),
        sender: String::new(),
        persist: 0,
        data: Some(json!({ "path": path.to_string_lossy() })),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    struct TestClient {
        events: StdMutex<Vec<(String, WaveEvent)>>,
    }

    impl super::super::wps::WpsClient for Arc<TestClient> {
        fn send_event(&self, route_id: &str, event: WaveEvent) {
            self.events.lock().unwrap().push((route_id.to_string(), event));
        }
    }

    #[tokio::test]
    async fn test_watch_unwatch_refcounting_does_not_panic() {
        let broker = Arc::new(Broker::new());
        let pool = FsWatchPool::new();
        let watcher = EditorFileWatcher::new(pool, broker);

        let tmp = std::env::temp_dir().join("agentmux_editor_watch_test.txt");
        std::fs::write(&tmp, "hello").unwrap();

        watcher.watch_path(&tmp, "block-1");
        watcher.watch_path(&tmp, "block-2");
        {
            let inner = watcher.inner.lock().unwrap();
            let canonical = tmp.canonicalize().unwrap();
            assert_eq!(inner.watched_paths.get(&canonical).map(|s| s.len()), Some(2));
        }

        watcher.unwatch_path(&tmp, "block-1");
        {
            let inner = watcher.inner.lock().unwrap();
            let canonical = tmp.canonicalize().unwrap();
            assert_eq!(inner.watched_paths.get(&canonical).map(|s| s.len()), Some(1));
        }

        watcher.unwatch_path(&tmp, "block-2");
        {
            let inner = watcher.inner.lock().unwrap();
            let canonical = tmp.canonicalize().unwrap();
            assert!(!inner.watched_paths.contains_key(&canonical));
        }

        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn test_unwatch_prunes_debounce_gen_entry() {
        // Regression: debounce_gens used to only ever grow — an entry was
        // minted on the first fs event for a path but never removed when
        // the last watcher for that path unsubscribed, so the map grew
        // unbounded over a long-running process's lifetime.
        let broker = Arc::new(Broker::new());
        let pool = FsWatchPool::new();
        let watcher = EditorFileWatcher::new(pool, broker);

        let tmp = std::env::temp_dir().join("agentmux_editor_watch_debounce_test.txt");
        std::fs::write(&tmp, "hello").unwrap();
        let canonical = tmp.canonicalize().unwrap();

        watcher.watch_path(&tmp, "block-1");
        // Simulate a debounce-generation entry the way handle_fs_event would
        // create one, without depending on a real fs-event round trip.
        watcher.debounce_gens.lock().unwrap().insert(canonical.clone(), Arc::new(AtomicU64::new(3)));
        assert!(watcher.debounce_gens.lock().unwrap().contains_key(&canonical));

        watcher.unwatch_path(&tmp, "block-1");
        assert!(
            !watcher.debounce_gens.lock().unwrap().contains_key(&canonical),
            "debounce_gens entry must be pruned once the last watcher for a path unsubscribes"
        );

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_publish_scopes_to_every_subscribed_block() {
        let broker = Arc::new(Broker::new());
        let client = Arc::new(TestClient { events: StdMutex::new(Vec::new()) });
        broker.set_client(Box::new(client.clone()));

        broker.subscribe(
            "route-1",
            super::super::wps::SubscriptionRequest {
                event: EVENT_EDITOR_FILE_CHANGED.to_string(),
                scopes: vec!["block:abc".to_string()],
                allscopes: false,
            },
        );

        publish_editor_file_changed(&broker, Path::new("/tmp/foo.md"), &["abc".to_string(), "xyz".to_string()]);

        let events = client.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "route-1");
        assert_eq!(events[0].1.event, EVENT_EDITOR_FILE_CHANGED);
    }

    #[tokio::test]
    async fn test_end_to_end_real_fs_write_triggers_publish() {
        // Regression for the migration itself: a real notify event flowing
        // through the shared pool must still reach this watcher's own
        // debounce + publish path, not just the bookkeeping unit tests above.
        let broker = Arc::new(Broker::new());
        let client = Arc::new(TestClient { events: StdMutex::new(Vec::new()) });
        broker.set_client(Box::new(client.clone()));
        broker.subscribe(
            "route-e2e",
            super::super::wps::SubscriptionRequest {
                event: EVENT_EDITOR_FILE_CHANGED.to_string(),
                scopes: vec!["block:e2e".to_string()],
                allscopes: false,
            },
        );

        let pool = FsWatchPool::new();
        let watcher = EditorFileWatcher::new(pool, broker);

        let dir = std::env::temp_dir().join("agentmux_editor_watch_e2e_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("watched.md");
        std::fs::write(&file, "v1").unwrap();

        watcher.watch_path(&file, "e2e");
        tokio::time::sleep(Duration::from_millis(200)).await;
        std::fs::write(&file, "v2").unwrap();

        let saw_it = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if !client.events.lock().unwrap().is_empty() {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap_or(false);

        assert!(saw_it, "expected a real on-disk change to reach the publish path via the shared pool");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
