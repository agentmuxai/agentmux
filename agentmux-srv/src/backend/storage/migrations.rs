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
    run_forge_v7_migrations(conn)?;
    run_forge_v8_migrations(conn)?;
    run_forge_v9_migrations(conn)?;
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
/// - `db_identities` (new): named Identity bundles. Each row has a unique
///   `name` (e.g. "Work", "Personal"). The `is_blank` flag tags the seeded
///   singleton row that the launch UI defaults to.
/// - `db_identity_bindings` (new): junction `(identity_id, provider) →
///   account_id`. Replaces the per-agent semantics of v6's
///   `db_forge_agent_identities` (which is left in place as legacy fallback;
///   a future v8 may drop it once all readers are migrated).
/// - `db_memories` (new): named Memory bundles holding the agent's
///   personality / capability stack — provider, model, instructions, context
///   files, MCP servers, skills. Forge agents (`db_forge_agents`) get
///   shadow-migrated into Memory rows so existing definitions remain
///   accessible from the new code paths without data loss.
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

    // ---- Add identity_id / memory_id columns to db_agent_instances ----
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

    #[test]
    fn test_forge_v7_creates_tables_and_singletons() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();

        run_forge_migrations(&conn).unwrap();

        // Tables exist
        for table in &["db_identities", "db_identity_bindings", "db_memories"] {
            let count: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "table {table} should exist");
        }

        // Blank singletons seeded
        let id_blank: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_identities WHERE id='blank' AND is_blank=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(id_blank, 1, "blank Identity singleton should be seeded");

        let mem_blank: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_memories WHERE id='blank' AND is_blank=1",
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

        // Singletons remain unique (INSERT OR IGNORE on second pass)
        let id_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_identities WHERE id='blank'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(id_count, 1, "blank Identity should remain a singleton");

        let mem_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_memories WHERE id='blank'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(mem_count, 1, "blank Memory should remain a singleton");
    }

    #[test]
    fn test_forge_v7_shadow_migrates_forge_agents_into_memories() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();

        // Run up through v6 first, insert a forge agent, then run v7.
        // We fake "before-v7" by re-running migrations after a manual delete
        // of the v7 rows the first run would have inserted.
        run_forge_migrations(&conn).unwrap();
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

        run_forge_migrations(&conn).unwrap();
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

        run_forge_migrations(&conn).unwrap();
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

        // Bring schema up via the full migration chain. This already creates
        // the v7 tables and runs the backfill. To prove the backfill works
        // on data that pre-dates v7, we (a) insert a forge agent + an
        // instance referencing it, then (b) zero memory_id and (c) re-run
        // v7 directly. After step (c) memory_id should be populated.
        run_forge_migrations(&conn).unwrap();

        conn.execute_batch(
            "INSERT INTO db_forge_agents
                (id, name, icon, provider, description, created_at)
             VALUES ('forge-1', 'My Coder', '✦', 'claude', 'demo', 1234);

             INSERT INTO db_agent_instances
                (id, definition_id, memory_id, created_at)
             VALUES ('inst-1', 'forge-1', '', 0);",
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
}
