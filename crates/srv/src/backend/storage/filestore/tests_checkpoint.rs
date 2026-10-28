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

/// An idle store is looked at less and less often (checkpointer.rs `IDLE_BACKOFF`); the first writes
/// after a long quiet stretch must still be noticed and checkpointed within a few looks.
#[test]
fn a_checkpoint_still_runs_promptly_when_writes_resume_after_a_long_quiet_stretch() {
    let (_dir, path, conn) = wal_db();
    let shm = sidecar(&path, "-shm");
    write_until(&conn, &shm, 300);
    let poll = Duration::from_millis(10);
    let checkpointer = Checkpointer::start_with(&path, poll, FRAMES_THRESHOLD).unwrap();
    // Long enough for the wait to have grown to its largest (80 ms) and stayed there.
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(checkpointer.runs(), 0);

    write_until(&conn, &shm, FRAMES_THRESHOLD);
    // From the end of the burst: the burst itself is a few MB of writes, which a slow disk (a CI runner's)
    // takes seconds over, and the checkpointer may have run during it.
    let resumed = Instant::now();
    wait_for("a checkpoint after the writes resumed", || checkpointer.runs() >= 1);
    assert!(
        resumed.elapsed() < Duration::from_secs(2),
        "took {:?} after the writes: the slowest wait is 80 ms, so a few looks at most",
        resumed.elapsed()
    );
    wait_for("the backlog to be copied back", || wal_backlog(&shm) == Some(0));
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

// ---------------------------------------------------------------------------
// Reading the WAL index must not close a handle on the `-shm` file: on Unix that
// releases every lock the process holds on it, SQLite's own included.
// ---------------------------------------------------------------------------

/// Run by `reading_the_wal_index_leaves_the_writers_lock_in_place` in a process of its own:
/// try to take the database's write lock without waiting, and say whether it was free.
#[test]
#[ignore]
fn lock_probe_child() {
    let Some(db) = std::env::var_os("LOCK_PROBE_DB") else { return };
    let conn = Connection::open(db).unwrap();
    conn.execute_batch("PRAGMA busy_timeout=0;").unwrap();
    match conn.execute_batch("BEGIN IMMEDIATE") {
        Ok(()) => println!("PROBE-GOT-THE-WRITE-LOCK"),
        Err(_) => println!("PROBE-WRITE-LOCK-BUSY"),
    }
}

#[cfg(unix)]
#[test]
fn reading_the_wal_index_leaves_the_writers_lock_in_place() {
    let (_dir, path, conn) = wal_db();
    let shm = sidecar(&path, "-shm");
    conn.execute("INSERT INTO t (b) VALUES (zeroblob(100))", []).unwrap();
    // A write transaction in progress, as the framer's is for a good part of a second.
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    conn.execute("INSERT INTO t (b) VALUES (zeroblob(100))", []).unwrap();
    for _ in 0..5 {
        assert!(wal_backlog(&shm).is_some());
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "backend::storage::filestore::tests_checkpoint::lock_probe_child", "--ignored", "--nocapture", "--test-threads=1"])
        .env("LOCK_PROBE_DB", &path)
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.contains("PROBE-WRITE-LOCK-BUSY"),
        "another process got the write lock while a transaction held it, after the WAL index was read: {out}"
    );
    conn.execute_batch("ROLLBACK").unwrap();
}

#[cfg(unix)]
#[test]
fn the_kept_handle_on_the_wal_index_is_not_inherited_by_child_processes() {
    use std::os::unix::fs::MetadataExt;
    let (_dir, path, conn) = wal_db();
    let shm = sidecar(&path, "-shm");
    conn.execute("INSERT INTO t (b) VALUES (zeroblob(100))", []).unwrap();
    assert!(wal_backlog(&shm).is_some());
    let want = std::fs::metadata(&shm).unwrap();
    let want = (want.dev(), want.ino());
    // SQLite keeps handles on the same file; every one of them, and ours, is close-on-exec.
    let mut seen = 0;
    for fd in 3..1024 {
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(fd, &mut st) } != 0 || (st.st_dev as u64, st.st_ino as u64) != want {
            continue;
        }
        seen += 1;
        assert!(unsafe { libc::fcntl(fd, libc::F_GETFD) } & libc::FD_CLOEXEC != 0, "fd {fd} on the -shm file is inherited by children");
    }
    assert!(seen >= 1, "no open handle on the -shm file found");
}

#[test]
fn a_wal_index_that_is_replaced_or_gone_is_read_afresh_or_not_at_all() {
    let (_dir, path, conn) = wal_db();
    let shm = sidecar(&path, "-shm");
    write_until(&conn, &shm, 50);
    assert!(wal_backlog(&shm).unwrap() >= 50);
    // Every connection closed: the -shm goes away and the handle kept for it must not answer for it.
    drop(conn);
    assert_eq!(wal_backlog(&shm), None, "no index: the checkpointer falls back to the WAL's write time");
    // A new connection creates a new index, which starts again from nothing.
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(&format!("PRAGMA wal_autocheckpoint=0; {JOURNAL_SIZE_LIMIT}")).unwrap();
    conn.execute("INSERT INTO t (b) VALUES (zeroblob(3000))", []).unwrap();
    let backlog = wal_backlog(&shm).expect("the new index is read");
    assert!(backlog >= 1 && backlog < 50, "read from the old index: {backlog}");
}
