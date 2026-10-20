// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use super::*;
use crate::drone::storage::DroneStore;
use crate::drone::types::DroneRun;

fn run(id: &str, drone_id: &str) -> DroneRun {
    DroneRun {
        id: id.to_string(),
        drone_id: drone_id.to_string(),
        status: "running".to_string(),
        started_at: 1,
        ended_at: 0,
        block_states: HashMap::new(),
        output: String::new(),
        error: String::new(),
    }
}

fn index_names(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'db_drone_runs' AND name LIKE 'idx_%' ORDER BY name")
        .unwrap();
    stmt.query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap()
}

/// A store whose `db_drone_runs` still has the FK, as every store written
/// before this migration does.
fn legacy_store() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("objects.db");
    drop(Store::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "PRAGMA foreign_keys=OFF;
         DROP TABLE db_drone_runs;
         CREATE TABLE db_drone_runs (
            id           TEXT PRIMARY KEY,
            drone_id     TEXT NOT NULL,
            status       TEXT NOT NULL DEFAULT 'running',
            started_at   INTEGER NOT NULL DEFAULT 0,
            ended_at     INTEGER NOT NULL DEFAULT 0,
            block_states TEXT NOT NULL DEFAULT '{}',
            output       TEXT NOT NULL DEFAULT '',
            error        TEXT NOT NULL DEFAULT '',
            FOREIGN KEY (drone_id) REFERENCES db_drone_definitions(id) ON DELETE CASCADE
         );
         CREATE INDEX idx_drone_runs_drone_started ON db_drone_runs(drone_id, started_at DESC);
         CREATE INDEX idx_drone_runs_status ON db_drone_runs(status);
         PRAGMA foreign_keys=ON;",
    )
    .unwrap();
    (dir, path)
}

#[test]
fn a_run_for_a_drone_only_in_the_shared_store_fails_on_the_old_table() {
    let (_dir, path) = legacy_store();
    let store = Store::open(&path).unwrap();
    let err = store.drone_run_insert(&run("run-1", "shared-only-drone"));
    assert!(err.is_err(), "the old FK rejects a run whose definition isn't in this objects.db");
}

#[test]
fn a_fresh_store_accepts_a_run_for_a_drone_only_in_the_shared_store() {
    let store = Store::open_in_memory().unwrap();
    store
        .drone_run_insert(&run("run-1", "shared-only-drone"))
        .expect("a fresh store's db_drone_runs has no FK to the local definitions");
    assert_eq!(store.drone_runs_for("shared-only-drone", 10).unwrap().len(), 1);
}

#[test]
fn the_rebuild_keeps_every_row_and_both_indexes_and_drops_the_fk() {
    let (_dir, path) = legacy_store();
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "INSERT INTO db_drone_definitions (id, name) VALUES ('local-drone', 'Old');
         INSERT INTO db_drone_runs (id, drone_id, status, started_at, output)
             VALUES ('run-old', 'local-drone', 'done', 5, 'result');",
    )
    .unwrap();

    assert!(drop_drone_runs_fk(&conn).unwrap(), "the old table needs a rebuild");

    assert!(already_dropped(&conn, &DRONE_RUNS).unwrap());
    let (count, output): (i64, String) = conn
        .query_row("SELECT COUNT(*), MAX(output) FROM db_drone_runs", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!((count, output.as_str()), (1, "result"), "the existing run survives verbatim");
    assert_eq!(index_names(&conn), vec!["idx_drone_runs_drone_started", "idx_drone_runs_status"]);

    drop(conn);
    let store = Store::open(&path).unwrap();
    store
        .drone_run_insert(&run("run-new", "shared-only-drone"))
        .expect("after the rebuild a run needs no local definition");
}

#[test]
fn a_fresh_store_needs_no_rebuild_and_a_second_pass_does_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("objects.db");
    drop(Store::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    assert!(!drop_drone_runs_fk(&conn).unwrap(), "nothing to rebuild on a fresh store");

    let (_legacy_dir, legacy) = legacy_store();
    let conn = Connection::open(&legacy).unwrap();
    assert!(drop_drone_runs_fk(&conn).unwrap());
    assert!(!drop_drone_runs_fk(&conn).unwrap(), "a second pass finds nothing left to rebuild");
}

#[test]
fn a_rebuild_interrupted_before_its_indexes_gets_them_on_the_next_pass() {
    let (_dir, path) = legacy_store();
    let conn = Connection::open(&path).unwrap();
    // The rebuild committed, then the process died before the indexes.
    rebuild_table(&conn, &DRONE_RUNS).unwrap();
    assert!(index_names(&conn).is_empty());
    assert!(!drop_drone_runs_fk(&conn).unwrap(), "the FK is already gone");
    assert_eq!(index_names(&conn), vec!["idx_drone_runs_drone_started", "idx_drone_runs_status"]);
}

#[test]
fn a_missing_channel_store_is_a_no_op() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = MigrationContext {
        home: dir.path().to_path_buf(),
        data_dir: dir.path().to_path_buf(),
        shared_store_path: dir.path().join("unused-shared-store.db"),
        channel_store_path: dir.path().join("does-not-exist.db"),
    };
    assert!(M0038DropDroneRunsDefinitionFk.up(&ctx).is_ok());
    assert!(!ctx.channel_store_path.exists(), "up() must not create a store that wasn't there");
}

#[test]
fn deleting_a_drones_runs_leaves_other_drones_runs() {
    let store = Store::open_in_memory().unwrap();
    store.drone_run_insert(&run("a-1", "drone-a")).unwrap();
    store.drone_run_insert(&run("a-2", "drone-a")).unwrap();
    store.drone_run_insert(&run("b-1", "drone-b")).unwrap();
    assert_eq!(store.drone_run_delete("a-1").unwrap(), 1, "one run by id");
    assert_eq!(store.drone_runs_for("drone-a", 10).unwrap().len(), 1);
    assert_eq!(store.drone_runs_delete_for("drone-a").unwrap(), 1);
    assert!(store.drone_runs_for("drone-a", 10).unwrap().is_empty());
    assert_eq!(store.drone_runs_for("drone-b", 10).unwrap().len(), 1);
}
