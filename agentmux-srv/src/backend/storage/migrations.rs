// Copyright 2025-2026, AgentMux Corp.
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
    run_forge_v5_migrations(conn)?;
    run_forge_v6_migrations(conn)?;
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

/// Forge v4 migrations: add `slug` column — a stable, filesystem-safe
/// identifier distinct from the renameable `name`. Backfills existing
/// rows by deriving slug from name (with collision suffixes), then
/// adds a unique index.
///
/// See specs/SPEC_AGENT_IDENTITY_RESTRUCTURE_2026_04_14.md for design.
pub fn run_forge_v4_migrations(conn: &Connection) -> Result<(), StoreError> {
    use std::collections::HashSet;

    // 1. Add the column (empty default so existing rows survive)
    match conn.execute_batch(
        "ALTER TABLE db_forge_agents ADD COLUMN slug TEXT NOT NULL DEFAULT ''",
    ) {
        Ok(_) => {}
        Err(e) => {
            let msg = e.to_string();
            if !msg.contains("duplicate column") {
                return Err(StoreError::Sqlite(match e {
                    rusqlite::Error::SqliteFailure(code, _) => {
                        rusqlite::Error::SqliteFailure(code, Some(msg))
                    }
                    other => other,
                }));
            }
        }
    }

    // 2. Backfill: find rows with empty slug, derive from name, collision-resolve.
    let rows_to_backfill: Vec<(String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, name FROM db_forge_agents WHERE slug IS NULL OR slug = ''",
        )?;
        let mapped = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        mapped
    };

    let mut used_slugs: HashSet<String> = {
        let mut stmt =
            conn.prepare("SELECT slug FROM db_forge_agents WHERE slug != ''")?;
        let collected = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<HashSet<_>, _>>()?;
        collected
    };

    for (id, name) in rows_to_backfill {
        let base = super::wstore::derive_slug(&name);
        let mut candidate = base.clone();
        let mut n: u32 = 2;
        while used_slugs.contains(&candidate) {
            candidate = format!("{}-{}", base, n);
            n += 1;
        }
        used_slugs.insert(candidate.clone());
        conn.execute(
            "UPDATE db_forge_agents SET slug = ?1 WHERE id = ?2",
            rusqlite::params![candidate, id],
        )?;
    }

    // 3. Unique index on slug (after backfill so existing dupes are resolved).
    conn.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_forge_agents_slug
             ON db_forge_agents(slug)",
    )?;

    Ok(())
}

/// Forge v5 migrations: add `accounts` column — JSON-encoded per-provider
/// account references owned by each agent.
/// Shape: `{"github":"acct-id"|null,"aws":"acct-id"|null,...}`
/// Empty string = no accounts assigned. Existing rows default to ''.
///
/// See SPEC_AGENT_IDENTITY_RESTRUCTURE_2026_04_14.md §3.3.
pub fn run_forge_v5_migrations(conn: &Connection) -> Result<(), StoreError> {
    match conn.execute_batch(
        "ALTER TABLE db_forge_agents ADD COLUMN accounts TEXT NOT NULL DEFAULT ''",
    ) {
        Ok(_) => {}
        Err(e) => {
            let msg = e.to_string();
            if !msg.contains("duplicate column") {
                return Err(StoreError::Sqlite(match e {
                    rusqlite::Error::SqliteFailure(code, _) => {
                        rusqlite::Error::SqliteFailure(code, Some(msg))
                    }
                    other => other,
                }));
            }
        }
    }
    Ok(())
}

/// Forge v6 migrations: identity accounts + agent instances + definition
/// branching. See specs/SPEC_FORGE_IDENTITY_AGENT_INSTANCES_IMPL_2026_04_20.md.
///
/// - `db_identity_accounts`: replaces the browser-localStorage identity store
///   with a real, portable-aware, DB-backed record.
/// - `db_forge_agent_identities`: junction table linking agents to identities
///   (replaces the unused v5 `accounts` JSON blob).
/// - `db_agent_instances`: one row per running/historical execution of an
///   agent definition. Tracks which pane it lives in, which provider
///   session, and optional GitHub context (which PR/branch this run is
///   operating against).
/// - `parent_id` + `branch_label` columns on `db_forge_agents`: lets a
///   definition be forked from another with lineage preserved.
///
/// Since no user has populated the v5 `accounts` column meaningfully (it's
/// always been dev-only localStorage in practice), we don't migrate data from
/// it — existing identities must be recreated. The `accounts` column itself
/// stays in the schema as dead weight for now rather than risk a `DROP COLUMN`
/// on rows that might have partial data; a future v7 can clean it up.
pub fn run_forge_v6_migrations(conn: &Connection) -> Result<(), StoreError> {
    // ---- Lineage columns on existing agents table ----
    let alter_statements = [
        "ALTER TABLE db_forge_agents ADD COLUMN parent_id TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_forge_agents ADD COLUMN branch_label TEXT NOT NULL DEFAULT ''",
    ];
    for stmt in &alter_statements {
        match conn.execute_batch(stmt) {
            Ok(_) => {}
            Err(e) => {
                let msg = e.to_string();
                if !msg.contains("duplicate column") {
                    return Err(StoreError::Sqlite(match e {
                        rusqlite::Error::SqliteFailure(code, _) => {
                            rusqlite::Error::SqliteFailure(code, Some(msg))
                        }
                        other => other,
                    }));
                }
            }
        }
    }

    // ---- New tables ----
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS db_identity_accounts (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            provider TEXT NOT NULL,
            kind TEXT NOT NULL,
            display_name TEXT NOT NULL DEFAULT '',
            secret_ref TEXT NOT NULL,
            context TEXT NOT NULL DEFAULT '{}',
            status TEXT NOT NULL DEFAULT 'unknown',
            created_at INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_identity_accounts_provider
            ON db_identity_accounts(provider);

        CREATE TABLE IF NOT EXISTS db_forge_agent_identities (
            agent_id TEXT NOT NULL,
            account_id TEXT NOT NULL,
            provider TEXT NOT NULL,
            PRIMARY KEY (agent_id, provider),
            FOREIGN KEY (agent_id) REFERENCES db_forge_agents(id) ON DELETE CASCADE,
            FOREIGN KEY (account_id) REFERENCES db_identity_accounts(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_forge_agent_identities_account
            ON db_forge_agent_identities(account_id);

        CREATE TABLE IF NOT EXISTS db_agent_instances (
            id TEXT PRIMARY KEY,
            definition_id TEXT NOT NULL,
            parent_instance_id TEXT NOT NULL DEFAULT '',
            block_id TEXT NOT NULL DEFAULT '',
            session_id TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL DEFAULT 'running',
            github_context TEXT NOT NULL DEFAULT '',
            started_at INTEGER NOT NULL DEFAULT 0,
            ended_at INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (definition_id) REFERENCES db_forge_agents(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_agent_instances_definition
            ON db_agent_instances(definition_id);
        CREATE INDEX IF NOT EXISTS idx_agent_instances_block
            ON db_agent_instances(block_id);
        CREATE INDEX IF NOT EXISTS idx_agent_instances_status
            ON db_agent_instances(status);
        CREATE INDEX IF NOT EXISTS idx_agent_instances_parent
            ON db_agent_instances(parent_instance_id);",
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

    #[test]
    fn test_forge_v4_slug_backfill_and_collision() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();

        // Run forge migrations up to v3 (no slug yet) manually to simulate
        // a pre-v4 database. We go through run_forge_migrations which now
        // includes v4 — but since the ALTER+backfill is idempotent, we can
        // inject pre-existing rows and re-run to exercise the backfill path.
        run_forge_migrations(&conn).unwrap();

        // Drop the unique index FIRST so we can stage rows with empty
        // slugs (which would otherwise collide with each other under
        // the UNIQUE constraint).
        conn.execute_batch("DROP INDEX IF EXISTS idx_forge_agents_slug")
            .unwrap();

        // Wipe the slug and insert rows as if they came from v3
        conn.execute_batch("UPDATE db_forge_agents SET slug = ''").unwrap();

        conn.execute(
            "INSERT INTO db_forge_agents (id, name, icon, provider, description,
             created_at, is_seeded)
             VALUES (?1, ?2, '✦', 'claude', '', 0, 0)",
            rusqlite::params!["id-a", "AgentX"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO db_forge_agents (id, name, icon, provider, description,
             created_at, is_seeded)
             VALUES (?1, ?2, '✦', 'claude', '', 0, 0)",
            rusqlite::params!["id-b", "AgentX"], // collision!
        )
        .unwrap();
        conn.execute(
            "INSERT INTO db_forge_agents (id, name, icon, provider, description,
             created_at, is_seeded)
             VALUES (?1, ?2, '✦', 'claude', '', 0, 0)",
            rusqlite::params!["id-c", "My Fancy Agent!"],
        )
        .unwrap();

        // Re-run the v4 migration — should backfill all three rows
        run_forge_v4_migrations(&conn).unwrap();

        let mut stmt = conn
            .prepare("SELECT id, slug FROM db_forge_agents ORDER BY id")
            .unwrap();
        let rows: Vec<(String, String)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        // id-a wins "agentx", id-b gets "agentx-2", id-c gets "my-fancy-agent"
        assert_eq!(rows.len(), 3);
        let a = rows.iter().find(|(id, _)| id == "id-a").unwrap();
        let b = rows.iter().find(|(id, _)| id == "id-b").unwrap();
        let c = rows.iter().find(|(id, _)| id == "id-c").unwrap();
        assert_eq!(a.1, "agentx");
        assert_eq!(b.1, "agentx-2");
        assert_eq!(c.1, "my-fancy-agent");

        // Unique index must exist after the migration
        let idx_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master
                 WHERE type='index' AND name='idx_forge_agents_slug'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(idx_count, 1);
    }

    #[test]
    fn test_forge_v6_migrations_create_tables_and_columns() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();

        // Full forge migration chain creates v1..v6
        run_forge_migrations(&conn).unwrap();

        // All three new tables exist
        for table in &["db_identity_accounts", "db_forge_agent_identities", "db_agent_instances"] {
            let count: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "table {table} should exist");
        }

        // Lineage columns added to db_forge_agents
        let cols: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(db_forge_agents)").unwrap();
            stmt.query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert!(cols.contains(&"parent_id".to_string()));
        assert!(cols.contains(&"branch_label".to_string()));

        // Indexes created
        for idx in &[
            "idx_identity_accounts_provider",
            "idx_forge_agent_identities_account",
            "idx_agent_instances_definition",
            "idx_agent_instances_block",
            "idx_agent_instances_status",
            "idx_agent_instances_parent",
        ] {
            let count: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='index' AND name=?1",
                    [idx],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "index {idx} should exist");
        }
    }

    #[test]
    fn test_forge_v6_migrations_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();

        run_forge_migrations(&conn).unwrap();
        run_forge_migrations(&conn).unwrap();  // second pass must not error

        // Insert an identity + agent + junction to exercise the FKs
        conn.execute(
            "INSERT INTO db_forge_agents (id, name, provider, description, slug)
             VALUES ('ag1', 'AgentX', 'claude', '', 'agentx')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO db_identity_accounts (id, name, provider, kind, secret_ref, created_at, updated_at)
             VALUES ('id1', 'asaf-github', 'github', 'pat', '{\"backend\":\"env\",\"env_var\":\"GITHUB_TOKEN\"}', 0, 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO db_forge_agent_identities (agent_id, account_id, provider)
             VALUES ('ag1', 'id1', 'github')",
            [],
        )
        .unwrap();

        // FK cascade: deleting the agent removes the junction row
        conn.execute("DELETE FROM db_forge_agents WHERE id='ag1'", []).unwrap();
        let remaining: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_forge_agent_identities WHERE agent_id='ag1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0, "junction row should cascade-delete");
    }
}
