// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Directory watcher for the Files pane (`fs.watch` / `fs.unwatch`).
//!
//! Sibling of `media_file_watcher.rs` on the same shared `FsWatchPool`, with
//! a different contract: a Files pane re-lists the whole folder on any
//! change, so this publishes a coalesced "this folder changed" signal per
//! directory rather than per file, for creates, modifications and removals
//! alike.
//!
//! Spec: docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §6.3, §10.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use tokio::sync::broadcast;

use super::fs_ops::platform::display_path;
use super::fs_watch::{FsWatchPool, Subscription};
use super::mps::{Broker, MuxEvent};

/// MPS event telling a Files pane to re-list a folder. Scoped to
/// `block:<id>`; payload `{ dir }`, the folder's display path exactly as
/// `fs.list` returns it.
pub const EVENT_FILES_CHANGED: &str = "files:changed";

/// Changes to one folder within this window are folded into one event
/// (spec §10: one patch per ~100 ms window, applied within 200 ms).
const COALESCE: Duration = Duration::from_millis(150);

/// Watches one block may hold. Past this the oldest is dropped (spec §10:
/// "hard cap, LRU"). Also bounds what a pane that never unwatches (a
/// window reload skips its dispose) can leak.
pub const MAX_WATCHES_PER_BLOCK: usize = 64;

struct Watch {
    block_id: String,
    dir: PathBuf,
    sub: Subscription,
}

#[derive(Default)]
struct Inner {
    watches: HashMap<String, Watch>,
    /// Each block's watch ids, oldest first.
    by_block: HashMap<String, VecDeque<String>>,
    /// Canonical directory -> (display path, watch ids on it).
    by_dir: HashMap<PathBuf, (String, HashSet<String>)>,
    /// Directories with a publish already scheduled.
    pending: HashSet<PathBuf>,
}

pub struct FilesWatcher {
    pool: Arc<FsWatchPool>,
    broker: Arc<Broker>,
    inner: Mutex<Inner>,
}

impl FilesWatcher {
    /// Construct and start the watcher on `pool`'s shared event stream.
    pub fn new(pool: Arc<FsWatchPool>, broker: Arc<Broker>) -> Arc<Self> {
        let this = Arc::new(Self { pool: pool.clone(), broker, inner: Mutex::new(Inner::default()) });
        let mut events = pool.events();
        let worker = this.clone();
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(ev) => worker.on_fs_event(&ev.path),
                    Err(broadcast::error::RecvError::Closed) => break,
                    // Events were dropped, so any folder may have changed
                    // unseen: re-list them all. The frontend re-lists on each
                    // event, so this is a full rescan.
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "files watcher lagged behind the fs_watch stream; rescanning every watched folder");
                        worker.on_lagged();
                    }
                }
            }
        });
        this
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Watch the canonical directory `dir` for `block_id`; returns the id to
    /// unwatch with. Each call is its own registration (the pool
    /// reference-counts the OS watch), so two panes on one folder, or one
    /// pane watching it twice, never cancel each other.
    pub fn watch(&self, dir: &Path, block_id: &str) -> String {
        let watch_id = uuid::Uuid::new_v4().to_string();
        let sub = self.pool.subscribe_dir(dir);
        let mut inner = self.lock();
        inner.watches.insert(watch_id.clone(), Watch { block_id: block_id.to_string(), dir: dir.to_path_buf(), sub });
        inner
            .by_dir
            .entry(dir.to_path_buf())
            .or_insert_with(|| (display_path(dir), HashSet::new()))
            .1
            .insert(watch_id.clone());
        let ids = inner.by_block.entry(block_id.to_string()).or_default();
        ids.push_back(watch_id.clone());
        let evicted: Vec<String> = if ids.len() > MAX_WATCHES_PER_BLOCK {
            ids.drain(..ids.len() - MAX_WATCHES_PER_BLOCK).collect()
        } else {
            Vec::new()
        };
        for id in &evicted {
            self.remove_locked(&mut inner, id);
        }
        if !evicted.is_empty() {
            tracing::debug!(block_id, dropped = evicted.len(), "fs.watch: per-block cap reached, dropped the oldest");
        }
        tracing::debug!(dir = %dir.display(), block_id, watch_id, "fs.watch: started");
        watch_id
    }

    /// Stop a watch. An unknown id (already dropped by the cap, or never
    /// issued) is not an error.
    pub fn unwatch(&self, watch_id: &str) {
        let mut inner = self.lock();
        let Some(block_id) = inner.watches.get(watch_id).map(|w| w.block_id.clone()) else {
            return;
        };
        if let Some(ids) = inner.by_block.get_mut(&block_id) {
            ids.retain(|id| id != watch_id);
            if ids.is_empty() {
                inner.by_block.remove(&block_id);
            }
        }
        self.remove_locked(&mut inner, watch_id);
    }

    /// Drop `watch_id` from `watches` and `by_dir` and release its pool
    /// subscription. The caller has already removed it from `by_block`.
    fn remove_locked(&self, inner: &mut Inner, watch_id: &str) {
        let Some(w) = inner.watches.remove(watch_id) else { return };
        if let Some((_, ids)) = inner.by_dir.get_mut(&w.dir) {
            ids.remove(watch_id);
            if ids.is_empty() {
                inner.by_dir.remove(&w.dir);
            }
        }
        self.pool.unsubscribe(w.sub);
    }

    /// A raw pool event. A change to an entry means its parent folder's
    /// listing changed; a change to a watched folder itself (removed,
    /// renamed) is reported for that folder too, so its pane re-lists and
    /// finds it gone.
    fn on_fs_event(self: &Arc<Self>, path: &Path) {
        let parent = path.parent();
        let (parent_hit, self_hit) = {
            let inner = self.lock();
            // The pool is shared with every other watcher; most events are
            // not for us.
            if inner.by_dir.is_empty() {
                return;
            }
            (parent.filter(|p| inner.by_dir.contains_key(*p)).map(Path::to_path_buf), inner.by_dir.contains_key(path))
        };
        // `notify` usually reports paths under the exact (canonical) path
        // that was watched, but not always (symlinked ancestors, `\\?\`
        // prefixing on Windows), so fall back to the canonical form, as
        // `media_file_watcher` does. Outside the lock: it is a syscall.
        let parent_hit = parent_hit.or_else(|| {
            let canonical = parent?.canonicalize().ok()?;
            self.lock().by_dir.contains_key(&canonical).then_some(canonical)
        });
        if let Some(dir) = parent_hit {
            self.schedule(dir);
        }
        if self_hit {
            self.schedule(path.to_path_buf());
        }
    }

    fn on_lagged(self: &Arc<Self>) {
        let dirs: Vec<PathBuf> = self.lock().by_dir.keys().cloned().collect();
        for dir in dirs {
            self.schedule(dir);
        }
    }

    /// Publish for `dir` once `COALESCE` from now, unless a publish is
    /// already scheduled, in which case this change rides along with it.
    fn schedule(self: &Arc<Self>, dir: PathBuf) {
        if !self.lock().pending.insert(dir.clone()) {
            return;
        }
        let this = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(COALESCE).await;
            let target = {
                let mut inner = this.lock();
                inner.pending.remove(&dir);
                // Re-read the watchers now: one may have unwatched in the
                // meantime.
                inner.by_dir.get(&dir).map(|(display, ids)| {
                    let blocks: HashSet<String> =
                        ids.iter().filter_map(|id| inner.watches.get(id)).map(|w| w.block_id.clone()).collect();
                    (display.clone(), blocks)
                })
            };
            if let Some((display, blocks)) = target {
                publish_files_changed(&this.broker, &display, blocks.iter());
            }
        });
    }

    #[cfg(test)]
    fn watch_count(&self, block_id: &str) -> usize {
        self.lock().by_block.get(block_id).map_or(0, VecDeque::len)
    }
}

/// Publish `EVENT_FILES_CHANGED` for `display_dir`, scoped to each block.
/// Never a global broadcast.
fn publish_files_changed<'a>(broker: &Broker, display_dir: &str, block_ids: impl Iterator<Item = &'a String>) {
    let scopes: Vec<String> = block_ids.map(|id| format!("block:{id}")).collect();
    if scopes.is_empty() {
        return;
    }
    broker.publish(MuxEvent {
        event: EVENT_FILES_CHANGED.to_string(),
        scopes,
        sender: String::new(),
        persist: 0,
        data: Some(json!({ "dir": display_dir })),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    struct TestClient {
        events: StdMutex<Vec<(String, MuxEvent)>>,
    }

    impl super::super::mps::WpsClient for Arc<TestClient> {
        fn send_event(&self, route_id: &str, event: MuxEvent) {
            self.events.lock().unwrap().push((route_id.to_string(), event));
        }
    }

    fn subscribed_broker(route: &str, block: &str) -> (Arc<Broker>, Arc<TestClient>) {
        let broker = Arc::new(Broker::new());
        let client = Arc::new(TestClient { events: StdMutex::new(Vec::new()) });
        broker.set_client(Box::new(client.clone()));
        broker.subscribe(
            route,
            super::super::mps::SubscriptionRequest {
                event: EVENT_FILES_CHANGED.to_string(),
                scopes: vec![format!("block:{block}")],
                allscopes: false,
            },
        );
        (broker, client)
    }

    #[tokio::test]
    async fn each_block_keeps_at_most_the_cap_dropping_the_oldest() {
        let watcher = FilesWatcher::new(FsWatchPool::new(), Arc::new(Broker::new()));
        let dir = tempfile::tempdir().unwrap();
        let canonical = dir.path().canonicalize().unwrap();
        let first = watcher.watch(&canonical, "b1");
        for _ in 0..MAX_WATCHES_PER_BLOCK {
            watcher.watch(&canonical, "b1");
        }
        assert_eq!(watcher.watch_count("b1"), MAX_WATCHES_PER_BLOCK);
        assert!(!watcher.lock().watches.contains_key(&first), "the oldest was dropped");

        // Another block is unaffected by b1's cap.
        watcher.watch(&canonical, "b2");
        assert_eq!(watcher.watch_count("b2"), 1);
    }

    #[tokio::test]
    async fn unwatch_releases_and_unknown_ids_are_fine() {
        let watcher = FilesWatcher::new(FsWatchPool::new(), Arc::new(Broker::new()));
        let dir = tempfile::tempdir().unwrap();
        let canonical = dir.path().canonicalize().unwrap();
        let a = watcher.watch(&canonical, "b1");
        let b = watcher.watch(&canonical, "b2");
        watcher.unwatch(&a);
        assert!(watcher.lock().by_dir.contains_key(&canonical), "b2 still watches it");
        watcher.unwatch(&b);
        assert!(watcher.lock().by_dir.is_empty());
        assert!(watcher.lock().by_block.is_empty());
        watcher.unwatch(&b);
        watcher.unwatch("never-issued");
    }

    #[test]
    fn publish_is_scoped_to_the_watching_blocks() {
        let (broker, client) = subscribed_broker("route-1", "abc");
        publish_files_changed(&broker, "/tmp/x", ["abc".to_string(), "xyz".to_string()].iter());
        let events = client.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].1.event, EVENT_FILES_CHANGED);
        assert_eq!(events[0].1.data.as_ref().unwrap()["dir"], "/tmp/x");
        assert_eq!(events[0].1.persist, 0);
    }

    #[tokio::test]
    async fn a_real_change_publishes_once_for_its_folder() {
        let (broker, client) = subscribed_broker("route-e2e", "e2e");
        let watcher = FilesWatcher::new(FsWatchPool::new(), broker);
        let dir = tempfile::tempdir().unwrap();
        let canonical = dir.path().canonicalize().unwrap();
        watcher.watch(&canonical, "e2e");
        tokio::time::sleep(Duration::from_millis(200)).await;

        // A burst of changes inside one coalescing window.
        for i in 0..5 {
            std::fs::write(canonical.join(format!("f{i}.txt")), "x").unwrap();
        }
        let saw = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if !client.events.lock().unwrap().is_empty() {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap_or(false);
        assert!(saw, "a write in a watched folder must publish files:changed");

        tokio::time::sleep(Duration::from_millis(400)).await;
        let events = client.events.lock().unwrap();
        assert!(events.len() <= 2, "a burst is coalesced, got {} events", events.len());
        assert_eq!(events[0].1.data.as_ref().unwrap()["dir"], display_path(&canonical));
    }

    #[tokio::test]
    async fn lag_rescans_every_watched_folder() {
        let (broker, client) = subscribed_broker("route-lag", "lag");
        let watcher = FilesWatcher::new(FsWatchPool::new(), broker);
        let (d1, d2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        watcher.watch(&d1.path().canonicalize().unwrap(), "lag");
        watcher.watch(&d2.path().canonicalize().unwrap(), "lag");
        watcher.on_lagged();
        tokio::time::sleep(COALESCE + Duration::from_millis(200)).await;
        let dirs: HashSet<String> = client
            .events
            .lock()
            .unwrap()
            .iter()
            .map(|(_, e)| e.data.as_ref().unwrap()["dir"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(dirs.len(), 2, "one event per watched folder");
    }
}
