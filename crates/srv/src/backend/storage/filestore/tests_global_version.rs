// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The shared transcript store is stamped with a `user_version` above the per-channel
//! filestore's, so a build that predates the SQLite 3.51.3 upgrade (which opens a
//! filestore as version 1) refuses it instead of writing to it with the old library.

use super::*;
use crate::backend::storage::error::StoreError;
use crate::backend::storage::migrations::{FILESTORE_SCHEMA_VERSION, GLOBAL_TRANSCRIPT_STORE_VERSION};
use std::path::Path;

fn user_version(path: &Path) -> i64 {
    rusqlite::Connection::open(path).unwrap().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap()
}

fn set_user_version(path: &Path, version: i64) {
    rusqlite::Connection::open(path).unwrap().execute_batch(&format!("PRAGMA user_version = {version};")).unwrap();
}

fn put(fs: &FileStore, text: &[u8]) {
    fs.make_file("agent:x:current", "output", FileMeta::default(), FileOpts::default()).unwrap();
    fs.append_lines("agent:x:current", "output", text).unwrap();
}

fn read(fs: &FileStore) -> Vec<u8> {
    fs.read_file("agent:x:current", "output").unwrap().unwrap()
}

#[test]
fn the_global_store_is_stamped_above_the_per_channel_version() {
    assert!(GLOBAL_TRANSCRIPT_STORE_VERSION > FILESTORE_SCHEMA_VERSION);
    let dir = tempfile::tempdir().unwrap();
    let (global, channel) = (dir.path().join("global.db"), dir.path().join("channel.db"));
    drop(FileStore::open_global_transcripts(&global).unwrap());
    drop(FileStore::open(&channel).unwrap());
    assert_eq!(user_version(&global), GLOBAL_TRANSCRIPT_STORE_VERSION);
    assert_eq!(user_version(&channel), FILESTORE_SCHEMA_VERSION, "a per-channel store keeps its version");
}

#[test]
fn a_build_that_predates_the_stamp_refuses_the_global_store_and_leaves_it_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("filestore.db");
    {
        let fs = FileStore::open_global_transcripts(&path).unwrap();
        put(&fs, b"{\"a\":1}\n");
    }
    // `FileStore::open` is what every earlier build runs on the shared store.
    match FileStore::open(&path) {
        Err(StoreError::SchemaTooNew { found, expected, .. }) => {
            assert_eq!((found, expected), (GLOBAL_TRANSCRIPT_STORE_VERSION, FILESTORE_SCHEMA_VERSION));
        }
        Err(e) => panic!("expected SchemaTooNew, got {e}"),
        Ok(_) => panic!("an older build opened the shared store"),
    }
    assert_eq!(user_version(&path), GLOBAL_TRANSCRIPT_STORE_VERSION, "the refusal changed nothing");
    let again = FileStore::open_global_transcripts(&path).unwrap();
    assert_eq!(read(&again), b"{\"a\":1}\n");
}

#[test]
fn an_existing_store_at_the_old_version_is_upgraded_in_place_with_its_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("filestore.db");
    {
        let fs = FileStore::open(&path).unwrap();
        put(&fs, b"{\"b\":2}\n");
    }
    assert_eq!(user_version(&path), FILESTORE_SCHEMA_VERSION);
    let fs = FileStore::open_global_transcripts(&path).unwrap();
    assert_eq!(read(&fs), b"{\"b\":2}\n");
    drop(fs);
    assert_eq!(user_version(&path), GLOBAL_TRANSCRIPT_STORE_VERSION);
    assert!(matches!(FileStore::open(&path), Err(StoreError::SchemaTooNew { .. })));
}

#[test]
fn a_store_from_a_later_build_is_refused_too() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("filestore.db");
    drop(FileStore::open_global_transcripts(&path).unwrap());
    set_user_version(&path, GLOBAL_TRANSCRIPT_STORE_VERSION + 1);
    assert!(matches!(FileStore::open_global_transcripts(&path), Err(StoreError::SchemaTooNew { .. })));
}
