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
//! stay in the WAL until a later pass.
//!
//! **When it runs** is decided by the frames the WAL holds that have not been
//! copied back yet (`mxFrame - nBackfill`, read from the WAL index), not by the
//! WAL *file's* size: a WAL file never shrinks once it has grown, so a size test
//! stays true for good and every process on the machine would checkpoint four
//! times a second for ever, which is the "second checkpoint shortly after the
//! first" pattern of SQLite's WAL-reset bug (fixed in 3.51.3). An idle store
//! runs no checkpoint. Where the WAL index can't be read, a pass runs only if the
//! WAL file was written since the last look.
//!
//! The file itself is cut back by `journal_size_limit` ([`JOURNAL_SIZE_LIMIT`],
//! set on every connection to the store) when a writer restarts the log, and by
//! the periodic TRUNCATE (`bootstrap::spawn_global_transcripts_truncate_loop`).

#[cfg(not(unix))]
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// Copy the WAL back once this many frames (4 KB pages) are waiting: SQLite's
/// own automatic-checkpoint default.
pub(super) const FRAMES_THRESHOLD: u32 = 1000;
/// How often the WAL index is looked at while the WAL is changing.
const POLL: Duration = Duration::from_millis(250);
/// While nothing in the WAL has changed between looks, the wait doubles up to this many times `POLL`.
/// An idle srv then looks every two seconds instead of four times a second, and each look on
/// Windows opens, reads and closes two files.
const IDLE_BACKOFF: u32 = 8;
/// Cap the WAL file at 64 MB when a writer restarts the log after a complete
/// checkpoint. It is per connection, so every connection to the store sets it.
pub(super) const JOURNAL_SIZE_LIMIT: &str = "PRAGMA journal_size_limit=67108864;";

pub(super) struct Checkpointer {
    stop: Arc<AtomicBool>,
    /// Checkpoints this thread has run.
    runs: Arc<AtomicU64>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Checkpointer {
    /// Start checkpointing `db`. `None` if the thread's connection can't be
    /// opened — the caller then keeps SQLite's automatic checkpoints.
    pub(super) fn start(db: &Path) -> Option<Self> {
        Self::start_with(db, POLL, FRAMES_THRESHOLD)
    }

    pub(super) fn start_with(db: &Path, poll: Duration, threshold: u32) -> Option<Self> {
        let conn = rusqlite::Connection::open(db).ok()?;
        conn.execute_batch(&format!("PRAGMA busy_timeout=5000; {JOURNAL_SIZE_LIMIT}")).ok()?;
        let wal = sidecar(db, "-wal");
        let shm = sidecar(db, "-shm");
        let stop = Arc::new(AtomicBool::new(false));
        let runs = Arc::new(AtomicU64::new(0));
        let (thread_stop, thread_runs) = (Arc::clone(&stop), Arc::clone(&runs));
        let slowest = poll * IDLE_BACKOFF;
        let thread = std::thread::Builder::new()
            .name("filestore-checkpoint".into())
            .spawn(move || {
                let mut last_written: Option<SystemTime> = None;
                let (mut wait, mut last_look) = (poll, None);
                while !thread_stop.load(Ordering::Relaxed) {
                    std::thread::park_timeout(wait);
                    if thread_stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let written = std::fs::metadata(&wal).and_then(|m| m.modified()).ok();
                    let backlog = wal_backlog(&shm);
                    wait = next_wait(wait, poll, slowest, last_look != Some((backlog, written)));
                    last_look = Some((backlog, written));
                    if !due(backlog, threshold, written, &mut last_written) {
                        continue;
                    }
                    thread_runs.fetch_add(1, Ordering::Relaxed);
                    // (busy, frames in the WAL, frames copied back)
                    let pass = conn.query_row("PRAGMA wal_checkpoint(PASSIVE);", [], |r| {
                        Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
                    });
                    match pass {
                        Ok((busy, log, done)) => tracing::trace!(busy, log, done, "filestore: background checkpoint"),
                        Err(e) => tracing::debug!(error = %e, "filestore: background checkpoint failed; retrying next poll"),
                    }
                }
            })
            .ok()?;
        Some(Self { stop, runs, thread: Some(thread) })
    }

    /// Checkpoints run so far.
    #[cfg(test)]
    pub(super) fn runs(&self) -> u64 {
        self.runs.load(Ordering::Relaxed)
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

/// The wait before the next look: `fast` once the WAL changed since the last one, else twice the
/// current wait, up to `slowest`.
fn next_wait(current: Duration, fast: Duration, slowest: Duration, changed: bool) -> Duration {
    if changed {
        fast
    } else {
        (current * 2).min(slowest)
    }
}

/// Whether to checkpoint now. `backlog` is the frames not yet copied back, when
/// the WAL index could be read; without it, `written` (the WAL file's last write)
/// must differ from the one seen at the last look, which is then remembered.
fn due(backlog: Option<u32>, threshold: u32, written: Option<SystemTime>, last_written: &mut Option<SystemTime>) -> bool {
    match backlog {
        Some(frames) => frames >= threshold,
        None => {
            let changed = written.is_some() && written != *last_written;
            *last_written = written;
            changed
        }
    }
}

/// Frames in the WAL that have not been copied back to the database, read from
/// the start of the WAL index (`-shm`): the header (`mxFrame`, the last valid
/// frame) and, after its two copies, the checkpoint info (`nBackfill`, how far
/// the database has caught up). Format: https://www.sqlite.org/walformat.html.
/// A plain read takes no lock; `None` when the file is missing, short, or being
/// rewritten (the two header copies differ), so a pass can fall back.
///
/// **On Unix this never closes a handle on the file.** POSIX advisory locks belong to the
/// process, not to the descriptor: `close()` of *any* descriptor on a file releases *all* of
/// the process's locks on it, including the WAL-index locks SQLite holds for this process's
/// own connections. Opening the `-shm` every poll and dropping the handle took the write
/// lock away from a transaction in progress on this process's writer, so another srv
/// could write at the same time and the store's pages were cross-linked
/// (`framer_tests::stress_parent` reproduces it in seconds; a long framer transaction is
/// the likeliest to be hit). One read-only handle per file is kept for the life of the
/// process instead (https://www.sqlite.org/howtocorrupt.html §2.2).
pub(super) fn wal_backlog(shm: &Path) -> Option<u32> {
    let mut head = [0u8; 100];
    read_index_head(shm, &mut head)?;
    backlog_from_index(&head)
}

#[cfg(unix)]
fn read_index_head(shm: &Path, head: &mut [u8; 100]) -> Option<()> {
    use std::collections::HashMap;
    use std::os::unix::fs::{FileExt, MetadataExt};
    use std::sync::{Mutex, OnceLock};

    /// The handle kept for each `-shm` path, and the file (device, inode) it was opened on.
    static HANDLES: OnceLock<Mutex<HashMap<PathBuf, (std::fs::File, (u64, u64))>>> = OnceLock::new();
    // `metadata` stats the path; it opens nothing, so it is safe to call here.
    let meta = std::fs::metadata(shm).ok()?;
    let id = (meta.dev(), meta.ino());
    let mut handles = HANDLES.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if handles.get(shm).is_none_or(|(_, kept)| *kept != id) {
        // First look, or the file was replaced (every connection closed and a new one made it).
        let file = std::fs::File::open(shm).ok()?;
        if let Some((old, _)) = handles.insert(shm.to_path_buf(), (file, id)) {
            // Never closed: even a handle on a replaced file is left open, for the reason above.
            std::mem::forget(old);
        }
    }
    let (file, _) = handles.get(shm)?;
    file.read_exact_at(head, 0).ok()
}

/// Windows locks belong to the handle, so opening and closing the file here costs the
/// store nothing; and a handle kept open would stop SQLite deleting the file.
#[cfg(not(unix))]
fn read_index_head(shm: &Path, head: &mut [u8; 100]) -> Option<()> {
    std::fs::File::open(shm).ok()?.read_exact(head).ok()
}

fn backlog_from_index(head: &[u8; 100]) -> Option<u32> {
    // WAL index version 3007000, initialised; the two header copies agree.
    let word = |at: usize| u32::from_ne_bytes([head[at], head[at + 1], head[at + 2], head[at + 3]]);
    if word(0) != 3_007_000 || head[12] != 1 || head[..48] != head[48..96] {
        return None;
    }
    Some(word(16).saturating_sub(word(96)))
}

pub(super) fn sidecar(db: &Path, suffix: &str) -> PathBuf {
    let mut name = db.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(version: u32, init: u8, mx_frame: u32, backfill: u32) -> [u8; 100] {
        let mut head = [0u8; 100];
        head[0..4].copy_from_slice(&version.to_ne_bytes());
        head[12] = init;
        head[16..20].copy_from_slice(&mx_frame.to_ne_bytes());
        let (first, second) = head.split_at_mut(48);
        second[..48].copy_from_slice(first);
        head[96..100].copy_from_slice(&backfill.to_ne_bytes());
        head
    }

    #[test]
    fn the_backlog_is_the_frames_after_the_last_one_copied_back() {
        assert_eq!(backlog_from_index(&index(3_007_000, 1, 5000, 4200)), Some(800));
        assert_eq!(backlog_from_index(&index(3_007_000, 1, 0, 0)), Some(0));
        // The log restarted (mxFrame 0) before the checkpoint info was reset.
        assert_eq!(backlog_from_index(&index(3_007_000, 1, 0, 9)), Some(0));
    }

    #[test]
    fn an_index_that_is_not_valid_or_is_being_rewritten_gives_no_backlog() {
        assert_eq!(backlog_from_index(&index(3_006_000, 1, 5000, 0)), None, "another version");
        assert_eq!(backlog_from_index(&index(3_007_000, 0, 5000, 0)), None, "not initialised");
        let mut torn = index(3_007_000, 1, 5000, 0);
        torn[50] ^= 0xff;
        assert_eq!(backlog_from_index(&torn), None, "the two copies differ");
    }

    #[test]
    fn the_wait_doubles_while_nothing_changes_and_drops_back_when_something_does() {
        let (fast, slowest) = (Duration::from_millis(250), Duration::from_millis(2000));
        let mut wait = fast;
        let mut seen = Vec::new();
        for _ in 0..5 {
            wait = next_wait(wait, fast, slowest, false);
            seen.push(wait.as_millis());
        }
        assert_eq!(seen, [500, 1000, 2000, 2000, 2000], "doubles, then stays at the slowest");
        assert_eq!(next_wait(slowest, fast, slowest, true), fast, "a change brings the fast poll straight back");
    }

    #[test]
    fn a_pass_needs_waiting_frames_or_a_write_since_the_last_look() {
        let mut seen = None;
        assert!(!due(Some(999), 1000, None, &mut seen));
        assert!(due(Some(1000), 1000, None, &mut seen));
        // No index: only a changed write time counts, and an idle file is left alone.
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        let t1 = t0 + Duration::from_secs(1);
        assert!(due(None, 1000, Some(t0), &mut seen), "first sight of a written WAL");
        assert!(!due(None, 1000, Some(t0), &mut seen), "nothing written since");
        assert!(due(None, 1000, Some(t1), &mut seen), "written again");
        assert!(!due(None, 1000, None, &mut seen), "no WAL file");
    }
}
