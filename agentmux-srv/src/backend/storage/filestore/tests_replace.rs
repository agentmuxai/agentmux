// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Multi-file replace/delete in one transaction (`replace.rs`, Phase 5a-2b).

use super::{FileMeta, FileOpts, FileStore};
use crate::backend::blockcontroller::shell::rebuild_output_idx;

const ZONE: &str = "block-1";

fn mem() -> FileStore {
    FileStore::open_in_memory().unwrap()
}

fn gen(fs: &FileStore, name: &str) -> String {
    fs.line_state(ZONE, name).unwrap().unwrap().counted.unwrap().gen
}

fn exists(fs: &FileStore, name: &str) -> bool {
    fs.line_state(ZONE, name).unwrap().is_some()
}

fn put(fs: &FileStore, name: &str, data: &[u8]) {
    fs.make_file(ZONE, name, FileMeta::new(), FileOpts::default()).unwrap();
    fs.write_file(ZONE, name, data).unwrap();
}

fn fail_on(fs: &FileStore, sql: &str) {
    fs.conn().lock().unwrap().execute_batch(sql).unwrap();
}

#[test]
fn replace_file_creates_writes_and_drops_the_sidecars_together() {
    let fs = mem();
    // Missing file: created.
    fs.replace_file(ZONE, "output", b"a\nb\n", &["output.idx", "output.tsidx"]).unwrap();
    assert_eq!(fs.read_bytes_db(ZONE, "output", 0, 4).unwrap(), b"a\nb\n");
    let g0 = gen(&fs, "output");
    assert_eq!(fs.line_state(ZONE, "output").unwrap().unwrap().counted.unwrap().lines, 2);

    put(&fs, "output.idx", b"stale");
    put(&fs, "output.tsidx", b"stale");
    fs.replace_file(ZONE, "output", b"x\n", &["output.idx", "output.tsidx"]).unwrap();
    assert_ne!(gen(&fs, "output"), g0, "replaced content gets a new generation");
    assert!(!exists(&fs, "output.idx") && !exists(&fs, "output.tsidx"));
    // Through the cache too: the replaced row isn't served stale.
    assert_eq!(fs.stat(ZONE, "output").unwrap().unwrap().size, 2);
    assert_eq!(fs.read_file(ZONE, "output").unwrap().unwrap(), b"x\n");
}

#[test]
fn a_failed_replace_changes_nothing() {
    let fs = mem();
    put(&fs, "output", b"old\n");
    put(&fs, "output.idx", b"idx");
    let g0 = gen(&fs, "output");
    // The sidecar delete is the last step; make it fail.
    fail_on(
        &fs,
        "CREATE TRIGGER no_idx_delete BEFORE DELETE ON db_wave_file WHEN OLD.name = 'output.idx'
         BEGIN SELECT RAISE(ABORT, 'injected'); END;",
    );
    assert!(fs.replace_file(ZONE, "output", b"new content\n", &["output.idx"]).is_err());
    assert_eq!(fs.read_bytes_db(ZONE, "output", 0, 4).unwrap(), b"old\n");
    assert_eq!(fs.line_state(ZONE, "output").unwrap().unwrap().size, 4);
    assert_eq!(gen(&fs, "output"), g0);
    assert!(exists(&fs, "output.idx"));
}

#[test]
fn delete_files_is_all_or_nothing_and_ignores_missing_files() {
    let fs = mem();
    put(&fs, "output", b"a\n");
    put(&fs, "output.idx", b"i");
    fs.delete_files(ZONE, &["output", "output.idx", "never-existed"]).unwrap();
    assert!(!exists(&fs, "output") && !exists(&fs, "output.idx"));
    assert!(fs.stat(ZONE, "output").unwrap().is_none(), "the cache forgets deleted rows");

    put(&fs, "output", b"a\n");
    put(&fs, "output.idx", b"i");
    fail_on(
        &fs,
        "CREATE TRIGGER no_idx_delete BEFORE DELETE ON db_wave_file WHEN OLD.name = 'output.idx'
         BEGIN SELECT RAISE(ABORT, 'injected'); END;",
    );
    assert!(fs.delete_files(ZONE, &["output", "output.idx"]).is_err());
    assert!(exists(&fs, "output"), "output must not be deleted without its sidecar");
}

#[test]
fn put_file_with_meta_writes_content_and_merges_metadata() {
    let fs = mem();
    let mut m = FileMeta::new();
    m.insert("keep".into(), serde_json::json!(1));
    m.insert("for_gen".into(), serde_json::json!("g1"));
    fs.put_file_with_meta(ZONE, "output.idx", b"one", m).unwrap();
    let mut m = FileMeta::new();
    m.insert("for_gen".into(), serde_json::json!("g2"));
    fs.put_file_with_meta(ZONE, "output.idx", b"two", m).unwrap();
    let meta = fs.meta_db(ZONE, "output.idx").unwrap().unwrap();
    assert_eq!(meta.get("keep"), Some(&serde_json::json!(1)));
    assert_eq!(meta.get("for_gen"), Some(&serde_json::json!("g2")));
    assert_eq!(fs.read_bytes_db(ZONE, "output.idx", 0, 3).unwrap(), b"two");
}

#[test]
fn bytes_a_file_claims_but_does_not_store_are_never_served() {
    // A build without transactions raises `size` before inserting parts.
    let fs = mem();
    fs.make_file(ZONE, "output", FileMeta::new(), FileOpts::default()).unwrap();
    fs.conn()
        .lock()
        .unwrap()
        .execute("UPDATE db_wave_file SET size = 6 WHERE zoneid = ?1 AND name = 'output'", rusqlite::params![ZONE])
        .unwrap();
    assert!(fs.read_bytes_db(ZONE, "output", 0, 6).is_err());
    // The indexer can't build an index over the hole (callers fall back).
    assert_eq!(rebuild_output_idx(&fs, ZONE, 6), None);
    fs.conn()
        .lock()
        .unwrap()
        .execute("INSERT INTO db_file_data (zoneid, name, partidx, data) VALUES (?1, 'output', 0, ?2)", rusqlite::params![ZONE, b"a\nb\nc\n".to_vec()])
        .unwrap();
    assert_eq!(rebuild_output_idx(&fs, ZONE, 6), Some(3));
}

#[test]
fn the_indexer_labels_the_index_with_the_generation_it_read() {
    let fs = mem();
    fs.make_file(ZONE, "output", FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_lines(ZONE, "output", b"a\nb\nc\n").unwrap();
    let size = fs.line_state(ZONE, "output").unwrap().unwrap().size as u64;
    assert_eq!(rebuild_output_idx(&fs, ZONE, size), Some(3));
    let label = fs.meta_db(ZONE, "output.idx").unwrap().unwrap();
    assert_eq!(label.get("for_gen"), Some(&serde_json::json!(gen(&fs, "output"))));
}
