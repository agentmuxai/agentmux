// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! SQLite database migration utilities.
//! Port of Go's `pkg/util/migrateutil/migrateutil.go`.
//!
//! Provides simple schema migration using versioned SQL scripts.
//! Each migration is a `(version, sql)` pair applied in order.


use rusqlite::Connection;

/// A single migration step.
#[derive(Debug, Clone)]
pub struct Migration {
    /// Version number (must be sequential starting from 1).
    pub version: u32,
    /// SQL to execute for this migration.
    pub sql: &'static str,
}

/// Get the current migration version from the database.
/// Returns 0 if no migrations have been applied.
pub fn get_migrate_version(conn: &Connection) -> Result<u32, rusqlite::Error> {
    // Ensure the migrations table exists
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER NOT NULL,
            dirty INTEGER NOT NULL DEFAULT 0
        )",
    )?;

    let result: Result<(u32, bool), _> = conn.query_row(
        "SELECT version, dirty FROM schema_migrations LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get::<_, bool>(1)?)),
    );

    match result {
        Ok((version, _dirty)) => Ok(version),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
        Err(e) => Err(e),
    }
}

/// Check if the database is in a dirty state.
pub fn is_dirty(conn: &Connection) -> Result<bool, rusqlite::Error> {
    let result: Result<bool, _> = conn.query_row(
        "SELECT dirty FROM schema_migrations LIMIT 1",
        [],
        |row| row.get(0),
    );

    match result {
        Ok(dirty) => Ok(dirty),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Run migrations on the database.
///
/// - Checks current version
/// - Checks for dirty state (returns error if dirty)
/// - Applies all migrations above the current version
/// - Updates the version tracking table
///
/// Returns the new version number.
pub fn migrate(
    store_name: &str,
    conn: &Connection,
    migrations: &[Migration],
) -> Result<u32, String> {
    let cur_version = get_migrate_version(conn).map_err(|e| {
        format!(
            "{}: cannot get current migration version: {}",
            store_name, e
        )
    })?;

    if is_dirty(conn).map_err(|e| format!("{}: cannot check dirty state: {}", store_name, e))? {
        return Err(format!("{}: migrate up, database is dirty", store_name));
    }

    let mut new_version = cur_version;

    for m in migrations {
        if m.version <= cur_version {
            continue;
        }
        // Mark dirty before applying
        set_version(conn, m.version, true)
            .map_err(|e| format!("{}: setting dirty version {}: {}", store_name, m.version, e))?;

        conn.execute_batch(m.sql)
            .map_err(|e| format!("migrating {}: version {}: {}", store_name, m.version, e))?;

        // Mark clean after applying
        set_version(conn, m.version, false)
            .map_err(|e| format!("{}: setting clean version {}: {}", store_name, m.version, e))?;

        new_version = m.version;
    }

    if new_version != cur_version {
        tracing::info!(
            "[db] {} migration done, version {} -> {}",
            store_name,
            cur_version,
            new_version
        );
    }

    Ok(new_version)
}

/// Set the migration version and dirty flag.
fn set_version(conn: &Connection, version: u32, dirty: bool) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER NOT NULL,
            dirty INTEGER NOT NULL DEFAULT 0
        )",
    )?;

    conn.execute("DELETE FROM schema_migrations", [])?;
    conn.execute(
        "INSERT INTO schema_migrations (version, dirty) VALUES (?1, ?2)",
        rusqlite::params![version, dirty],
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL").unwrap();
        conn
    }

    #[test]
    fn test_get_version_empty_db() {
        let conn = setup_db();
        assert_eq!(get_migrate_version(&conn).unwrap(), 0);
    }

    #[test]
    fn test_migrate_single() {
        let conn = setup_db();
        let migrations = vec![Migration {
            version: 1,
            sql: "CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)",
        }];

        let version = migrate("test-store", &conn, &migrations).unwrap();
        assert_eq!(version, 1);
        assert_eq!(get_migrate_version(&conn).unwrap(), 1);

        // Verify table was created
        conn.execute("INSERT INTO test (id, name) VALUES (1, 'hello')", [])
            .unwrap();
    }

    #[test]
    fn test_migrate_multiple() {
        let conn = setup_db();
        let migrations = vec![
            Migration {
                version: 1,
                sql: "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)",
            },
            Migration {
                version: 2,
                sql: "ALTER TABLE users ADD COLUMN email TEXT",
            },
            Migration {
                version: 3,
                sql: "CREATE INDEX idx_users_email ON users(email)",
            },
        ];

        let version = migrate("test-store", &conn, &migrations).unwrap();
        assert_eq!(version, 3);

        // Verify all migrations applied
        conn.execute(
            "INSERT INTO users (id, name, email) VALUES (1, 'Alice', 'a@b.com')",
            [],
        )
        .unwrap();
    }

    #[test]
    fn test_migrate_idempotent() {
        let conn = setup_db();
        let migrations = vec![Migration {
            version: 1,
            sql: "CREATE TABLE test (id INTEGER PRIMARY KEY)",
        }];

        migrate("test-store", &conn, &migrations).unwrap();
        // Running again should be a no-op
        let version = migrate("test-store", &conn, &migrations).unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn test_migrate_incremental() {
        let conn = setup_db();

        // First batch
        let migrations1 = vec![Migration {
            version: 1,
            sql: "CREATE TABLE t1 (id INTEGER PRIMARY KEY)",
        }];
        migrate("test-store", &conn, &migrations1).unwrap();

        // Second batch (includes old + new)
        let migrations2 = vec![
            Migration {
                version: 1,
                sql: "CREATE TABLE t1 (id INTEGER PRIMARY KEY)",
            },
            Migration {
                version: 2,
                sql: "CREATE TABLE t2 (id INTEGER PRIMARY KEY)",
            },
        ];
        let version = migrate("test-store", &conn, &migrations2).unwrap();
        assert_eq!(version, 2);

        // Both tables should exist
        conn.execute("INSERT INTO t1 (id) VALUES (1)", []).unwrap();
        conn.execute("INSERT INTO t2 (id) VALUES (1)", []).unwrap();
    }

    #[test]
    fn test_migrate_bad_sql() {
        let conn = setup_db();
        let migrations = vec![Migration {
            version: 1,
            sql: "THIS IS NOT SQL",
        }];

        let result = migrate("test-store", &conn, &migrations);
        assert!(result.is_err());
    }

    #[test]
    fn test_is_dirty_clean() {
        let conn = setup_db();
        let migrations = vec![Migration {
            version: 1,
            sql: "CREATE TABLE test (id INTEGER PRIMARY KEY)",
        }];
        migrate("test-store", &conn, &migrations).unwrap();
        assert!(!is_dirty(&conn).unwrap());
    }

    #[test]
    fn test_set_version() {
        let conn = setup_db();
        set_version(&conn, 5, false).unwrap();
        assert_eq!(get_migrate_version(&conn).unwrap(), 5);
        assert!(!is_dirty(&conn).unwrap());
    }
}
