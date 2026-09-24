// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Atomicity of FileStore mutations across connections (Phase 5a-1,
//! SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md §6.3.7).
//!
//! The global transcript store is one SQLite file opened by every srv instance
//! on the machine, so two `FileStore`s on one path stand in for two processes:
//! each has its own connection, and the in-process mutex covers neither.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Barrier};

use rusqlite::params;

use super::core::PART_DATA_SIZE;
use super::{FileMeta, FileOpts, FileStore};
use crate::backend::storage::error::StoreError;

const ZONE: &str = "agent:def:current";
const NAME: &str = "output";

/// A store on `path` that has never cached anything, so reads come from the
/// database — what another process would see.
fn fresh(path: &Path) -> FileStore {
    FileStore::open(path).unwrap()
}

/// `(size, part lengths by index)` straight from the tables.
fn db_state(store: &FileStore) -> (i64, Vec<(i32, usize)>) {
    let conn = store.conn().lock().unwrap();
    let size: i64 = conn
        .query_row(
            "SELECT size FROM db_wave_file WHERE zoneid = ?1 AND name = ?2",
            params![ZONE, NAME],
            |r| r.get(0),
        )
        .unwrap();
    let mut stmt = conn
        .prepare("SELECT partidx, length(data) FROM db_file_data WHERE zoneid = ?1 AND name = ?2 ORDER BY partidx")
        .unwrap();
    let parts = stmt
        .query_map(params![ZONE, NAME], |r| Ok((r.get::<_, i32>(0)?, r.get::<_, i64>(1)? as usize)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    (size, parts)
}

/// Make the next insert of part `partidx` fail, the way a full disk or an I/O
/// error would part-way through a multi-statement write.
fn fail_inserts_of_part(store: &FileStore, partidx: i32) {
    store
        .conn()
        .lock()
        .unwrap()
        .execute_batch(&format!(
            "CREATE TRIGGER fail_part BEFORE INSERT ON db_file_data WHEN NEW.partidx = {partidx}
             BEGIN SELECT RAISE(ABORT, 'injected part failure'); END;"
        ))
        .unwrap();
}

fn clear_failure(store: &FileStore) {
    store.conn().lock().unwrap().execute_batch("DROP TRIGGER fail_part;").unwrap();
}

/// Line `i` of writer `w`: its id, then a payload derived from the id so a
/// torn or overwritten line can't pass the check. Every 25th line is larger
/// than a part, so appends regularly span part boundaries.
fn line(w: usize, i: usize) -> String {
    let len = if i % 25 == 7 { PART_DATA_SIZE + 5_000 } else { 40 + (i * 37 + w * 11) % 260 };
    let fill = (b'a' + ((w * 7 + i) % 26) as u8) as char;
    format!("{w}-{i}:{}\n", fill.to_string().repeat(len))
}

#[test]
fn concurrent_appends_from_two_stores_on_one_database_lose_nothing() {
    const STORES: usize = 2;
    const THREADS_PER_STORE: usize = 4;
    const LINES: usize = 120;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("filestore.db");
    let stores: Vec<Arc<FileStore>> = (0..STORES).map(|_| Arc::new(fresh(&path))).collect();
    stores[0].make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();

    let writers = STORES * THREADS_PER_STORE;
    let start = Arc::new(Barrier::new(writers));
    let handles: Vec<_> = (0..writers)
        .map(|w| {
            let store = Arc::clone(&stores[w % STORES]);
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                for i in 0..LINES {
                    store.append_data(ZONE, NAME, line(w, i).as_bytes()).unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }

    let data = fresh(&path).read_file(ZONE, NAME).unwrap().unwrap();
    let text = String::from_utf8(data).expect("an overwritten part boundary would break UTF-8 or a line");
    let mut seen: HashMap<String, usize> = HashMap::new();
    for got in text.split_inclusive('\n') {
        let id = got.split(':').next().unwrap().to_string();
        let (w, i) = id.split_once('-').map(|(w, i)| (w.parse().unwrap(), i.parse().unwrap())).unwrap_or_else(|| panic!("garbled line starting {:?}", &got[..got.len().min(40)]));
        assert_eq!(got, line(w, i), "line {id} is torn or overwritten");
        *seen.entry(id).or_default() += 1;
    }
    for w in 0..writers {
        for i in 0..LINES {
            assert_eq!(seen.get(&format!("{w}-{i}")), Some(&1), "line {w}-{i} must appear exactly once");
        }
    }
    let expected: usize = (0..writers).flat_map(|w| (0..LINES).map(move |i| line(w, i).len())).sum();
    assert_eq!(text.len(), expected);
}

#[test]
fn concurrent_make_file_from_two_stores_reports_already_exists() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("filestore.db");
    let stores: Vec<Arc<FileStore>> = (0..2).map(|_| Arc::new(fresh(&path))).collect();

    for round in 0..40 {
        let name = format!("output-{round}");
        let start = Arc::new(Barrier::new(8));
        let results: Vec<Result<(), StoreError>> = (0..8)
            .map(|t| {
                let store = Arc::clone(&stores[t % 2]);
                let start = Arc::clone(&start);
                let name = name.clone();
                std::thread::spawn(move || {
                    start.wait();
                    store.make_file(ZONE, &name, FileMeta::new(), FileOpts::default())
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect();
        let created = results.iter().filter(|r| r.is_ok()).count();
        let exists = results.iter().filter(|r| matches!(r, Err(StoreError::AlreadyExists))).count();
        // Callers treat only AlreadyExists as benign; any other error makes
        // `handle_append_block_file` skip the write entirely.
        assert_eq!((created, exists), (1, 7), "round {round}: {results:?}");
    }
}

#[test]
fn failed_append_leaves_the_file_unchanged_and_the_next_append_lands_after_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("filestore.db");
    let store = fresh(&path);
    store.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    store.append_data(ZONE, NAME, &[b'x'; 100]).unwrap();
    let before = db_state(&store);

    // Spans parts 0, 1 and 2; part 2's insert fails after 0 and 1 were written.
    fail_inserts_of_part(&store, 2);
    let big = vec![b'y'; 2 * PART_DATA_SIZE + 10];
    assert!(store.append_data(ZONE, NAME, &big).is_err());
    clear_failure(&store);
    assert_eq!(db_state(&store), before, "a failed append must not leave partial parts behind");

    store.append_data(ZONE, NAME, b"after\n").unwrap();
    let mut expected = vec![b'x'; 100];
    expected.extend_from_slice(b"after\n");
    assert_eq!(fresh(&path).read_file(ZONE, NAME).unwrap().unwrap(), expected);
}

#[test]
fn failed_write_file_keeps_the_old_content() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("filestore.db");
    let store = fresh(&path);
    store.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    let old = vec![b'o'; PART_DATA_SIZE + 500];
    store.write_file(ZONE, NAME, &old).unwrap();

    fail_inserts_of_part(&store, 1);
    assert!(store.write_file(ZONE, NAME, &vec![b'n'; 3 * PART_DATA_SIZE]).is_err());
    clear_failure(&store);

    assert_eq!(fresh(&path).read_file(ZONE, NAME).unwrap().unwrap(), old);
    assert_eq!(store.read_file(ZONE, NAME).unwrap().unwrap(), old);
}

/// Append latency on a file-backed store (WAL, like production): one writer
/// alone, then with a second store appending to the same file. Not a pass/fail
/// test; run with
/// `cargo test --bin agentmux-srv --release -- --ignored append_latency --nocapture`.
#[test]
#[ignore]
fn append_latency() {
    fn percentiles(label: &str, mut us: Vec<u128>) {
        us.sort_unstable();
        let p = |q: f64| us[((us.len() as f64 * q) as usize).min(us.len() - 1)];
        println!(
            "{label}: n={} p50={}us p90={}us p99={}us max={}us",
            us.len(),
            p(0.50),
            p(0.90),
            p(0.99),
            us[us.len() - 1]
        );
    }
    const N: usize = 3000;
    let payload = format!("{}\n", "x".repeat(1023));

    for contended in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("filestore.db");
        let store = Arc::new(fresh(&path));
        store.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let other = contended.then(|| {
            let other = fresh(&path);
            let stop = Arc::clone(&stop);
            let payload = payload.clone();
            std::thread::spawn(move || {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    other.append_data(ZONE, NAME, payload.as_bytes()).unwrap();
                }
            })
        });
        let mut us = Vec::with_capacity(N);
        for _ in 0..N {
            let t = std::time::Instant::now();
            store.append_data(ZONE, NAME, payload.as_bytes()).unwrap();
            us.push(t.elapsed().as_micros());
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(h) = other {
            h.join().unwrap();
        }
        percentiles(if contended { "contended (2 stores)" } else { "single writer" }, us);
    }
}

#[test]
fn write_meta_merge_uses_the_database_not_a_stale_cache() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("filestore.db");
    let a = fresh(&path);
    let b = fresh(&path);
    // Creating the file caches its row in `b` (a plain `stat` miss doesn't
    // cache), with empty meta. `a` has no cached row and reads the database.
    b.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();

    let mut m = FileMeta::new();
    m.insert("from_a".into(), serde_json::json!(1));
    a.write_meta(ZONE, NAME, m, true).unwrap();
    let mut m = FileMeta::new();
    m.insert("from_b".into(), serde_json::json!(2));
    b.write_meta(ZONE, NAME, m, true).unwrap();

    for (who, meta) in [("database", fresh(&path).stat(ZONE, NAME).unwrap().unwrap().meta), ("b's cache", b.stat(ZONE, NAME).unwrap().unwrap().meta)] {
        assert_eq!(meta.get("from_a"), Some(&serde_json::json!(1)), "{who}: b's merge must not drop a's key: {meta:?}");
        assert_eq!(meta.get("from_b"), Some(&serde_json::json!(2)), "{who}: {meta:?}");
    }
}
