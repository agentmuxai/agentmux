// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Background WAL checkpoints for a file-backed [`FileStore`] (Phase 5a-3,
//! SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md §6.3.7 item 5).
//!
//! With SQLite's automatic checkpoint, whichever write crosses the WAL
//! threshold (1000 pages) runs the checkpoint itself — and every transcript
//! append rewrites a whole 64 KB part, so that happens about every 60 lines,
//! holding that line's event (which since 5a-3 waits for its write) for the
//! checkpoint's duration: the p99 tail. Store connections therefore run with
//! `wal_autocheckpoint=0`, and this thread checkpoints instead, on its own
//! connection: in WAL mode a PASSIVE checkpoint runs alongside writers and
//! readers, taking no lock they wait on.
//!
//! PASSIVE copies what it can and never blocks; pages a reader still needs
//! stay in the WAL until a later pass. The srv's 30-minute TRUNCATE loop
//! (`bootstrap::spawn_wal_checkpoint_loop`) still resets the file.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Checkpoint once the WAL file is at least this large (SQLite's default
/// automatic threshold is 1000 pages of 4 KB).
const WAL_THRESHOLD_BYTES: u64 = 4 * 1024 * 1024;
/// How often the WAL size is looked at.
const POLL: Duration = Duration::from_millis(250);

pub(super) struct Checkpointer {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Checkpointer {
    /// Start checkpointing `db`. `None` if the thread's connection can't be
    /// opened — the caller then keeps SQLite's automatic checkpoints.
    pub(super) fn start(db: &Path) -> Option<Self> {
        let conn = rusqlite::Connection::open(db).ok()?;
        conn.execute_batch("PRAGMA busy_timeout=5000;").ok()?;
        let wal = wal_path(db);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("filestore-checkpoint".into())
            .spawn(move || {
                while !thread_stop.load(Ordering::Relaxed) {
                    std::thread::park_timeout(POLL);
                    if thread_stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let big = std::fs::metadata(&wal).map(|m| m.len() >= WAL_THRESHOLD_BYTES).unwrap_or(false);
                    if big {
                        if let Err(e) = conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);") {
                            tracing::debug!(error = %e, "filestore: background checkpoint failed; retrying next poll");
                        }
                    }
                }
            })
            .ok()?;
        Some(Self { stop, thread: Some(thread) })
    }
}

impl Drop for Checkpointer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

fn wal_path(db: &Path) -> PathBuf {
    let mut name = db.as_os_str().to_os_string();
    name.push("-wal");
    PathBuf::from(name)
}
