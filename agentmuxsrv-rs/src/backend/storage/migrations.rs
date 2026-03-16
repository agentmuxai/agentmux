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
    run_forge_v2_migrations(conn)?;
    run_forge_v3_migrations(conn)?;
    run_forge_v4_migrations(conn)?;
    Ok(())
}

/// Forge v2 migrations: extend db_forge_agents with operational fields
/// and create db_forge_content table for content blobs (soul, agentmd, mcp, env, memory).
pub fn run_forge_v2_migrations(conn: &Connection) -> Result<(), StoreError> {
    // Add new columns to db_forge_agents (ALTER TABLE ADD COLUMN is idempotent-safe
    // because we catch "duplicate column" errors).
    let alter_statements = [
        "ALTER TABLE db_forge_agents ADD COLUMN working_directory TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_forge_agents ADD COLUMN shell TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_forge_agents ADD COLUMN provider_flags TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_forge_agents ADD COLUMN auto_start INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE db_forge_agents ADD COLUMN restart_on_crash INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE db_forge_agents ADD COLUMN idle_timeout_minutes INTEGER NOT NULL DEFAULT 0",
    ];
    for stmt in &alter_statements {
        match conn.execute_batch(stmt) {
            Ok(_) => {}
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("duplicate column") {
                    // Column already exists, skip
                } else {
                    return Err(StoreError::Sqlite(
                        match e {
                            rusqlite::Error::SqliteFailure(code, _) => {
                                rusqlite::Error::SqliteFailure(code, Some(msg))
                            }
                            other => other,
                        },
                    ));
                }
            }
        }
    }

    // Create db_forge_content table for content blobs
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS db_forge_content (
            agent_id TEXT NOT NULL,
            content_type TEXT NOT NULL,
            content TEXT NOT NULL DEFAULT '',
            updated_at INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (agent_id, content_type),
            FOREIGN KEY (agent_id) REFERENCES db_forge_agents(id) ON DELETE CASCADE
        );",
    )?;

    // Create db_forge_skills table for reusable agent skills
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS db_forge_skills (
            id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            name TEXT NOT NULL,
            trigger TEXT NOT NULL DEFAULT '',
            skill_type TEXT NOT NULL DEFAULT 'prompt',
            description TEXT NOT NULL DEFAULT '',
            content TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (agent_id) REFERENCES db_forge_agents(id) ON DELETE CASCADE
        );",
    )?;

    // Create db_forge_history table for append-only session logs
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS db_forge_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id TEXT NOT NULL,
            session_date TEXT NOT NULL,
            entry TEXT NOT NULL,
            timestamp INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (agent_id) REFERENCES db_forge_agents(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_forge_history_agent_date
            ON db_forge_history(agent_id, session_date);",
    )?;

    Ok(())
}

/// Forge v3 migrations: add agent_type, environment, agent_bus_id, and is_seeded
/// to support host/container agent classification and seed-based preloading.
pub fn run_forge_v3_migrations(conn: &Connection) -> Result<(), StoreError> {
    let alter_statements = [
        "ALTER TABLE db_forge_agents ADD COLUMN agent_type TEXT NOT NULL DEFAULT 'standalone'",
        "ALTER TABLE db_forge_agents ADD COLUMN environment TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_forge_agents ADD COLUMN agent_bus_id TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_forge_agents ADD COLUMN is_seeded INTEGER NOT NULL DEFAULT 0",
    ];
    for stmt in &alter_statements {
        match conn.execute_batch(stmt) {
            Ok(_) => {}
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("duplicate column") {
                    // Column already exists, skip
                } else {
                    return Err(StoreError::Sqlite(
                        match e {
                            rusqlite::Error::SqliteFailure(code, _) => {
                                rusqlite::Error::SqliteFailure(code, Some(msg))
                            }
                            other => other,
                        },
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Forge v4 migration: fix provider "claude-code" → "claude" for seeded agents.
/// The forge-seed.json originally used "claude-code" but the frontend PROVIDERS
/// map uses "claude" as the key. This one-time UPDATE corrects existing rows.
pub fn run_forge_v4_migrations(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        "UPDATE db_forge_agents SET provider = 'claude' WHERE provider = 'claude-code';",
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
