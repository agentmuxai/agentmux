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

/// Initialize the saga durability schema.
/// Creates the `saga` + `saga_step` tables and their indexes.
///
/// See `docs/specs/SPEC_SAGA_DURABILITY_2026-05-01.md` §2.2 — the
/// durable on-disk record of every saga's lifecycle. The coordinator
/// writes here from `SagaCtx::dispatch` / `compensate` (per-step) and
/// `emit_terminal` (per-saga) so that a srv crash mid-saga leaves a
/// recoverable trail.
///
/// PR 1 (this migration) only ships the schema + log API + ctx
/// instrumentation. Resume-on-startup + `--diag sagas` + crash-
/// recovery integration tests follow in PR 2.
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
    run_forge_v1_migrations(conn)?;
    run_forge_v2_migrations(conn)?;
    run_forge_v3_migrations(conn)?;
    run_forge_v4_migrations(conn)?;
    run_forge_v5_migrations(conn)?;
    run_forge_v6_migrations(conn)?;
    run_forge_v7_migrations(conn)?;
    run_forge_v8_migrations(conn)?;
    run_forge_v9_migrations(conn)?;
    run_forge_v10_migrations(conn)?;
    run_forge_v11_migrations(conn)?;
    Ok(())
}

/// Forge v1: original `db_forge_agents` base table. Kept as a separate
/// step so tests can stage a pre-v7 state by composing v1..v6 without
/// pulling later migrations (notably v11's rename to bundle tables).
pub fn run_forge_v1_migrations(conn: &Connection) -> Result<(), StoreError> {
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

/// Forge v7 migrations: promote Identity to a named-bundle entity, introduce
/// Memory as a sibling first-class entity, and rewire `db_agent_instances`
/// to reference both via FKs. See
/// `docs/specs/identity-forge-integration-and-vault-2026-05-08.md`.
///
/// Schema changes:
///
/// - `db_identities` (new — later renamed to `db_identity_bundles` by v11):
///   named Identity bundles. Each row has a unique `name` (e.g. "Work",
///   "Personal"). The `is_blank` flag tags the seeded singleton row that the
///   launch UI defaults to.
/// - `db_identity_bindings` (new): junction `(identity_id, provider) →
///   account_id`. Replaces the per-agent semantics of v6's
///   `db_forge_agent_identities` (which is left in place as legacy fallback;
///   a future v8 may drop it once all readers are migrated).
/// - `db_memories` (new — later renamed to `db_memory_bundles` by v11): named
///   Memory bundles holding the agent's personality / capability stack —
///   provider, model, instructions, context files, MCP servers, skills.
///   Forge agents (`db_forge_agents`) get shadow-migrated into Memory rows so
///   existing definitions remain accessible from the new code paths without
///   data loss.
/// - `db_agent_instances.identity_id` / `db_agent_instances.memory_id` (new
///   columns): the launch composition. NULL means "use the blank
///   singleton".
///
/// Idempotent: re-running `run_forge_migrations` after v7 has executed is a
/// no-op. The legacy v6 tables (`db_forge_agents`, `db_forge_agent_identities`)
/// are intentionally NOT dropped here; readers in the migrated code path
/// ignore them, but they remain on disk so a downgrade path stays open until
/// v8.
pub fn run_forge_v7_migrations(conn: &Connection) -> Result<(), StoreError> {
    // ---- Add identity_id / memory_id columns to db_agent_instances ----
    // Always runs (idempotent: duplicate-column errors swallowed).
    // Unaffected by v11's bundle-table rename — these are columns on
    // db_agent_instances, not on the renamed tables.
    let alter_statements = [
        "ALTER TABLE db_agent_instances ADD COLUMN identity_id TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_agent_instances ADD COLUMN memory_id TEXT NOT NULL DEFAULT ''",
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

    // Guard: once v11 has renamed `db_identities` → `db_identity_bundles`
    // and `db_memories` → `db_memory_bundles`, the legacy-named CREATE +
    // singleton seed + shadow-migrate block below would either re-create
    // empty old-named tables alongside the renamed ones, or write to old
    // names that no longer exist. Skip the legacy block in that case —
    // the data is safe under the new names.
    let bundles_exist: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type='table' AND name='db_identity_bundles'",
        [],
        |row| row.get(0),
    )?;

    if bundles_exist == 0 {
        run_forge_v7_legacy_ddl_and_seed(conn)?;
    }

    Ok(())
}

/// Legacy-name DDL + singleton seed + shadow-migrate from v7. Split out
/// of `run_forge_v7_migrations` so the guard there can skip it once v11
/// has renamed the bundle tables. See v7's doc comment for what each
/// statement does and why.
fn run_forge_v7_legacy_ddl_and_seed(conn: &Connection) -> Result<(), StoreError> {
    // ---- New tables ----
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS db_identities (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            description TEXT NOT NULL DEFAULT '',
            is_blank INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_identities_is_blank
            ON db_identities(is_blank);

        CREATE TABLE IF NOT EXISTS db_identity_bindings (
            identity_id TEXT NOT NULL,
            provider TEXT NOT NULL,
            account_id TEXT NOT NULL,
            PRIMARY KEY (identity_id, provider),
            FOREIGN KEY (identity_id) REFERENCES db_identities(id) ON DELETE CASCADE,
            FOREIGN KEY (account_id) REFERENCES db_identity_accounts(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_identity_bindings_account
            ON db_identity_bindings(account_id);

        CREATE TABLE IF NOT EXISTS db_memories (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            description TEXT NOT NULL DEFAULT '',
            is_blank INTEGER NOT NULL DEFAULT 0,
            provider TEXT NOT NULL DEFAULT '',
            model TEXT NOT NULL DEFAULT '',
            instructions TEXT NOT NULL DEFAULT '',
            context_files TEXT NOT NULL DEFAULT '[]',
            mcp_servers TEXT NOT NULL DEFAULT '[]',
            skills TEXT NOT NULL DEFAULT '[]',
            created_at INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_memories_is_blank
            ON db_memories(is_blank);",
    )?;

    // ---- Seed blank singletons ----
    // The launch UI always renders these as the default option in the
    // Identity / Memory dropdowns. Use fixed UUIDs so callers can hard-code
    // references in tests + dev seed data.
    conn.execute_batch(
        "INSERT OR IGNORE INTO db_identities (id, name, description, is_blank, created_at, updated_at)
         VALUES ('blank', '__blank__', 'No credentials — use ambient', 1, 0, 0);

         INSERT OR IGNORE INTO db_memories (id, name, description, is_blank, created_at, updated_at)
         VALUES ('blank', '__blank__', 'Vanilla CLI — no instructions, no context', 1, 0, 0);",
    )?;

    // ---- Shadow-migrate existing forge agents into memories ----
    // Of the Memory shape, only `provider` lives directly on
    // db_forge_agents — the v6 schema doesn't have `model` or
    // `instructions` columns. Forge stored those in db_forge_content
    // rows keyed by content_type ("soul", "agentmd", etc.); importing
    // them is deferred to a follow-up so this migration stays small.
    // db_forge_content rows remain readable on disk until v8.
    //
    // What this SELECT does copy: id (preserved so existing
    // db_agent_instances.definition_id references resolve via either
    // reader during the transition window), name (with
    // disambiguation, see below), description, provider, created_at.
    // model / instructions / context_files / mcp_servers / skills are
    // seeded empty; users fill them in via the Memory pane.
    //
    // **Name disambiguation (codex P1, 2026-05-08):**
    // db_forge_agents.name allows duplicates (only `slug` is collision-
    // resolved by the v4 migration). db_memories.name has a UNIQUE
    // constraint, so a naive `INSERT OR IGNORE ... SELECT name` would
    // silently drop all but one duplicate; the later memory_id backfill
    // would then leave instances of the dropped definitions pointing at
    // a non-existent memory_id. We disambiguate explicitly:
    //
    //   - If the forge agent's name is unique among forge agents AND no
    //     existing db_memories row has the same name (e.g. user manually
    //     created a memory before the migration), keep the original.
    //   - Otherwise append " [<id-prefix>]" — first 8 chars of the agent
    //     id, which IS unique because id is the primary key. The suffix
    //     is deterministic so re-running the migration after a v8
    //     downgrade-then-re-upgrade produces the same name.
    //
    // The OR IGNORE on the INSERT remains as belt-and-braces against the
    // edge case where the user has both a forge agent "X" AND a memory
    // "X [<same-prefix>]" pre-existing; in that case we accept silent
    // skip rather than fail the whole migration.
    conn.execute_batch(
        "INSERT OR IGNORE INTO db_memories
            (id, name, description, is_blank, provider, model, instructions,
             context_files, mcp_servers, skills, created_at, updated_at)
         SELECT
            fa.id,
            CASE
                WHEN EXISTS (
                    SELECT 1 FROM db_forge_agents fb
                    WHERE fb.name = fa.name AND fb.id != fa.id
                ) OR EXISTS (
                    SELECT 1 FROM db_memories m WHERE m.name = fa.name
                )
                THEN fa.name || ' [' || substr(fa.id, 1, 8) || ']'
                ELSE fa.name
            END,
            COALESCE(fa.description, ''),
            0,
            COALESCE(fa.provider, ''),
            '',
            '',
            '[]',
            '[]',
            '[]',
            COALESCE(fa.created_at, 0),
            COALESCE(fa.created_at, 0)
         FROM db_forge_agents fa
         WHERE NOT EXISTS (SELECT 1 FROM db_memories m WHERE m.id = fa.id);",
    )?;

    // ---- Backfill memory_id on existing instances ----
    // Existing db_agent_instances rows reference a forge_agents.id via
    // definition_id. Since we copied each forge agent into db_memories with
    // the same id, the backfill is a straight assignment.
    conn.execute_batch(
        "UPDATE db_agent_instances
         SET memory_id = definition_id
         WHERE memory_id = '' AND definition_id <> '';",
    )?;

    Ok(())
}

/// Forge v8 migrations: persist the human-readable instance name +
/// resolved working directory on every `db_agent_instances` row so the
/// launch modal can list past named agents and continue them. Adds a
/// soft-delete column (`display_hidden`) for the "Forget agent"
/// affordance — destructive deletion of the row + working dir is
/// out of scope for v8 (separate confirm flow + spec).
///
/// See `docs/specs/SPEC_NAMED_AGENT_CONTINUATION_2026_05_12.md`.
///
/// Schema changes:
///
/// - `db_agent_instances.instance_name` (new column): user-chosen
///   instance name (becomes `AGENTMUX_AGENT_ID` in the spawn env).
///   Empty string for legacy rows — they don't appear in the dropdown.
/// - `db_agent_instances.working_directory` (new column): absolute
///   path returned by `allocate_agent_workdir` at spawn time. Stored
///   here (rather than re-derived from the slug at continue-time) to
///   stay robust against slug-rule changes and user-side renames.
/// - `db_agent_instances.display_hidden` (new column, INTEGER 0/1):
///   soft-delete flag for the "Forget agent" affordance. Hidden rows
///   stay on disk for audit + recovery.
///
/// Idempotent: re-running after v8 has executed is a no-op
/// ("duplicate column" errors are caught and ignored, matching v2/v7
/// precedent).
pub fn run_forge_v8_migrations(conn: &Connection) -> Result<(), StoreError> {
    let alter_statements = [
        "ALTER TABLE db_agent_instances ADD COLUMN instance_name TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_agent_instances ADD COLUMN working_directory TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_agent_instances ADD COLUMN display_hidden INTEGER NOT NULL DEFAULT 0",
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

    // Partial index supports the `ListNamedAgentsCommand` query: rows
    // with a non-empty instance_name and not hidden, ordered by
    // recency. Keeps the dropdown lookup to a single b-tree scan.
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_agent_instances_name_recent
            ON db_agent_instances(instance_name, started_at DESC)
            WHERE display_hidden = 0 AND instance_name != '';",
    )?;

    Ok(())
}

/// v9 — Workflows pane (issue #753, RFC: Workflows pane Phase 1).
///
/// Adds two tables for the visual DAG-of-blocks workflow engine:
///
///   * `db_workflow_definitions` — workflow JSON (nodes, edges, viewport)
///     keyed by id, used by the canvas + executor.
///   * `db_workflow_runs` — append-only run history. One row per
///     `RunWorkflow` invocation. Holds the per-block status snapshot
///     emitted on completion. High-churn time-series; oldest rows are
///     evicted by a future retention task.
///
/// Renumbered from v8 → v9 during the merge with main, where v8 was
/// taken by the named-agent-continuation migration (#816).
pub fn run_forge_v9_migrations(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS db_workflow_definitions (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            graph TEXT NOT NULL DEFAULT '{\"nodes\":[],\"edges\":[]}',
            viewport TEXT NOT NULL DEFAULT '{\"x\":0,\"y\":0,\"zoom\":1}',
            created_at INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_workflow_definitions_updated
            ON db_workflow_definitions(updated_at DESC);

        CREATE TABLE IF NOT EXISTS db_workflow_runs (
            id TEXT PRIMARY KEY,
            workflow_id TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'running',
            started_at INTEGER NOT NULL DEFAULT 0,
            ended_at INTEGER NOT NULL DEFAULT 0,
            block_states TEXT NOT NULL DEFAULT '{}',
            output TEXT NOT NULL DEFAULT '',
            error TEXT NOT NULL DEFAULT '',
            FOREIGN KEY (workflow_id) REFERENCES db_workflow_definitions(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_workflow_runs_workflow_started
            ON db_workflow_runs(workflow_id, started_at DESC);
        CREATE INDEX IF NOT EXISTS idx_workflow_runs_status
            ON db_workflow_runs(status);",
    )?;

    Ok(())
}

/// v10 — rename "Workflows" feature to "Drone".
///
/// Creates the drone tables (`db_drone_definitions`, `db_drone_runs`)
/// and copies existing data from the v9 workflow tables. Schema is
/// identical to v9 except that `db_drone_runs` renames the
/// `workflow_id` column to `drone_id` (and the FK now points at
/// `db_drone_definitions`).
///
/// Following the v7 pattern, the legacy v9 tables (`db_workflow_*`)
/// are NOT dropped — they remain on disk so a downgrade path stays
/// open. Readers in the renamed code path ignore them.
///
/// Idempotent: re-running this migration is a no-op. Schema creation
/// uses `CREATE TABLE IF NOT EXISTS`; the v9 → v10 data copy is gated
/// **per-row** by sentinel tables so legacy rows are migrated exactly
/// once each (see "Per-row copy gate" below).
///
/// **Per-row copy gate.** A naive `INSERT OR IGNORE` over the v9
/// `db_workflow_*` tables on every srv startup has two failure modes
/// that arise from the fact that we intentionally retain the v9
/// tables on disk for downgrade safety:
///
///   1. **Resurrection** (codex P2 round 1 on PR #912): a user deletes
///      a migrated drone; on the next launch the copy re-runs and
///      re-creates the row from the still-populated legacy table.
///   2. **Roundtrip loss** (codex P2 round 2): a user upgrades,
///      downgrades to the v9 build, creates a workflow under the old
///      schema, and upgrades again — a singleton "v10 copy done"
///      marker would short-circuit before noticing the new legacy row.
///
/// Sentinel tables `db_v10_migrated_legacy_defs` / `_runs` record each
/// legacy id that has been copied. Per-row gating gets both right:
///
///   * Deleted drones stay deleted (their legacy id is in the sentinel,
///     so re-copy is skipped).
///   * Newly-appearing legacy rows (downgrade-then-recreate) are copied
///     on the next v10 run because their ids aren't yet in the sentinel.
pub fn run_forge_v10_migrations(conn: &Connection) -> Result<(), StoreError> {
    // 1. Schema — always idempotent.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS db_drone_definitions (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            graph TEXT NOT NULL DEFAULT '{\"nodes\":[],\"edges\":[]}',
            viewport TEXT NOT NULL DEFAULT '{\"x\":0,\"y\":0,\"zoom\":1}',
            created_at INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_drone_definitions_updated
            ON db_drone_definitions(updated_at DESC);

        CREATE TABLE IF NOT EXISTS db_drone_runs (
            id TEXT PRIMARY KEY,
            drone_id TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'running',
            started_at INTEGER NOT NULL DEFAULT 0,
            ended_at INTEGER NOT NULL DEFAULT 0,
            block_states TEXT NOT NULL DEFAULT '{}',
            output TEXT NOT NULL DEFAULT '',
            error TEXT NOT NULL DEFAULT '',
            FOREIGN KEY (drone_id) REFERENCES db_drone_definitions(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_drone_runs_drone_started
            ON db_drone_runs(drone_id, started_at DESC);
        CREATE INDEX IF NOT EXISTS idx_drone_runs_status
            ON db_drone_runs(status);

        CREATE TABLE IF NOT EXISTS db_v10_migrated_legacy_defs (
            legacy_id TEXT PRIMARY KEY,
            copied_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS db_v10_migrated_legacy_runs (
            legacy_id TEXT PRIMARY KEY,
            copied_at INTEGER NOT NULL DEFAULT 0
        );",
    )?;

    // 2. Per-row copy from v9 → v10. Skip rows we've already migrated
    //    (sentinel hit) and rows whose legacy table doesn't exist
    //    (fresh DB built straight at v10). Copy + sentinel insert run
    //    in a single transaction so partial failure rolls back and
    //    the next launch retries the same row set.
    let defs_exist: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master
             WHERE type='table' AND name='db_workflow_definitions'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let runs_exist: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master
             WHERE type='table' AND name='db_workflow_runs'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if defs_exist == 0 && runs_exist == 0 {
        return Ok(());
    }

    let mut sql = String::from("BEGIN IMMEDIATE;\n");
    if defs_exist > 0 {
        sql.push_str(
            "INSERT OR IGNORE INTO db_drone_definitions
                (id, name, description, graph, viewport, created_at, updated_at)
             SELECT id, name, description, graph, viewport, created_at, updated_at
             FROM db_workflow_definitions wf
             WHERE NOT EXISTS (
                 SELECT 1 FROM db_v10_migrated_legacy_defs s
                  WHERE s.legacy_id = wf.id
             );

             INSERT OR IGNORE INTO db_v10_migrated_legacy_defs
                (legacy_id, copied_at)
             SELECT id, CAST(strftime('%s', 'now') AS INTEGER) * 1000
             FROM db_workflow_definitions wf
             WHERE NOT EXISTS (
                 SELECT 1 FROM db_v10_migrated_legacy_defs s
                  WHERE s.legacy_id = wf.id
             );\n",
        );
    }
    if runs_exist > 0 {
        // Both the INSERT and the sentinel insert require the parent
        // drone to exist. PRAGMA foreign_keys=ON in wstore.rs means an
        // orphan-run insert would fail the FK and roll back the whole
        // migration transaction, blocking startup. Orphans arise if a
        // user deletes a migrated drone, downgrades, runs the still-
        // present legacy workflow, and re-upgrades — the parent's
        // sentinel keeps the drone from coming back, so the runs need
        // to filter on parent presence too. (codex P2 round 3 on #912.)
        // Unmarked orphans are re-checked next launch for free; if the
        // parent never appears again the runs stay invisible, which
        // matches the user's intent (they deleted the drone).
        sql.push_str(
            "INSERT OR IGNORE INTO db_drone_runs
                (id, drone_id, status, started_at, ended_at, block_states, output, error)
             SELECT id, workflow_id, status, started_at, ended_at, block_states, output, error
             FROM db_workflow_runs wr
             WHERE NOT EXISTS (
                 SELECT 1 FROM db_v10_migrated_legacy_runs s
                  WHERE s.legacy_id = wr.id
             ) AND EXISTS (
                 SELECT 1 FROM db_drone_definitions d
                  WHERE d.id = wr.workflow_id
             );

             INSERT OR IGNORE INTO db_v10_migrated_legacy_runs
                (legacy_id, copied_at)
             SELECT id, CAST(strftime('%s', 'now') AS INTEGER) * 1000
             FROM db_workflow_runs wr
             WHERE NOT EXISTS (
                 SELECT 1 FROM db_v10_migrated_legacy_runs s
                  WHERE s.legacy_id = wr.id
             ) AND EXISTS (
                 SELECT 1 FROM db_drone_definitions d
                  WHERE d.id = wr.workflow_id
             );\n",
        );
    }
    sql.push_str("COMMIT;");
    conn.execute_batch(&sql)?;

    Ok(())
}

/// Forge v11 migrations: rename `db_identities` → `db_identity_bundles`
/// and `db_memories` → `db_memory_bundles`. The "bundle" suffix conveys
/// that each row carries multiple facets — provider bindings + display
/// name (for identities) and instructions + context_files + mcp_servers
/// + skills (for memories) — so the UI's "Identity bundle" / "Memory
/// bundle" terminology matches the storage layer.
///
/// Closes AUDIT_SQLITE_SYSTEMS §8.1.
///
/// Idempotent: skips when the new names already exist. SQLite ≥ 3.25
/// auto-updates the FK reference in `db_identity_bindings` when its
/// parent table is renamed, so no separate touch of the binding table
/// is required.
///
/// `db_identity_bindings` itself is NOT renamed — its name was already
/// suffix-consistent with the bundle vocabulary (a binding binds an
/// identity bundle to a provider account; the surrounding object is the
/// binding, not the bundle).
pub fn run_forge_v11_migrations(conn: &Connection) -> Result<(), StoreError> {
    let identities_exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type='table' AND name='db_identities'",
        [],
        |row| row.get(0),
    )?;
    let identity_bundles_exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type='table' AND name='db_identity_bundles'",
        [],
        |row| row.get(0),
    )?;

    if identities_exists == 1 && identity_bundles_exists == 0 {
        conn.execute_batch(
            "ALTER TABLE db_identities RENAME TO db_identity_bundles;
             DROP INDEX IF EXISTS idx_identities_is_blank;
             CREATE INDEX IF NOT EXISTS idx_identity_bundles_is_blank
                 ON db_identity_bundles(is_blank);",
        )?;
    }

    let memories_exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type='table' AND name='db_memories'",
        [],
        |row| row.get(0),
    )?;
    let memory_bundles_exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type='table' AND name='db_memory_bundles'",
        [],
        |row| row.get(0),
    )?;

    if memories_exists == 1 && memory_bundles_exists == 0 {
        conn.execute_batch(
            "ALTER TABLE db_memories RENAME TO db_memory_bundles;
             DROP INDEX IF EXISTS idx_memories_is_blank;
             CREATE INDEX IF NOT EXISTS idx_memory_bundles_is_blank
                 ON db_memory_bundles(is_blank);",
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run all forge migrations up to and including v7, but NOT v11. Used by
    /// tests that need to exercise v7's legacy-named DDL/seed/shadow-migrate
    /// path (which the v11 rename + the guard in `run_forge_v7_migrations`
    /// skip on post-v11 schemas).
    fn migrate_through_v7(conn: &Connection) -> Result<(), StoreError> {
        run_forge_v1_migrations(conn)?;
        run_forge_v2_migrations(conn)?;
        run_forge_v3_migrations(conn)?;
        run_forge_v4_migrations(conn)?;
        run_forge_v5_migrations(conn)?;
        run_forge_v6_migrations(conn)?;
        run_forge_v7_migrations(conn)?;
        Ok(())
    }

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

    #[test]
    fn test_forge_v7_creates_tables_and_singletons() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();

        run_forge_migrations(&conn).unwrap();

        // Tables exist under their post-v11 names (v7 creates legacy names,
        // v11 renames them to `*_bundles`). `db_identity_bindings` is not
        // renamed.
        for table in &[
            "db_identity_bundles",
            "db_identity_bindings",
            "db_memory_bundles",
        ] {
            let count: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "table {table} should exist");
        }

        // Legacy names are gone after v11's rename.
        for legacy in &["db_identities", "db_memories"] {
            let count: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [legacy],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "legacy table {legacy} should be renamed");
        }

        // Blank singletons seeded (seeded into legacy names by v7, carried
        // across the v11 rename into the bundle tables).
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

        // identity_id + memory_id columns added to db_agent_instances
        let cols: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(db_agent_instances)").unwrap();
            let mut rows = stmt.query([]).unwrap();
            let mut out = Vec::new();
            while let Some(row) = rows.next().unwrap() {
                out.push(row.get::<_, String>(1).unwrap());
            }
            out
        };
        assert!(cols.contains(&"identity_id".to_string()));
        assert!(cols.contains(&"memory_id".to_string()));
    }

    #[test]
    fn test_forge_v7_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();

        run_forge_migrations(&conn).unwrap();
        run_forge_migrations(&conn).unwrap(); // second pass

        // Singletons remain unique. On the second pass v7's guard skips the
        // legacy-name INSERTs (bundles already exist); the rows live in the
        // renamed bundle tables.
        let id_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_identity_bundles WHERE id='blank'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(id_count, 1, "blank Identity should remain a singleton");

        let mem_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_memory_bundles WHERE id='blank'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(mem_count, 1, "blank Memory should remain a singleton");

        // Legacy names must not be re-created by the second pass — v7's
        // guard must keep the post-v11 schema stable across replays.
        for legacy in &["db_identities", "db_memories"] {
            let count: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [legacy],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                count, 0,
                "legacy {legacy} must not be resurrected on migration replay"
            );
        }
    }

    #[test]
    fn test_forge_v7_shadow_migrates_forge_agents_into_memories() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();

        // Stop the migration chain at v7 so we can verify v7's legacy-named
        // shadow-migrate path. Once v11 has run, v7's guard skips this block
        // entirely (the data lives in the renamed bundle tables instead).
        migrate_through_v7(&conn).unwrap();
        conn.execute_batch(
            "DELETE FROM db_memories;
             INSERT INTO db_forge_agents
                (id, name, icon, provider, description, created_at)
             VALUES ('forge-1', 'My Coder', '✦', 'claude', 'demo', 1234);",
        )
        .unwrap();

        // Re-run the v7 migration directly. The shadow-copy is INSERT OR
        // IGNORE so we need a clean slate for db_memories first (done above).
        run_forge_v7_migrations(&conn).unwrap();

        let migrated_provider: String = conn
            .query_row(
                "SELECT provider FROM db_memories WHERE id='forge-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(migrated_provider, "claude");

        let migrated_name: String = conn
            .query_row(
                "SELECT name FROM db_memories WHERE id='forge-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(migrated_name, "My Coder");
    }

    #[test]
    fn test_forge_v7_disambiguates_duplicate_forge_agent_names() {
        // codex P1, 2026-05-08: db_forge_agents.name allows duplicates
        // (only slug is collision-resolved). Before this fix the migration
        // used INSERT OR IGNORE ... SELECT name, silently dropping all but
        // one duplicate; instances of the dropped definitions were left
        // pointing at a non-existent memory_id.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();

        migrate_through_v7(&conn).unwrap();
        conn.execute_batch(
            "DELETE FROM db_memories;
             INSERT INTO db_forge_agents
                (id, slug, name, icon, provider, description, created_at)
             VALUES
                ('forge-aaaaaaaa', 'duplicate-a', 'Duplicate', '✦', 'claude', '', 100),
                ('forge-bbbbbbbb', 'duplicate-b', 'Duplicate', '✦', 'codex',  '', 200);",
        )
        .unwrap();

        run_forge_v7_migrations(&conn).unwrap();

        // Both forge agents should have a corresponding memory row
        // (plus the blank singleton — DELETE FROM db_memories above
        // wiped that, but v7 re-seeds it via INSERT OR IGNORE).
        let migrated: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_memories WHERE is_blank = 0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            migrated, 2,
            "both duplicate-named forge agents should migrate"
        );

        // The names should be disambiguated with the id-prefix suffix.
        let name_a: String = conn
            .query_row(
                "SELECT name FROM db_memories WHERE id='forge-aaaaaaaa'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let name_b: String = conn
            .query_row(
                "SELECT name FROM db_memories WHERE id='forge-bbbbbbbb'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(name_a, "Duplicate [forge-aa]");
        assert_eq!(name_b, "Duplicate [forge-bb]");
        assert_ne!(name_a, name_b, "disambiguated names must be unique");
    }

    #[test]
    fn test_forge_v7_disambiguates_against_existing_memory_name() {
        // If a user manually created a Memory bundle named "X" before this
        // migration, and then has an old forge agent named "X", the
        // migration must rename the migrated row to avoid a UNIQUE
        // constraint failure.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();

        migrate_through_v7(&conn).unwrap();
        conn.execute_batch(
            "DELETE FROM db_memories;
             INSERT INTO db_memories
                (id, name, description, is_blank, created_at, updated_at)
             VALUES ('user-mem', 'Coder', '', 0, 0, 0);
             INSERT INTO db_forge_agents
                (id, slug, name, icon, provider, description, created_at)
             VALUES ('forge-cccccccc', 'coder', 'Coder', '✦', 'claude', '', 100);",
        )
        .unwrap();

        run_forge_v7_migrations(&conn).unwrap();

        let migrated_name: String = conn
            .query_row(
                "SELECT name FROM db_memories WHERE id='forge-cccccccc'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(migrated_name, "Coder [forge-cc]");

        // Original user memory remains intact.
        let user_name: String = conn
            .query_row(
                "SELECT name FROM db_memories WHERE id='user-mem'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(user_name, "Coder");
    }

    #[test]
    fn test_forge_v7_backfills_memory_id_on_existing_instances() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();

        // Stage a pre-v7 state: v1..v6 brings the schema to where
        // db_agent_instances exists without identity_id/memory_id, then we
        // insert a forge agent + an instance referencing it. v7's ALTER
        // adds the columns and the backfill copies definition_id into
        // memory_id.
        run_forge_v1_migrations(&conn).unwrap();
        run_forge_v2_migrations(&conn).unwrap();
        run_forge_v3_migrations(&conn).unwrap();
        run_forge_v4_migrations(&conn).unwrap();
        run_forge_v5_migrations(&conn).unwrap();
        run_forge_v6_migrations(&conn).unwrap();

        conn.execute_batch(
            "INSERT INTO db_forge_agents
                (id, name, icon, provider, description, created_at)
             VALUES ('forge-1', 'My Coder', '✦', 'claude', 'demo', 1234);

             INSERT INTO db_agent_instances
                (id, definition_id, created_at)
             VALUES ('inst-1', 'forge-1', 0);",
        )
        .unwrap();

        run_forge_v7_migrations(&conn).unwrap();

        let backfilled: String = conn
            .query_row(
                "SELECT memory_id FROM db_agent_instances WHERE id='inst-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(backfilled, "forge-1");
    }

    #[test]
    fn test_forge_v11_renames_legacy_to_bundle_tables() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();

        // Build a pre-v11 schema (v1..v7) so the legacy-named tables are
        // present with their seeded singletons. v11 then renames them.
        migrate_through_v7(&conn).unwrap();

        // Sanity check: legacy names exist, bundle names don't.
        let legacy_id: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='db_identities'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let legacy_mem: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='db_memories'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(legacy_id, 1, "v7 should have created db_identities");
        assert_eq!(legacy_mem, 1, "v7 should have created db_memories");

        run_forge_v11_migrations(&conn).unwrap();

        // Legacy names gone, bundle names present, data preserved.
        for legacy in &["db_identities", "db_memories"] {
            let count: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [legacy],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "{legacy} should be renamed away");
        }
        for bundle in &["db_identity_bundles", "db_memory_bundles"] {
            let count: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [bundle],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "{bundle} should exist after rename");
        }

        let id_blank: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_identity_bundles WHERE id='blank' AND is_blank=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(id_blank, 1, "blank Identity row must survive rename");
        let mem_blank: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_memory_bundles WHERE id='blank' AND is_blank=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(mem_blank, 1, "blank Memory row must survive rename");

        // Indexes are renamed too.
        for idx in &[
            "idx_identity_bundles_is_blank",
            "idx_memory_bundles_is_blank",
        ] {
            let count: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='index' AND name=?1",
                    [idx],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "{idx} should exist post-rename");
        }
        for old_idx in &["idx_identities_is_blank", "idx_memories_is_blank"] {
            let count: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='index' AND name=?1",
                    [old_idx],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "legacy index {old_idx} should be dropped");
        }
    }

    #[test]
    fn test_forge_v11_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();

        // Full chain twice — must not error and must not leave duplicate
        // tables behind.
        run_forge_migrations(&conn).unwrap();
        run_forge_migrations(&conn).unwrap();
        // Direct v11 invocation on an already-migrated db must no-op.
        run_forge_v11_migrations(&conn).unwrap();
    }

    #[test]
    fn test_forge_v11_fresh_install_skips_rename() {
        // On a database that never had the legacy tables (fresh install
        // built from v1..v10 with a hypothetical future v7 already writing
        // to bundle names, or — more realistically — a test that creates
        // the bundle table directly), v11 must be a no-op.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();

        // Mint just the bundle tables, no legacy.
        conn.execute_batch(
            "CREATE TABLE db_identity_bundles (id TEXT PRIMARY KEY, is_blank INTEGER);
             CREATE TABLE db_memory_bundles  (id TEXT PRIMARY KEY, is_blank INTEGER);",
        )
        .unwrap();

        run_forge_v11_migrations(&conn).unwrap();

        // No legacy tables created.
        for legacy in &["db_identities", "db_memories"] {
            let count: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [legacy],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "v11 must not resurrect {legacy}");
        }
    }

    #[test]
    fn test_forge_v11_preserves_identity_bindings_fk_cascade() {
        // SQLite >= 3.25 auto-updates FK references in db_identity_bindings
        // when its parent (db_identities) is renamed to db_identity_bundles.
        // Verify the cascade-delete still fires post-rename.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();

        run_forge_migrations(&conn).unwrap();

        // Seed a non-blank identity, an account, and a binding row.
        conn.execute_batch(
            "INSERT INTO db_identity_bundles
                (id, name, description, is_blank, created_at, updated_at)
             VALUES ('id1', 'Work', '', 0, 0, 0);

             INSERT INTO db_identity_accounts
                (id, name, provider, kind, secret_ref, created_at, updated_at)
             VALUES ('acc1', 'gh', 'github', 'pat',
                     '{\"backend\":\"env\",\"env_var\":\"GITHUB_TOKEN\"}', 0, 0);

             INSERT INTO db_identity_bindings (identity_id, provider, account_id)
             VALUES ('id1', 'github', 'acc1');",
        )
        .unwrap();

        // Delete the parent bundle row — cascade should remove the binding.
        conn.execute("DELETE FROM db_identity_bundles WHERE id='id1'", [])
            .unwrap();

        let remaining: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_identity_bindings WHERE identity_id='id1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            remaining, 0,
            "FK cascade from db_identity_bundles → db_identity_bindings must \
             survive the v11 rename"
        );
    }

    #[test]
    fn test_forge_v10_copies_workflow_data_to_drone_tables() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();

        // Run forge migrations up to v9 so the workflow tables exist
        // (v10 is included by run_forge_migrations; we'll seed BEFORE
        // running it explicitly by first wiping the drone tables and
        // re-running just v10 to verify the copy path).
        run_forge_v9_migrations(&conn).unwrap();

        // Seed a workflow definition and a workflow run in the v9
        // tables.
        conn.execute_batch(
            "INSERT INTO db_workflow_definitions
                (id, name, description, graph, viewport, created_at, updated_at)
             VALUES (
                'wf-1', 'My Drone', 'desc',
                '{\"nodes\":[],\"edges\":[]}',
                '{\"x\":0,\"y\":0,\"zoom\":1}',
                1000, 2000
             );

             INSERT INTO db_workflow_runs
                (id, workflow_id, status, started_at, ended_at,
                 block_states, output, error)
             VALUES (
                'run-1', 'wf-1', 'done', 1100, 1500,
                '{}', 'hello', ''
             );",
        )
        .unwrap();

        // Run v10 — should create drone tables and copy data.
        run_forge_v10_migrations(&conn).unwrap();

        let def_row: (String, String, i64) = conn
            .query_row(
                "SELECT id, name, updated_at FROM db_drone_definitions
                 WHERE id = 'wf-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(def_row.0, "wf-1");
        assert_eq!(def_row.1, "My Drone");
        assert_eq!(def_row.2, 2000);

        let run_row: (String, String, String, String) = conn
            .query_row(
                "SELECT id, drone_id, status, output FROM db_drone_runs
                 WHERE id = 'run-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(run_row.0, "run-1");
        assert_eq!(run_row.1, "wf-1");
        assert_eq!(run_row.2, "done");
        assert_eq!(run_row.3, "hello");

        // v9 tables remain on disk (downgrade safety).
        let wf_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_workflow_definitions",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(wf_count, 1);

        // Re-running v10 is idempotent.
        run_forge_v10_migrations(&conn).unwrap();
        let drone_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_drone_definitions",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(drone_count, 1);
    }

    #[test]
    fn test_forge_v10_does_not_resurrect_deleted_drones() {
        // codex P2 round 1 on PR #912: the v10 copy used to run on
        // every srv startup. Because the v9 `db_workflow_*` tables are
        // intentionally retained on disk for downgrade safety, a user
        // who deleted a migrated drone saw it reappear on the next
        // launch (the next INSERT OR IGNORE re-created the row from
        // the legacy table). Per-row sentinel tables gate the copy.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();

        run_forge_migrations(&conn).unwrap();

        // Seed a v9 workflow row and force a v10 re-run. (run_forge_v10
        // is part of the chain above; we wipe the sentinel + drone row
        // so the copy path actually executes here.)
        conn.execute_batch(
            "INSERT INTO db_workflow_definitions
                (id, name, description, graph, viewport, created_at, updated_at)
             VALUES ('drone-1', 'demo', '', '{}', '{}', 100, 100);

             DELETE FROM db_v10_migrated_legacy_defs WHERE legacy_id='drone-1';
             DELETE FROM db_drone_definitions WHERE id='drone-1';",
        )
        .unwrap();

        run_forge_v10_migrations(&conn).unwrap();

        let after_copy: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_drone_definitions WHERE id='drone-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(after_copy, 1, "v10 should copy the v9 workflow row");

        // Sentinel row recorded.
        let sentinel: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_v10_migrated_legacy_defs
                 WHERE legacy_id='drone-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sentinel, 1, "sentinel must record the copied legacy id");

        // User deletes the drone — simulates real-world delete via the
        // Drone pane UI.
        conn.execute("DELETE FROM db_drone_definitions WHERE id='drone-1'", [])
            .unwrap();

        // Next srv launch — re-run all migrations. The sentinel should
        // gate the v10 copy so the deleted drone STAYS deleted.
        run_forge_migrations(&conn).unwrap();

        let after_relaunch: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_drone_definitions WHERE id='drone-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            after_relaunch, 0,
            "deleted drone must NOT come back when migrations re-run"
        );

        // Legacy v9 row is preserved (downgrade safety).
        let legacy: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_workflow_definitions WHERE id='drone-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(legacy, 1, "v9 legacy row preserved on disk for downgrade");
    }

    #[test]
    fn test_forge_v10_copies_new_legacy_rows_after_downgrade_roundtrip() {
        // codex P2 round 2 on PR #912: with a singleton "v10 copy done"
        // marker, a user who upgrades, downgrades to the v9 build,
        // creates a workflow under the old schema, then re-upgrades
        // would never see that new legacy row migrated. Per-row
        // sentinels copy any legacy id that isn't yet recorded — new
        // post-downgrade rows are picked up, previously-migrated-then-
        // deleted rows are not.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();

        // First boot at v10: legacy "drone-1" gets copied.
        run_forge_migrations(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO db_workflow_definitions
                (id, name, description, graph, viewport, created_at, updated_at)
             VALUES ('drone-1', 'demo-1', '', '{}', '{}', 100, 100);",
        )
        .unwrap();
        run_forge_v10_migrations(&conn).unwrap();
        let copied: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_drone_definitions",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(copied, 1);

        // User deletes "drone-1" via the UI.
        conn.execute("DELETE FROM db_drone_definitions WHERE id='drone-1'", [])
            .unwrap();

        // User downgrades. The old v9 build reads `db_workflow_*` and
        // the user creates a new workflow "drone-2" there.
        conn.execute_batch(
            "INSERT INTO db_workflow_definitions
                (id, name, description, graph, viewport, created_at, updated_at)
             VALUES ('drone-2', 'demo-2', '', '{}', '{}', 200, 200);",
        )
        .unwrap();

        // User re-upgrades — v10 fires again.
        run_forge_v10_migrations(&conn).unwrap();

        // "drone-2" got migrated (new legacy id, no sentinel yet).
        let drone2: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_drone_definitions WHERE id='drone-2'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(drone2, 1, "new legacy row created during downgrade must migrate on re-upgrade");

        // "drone-1" did NOT come back (sentinel from first copy still present).
        let drone1: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_drone_definitions WHERE id='drone-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(drone1, 0, "previously-deleted drone must stay deleted across roundtrip");
    }

    #[test]
    fn test_forge_v10_skips_orphan_runs_under_foreign_keys_on() {
        // codex P2 round 3 on PR #912: PRAGMA foreign_keys=ON is set
        // by wstore.rs before migrations run. A user who deletes a
        // migrated drone, downgrades, runs the still-present legacy
        // workflow, and re-upgrades would produce a new legacy run
        // whose workflow_id points at a drone the sentinel prevents
        // from re-appearing. Without the parent-exists filter on the
        // run copy, the INSERT INTO db_drone_runs fails FK and rolls
        // back the entire migration transaction — blocking startup.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();

        // First boot at v10: copy a drone + its run.
        run_forge_migrations(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO db_workflow_definitions
                (id, name, description, graph, viewport, created_at, updated_at)
             VALUES ('d1', 'demo', '', '{}', '{}', 100, 100);

             INSERT INTO db_workflow_runs
                (id, workflow_id, status, started_at, ended_at,
                 block_states, output, error)
             VALUES ('r1', 'd1', 'done', 100, 200, '{}', '', '');",
        )
        .unwrap();
        run_forge_v10_migrations(&conn).unwrap();

        // User deletes the drone via the UI — the run cascade-deletes
        // (FK ON DELETE CASCADE on db_drone_runs).
        conn.execute("DELETE FROM db_drone_definitions WHERE id='d1'", [])
            .unwrap();

        // User downgrades, runs legacy workflow d1 — appears as new
        // run row in the legacy table.
        conn.execute_batch(
            "INSERT INTO db_workflow_runs
                (id, workflow_id, status, started_at, ended_at,
                 block_states, output, error)
             VALUES ('r2', 'd1', 'done', 300, 400, '{}', '', '');",
        )
        .unwrap();

        // Re-upgrade: v10 must NOT fail. Parent d1 is intentionally
        // gone from drone tables, so r2 is filtered out.
        run_forge_v10_migrations(&conn)
            .expect("v10 must not error when legacy runs have no drone parent");

        // Drone tables: empty (d1 deleted, r1 cascade-deleted, r2 skipped).
        let drone_defs: i64 = conn
            .query_row("SELECT count(*) FROM db_drone_definitions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(drone_defs, 0, "deleted drone must stay deleted");
        let drone_runs: i64 = conn
            .query_row("SELECT count(*) FROM db_drone_runs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(drone_runs, 0, "orphan run must NOT be copied (no parent)");

        // r2 is NOT marked in the sentinel — unmarked so a future
        // launch can re-evaluate cheaply if the parent ever reappears.
        let r2_sentinel: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_v10_migrated_legacy_runs WHERE legacy_id='r2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(r2_sentinel, 0, "orphan run sentinel must NOT be set");
    }
}
