// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use rusqlite::Connection;

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self { dir: tempfile::tempdir().unwrap() }
    }

    fn shared(&self) -> std::path::PathBuf {
        self.dir.path().join("shared-store.db")
    }

    fn channel(&self) -> std::path::PathBuf {
        self.dir.path().join("objects.db")
    }

    /// A store that predates this migration: it has another table, not this one.
    fn store_without_versions(path: &std::path::Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch("CREATE TABLE someone_elses (id TEXT PRIMARY KEY);").unwrap();
    }

    fn ctx(&self) -> MigrationContext {
        MigrationContext {
            home: self.dir.path().to_path_buf(),
            data_dir: self.dir.path().to_path_buf(),
            shared_store_path: self.shared(),
            channel_store_path: self.channel(),
        }
    }
}

fn has_versions_table(path: &std::path::Path) -> bool {
    let conn = Connection::open(path).unwrap();
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'db_drone_versions'",
        [],
        |row| row.get::<_, i64>(0),
    )
    .unwrap()
        == 1
}

#[test]
fn both_stores_get_the_table_and_a_second_run_changes_nothing() {
    let f = Fixture::new();
    Fixture::store_without_versions(&f.shared());
    Fixture::store_without_versions(&f.channel());

    M0049DroneVersions.up(&f.ctx()).unwrap();
    assert!(has_versions_table(&f.shared()));
    assert!(has_versions_table(&f.channel()));

    M0049DroneVersions.up(&f.ctx()).unwrap();
    assert!(has_versions_table(&f.shared()));
    assert!(has_versions_table(&f.channel()));
}

#[test]
fn the_key_is_drone_and_version_and_there_is_no_foreign_key() {
    let f = Fixture::new();
    Fixture::store_without_versions(&f.shared());
    M0049DroneVersions.up(&f.ctx()).unwrap();

    let conn = Connection::open(f.shared()).unwrap();
    let mut stmt = conn.prepare("PRAGMA table_info(db_drone_versions)").unwrap();
    let columns: Vec<(String, i64)> = stmt
        .query_map([], |row| Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    let pk: Vec<&str> = columns.iter().filter(|(_, pk)| *pk > 0).map(|(name, _)| name.as_str()).collect();
    assert_eq!(pk, ["drone_id", "version"]);

    let fk: i64 = conn
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_list('db_drone_versions')", [], |row| row.get(0))
        .unwrap();
    assert_eq!(fk, 0, "no FK to the definitions (m0038)");
}

#[test]
fn a_missing_store_is_left_alone() {
    let f = Fixture::new();
    Fixture::store_without_versions(&f.channel());
    M0049DroneVersions.up(&f.ctx()).unwrap();
    assert!(!f.shared().exists(), "nothing creates a store that was not there");
    assert!(has_versions_table(&f.channel()));
}
