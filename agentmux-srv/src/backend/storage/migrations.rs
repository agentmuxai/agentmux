// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! SQL schema setup for WaveStore, FileStore, and the saga log.
//!
//! `objects.db` uses a **flat schema**: `run_object_schema` defines the
//! final table set directly in one idempotent `CREATE TABLE IF NOT EXISTS`
//! batch. It replaced an 11-step incremental migration chain
//! (`run_forge_v1` … `run_forge_v11`) — see
//! `docs/specs/SPEC_SCHEMA_FLATTENING_2026_05_19.md`. The chain was pure
//! historical accretion: per-version data dirs mean every new version is
//! born with a fresh `objects.db` and ran the whole chain top-to-bottom
//! anyway, so the intermediate states were never reachable in production.
//!
//! `filestore.db` and `sagas.db` were already single-DDL stores; they keep
//! their existing schema functions and gain only the `user_version`
//! tripwire (`stamp_and_check_version`).

use rusqlite::Connection;
use tracing::warn;

use super::error::StoreError;

/// `user_version` value stamped into `objects.db` after `run_object_schema`.
/// The flat schema resets the version counter to 1 — the pre-flatten
/// migration chain never set `user_version`, so legacy files read 0.
pub const OBJECT_SCHEMA_VERSION: i64 = 1;
/// `user_version` value stamped into `filestore.db`.
pub const FILESTORE_SCHEMA_VERSION: i64 = 1;
/// `user_version` value stamped into `sagas.db`.
pub const SAGA_LOG_SCHEMA_VERSION: i64 = 1;

/// Object type table names matching the `db_<otype>` convention.
const WSTORE_OTYPES: &[&str] = &[
    "client",
    "window",
    "workspace",
    "tab",
    "layout",
    "block",
    "temp",
];

/// Legacy `objects.db` table names retired by the de-forge rename, paired
/// with their replacement. `adopt_legacy_table_names` renames any of these
/// it finds — the single surviving piece of the old migration chain (it
/// also subsumes the v11 `db_identities`/`db_memories` rename).
const LEGACY_TABLE_RENAMES: &[(&str, &str)] = &[
    ("db_forge_agents", "db_agent_definitions"),
    ("db_forge_content", "db_agent_content"),
    ("db_forge_skills", "db_agent_skills"),
    ("db_forge_history", "db_agent_history"),
    ("db_forge_agent_identities", "db_agent_identity_links"),
    ("db_identities", "db_identity_bundles"),
    ("db_memories", "db_memory_bundles"),
];

/// Legacy index names that must be dropped after their table is renamed —
/// `ALTER TABLE … RENAME` keeps indexes attached but under their old names,
/// which would collide with the flat DDL's `CREATE INDEX`. The flat DDL
/// recreates each under the new name.
const LEGACY_INDEX_DROPS: &[&str] = &[
    "idx_forge_agents_slug",
    "idx_forge_history_agent_date",
    "idx_forge_agent_identities_account",
    "idx_identities_is_blank",
    "idx_memories_is_blank",
];

/// Tables retained by the old chain only for a downgrade path the flatten
/// abandons. Dropped from any legacy DB by the adopt step; never created
/// by the flat schema. `db_workflow_*` data was already copied into
/// `db_drone_*` by the old v10 migration, so dropping loses nothing.
const DEAD_TABLE_DROPS: &[&str] = &[
    "db_workflow_definitions",
    "db_workflow_runs",
    "db_v10_migrated_legacy_defs",
    "db_v10_migrated_legacy_runs",
];

/// Initialize (or re-validate) the full `objects.db` schema.
///
/// Idempotent — safe on every srv startup. Steps:
///
/// 1. `adopt_legacy_table_names` — renames any pre-flatten forge/bundle
///    tables found (protects dev databases created before the flatten;
///    see the spec §3/§7) and drops the dead workflow/sentinel tables.
/// 2. The flat `CREATE TABLE IF NOT EXISTS` batch — the canonical schema.
/// 3. Seeds the blank Identity / Memory singleton rows.
///
/// A database stuck at a pre-v11 intermediate schema cannot be fully
/// adopted (its tables predate later columns). The adopt step still
/// renames what it finds; the first query referencing a missing column
/// then fails loudly with `no such column` — a hard error, not silent
/// empty state, with the data preserved on disk. Per-version data dirs
/// make that case unreachable for released builds.
pub fn run_object_schema(conn: &Connection) -> Result<(), StoreError> {
    adopt_legacy_table_names(conn)?;

    // ---- Generic WaveObj object tables ----
    for otype in WSTORE_OTYPES {
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS db_{otype} (
                oid     TEXT PRIMARY KEY,
                version INTEGER NOT NULL DEFAULT 1,
                data    TEXT NOT NULL
            );"
        ))?;
    }

    // ---- Agent + identity + memory + drone schema ----
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS db_agent_definitions (
            id                   TEXT PRIMARY KEY,
            slug                 TEXT NOT NULL DEFAULT '',
            name                 TEXT NOT NULL,
            icon                 TEXT NOT NULL DEFAULT '✦',
            provider             TEXT NOT NULL,
            description          TEXT NOT NULL DEFAULT '',
            working_directory    TEXT NOT NULL DEFAULT '',
            shell                TEXT NOT NULL DEFAULT '',
            provider_flags       TEXT NOT NULL DEFAULT '',
            auto_start           INTEGER NOT NULL DEFAULT 0,
            restart_on_crash     INTEGER NOT NULL DEFAULT 0,
            idle_timeout_minutes INTEGER NOT NULL DEFAULT 0,
            agent_type           TEXT NOT NULL DEFAULT 'standalone',
            environment          TEXT NOT NULL DEFAULT '',
            agent_bus_id         TEXT NOT NULL DEFAULT '',
            is_seeded            INTEGER NOT NULL DEFAULT 0,
            accounts             TEXT NOT NULL DEFAULT '',
            parent_id            TEXT NOT NULL DEFAULT '',
            branch_label         TEXT NOT NULL DEFAULT '',
            created_at           INTEGER NOT NULL DEFAULT 0
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_agent_definitions_slug
            ON db_agent_definitions(slug);

        CREATE TABLE IF NOT EXISTS db_agent_content (
            agent_id     TEXT NOT NULL,
            content_type TEXT NOT NULL,
            content      TEXT NOT NULL DEFAULT '',
            updated_at   INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (agent_id, content_type),
            FOREIGN KEY (agent_id) REFERENCES db_agent_definitions(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS db_agent_skills (
            id          TEXT PRIMARY KEY,
            agent_id    TEXT NOT NULL,
            name        TEXT NOT NULL,
            trigger     TEXT NOT NULL DEFAULT '',
            skill_type  TEXT NOT NULL DEFAULT 'prompt',
            description TEXT NOT NULL DEFAULT '',
            content     TEXT NOT NULL DEFAULT '',
            created_at  INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (agent_id) REFERENCES db_agent_definitions(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS db_agent_history (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id     TEXT NOT NULL,
            session_date TEXT NOT NULL,
            entry        TEXT NOT NULL,
            timestamp    INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (agent_id) REFERENCES db_agent_definitions(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_agent_history_agent_date
            ON db_agent_history(agent_id, session_date);

        CREATE TABLE IF NOT EXISTS db_identity_accounts (
            id           TEXT PRIMARY KEY,
            name         TEXT NOT NULL,
            provider     TEXT NOT NULL,
            kind         TEXT NOT NULL,
            display_name TEXT NOT NULL DEFAULT '',
            secret_ref   TEXT NOT NULL,
            context      TEXT NOT NULL DEFAULT '{}',
            status       TEXT NOT NULL DEFAULT 'unknown',
            created_at   INTEGER NOT NULL DEFAULT 0,
            updated_at   INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_identity_accounts_provider
            ON db_identity_accounts(provider);

        CREATE TABLE IF NOT EXISTS db_agent_identity_links (
            agent_id   TEXT NOT NULL,
            account_id TEXT NOT NULL,
            provider   TEXT NOT NULL,
            PRIMARY KEY (agent_id, provider),
            FOREIGN KEY (agent_id)   REFERENCES db_agent_definitions(id) ON DELETE CASCADE,
            FOREIGN KEY (account_id) REFERENCES db_identity_accounts(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_agent_identity_links_account
            ON db_agent_identity_links(account_id);

        CREATE TABLE IF NOT EXISTS db_identity_bundles (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL UNIQUE,
            description TEXT NOT NULL DEFAULT '',
            is_blank    INTEGER NOT NULL DEFAULT 0,
            created_at  INTEGER NOT NULL DEFAULT 0,
            updated_at  INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_identity_bundles_is_blank
            ON db_identity_bundles(is_blank);

        CREATE TABLE IF NOT EXISTS db_identity_bindings (
            identity_id TEXT NOT NULL,
            provider    TEXT NOT NULL,
            account_id  TEXT NOT NULL,
            PRIMARY KEY (identity_id, provider),
            FOREIGN KEY (identity_id) REFERENCES db_identity_bundles(id)  ON DELETE CASCADE,
            FOREIGN KEY (account_id)  REFERENCES db_identity_accounts(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_identity_bindings_account
            ON db_identity_bindings(account_id);

        CREATE TABLE IF NOT EXISTS db_memory_bundles (
            id            TEXT PRIMARY KEY,
            name          TEXT NOT NULL UNIQUE,
            description   TEXT NOT NULL DEFAULT '',
            is_blank      INTEGER NOT NULL DEFAULT 0,
            provider      TEXT NOT NULL DEFAULT '',
            model         TEXT NOT NULL DEFAULT '',
            instructions  TEXT NOT NULL DEFAULT '',
            context_files TEXT NOT NULL DEFAULT '[]',
            mcp_servers   TEXT NOT NULL DEFAULT '[]',
            skills        TEXT NOT NULL DEFAULT '[]',
            created_at    INTEGER NOT NULL DEFAULT 0,
            updated_at    INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_memory_bundles_is_blank
            ON db_memory_bundles(is_blank);

        CREATE TABLE IF NOT EXISTS db_agent_instances (
            id                 TEXT PRIMARY KEY,
            definition_id      TEXT NOT NULL,
            parent_instance_id TEXT NOT NULL DEFAULT '',
            block_id           TEXT NOT NULL DEFAULT '',
            session_id         TEXT NOT NULL DEFAULT '',
            status             TEXT NOT NULL DEFAULT 'running',
            github_context     TEXT NOT NULL DEFAULT '',
            identity_id        TEXT NOT NULL DEFAULT '',
            memory_id          TEXT NOT NULL DEFAULT '',
            instance_name      TEXT NOT NULL DEFAULT '',
            working_directory  TEXT NOT NULL DEFAULT '',
            display_hidden     INTEGER NOT NULL DEFAULT 0,
            started_at         INTEGER NOT NULL DEFAULT 0,
            ended_at           INTEGER NOT NULL DEFAULT 0,
            created_at         INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (definition_id) REFERENCES db_agent_definitions(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_agent_instances_definition
            ON db_agent_instances(definition_id);
        CREATE INDEX IF NOT EXISTS idx_agent_instances_block
            ON db_agent_instances(block_id);
        CREATE INDEX IF NOT EXISTS idx_agent_instances_status
            ON db_agent_instances(status);
        CREATE INDEX IF NOT EXISTS idx_agent_instances_parent
            ON db_agent_instances(parent_instance_id);
        CREATE INDEX IF NOT EXISTS idx_agent_instances_name_recent
            ON db_agent_instances(instance_name, started_at DESC)
            WHERE display_hidden = 0 AND instance_name != '';

        CREATE TABLE IF NOT EXISTS db_drone_definitions (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            graph       TEXT NOT NULL DEFAULT '{\"nodes\":[],\"edges\":[]}',
            viewport    TEXT NOT NULL DEFAULT '{\"x\":0,\"y\":0,\"zoom\":1}',
            created_at  INTEGER NOT NULL DEFAULT 0,
            updated_at  INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_drone_definitions_updated
            ON db_drone_definitions(updated_at DESC);

        CREATE TABLE IF NOT EXISTS db_drone_runs (
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
        CREATE INDEX IF NOT EXISTS idx_drone_runs_drone_started
            ON db_drone_runs(drone_id, started_at DESC);
        CREATE INDEX IF NOT EXISTS idx_drone_runs_status
            ON db_drone_runs(status);",
    )?;

    // ---- Seed blank Identity / Memory singletons ----
    // The launch UI renders these as the default option in its Identity /
    // Memory dropdowns. Fixed ids so tests + dev seed data can hard-code
    // references.
    conn.execute_batch(
        "INSERT OR IGNORE INTO db_identity_bundles
            (id, name, description, is_blank, created_at, updated_at)
         VALUES ('blank', '__blank__', 'No credentials — use ambient', 1, 0, 0);

         INSERT OR IGNORE INTO db_memory_bundles
            (id, name, description, is_blank, created_at, updated_at)
         VALUES ('blank', '__blank__', 'Vanilla CLI — no instructions, no context', 1, 0, 0);",
    )?;

    Ok(())
}

/// Rename any pre-flatten `objects.db` tables to their de-forged names and
/// drop the dead workflow/sentinel tables. Idempotent — on a fresh or
/// already-flat database every check is a no-op.
///
/// This is the single surviving fragment of the old v1–v11 chain: it
/// exists only to carry a developer's pre-flatten `objects.db` (always at
/// the post-v11 schema, since v11 is merged) forward without data loss.
/// SQLite ≥ 3.25 auto-updates foreign-key references in child tables when
/// a parent table is renamed, so the agent/identity cascades survive.
fn adopt_legacy_table_names(conn: &Connection) -> Result<(), StoreError> {
    for (legacy, current) in LEGACY_TABLE_RENAMES {
        let legacy_exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [legacy],
            |row| row.get(0),
        )?;
        if legacy_exists == 0 {
            continue;
        }
        let current_exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [current],
            |row| row.get(0),
        )?;
        if current_exists == 1 {
            // Both present — only reachable on a deliberate
            // downgrade-roundtrip dev DB (flat build → pre-flatten build,
            // which re-creates the legacy name → flat build again). The
            // flatten abandons the downgrade path, but the legacy table
            // may hold rows the downgraded build wrote. Do NOT drop it —
            // that would be silent data loss (the bug class behind PR
            // #933's Codex P1). Leave it on disk and warn loudly so the
            // developer can recover or delete it manually.
            warn!(
                legacy_table = *legacy,
                current_table = *current,
                "objects.db has both a legacy table and its de-forged \
                 replacement — this only happens after a downgrade to a \
                 pre-flatten build; the legacy table is left untouched \
                 for manual recovery and is otherwise unused",
            );
        } else {
            conn.execute_batch(&format!("ALTER TABLE {legacy} RENAME TO {current};"))?;
        }
    }

    // Drop indexes orphaned by the renames — the flat DDL recreates them
    // under the new names.
    for idx in LEGACY_INDEX_DROPS {
        conn.execute_batch(&format!("DROP INDEX IF EXISTS {idx};"))?;
    }

    // Drop tables retained only for the abandoned downgrade path.
    for table in DEAD_TABLE_DROPS {
        conn.execute_batch(&format!("DROP TABLE IF EXISTS {table};"))?;
    }

    Ok(())
}

/// Initialize the FileStore schema. Creates the wave_file and file_data
/// tables. Already a flat single-DDL store — unaffected by the
/// `objects.db` flattening.
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

/// Initialize the saga durability schema (`saga` + `saga_step` tables and
/// their indexes). See `docs/specs/SPEC_SAGA_DURABILITY_2026-05-01.md` §2.2.
/// Already a flat single-DDL store.
pub fn run_saga_log_migrations(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS saga (
            saga_id        INTEGER PRIMARY KEY,
            name           TEXT NOT NULL,
            state          TEXT NOT NULL,
            started_at     INTEGER NOT NULL,
            terminal_at    INTEGER,
            failure_reason TEXT,
            input_json     TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS saga_step (
            saga_id     INTEGER NOT NULL REFERENCES saga(saga_id),
            step_index  INTEGER NOT NULL,
            name        TEXT NOT NULL,
            state       TEXT NOT NULL,
            cmd_json    TEXT NOT NULL,
            output_json TEXT,
            started_at  INTEGER NOT NULL,
            ended_at    INTEGER,
            PRIMARY KEY (saga_id, step_index)
        );

        CREATE INDEX IF NOT EXISTS saga_state_idx
            ON saga(state) WHERE state IN ('running', 'compensating');
        CREATE INDEX IF NOT EXISTS saga_terminal_idx
            ON saga(terminal_at);",
    )?;
    Ok(())
}

/// `PRAGMA user_version` tripwire (AUDIT_SQLITE_SYSTEMS §8.5).
///
/// Reads the file's `user_version`; if it exceeds `current` the database
/// was written by a newer AgentMux build — log a loud warning (new
/// tables/columns from that build are invisible here) but proceed,
/// because the idempotent `CREATE … IF NOT EXISTS` DDL keeps a forward-
/// compatible read working. Then stamps `current`.
///
/// Deliberately a tripwire, not a migration gate — the idempotent DDL
/// remains the schema mechanism; this only records the version and warns
/// on downgrade.
pub fn stamp_and_check_version(
    conn: &Connection,
    current: i64,
    db_label: &str,
) -> Result<(), StoreError> {
    let found: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if found > current {
        warn!(
            db = db_label,
            found, expected = current,
            "database user_version is newer than this build — it was written \
             by a newer AgentMux version; proceeding read-compatible, but \
             newer schema additions are not visible here",
        );
    }
    conn.execute_batch(&format!("PRAGMA user_version = {current};"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every table the flat `objects.db` schema must contain.
    const EXPECTED_TABLES: &[&str] = &[
        "db_client",
        "db_window",
        "db_workspace",
        "db_tab",
        "db_layout",
        "db_block",
        "db_temp",
        "db_agent_definitions",
        "db_agent_content",
        "db_agent_skills",
        "db_agent_history",
        "db_identity_accounts",
        "db_agent_identity_links",
        "db_identity_bundles",
        "db_identity_bindings",
        "db_memory_bundles",
        "db_agent_instances",
        "db_drone_definitions",
        "db_drone_runs",
    ];

    fn table_exists(conn: &Connection, name: &str) -> bool {
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [name],
                |row| row.get(0),
            )
            .unwrap();
        count == 1
    }

    fn index_exists(conn: &Connection, name: &str) -> bool {
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='index' AND name=?1",
                [name],
                |row| row.get(0),
            )
            .unwrap();
        count == 1
    }

    #[test]
    fn test_object_schema_creates_all_tables_and_singletons() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        run_object_schema(&conn).unwrap();

        for table in EXPECTED_TABLES {
            assert!(table_exists(&conn, table), "{table} should exist");
        }
        // De-forged + bundle indexes.
        for idx in &[
            "idx_agent_definitions_slug",
            "idx_agent_history_agent_date",
            "idx_agent_identity_links_account",
            "idx_identity_accounts_provider",
            "idx_identity_bundles_is_blank",
            "idx_identity_bindings_account",
            "idx_memory_bundles_is_blank",
            "idx_agent_instances_definition",
            "idx_agent_instances_name_recent",
            "idx_drone_definitions_updated",
            "idx_drone_runs_status",
        ] {
            assert!(index_exists(&conn, idx), "{idx} should exist");
        }

        // Blank singletons seeded.
        let id_blank: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_identity_bundles WHERE id='blank' AND is_blank=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(id_blank, 1, "blank Identity singleton should be seeded");
        let mem_blank: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_memory_bundles WHERE id='blank' AND is_blank=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(mem_blank, 1, "blank Memory singleton should be seeded");
    }

    #[test]
    fn test_object_schema_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        run_object_schema(&conn).unwrap();
        run_object_schema(&conn).unwrap(); // second pass must not error

        // Singletons stay unique.
        let id_count: i64 = conn
            .query_row("SELECT count(*) FROM db_identity_bundles", [], |r| r.get(0))
            .unwrap();
        assert_eq!(id_count, 1);
    }

    #[test]
    fn test_object_schema_omits_dead_tables() {
        let conn = Connection::open_in_memory().unwrap();
        run_object_schema(&conn).unwrap();
        for dead in DEAD_TABLE_DROPS {
            assert!(!table_exists(&conn, dead), "{dead} must not be created");
        }
        // Legacy forge names are never created either.
        for (legacy, _) in LEGACY_TABLE_RENAMES {
            assert!(
                !table_exists(&conn, legacy),
                "legacy {legacy} must not be created by the flat schema"
            );
        }
    }

    #[test]
    fn test_adopt_legacy_renames_forge_tables() {
        // Simulate a pre-flatten (post-v11) dev DB: legacy forge table
        // names + a dead workflow table, with seeded rows.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        conn.execute_batch(
            "CREATE TABLE db_forge_agents (
                id TEXT PRIMARY KEY, slug TEXT NOT NULL DEFAULT '', name TEXT NOT NULL,
                icon TEXT NOT NULL DEFAULT '✦', provider TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '', working_directory TEXT NOT NULL DEFAULT '',
                shell TEXT NOT NULL DEFAULT '', provider_flags TEXT NOT NULL DEFAULT '',
                auto_start INTEGER NOT NULL DEFAULT 0, restart_on_crash INTEGER NOT NULL DEFAULT 0,
                idle_timeout_minutes INTEGER NOT NULL DEFAULT 0,
                agent_type TEXT NOT NULL DEFAULT 'standalone', environment TEXT NOT NULL DEFAULT '',
                agent_bus_id TEXT NOT NULL DEFAULT '', is_seeded INTEGER NOT NULL DEFAULT 0,
                accounts TEXT NOT NULL DEFAULT '',
                parent_id TEXT NOT NULL DEFAULT '', branch_label TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL DEFAULT 0
            );
            CREATE UNIQUE INDEX idx_forge_agents_slug ON db_forge_agents(slug);
            INSERT INTO db_forge_agents (id, slug, name, provider)
                VALUES ('a1', 'coder', 'Coder', 'claude');

            CREATE TABLE db_workflow_definitions (id TEXT PRIMARY KEY);",
        )
        .unwrap();

        run_object_schema(&conn).unwrap();

        // Renamed, data preserved.
        assert!(table_exists(&conn, "db_agent_definitions"));
        assert!(!table_exists(&conn, "db_forge_agents"));
        let name: String = conn
            .query_row(
                "SELECT name FROM db_agent_definitions WHERE id='a1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(name, "Coder");
        // Old index dropped, new index present.
        assert!(!index_exists(&conn, "idx_forge_agents_slug"));
        assert!(index_exists(&conn, "idx_agent_definitions_slug"));
        // Dead table dropped.
        assert!(!table_exists(&conn, "db_workflow_definitions"));
    }

    #[test]
    fn test_adopt_legacy_is_noop_on_fresh_db() {
        let conn = Connection::open_in_memory().unwrap();
        run_object_schema(&conn).unwrap();
        // Re-running schema (which re-runs adopt) on the already-flat DB
        // leaves the de-forged tables intact and creates no legacy names.
        run_object_schema(&conn).unwrap();
        assert!(table_exists(&conn, "db_agent_definitions"));
        assert!(!table_exists(&conn, "db_forge_agents"));
    }

    #[test]
    fn test_adopt_legacy_both_tables_present_is_non_destructive() {
        // Downgrade-roundtrip: a flat DB (db_agent_definitions) where a
        // pre-flatten build later re-created db_forge_agents and wrote a
        // row. The adopt step must NOT drop the legacy table — silent
        // data loss is the bug class behind PR #933's Codex P1.
        let conn = Connection::open_in_memory().unwrap();
        run_object_schema(&conn).unwrap(); // creates db_agent_definitions
        conn.execute_batch(
            "CREATE TABLE db_forge_agents (id TEXT PRIMARY KEY, name TEXT NOT NULL);
             INSERT INTO db_forge_agents (id, name) VALUES ('downgrade-era', 'Recover Me');",
        )
        .unwrap();

        run_object_schema(&conn).unwrap();

        // Legacy table left intact — data recoverable, not dropped.
        assert!(table_exists(&conn, "db_forge_agents"));
        let name: String = conn
            .query_row(
                "SELECT name FROM db_forge_agents WHERE id='downgrade-era'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(name, "Recover Me");
        // Flat table still present and authoritative.
        assert!(table_exists(&conn, "db_agent_definitions"));
    }

    #[test]
    fn test_adopt_legacy_fk_cascade_survives_rename() {
        // A renamed parent must keep cascading into renamed children.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        conn.execute_batch(
            "CREATE TABLE db_forge_agents (
                id TEXT PRIMARY KEY, slug TEXT NOT NULL DEFAULT '', name TEXT NOT NULL,
                icon TEXT NOT NULL DEFAULT '✦', provider TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '', working_directory TEXT NOT NULL DEFAULT '',
                shell TEXT NOT NULL DEFAULT '', provider_flags TEXT NOT NULL DEFAULT '',
                auto_start INTEGER NOT NULL DEFAULT 0, restart_on_crash INTEGER NOT NULL DEFAULT 0,
                idle_timeout_minutes INTEGER NOT NULL DEFAULT 0,
                agent_type TEXT NOT NULL DEFAULT 'standalone', environment TEXT NOT NULL DEFAULT '',
                agent_bus_id TEXT NOT NULL DEFAULT '', is_seeded INTEGER NOT NULL DEFAULT 0,
                accounts TEXT NOT NULL DEFAULT '',
                parent_id TEXT NOT NULL DEFAULT '', branch_label TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE db_forge_content (
                agent_id TEXT NOT NULL, content_type TEXT NOT NULL,
                content TEXT NOT NULL DEFAULT '', updated_at INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (agent_id, content_type),
                FOREIGN KEY (agent_id) REFERENCES db_forge_agents(id) ON DELETE CASCADE
            );
            INSERT INTO db_forge_agents (id, name, provider) VALUES ('a1', 'Coder', 'claude');
            INSERT INTO db_forge_content (agent_id, content_type, content)
                VALUES ('a1', 'soul', 'hello');",
        )
        .unwrap();

        run_object_schema(&conn).unwrap();

        conn.execute("DELETE FROM db_agent_definitions WHERE id='a1'", [])
            .unwrap();
        let remaining: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_agent_content WHERE agent_id='a1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            remaining, 0,
            "FK cascade must survive the forge→agent table rename"
        );
    }

    #[test]
    fn test_stamp_and_check_version() {
        let conn = Connection::open_in_memory().unwrap();
        stamp_and_check_version(&conn, OBJECT_SCHEMA_VERSION, "objects.db").unwrap();
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, OBJECT_SCHEMA_VERSION);

        // A DB stamped with a higher version still opens (tripwire warns,
        // does not refuse) and gets re-stamped to the current value.
        conn.execute_batch("PRAGMA user_version = 99;").unwrap();
        stamp_and_check_version(&conn, OBJECT_SCHEMA_VERSION, "objects.db").unwrap();
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, OBJECT_SCHEMA_VERSION);
    }

    #[test]
    fn test_filestore_migrations_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run_filestore_migrations(&conn).unwrap();
        run_filestore_migrations(&conn).unwrap();
        assert!(table_exists(&conn, "db_wave_file"));
        assert!(table_exists(&conn, "db_file_data"));
    }

    #[test]
    fn test_saga_log_migrations_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run_saga_log_migrations(&conn).unwrap();
        run_saga_log_migrations(&conn).unwrap();
        assert!(table_exists(&conn, "saga"));
        assert!(table_exists(&conn, "saga_step"));
    }
}
