// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Filesystem watcher for files open in editor/preview panes — detects
//! external changes (an agent's `Edit`/`Write` tool, another process, a
//! second AgentMux window) and pushes a per-block "this file changed on
//! disk" wake signal so panes can refresh instead of silently going stale.
//!
//! Generalizes `config_watcher_fs.rs`'s single-file `notify`-watcher pattern
//! to an arbitrary, dynamically-changing set of paths anywhere under the
//! user's home directory — one per open editor tab, refcounted by block id
//! so a path is watched only while at least one tab has it open.
//!
//! Spec: docs/specs/SPEC_EDITOR_LIVE_FILE_RELOAD_2026_07_18.md

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::json;
use tokio::sync::mpsc;

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
    /// Parent directory -> number of distinct watched paths under it.
    /// `notify` watches directories (non-recursive) rather than individual
    /// files, matching `config_watcher_fs.rs`'s approach; this refcount
    /// decides when a directory watch is added/removed.
    watched_dirs: HashMap<PathBuf, usize>,
}

/// Per-path debounce generation counters. A new fs event for a path bumps
/// its counter; the delayed publish task only fires if its captured
/// generation is still current when its sleep completes, so a burst of
/// writes (e.g. an editor's multi-syscall save) collapses into one event.
type DebounceGens = Mutex<HashMap<PathBuf, Arc<AtomicU64>>>;

pub struct EditorFileWatcher {
    // Held only to keep the watcher alive — never read directly outside `new`.
    _watcher: Mutex<RecommendedWatcher>,
    inner: Mutex<Inner>,
    debounce_gens: DebounceGens,
    broker: Arc<Broker>,
}

impl EditorFileWatcher {
    /// Construct and start the watcher. Returns `None` if the underlying
    /// `notify` watcher can't be created (matches
    /// `config_watcher_fs::spawn_settings_watcher`'s fallible-but-non-fatal
    /// convention — live-reload is a nice-to-have, not a boot requirement).
    pub fn new(broker: Arc<Broker>) -> Option<Arc<Self>> {
        let (tx, mut rx) = mpsc::unbounded_channel::<PathBuf>();

        let watcher = match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            match res {
                Ok(event) => {
                    if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                        for path in event.paths {
                            let _ = tx.send(path);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "editor file watcher error");
                }
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(error = %e, "failed to create editor file watcher");
                return None;
            }
        };

        let this = Arc::new(Self {
            _watcher: Mutex::new(watcher),
            inner: Mutex::new(Inner {
                watched_paths: HashMap::new(),
                watched_dirs: HashMap::new(),
            }),
            debounce_gens: Mutex::new(HashMap::new()),
            broker,
        });

        let worker = this.clone();
        tokio::spawn(async move {
            while let Some(changed_path) = rx.recv().await {
                worker.handle_fs_event(changed_path);
            }
        });

        Some(this)
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

        let Some(dir) = canonical.parent().map(Path::to_path_buf) else {
            return;
        };
        let refcount = inner.watched_dirs.entry(dir.clone()).or_insert(0);
        *refcount += 1;
        if *refcount == 1 {
            let mut w = self._watcher.lock().unwrap();
            if let Err(e) = w.watch(&dir, RecursiveMode::NonRecursive) {
                tracing::warn!(dir = %dir.display(), error = %e, "failed to watch editor file directory");
            }
        }
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

        let Some(dir) = canonical.parent().map(Path::to_path_buf) else {
            return;
        };
        if let Some(refcount) = inner.watched_dirs.get_mut(&dir) {
            *refcount -= 1;
            if *refcount == 0 {
                inner.watched_dirs.remove(&dir);
                let mut w = self._watcher.lock().unwrap();
                let _ = w.unwatch(&dir);
            }
        }
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
        // Exercises the refcount bookkeeping without touching the real
        // filesystem watcher backend (CI sandboxes may not support inotify).
        let broker = Arc::new(Broker::new());
        let watcher = EditorFileWatcher::new(broker).expect("watcher should construct");

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
}
