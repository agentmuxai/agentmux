// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Reads on a file-backed store go through their own read-only connections
//! (`ReaderPool`, core.rs), so they never wait for the writer's mutex.

use super::*;
use std::sync::Arc;
use std::time::{Duration, Instant};

const ZONE: &str = "zone-readers";

fn store() -> (tempfile::TempDir, Arc<FileStore>) {
    let dir = tempfile::tempdir().unwrap();
    let fs = Arc::new(FileStore::open(&dir.path().join("filestore.db")).unwrap());
    fs.make_file(ZONE, "output", FileMeta::default(), FileOpts::default()).unwrap();
    fs.append_lines(ZONE, "output", b"{\"a\":1}\n{\"b\":2}\n").unwrap();
    (dir, fs)
}

/// The point of the pool: with the writer's mutex held (a long write, or one
/// waiting on another process's write lock), a history read still completes.
#[test]
fn a_read_completes_while_the_writer_holds_its_connection() {
    let (_dir, fs) = store();
    let held = fs.lock_conn();
    let reader = {
        let fs = fs.clone();
        std::thread::spawn(move || {
            let started = Instant::now();
            let size = fs.read_snapshot(|snap| Ok(snap.file(ZONE, "output")?.map(|f| f.size))).unwrap();
            let state = fs.line_state(ZONE, "output").unwrap().unwrap();
            let bytes = fs.read_bytes_db(ZONE, "output", 0, 8).unwrap();
            (started.elapsed(), size, state.size, bytes)
        })
    };
    // Give a mutex-bound read every chance to block before releasing.
    let deadline = Instant::now() + Duration::from_secs(2);
    while !reader.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(reader.is_finished(), "the read waited for the writer's mutex");
    drop(held);
    let (took, size, state_size, bytes) = reader.join().unwrap();
    assert!(took < Duration::from_secs(1), "read took {took:?}");
    assert_eq!(size, Some(16));
    assert_eq!(state_size, 16);
    assert_eq!(bytes, b"{\"a\":1}\n");
}

/// A reader sees every write committed before it starts.
#[test]
fn reads_see_every_committed_write() {
    let (_dir, fs) = store();
    for i in 0..20 {
        fs.append_lines(ZONE, "output", format!("{{\"n\":{i}}}\n").as_bytes()).unwrap();
        let size = fs.read_snapshot(|snap| Ok(snap.file(ZONE, "output")?.unwrap().size)).unwrap();
        assert_eq!(size, fs.line_state(ZONE, "output").unwrap().unwrap().size);
        let tail = fs.read_bytes_db(ZONE, "output", size - 1, 1).unwrap();
        assert_eq!(tail, b"\n");
    }
}

/// A pooled connection can't write: `query_only` refuses it.
#[test]
fn a_reader_connection_refuses_writes() {
    let (_dir, fs) = store();
    let err = fs.read_conn(|conn| {
        conn.execute("DELETE FROM db_wave_file", [])?;
        Ok(())
    });
    assert!(err.is_err(), "a reader wrote");
    assert!(fs.line_state(ZONE, "output").unwrap().is_some(), "the row is still there");
}

/// An in-memory store has no pool (each connection would be its own
/// database) and reads through its single connection, as before.
#[test]
fn an_in_memory_store_reads_through_its_own_connection() {
    let fs = FileStore::open_in_memory().unwrap();
    assert!(fs.readers.is_none());
    fs.make_file(ZONE, "output", FileMeta::default(), FileOpts::default()).unwrap();
    fs.append_lines(ZONE, "output", b"x\n").unwrap();
    assert_eq!(fs.line_state(ZONE, "output").unwrap().unwrap().size, 2);
}
