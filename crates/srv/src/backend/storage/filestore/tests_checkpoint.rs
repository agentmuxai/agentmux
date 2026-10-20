// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Background checkpoints run on frames waiting in the WAL, not on the WAL file's
//! size (checkpointer.rs): an idle store runs none. And the shared store's WAL is
//! cut back by one srv, off the writer's mutex.

use super::checkpointer::{sidecar, wal_backlog, Checkpointer, FRAMES_THRESHOLD, JOURNAL_SIZE_LIMIT};
use super::*;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn wal_db() -> (tempfile::TempDir, PathBuf, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(&format!(
        "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; {JOURNAL_SIZE_LIMIT}
         CREATE TABLE t (id INTEGER PRIMARY KEY, b BLOB);"
    ))
    .unwrap();
    (dir, path, conn)
}

/// Commit small writes until the WAL holds at least `frames` not yet copied back.
fn write_until(conn: &Connection, shm: &std::path::Path, frames: u32) {
    while wal_backlog(shm).unwrap() < frames {
        conn.execute("INSERT INTO t (b) VALUES (zeroblob(3000))", []).unwrap();
    }
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_wal_index_says_how_many_frames_wait_to_be_copied_back() {
    let (_dir, path, conn) = wal_db();
    let shm = sidecar(&path, "-shm");
    write_until(&conn, &shm, 300);
    let waiting = wal_backlog(&shm).unwrap();
    // PASSIVE reports (busy, frames in the WAL, frames copied back): nothing was
    // copied before, so the log length is what the index called the backlog.
    let (busy, log, copied) =
        conn.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))).unwrap();
    assert_eq!((busy, log as u32, copied as u32), (0, waiting, waiting));
    assert_eq!(wal_backlog(&shm), Some(0), "all copied back");
}

#[test]
fn an_idle_store_runs_no_checkpoint_and_a_busy_one_runs_until_it_has_caught_up() {
    let (_dir, path, conn) = wal_db();
    let shm = sidecar(&path, "-shm");
    write_until(&conn, &shm, 300);
    assert!(wal_backlog(&shm).unwrap() < FRAMES_THRESHOLD, "the test needs a backlog under the threshold");
    let checkpointer = Checkpointer::start_with(&path, Duration::from_millis(10), FRAMES_THRESHOLD).unwrap();

    // Frames are waiting but too few: many polls go by and none checkpoints.
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(checkpointer.runs(), 0, "idle with a small backlog");

    // Past the threshold it runs, and once caught up it stops.
    write_until(&conn, &shm, FRAMES_THRESHOLD);
    wait_for("a checkpoint", || checkpointer.runs() >= 1);
    wait_for("the backlog to be copied back", || wal_backlog(&shm) == Some(0));
    let runs = checkpointer.runs();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(checkpointer.runs(), runs, "caught up: idle again, no checkpoint");
}

/// The old trigger: the WAL *file* at 4 MB. After a burst it stays that size
/// (SQLite only cuts it back with a TRUNCATE or `journal_size_limit`), so it would
/// have checkpointed every poll for ever.
#[test]
fn a_large_wal_file_with_nothing_waiting_is_left_alone() {
    let (_dir, path, conn) = wal_db();
    let shm = sidecar(&path, "-shm");
    write_until(&conn, &shm, FRAMES_THRESHOLD);
    conn.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(())).unwrap();
    let wal_len = std::fs::metadata(sidecar(&path, "-wal")).unwrap().len();
    assert!(wal_len > 1000 * 1024, "the WAL file keeps its size after a checkpoint ({wal_len} bytes)");
    let checkpointer = Checkpointer::start_with(&path, Duration::from_millis(10), FRAMES_THRESHOLD).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(checkpointer.runs(), 0);
}

#[test]
fn only_one_srv_owns_the_truncate_and_another_takes_over_when_it_goes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("filestore.db");
    let store = FileStore::open(&path).unwrap();
    let mut first = TruncateOwner::for_store(&store).unwrap();
    let mut second = TruncateOwner::for_store(&store).unwrap();
    assert!(first.is_owner());
    assert!(!second.is_owner(), "the lock is taken");
    assert!(first.is_owner(), "and stays with its owner");
    drop(first);
    assert!(second.is_owner(), "released when the owner is gone");
    assert!(TruncateOwner::for_store(&FileStore::open_in_memory().unwrap()).is_none(), "nothing shared in memory");
}

#[test]
fn the_truncate_runs_without_the_writer_mutex_and_cuts_the_wal_file_back() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(FileStore::open(&dir.path().join("filestore.db")).unwrap());
    store.make_file("z", "output", FileMeta::default(), FileOpts::default()).unwrap();
    for _ in 0..40 {
        store.append_lines("z", "output", &vec![b'x'; 60_000]).unwrap();
    }
    let wal = dir.path().join("filestore.db-wal");
    assert!(std::fs::metadata(&wal).unwrap().len() > 0);
    let held = store.lock_conn();
    let apart = {
        let store = Arc::clone(&store);
        std::thread::spawn(move || store.checkpoint_truncate_apart())
    };
    wait_for("the truncate while the writer is held", || apart.is_finished());
    assert!(apart.join().unwrap().unwrap(), "ran to the end");
    drop(held);
    assert_eq!(std::fs::metadata(&wal).unwrap().len(), 0, "the WAL file is cut back");
}
