// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! SQL schema setup for WaveStore and FileStore.
//! Uses CREATE TABLE IF NOT EXISTS for idempotent initialization.
//! Matches Go's migration schemas from db/migrations-wstore and db/migrations-filestore.

use rusqlite::Connection;

use super::error::StoreError;

/// Object type table names matching Go's `db_<otype>` convention.
const WSTORE_OTYPES: &[&str] = &[
    "client",
    "window",
    "workspace",
    "tab",
    "layout",
    "block",
    "temp",
];

/// Initialize the WaveStore schema.
/// Creates one table per object type, each with (oid, version, data).
pub fn run_wstore_migrations(conn: &Connection) -> Result<(), StoreError> {
    for otype in WSTORE_OTYPES {
        let table = format!("db_{otype}");
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {table} (
                oid TEXT PRIMARY KEY,
                version INTEGER NOT NULL DEFAULT 1,
                data TEXT NOT NULL
            );"
        ))?;
    }
    Ok(())
}

/// Initialize the FileStore schema.
/// Creates the wave_file and file_data tables.
pub fn run_filestore_migrations(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS db_wave_file (
            zoneid TEXT NOT NULL,
            name TEXT NOT NULL,
            size INTEGER NOT NULL DEFAULT 0,
            createdts INTEGER NOT NULL DEFAULT 0,
            modts INTEGER NOT NULL DEFAULT 0,
            opts TEXT NOT NULL DEFAULT '{}',
            meta TEXT NOT NULL DEFAULT '{}',
            PRIMARY KEY (zoneid, name)
        );

        CREATE TABLE IF NOT EXISTS db_file_data (
            zoneid TEXT NOT NULL,
            name TEXT NOT NULL,
            partidx INTEGER NOT NULL,
            data BLOB NOT NULL,
            PRIMARY KEY (zoneid, name, partidx)
        );",
    )?;
    Ok(())
}

/// Initialize the Forge schema.
/// Creates the db_forge_agents table for user-defined AI agents.
pub fn run_forge_migrations(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS db_forge_agents (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            icon TEXT NOT NULL DEFAULT '✦',
            provider TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL DEFAULT 0
        );",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wstore_migrations_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();

        // Run twice — should not error
        run_wstore_migrations(&conn).unwrap();
        run_wstore_migrations(&conn).unwrap();

        // Verify tables exist
        for otype in WSTORE_OTYPES {
            let table = format!("db_{otype}");
            let count: i64 = conn
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0);
        }
    }

    #[test]
    fn test_filestore_migrations_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();

        run_filestore_migrations(&conn).unwrap();
        run_filestore_migrations(&conn).unwrap();

        let count: i64 = conn
            .query_row("SELECT count(*) FROM db_wave_file", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
