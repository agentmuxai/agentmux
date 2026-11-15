// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use rusqlite::Connection;

fn ctx(dir: &tempfile::TempDir) -> MigrationContext {
    MigrationContext {
        home: dir.path().to_path_buf(),
        data_dir: dir.path().to_path_buf(),
        shared_store_path: dir.path().join("shared-store.db"),
        channel_store_path: dir.path().join("objects.db"),
    }
}

fn columns(path: &std::path::Path) -> Vec<(String, i64)> {
    let conn = Connection::open(path).unwrap();
    let mut stmt = conn.prepare("PRAGMA table_info(db_drone_checkpoints)").unwrap();
    stmt.query_map([], |row| Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))).unwrap().map(|r| r.unwrap()).collect()
}

#[test]
fn the_channel_store_gets_the_table_and_a_second_run_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(&dir);
    Connection::open(&c.channel_store_path).unwrap().execute_batch("CREATE TABLE someone_elses (id TEXT PRIMARY KEY);").unwrap();

    M0050DroneCheckpoints.up(&c).unwrap();
    let first = columns(&c.channel_store_path);
    assert!(!first.is_empty(), "the table exists");
    M0050DroneCheckpoints.up(&c).unwrap();
    assert_eq!(columns(&c.channel_store_path), first);
}

#[test]
fn the_key_is_run_and_step() {
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(&dir);
    Connection::open(&c.channel_store_path).unwrap().execute_batch("CREATE TABLE someone_elses (id TEXT PRIMARY KEY);").unwrap();
    M0050DroneCheckpoints.up(&c).unwrap();
    let cols = columns(&c.channel_store_path);
    let pk: Vec<&str> = cols.iter().filter(|(_, pk)| *pk > 0).map(|(name, _)| name.as_str()).collect();
    assert_eq!(pk, ["run_id", "step"]);
}

#[test]
fn a_store_that_does_not_exist_yet_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let c = ctx(&dir);
    M0050DroneCheckpoints.up(&c).unwrap();
    assert!(!c.channel_store_path.exists(), "the migration does not create a store");
}
