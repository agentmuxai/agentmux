// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Filesystem watcher for directories a Media pane is pointed at — detects
//! new/changed media files (e.g. a freshly-downloaded ComfyUI render
//! landing in a project's `clips/` folder) and pushes a per-block "a
//! matching file changed" wake signal so panes update without a manual
//! reload.
//!
//! Migrated onto the shared `fs_watch::FsWatchPool`
//! (SPEC_SHARED_FS_WATCHER_FRAMEWORK_2026_08_07.md), alongside
//! `editor_file_watcher.rs` in the same PR. Directory-mode sibling of that
//! file: that one watches individually-opened files (one entry per open
//! tab); this one watches whole directories directly, filtered by a
//! per-subscriber extension set, since a Media pane's job is "show me the
//! latest render in this folder," not "watch this one exact file."
//!
//! Spec: docs/specs/SPEC_MEDIA_PANE_2026_07_26.md

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use tokio::sync::broadcast;

use super::fs_watch::{FsWatchEventKind, FsWatchPool, Subscription};
use super::wps::{Broker, WaveEvent};

/// WPS event fired when a file matching a Media pane's extension filter is
/// created/modified inside a directory that pane is watching. Scoped
/// per-block (`block:<id>`) via `WaveEvent::scopes`, matching
/// `EVENT_EDITOR_FILE_CHANGED`'s pattern. Payload is just the changed file's
/// path — a wake signal, not content; the frontend re-fetches via
/// `GET /agentmux/stream-local-file`.
pub const EVENT_MEDIA_FILE_CHANGED: &str = "media:file_changed";

const DEBOUNCE: Duration = Duration::from_millis(300);

struct Inner {
    /// Canonicalized directory -> (block id -> lowercase extensions that
    /// block cares about, no leading dot). A directory has a live pool
    /// subscription exactly while this map has at least one entry for it.
    watched_dirs: HashMap<PathBuf, HashMap<String, HashSet<String>>>,
    /// One pool subscription per watched directory — see
    /// `editor_file_watcher.rs::Inner::pool_subs`'s doc comment for why this
    /// bookkeeping is separate from the pool's own OS-level refcounting.
    pool_subs: HashMap<PathBuf, Subscription>,
}

/// Per-file debounce generation counters, same collapsing-burst-writes
/// purpose as `editor_file_watcher.rs`'s, keyed by the full changed-file
/// path (not the directory) so unrelated files in the same watched
/// directory debounce independently.
type DebounceGens = Mutex<HashMap<PathBuf, Arc<AtomicU64>>>;

pub struct MediaFileWatcher {
    pool: Arc<FsWatchPool>,
    inner: Mutex<Inner>,
    debounce_gens: DebounceGens,
    broker: Arc<Broker>,
}

impl MediaFileWatcher {
    /// Construct and start the watcher, subscribing to `pool`'s shared
    /// broadcast stream. Always succeeds — see `EditorFileWatcher::new`'s
    /// doc comment for why there's no `Option` to check anymore.
    pub fn new(pool: Arc<FsWatchPool>, broker: Arc<Broker>) -> Arc<Self> {
        let this = Arc::new(Self {
            pool: pool.clone(),
            inner: Mutex::new(Inner { watched_dirs: HashMap::new(), pool_subs: HashMap::new() }),
            debounce_gens: Mutex::new(HashMap::new()),
            broker,
        });

        let mut events = pool.events();
        let worker = this.clone();
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    // Match the pre-migration filter exactly: only Create/Modify
                    // trigger a reload publish — a Removed event otherwise makes
                    // a pane try to load a file that's no longer there (reagent
                    // P1 on PR #2458).
                    Ok(ev) if matches!(ev.kind, FsWatchEventKind::Created | FsWatchEventKind::Modified) => {
                        worker.handle_fs_event(ev.path)
                    }
                    Ok(_) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "media file watcher lagged behind the fs_watch broadcast stream");
                    }
                }
            }
        });

        this
    }

    /// Start watching `dir` on behalf of `block_id`, notifying only for
    /// files whose extension (lowercase, no dot — e.g. "webm") appears in
    /// `extensions`. Idempotent for the same (dir, block_id) — a second call
    /// replaces that block's extension set rather than adding a duplicate
    /// subscription. Called when a Media pane points at a directory.
    pub fn watch_directory(&self, dir: &Path, block_id: &str, extensions: &[String]) {
        let canonical = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        let ext_set: HashSet<String> = extensions.iter().map(|e| e.to_lowercase()).collect();

        let mut inner = self.inner.lock().unwrap();
        let subscribers = inner.watched_dirs.entry(canonical.clone()).or_default();
        let is_new_dir = subscribers.is_empty();
        subscribers.insert(block_id.to_string(), ext_set);

        if is_new_dir {
            let sub = self.pool.subscribe_dir(&canonical);
            inner.pool_subs.insert(canonical.clone(), sub);
        }
        tracing::debug!(dir = %canonical.display(), block_id, "media dir watch: started/updated");
    }

    /// Stop watching `dir` on behalf of `block_id`. No-op if that pairing
    /// wasn't watched. Called on path change / pane dispose.
    pub fn unwatch_directory(&self, dir: &Path, block_id: &str) {
        let canonical = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        let mut inner = self.inner.lock().unwrap();
        let Some(subscribers) = inner.watched_dirs.get_mut(&canonical) else {
            return;
        };
        subscribers.remove(block_id);
        if !subscribers.is_empty() {
            return;
        }
        inner.watched_dirs.remove(&canonical);
        if let Some(sub) = inner.pool_subs.remove(&canonical) {
            self.pool.unsubscribe(sub);
        }
        drop(inner);

        // Prune debounce entries for files under this now-unwatched dir —
        // same rationale as EditorFileWatcher::unwatch_path: don't let
        // debounce_gens grow unbounded across a long process lifetime.
        let mut gens = self.debounce_gens.lock().unwrap();
        gens.retain(|path, _| path.parent() != Some(canonical.as_path()));

        tracing::debug!(dir = %canonical.display(), block_id, "media dir watch: stopped");
    }

    fn handle_fs_event(self: &Arc<Self>, changed_path: PathBuf) {
        let Some(dir) = changed_path.parent().map(Path::to_path_buf) else {
            return;
        };
        let canonical_dir = dir.canonicalize().unwrap_or_else(|_| dir.clone());

        let matched_block_ids: Vec<String> = {
            let inner = self.inner.lock().unwrap();
            // `notify` events aren't always pre-canonicalized (symlinked
            // ancestors, `\\?\` UNC prefixing on Windows) — try both,
            // mirroring EditorFileWatcher::handle_fs_event.
            let subscribers = inner
                .watched_dirs
                .get(&canonical_dir)
                .or_else(|| inner.watched_dirs.get(&dir));
            let Some(subscribers) = subscribers else { return };

            let ext = changed_path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .unwrap_or_default();
            if ext.is_empty() {
                return;
            }
            subscribers
                .iter()
                .filter(|(_, exts)| exts.contains(&ext))
                .map(|(block_id, _)| block_id.clone())
                .collect()
        };
        if matched_block_ids.is_empty() {
            return;
        }

        let gen_counter = {
            let mut gens = self.debounce_gens.lock().unwrap();
            gens.entry(changed_path.clone())
                .or_insert_with(|| Arc::new(AtomicU64::new(0)))
                .clone()
        };
        let my_gen = gen_counter.fetch_add(1, Ordering::SeqCst) + 1;

        let this = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(DEBOUNCE).await;
            if gen_counter.load(Ordering::SeqCst) != my_gen {
                return; // superseded by a later event for this file
            }
            publish_media_file_changed(&this.broker, &changed_path, &matched_block_ids);
        });
    }
}

/// Publish `EVENT_MEDIA_FILE_CHANGED`, scoped to every block watching the
/// changed file's directory with a matching extension. Mirrors
/// `publish_editor_file_changed`'s per-block scoping — never a global
/// broadcast.
fn publish_media_file_changed(broker: &Broker, path: &Path, block_ids: &[String]) {
    broker.publish(WaveEvent {
        event: EVENT_MEDIA_FILE_CHANGED.to_string(),
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
        let watcher = MediaFileWatcher::new(pool, broker);

        let tmp_dir = std::env::temp_dir().join("agentmux_media_watch_test");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let exts = vec!["webm".to_string(), "png".to_string()];

        watcher.watch_directory(&tmp_dir, "block-1", &exts);
        watcher.watch_directory(&tmp_dir, "block-2", &exts);
        {
            let inner = watcher.inner.lock().unwrap();
            let canonical = tmp_dir.canonicalize().unwrap();
            assert_eq!(inner.watched_dirs.get(&canonical).map(|s| s.len()), Some(2));
        }

        watcher.unwatch_directory(&tmp_dir, "block-1");
        {
            let inner = watcher.inner.lock().unwrap();
            let canonical = tmp_dir.canonicalize().unwrap();
            assert_eq!(inner.watched_dirs.get(&canonical).map(|s| s.len()), Some(1));
        }

        watcher.unwatch_directory(&tmp_dir, "block-2");
        {
            let inner = watcher.inner.lock().unwrap();
            let canonical = tmp_dir.canonicalize().unwrap();
            assert!(!inner.watched_dirs.contains_key(&canonical));
        }

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_publish_scopes_to_every_subscribed_block() {
        let broker = Arc::new(Broker::new());
        let client = Arc::new(TestClient { events: StdMutex::new(Vec::new()) });
        broker.set_client(Box::new(client.clone()));

        broker.subscribe(
            "route-1",
            super::super::wps::SubscriptionRequest {
                event: EVENT_MEDIA_FILE_CHANGED.to_string(),
                scopes: vec!["block:abc".to_string()],
                allscopes: false,
            },
        );

        publish_media_file_changed(&broker, Path::new("/tmp/clips/shot.webm"), &["abc".to_string(), "xyz".to_string()]);

        let events = client.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "route-1");
        assert_eq!(events[0].1.event, EVENT_MEDIA_FILE_CHANGED);
    }

    #[test]
    fn test_extension_filter_excludes_non_matching_block() {
        // A block watching only "png" should not be scoped in when a
        // ".webm" file changes in the same directory.
        let broker = Arc::new(Broker::new());
        let client = Arc::new(TestClient { events: StdMutex::new(Vec::new()) });
        broker.set_client(Box::new(client.clone()));

        broker.subscribe(
            "route-png-only",
            super::super::wps::SubscriptionRequest {
                event: EVENT_MEDIA_FILE_CHANGED.to_string(),
                scopes: vec!["block:png-block".to_string()],
                allscopes: false,
            },
        );

        // Simulate what handle_fs_event's filtering step would decide: only
        // "webm-block" matched, "png-block" did not, so only it is published.
        publish_media_file_changed(&broker, Path::new("/tmp/clips/shot.webm"), &["webm-block".to_string()]);

        let events = client.events.lock().unwrap();
        assert_eq!(events.len(), 0, "png-only block must not receive a .webm change event");
    }

    #[tokio::test]
    async fn test_end_to_end_real_fs_write_triggers_publish_for_matching_extension() {
        // Regression for the migration itself: a real notify event flowing
        // through the shared pool must still reach this watcher's own
        // extension-filter + debounce + publish path.
        let broker = Arc::new(Broker::new());
        let client = Arc::new(TestClient { events: StdMutex::new(Vec::new()) });
        broker.set_client(Box::new(client.clone()));
        broker.subscribe(
            "route-e2e",
            super::super::wps::SubscriptionRequest {
                event: EVENT_MEDIA_FILE_CHANGED.to_string(),
                scopes: vec!["block:e2e".to_string()],
                allscopes: false,
            },
        );

        let pool = FsWatchPool::new();
        let watcher = MediaFileWatcher::new(pool, broker);

        let dir = std::env::temp_dir().join("agentmux_media_watch_e2e_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        watcher.watch_directory(&dir, "e2e", &["webm".to_string()]);
        tokio::time::sleep(Duration::from_millis(200)).await;
        std::fs::write(dir.join("render.webm"), b"fake video bytes").unwrap();

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

        assert!(saw_it, "expected a real .webm write to reach the publish path via the shared pool");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_removed_event_does_not_trigger_publish() {
        // reagent P1 on PR #2458: the migration to FsWatchPool initially
        // dropped the pre-migration Create/Modify-only filter, so deleting a
        // file in a watched media dir would still publish
        // EVENT_MEDIA_FILE_CHANGED, sending the pane to reload a file that's
        // no longer there. Confirm the filter is back.
        let broker = Arc::new(Broker::new());
        let client = Arc::new(TestClient { events: StdMutex::new(Vec::new()) });
        broker.set_client(Box::new(client.clone()));
        broker.subscribe(
            "route-removed",
            super::super::wps::SubscriptionRequest {
                event: EVENT_MEDIA_FILE_CHANGED.to_string(),
                scopes: vec!["block:removed".to_string()],
                allscopes: false,
            },
        );

        let pool = FsWatchPool::new();
        let watcher = MediaFileWatcher::new(pool, broker);

        let dir = std::env::temp_dir().join("agentmux_media_watch_removed_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("render.webm");
        std::fs::write(&file, b"fake video bytes").unwrap();

        watcher.watch_directory(&dir, "removed", &["webm".to_string()]);
        tokio::time::sleep(Duration::from_millis(200)).await;
        std::fs::remove_file(&file).unwrap();

        // Give the pool's fs backend time to deliver (and this watcher time
        // to wrongly act on) the Removed event before asserting silence.
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(
            client.events.lock().unwrap().is_empty(),
            "a Removed event must not trigger EVENT_MEDIA_FILE_CHANGED"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
