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
/// The flat schema reset the counter to 1 (the pre-flatten chain never set
/// `user_version`, so legacy files read 0). Bumped per additive migration:
///   v1 — flat schema baseline
///   v2 — db_agent_definitions.updated_at
///   v3 — db_agent_definitions.user_hidden (Phase 2 hide templates,
///        SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md Q2 Decision Y)
///   v4 — db_agents consolidation table (Phase 3a; dual-write only,
///        reads still on db_agent_definitions / db_agent_instances)
///   v5 — Phase 3c retires the two old tables (`db_agent_definitions`
///        and `db_agent_instances`) now that `db_agents` is the
///        canonical source. The new instance-lifecycle columns
///        (`definition_id`, `parent_instance_id`, `block_id`,
///        `session_id`, `status`, `started_at`, `ended_at`) are added
///        to `db_agents` to absorb the instance shape. A one-shot
///        backfill from the old tables runs immediately before the
///        DROPs so no data is lost on the v4 → v5 transition.
///        See `docs/specs/SPEC_AGENT_CONCEPT_CONSOLIDATION_2026_05_24.md`.
pub const OBJECT_SCHEMA_VERSION: i64 = 5;
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
    //
    // Phase 3c retired `db_agent_definitions` + `db_agent_instances`; the
    // child tables below (`db_agent_content`, `db_agent_skills`,
    // `db_agent_history`, `db_agent_identity_links`) now FK to
    // `db_agents(id)` — the consolidated canonical agent row. The
    // v4 → v5 migration step below renames the parent FK target on these
    // tables for any pre-existing dev database.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS db_agent_content (
            agent_id     TEXT NOT NULL,
            content_type TEXT NOT NULL,
            content      TEXT NOT NULL DEFAULT '',
            updated_at   INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (agent_id, content_type),
            FOREIGN KEY (agent_id) REFERENCES db_agents(id) ON DELETE CASCADE
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
            FOREIGN KEY (agent_id) REFERENCES db_agents(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS db_agent_history (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id     TEXT NOT NULL,
            session_date TEXT NOT NULL,
            entry        TEXT NOT NULL,
            timestamp    INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (agent_id) REFERENCES db_agents(id) ON DELETE CASCADE
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
            FOREIGN KEY (agent_id)   REFERENCES db_agents(id)          ON DELETE CASCADE,
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

        -- Phase 3a consolidation, Phase 3c finalisation: `db_agents`
        -- collapses the retired `db_agent_definitions` +
        -- `db_agent_instances` into one canonical table per
        -- `docs/specs/SPEC_AGENT_CONCEPT_CONSOLIDATION_2026_05_24.md`.
        --
        --   - `is_template = 1` rows are templates (the old seeded
        --     `db_agent_definitions WHERE is_seeded = 1`).
        --   - `is_template = 0` rows are user-owned agents (cloned from
        --     a template OR a fold of an instance into its def). Carry
        --     the bindings + lifecycle (block_id/session_id/status/
        --     started_at/ended_at) the old `db_agent_instances` table
        --     held.
        --
        -- Column names align with the live shape the storage layer
        -- carried — NOT the inline draft in the spec (provider_id, cmd,
        -- cmd_args, …) — because the existing layer never grew the
        -- cmd-template columns the spec sketched; carrying the names
        -- we actually have avoids inventing data we don't store.
        CREATE TABLE IF NOT EXISTS db_agents (
            id                   TEXT PRIMARY KEY,
            name                 TEXT NOT NULL,
            icon                 TEXT NOT NULL DEFAULT '',
            description          TEXT NOT NULL DEFAULT '',

            -- Template vs user agent
            is_template          INTEGER NOT NULL DEFAULT 0,
            parent_template_id   TEXT NOT NULL DEFAULT '',

            -- Provider/cmd config (was on definition).
            provider             TEXT NOT NULL,
            provider_flags       TEXT NOT NULL DEFAULT '',
            shell                TEXT NOT NULL DEFAULT '',
            environment          TEXT NOT NULL DEFAULT '',
            agent_type           TEXT NOT NULL DEFAULT 'standalone',
            agent_bus_id         TEXT NOT NULL DEFAULT '',
            accounts             TEXT NOT NULL DEFAULT '',
            auto_start           INTEGER NOT NULL DEFAULT 0,
            restart_on_crash     INTEGER NOT NULL DEFAULT 0,
            idle_timeout_minutes INTEGER NOT NULL DEFAULT 0,
            slug                 TEXT NOT NULL DEFAULT '',
            branch_label         TEXT NOT NULL DEFAULT '',

            -- Bindings (was on instance — only meaningful when is_template=0).
            -- For template rows these stay empty.
            identity_id          TEXT NOT NULL DEFAULT '',
            memory_id            TEXT NOT NULL DEFAULT '',
            working_directory    TEXT NOT NULL DEFAULT '',
            github_context       TEXT NOT NULL DEFAULT '',
            instance_name        TEXT NOT NULL DEFAULT '',

            -- Instance lifecycle (was on `db_agent_instances`, retired
            -- in Phase 3c). For template rows these stay empty / 0.
            definition_id        TEXT NOT NULL DEFAULT '',
            parent_instance_id   TEXT NOT NULL DEFAULT '',
            block_id             TEXT NOT NULL DEFAULT '',
            session_id           TEXT NOT NULL DEFAULT '',
            status               TEXT NOT NULL DEFAULT '',
            started_at           INTEGER NOT NULL DEFAULT 0,
            ended_at             INTEGER NOT NULL DEFAULT 0,

            -- Provenance
            created_at           INTEGER NOT NULL DEFAULT 0,
            updated_at           INTEGER NOT NULL DEFAULT 0,
            is_seeded            INTEGER NOT NULL DEFAULT 0,
            user_hidden          INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_agents_is_template
            ON db_agents(is_template);
        CREATE INDEX IF NOT EXISTS idx_agents_parent_template_id
            ON db_agents(parent_template_id);
        CREATE INDEX IF NOT EXISTS idx_agents_is_seeded
            ON db_agents(is_seeded);
        CREATE INDEX IF NOT EXISTS idx_agents_slug
            ON db_agents(slug);
        -- Reagent P0 on #1015: indexes that reference v5 ALTER-added
        -- columns (block_id, status, definition_id, started_at) are
        -- created AFTER the ALTER batch -- see the post-ALTER
        -- execute_batch later in this function. Listing them here
        -- would fail with a no-such-column error on a v4 to v5
        -- upgrade because CREATE TABLE IF NOT EXISTS is a no-op
        -- when the table exists, so the v5 columns are not on it
        -- yet at this point in the chain.

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

    // ---- Additive column migrations (schema v2+) ----
    // The flat CREATE batch above covers fresh databases. These ALTERs
    // carry an existing database (e.g. a developer's dev DB that persists
    // across builds) forward. Idempotent: the "duplicate column" error is
    // swallowed. New additive columns append here + bump OBJECT_SCHEMA_VERSION.
    //
    // v2: db_agent_definitions.updated_at — last-modified timestamp.
    //     (Retired in v5 along with the table; the ALTER is gone too.)
    // v3: db_agent_definitions.user_hidden — per-user hide flag for
    //     templates (Phase 2 of the two-tier picker spec, Q2 Decision Y).
    //     (Retired in v5 along with the table.)
    // v4: db_agents created by the CREATE TABLE above (Phase 3a, dual-
    //     write). No ALTERs needed — fresh dbs land the table directly,
    //     and pre-v4 dbs land it via this idempotent CREATE.
    // v5: db_agents grows the instance-lifecycle columns
    //     (definition_id, parent_instance_id, block_id, session_id,
    //     status, started_at, ended_at) so it can absorb the retired
    //     `db_agent_instances`. Carry-forward ALTERs for pre-v5 dbs.
    for stmt in &[
        "ALTER TABLE db_agents ADD COLUMN definition_id      TEXT    NOT NULL DEFAULT ''",
        "ALTER TABLE db_agents ADD COLUMN parent_instance_id TEXT    NOT NULL DEFAULT ''",
        "ALTER TABLE db_agents ADD COLUMN block_id           TEXT    NOT NULL DEFAULT ''",
        "ALTER TABLE db_agents ADD COLUMN session_id         TEXT    NOT NULL DEFAULT ''",
        "ALTER TABLE db_agents ADD COLUMN status             TEXT    NOT NULL DEFAULT ''",
        "ALTER TABLE db_agents ADD COLUMN started_at         INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE db_agents ADD COLUMN ended_at           INTEGER NOT NULL DEFAULT 0",
    ] {
        if let Err(e) = conn.execute_batch(stmt) {
            let msg = e.to_string();
            if !msg.contains("duplicate column") {
                return Err(e.into());
            }
        }
    }

    // ---- v5: retire the old agent tables ----
    //
    // If a pre-v5 database is being opened (`db_agent_definitions` and/or
    // `db_agent_instances` still present), salvage any data they carry
    // into `db_agents` (Phase 3a's dual-write was best-effort; this is
    // the last-chance backfill), then DROP both. On a v5+ database both
    // tables are already gone and these statements are no-ops.
    //
    // Order matters: `db_agent_instances` has a FK on
    // `db_agent_definitions`, so it must go first. We disable foreign
    // keys for the duration so the cascade doesn't reach into
    // `db_agent_content` / `db_agent_skills` / `db_agent_history` /
    // `db_agent_identity_links` (those tables now FK to db_agents,
    // which carries the same ids — pulling the old parent away must
    // not delete the children).
    retire_old_agent_tables(conn)?;

    // ---- v5 indexes that depend on the ALTER-added columns ----
    // Reagent P0 on #1015: the four indexes below reference columns
    // (`block_id`, `status`, `definition_id`, `started_at`,
    // `instance_name`) that the v5 ALTER batch adds to a pre-v5
    // `db_agents`. Creating them in the upfront `CREATE TABLE` batch
    // would fail on the v4 → v5 path (the columns don't exist yet),
    // so they live here instead — they work for both fresh-v5
    // (columns added by CREATE TABLE) and v4 → v5 (added by ALTERs).
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_agents_block_id
            ON db_agents(block_id);
         CREATE INDEX IF NOT EXISTS idx_agents_status
            ON db_agents(status);
         CREATE INDEX IF NOT EXISTS idx_agents_definition_id
            ON db_agents(definition_id);
         CREATE INDEX IF NOT EXISTS idx_agents_name_recent
            ON db_agents(instance_name, started_at DESC)
            WHERE is_template = 0
              AND user_hidden = 0
              AND instance_name != '';",
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

/// Phase 3c — back-fill from the retired `db_agent_definitions` +
/// `db_agent_instances` into the canonical `db_agents` table, then DROP
/// both old tables. Idempotent: on a v5+ database where the old tables
/// are already gone, this is a no-op via `DROP TABLE IF EXISTS`.
///
/// The back-fill is a safety net for the v4 → v5 transition. Phase 3a's
/// dual-write kept `db_agents` populated for any mutation that went
/// through the storage layer after that PR landed; this pass catches
/// the residual rows (e.g. a write that raced the dual-write, or a row
/// inserted by a legacy migration that never went through the dual
/// path). On a v4 database where every mutation already dual-wrote, the
/// back-fill resolves to a series of `INSERT OR IGNORE` / `UPDATE`
/// statements that touch zero rows beyond what was already there.
///
/// Wrapped in a transaction so a mid-flight failure leaves the schema
/// in v4 (`db_agents` partial, old tables intact) — a future restart
/// retries. Foreign keys are disabled for the duration so the child
/// tables (`db_agent_content` / `db_agent_skills` / `db_agent_history`
/// / `db_agent_identity_links` — now FK to `db_agents(id)`) survive the
/// DROP of the old parents.
fn retire_old_agent_tables(conn: &Connection) -> Result<(), StoreError> {
    let defs_present: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='db_agent_definitions'",
        [],
        |row| row.get(0),
    )?;
    let insts_present: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='db_agent_instances'",
        [],
        |row| row.get(0),
    )?;
    if defs_present == 0 && insts_present == 0 {
        return Ok(());
    }

    // The pre-flatten schema didn't have `db_agent_definitions.updated_at`
    // / `user_hidden` (added in v2 / v3 respectively). A v4 db should
    // have both via the ALTERs that ran in that build's
    // `run_object_schema`; a pre-v4 dev DB that skipped those ALTERs
    // and lands here for the first time on v5 won't. Bring the table up
    // to the v3 shape FIRST so the backfill SELECT can read
    // `updated_at` / `user_hidden` unconditionally. The ALTERs are
    // idempotent: "duplicate column" is swallowed.
    if defs_present == 1 {
        for stmt in &[
            "ALTER TABLE db_agent_definitions ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0",
            "ALTER TABLE db_agent_definitions ADD COLUMN user_hidden INTEGER NOT NULL DEFAULT 0",
            "ALTER TABLE db_agent_definitions ADD COLUMN parent_id TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE db_agent_definitions ADD COLUMN branch_label TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE db_agent_definitions ADD COLUMN slug TEXT NOT NULL DEFAULT ''",
        ] {
            if let Err(e) = conn.execute_batch(stmt) {
                let msg = e.to_string();
                if !msg.contains("duplicate column") {
                    return Err(e.into());
                }
            }
        }
    }

    // Disable FK enforcement so the cascade from dropping the old
    // tables doesn't reach into the child tables (now repointed at
    // db_agents but still holding rows keyed by the same ids).
    let fk_setting: i64 = conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    conn.execute_batch("PRAGMA foreign_keys=OFF;")?;
    let result = (|| -> Result<(), StoreError> {
        conn.execute_batch("BEGIN;")?;
        if defs_present == 1 {
            // Mirror every definition into db_agents (idempotent via
            // INSERT OR IGNORE — Phase 3a's dual-write already populated
            // most rows). Templates land with `is_template = 1`,
            // user-clones with `is_template = 0` + the legacy
            // `parent_id` becoming `parent_template_id`.
            conn.execute_batch(
                "INSERT OR IGNORE INTO db_agents (
                    id, name, icon, description,
                    is_template, parent_template_id,
                    provider, provider_flags, shell, environment,
                    agent_type, agent_bus_id, accounts,
                    auto_start, restart_on_crash, idle_timeout_minutes,
                    slug, branch_label,
                    created_at, updated_at, is_seeded, user_hidden
                 )
                 SELECT
                    id,
                    name,
                    COALESCE(icon, ''),
                    COALESCE(description, ''),
                    CASE WHEN is_seeded = 1 THEN 1 ELSE 0 END,
                    COALESCE(parent_id, ''),
                    provider,
                    COALESCE(provider_flags, ''),
                    COALESCE(shell, ''),
                    COALESCE(environment, ''),
                    COALESCE(agent_type, 'standalone'),
                    COALESCE(agent_bus_id, ''),
                    COALESCE(accounts, ''),
                    COALESCE(auto_start, 0),
                    COALESCE(restart_on_crash, 0),
                    COALESCE(idle_timeout_minutes, 0),
                    COALESCE(slug, ''),
                    COALESCE(branch_label, ''),
                    COALESCE(created_at, 0),
                    COALESCE(updated_at, created_at),
                    COALESCE(is_seeded, 0),
                    COALESCE(user_hidden, 0)
                 FROM db_agent_definitions;",
            )?;
        }
        if insts_present == 1 {
            // Mirror every non-continuation instance into db_agents.
            // For template-instance projections (parent def is a
            // template), we INSERT a row keyed by the instance id with
            // `parent_template_id = definition_id`. For user-clone-def
            // instances (parent def is `is_seeded = 0`), the row keyed
            // by `def.id` already exists — UPDATE it in place to fold
            // the instance's bindings + lifecycle fields onto the def.
            //
            // Continuation rows (`parent_instance_id != ''`) are
            // skipped — the consolidated model has no place for them
            // (Option E retired the chain).
            conn.execute_batch(
                "INSERT OR IGNORE INTO db_agents (
                    id, name, icon, description,
                    is_template, parent_template_id,
                    provider, provider_flags, shell, environment,
                    agent_type, agent_bus_id, accounts,
                    auto_start, restart_on_crash, idle_timeout_minutes,
                    slug, branch_label,
                    identity_id, memory_id, working_directory,
                    github_context, instance_name,
                    definition_id, parent_instance_id,
                    block_id, session_id, status,
                    started_at, ended_at,
                    created_at, updated_at, is_seeded, user_hidden
                 )
                 SELECT
                    i.id,
                    CASE WHEN COALESCE(i.instance_name, '') = ''
                         THEN d.name
                         ELSE i.instance_name
                    END,
                    COALESCE(d.icon, ''),
                    COALESCE(d.description, ''),
                    0,
                    d.id,
                    d.provider,
                    COALESCE(d.provider_flags, ''),
                    COALESCE(d.shell, ''),
                    COALESCE(d.environment, ''),
                    COALESCE(d.agent_type, 'standalone'),
                    COALESCE(d.agent_bus_id, ''),
                    COALESCE(d.accounts, ''),
                    COALESCE(d.auto_start, 0),
                    COALESCE(d.restart_on_crash, 0),
                    COALESCE(d.idle_timeout_minutes, 0),
                    COALESCE(d.slug, ''),
                    COALESCE(d.branch_label, ''),
                    COALESCE(i.identity_id, ''),
                    COALESCE(i.memory_id, ''),
                    COALESCE(i.working_directory, ''),
                    COALESCE(i.github_context, ''),
                    COALESCE(i.instance_name, ''),
                    i.definition_id,
                    COALESCE(i.parent_instance_id, ''),
                    COALESCE(i.block_id, ''),
                    COALESCE(i.session_id, ''),
                    COALESCE(i.status, 'running'),
                    COALESCE(i.started_at, 0),
                    COALESCE(i.ended_at, 0),
                    COALESCE(i.created_at, 0),
                    COALESCE(i.created_at, 0),
                    0,
                    CASE WHEN COALESCE(i.display_hidden, 0) = 1 THEN 1 ELSE 0 END
                 FROM db_agent_instances i
                 INNER JOIN db_agent_definitions d ON i.definition_id = d.id
                 WHERE COALESCE(i.parent_instance_id, '') = ''
                   AND d.is_seeded = 1;",
            )?;
            // User-clone-def instance fold: UPDATE the existing
            // `db_agents` row keyed by `def.id` so the instance's
            // bindings + lifecycle land on the canonical row.
            // Aggregate so multiple instances on one user-clone-def
            // resolve to the most-recent (MAX(created_at)) winner.
            conn.execute_batch(
                "UPDATE db_agents SET
                    identity_id = COALESCE((
                        SELECT i.identity_id FROM db_agent_instances i
                        INNER JOIN db_agent_definitions d ON i.definition_id = d.id
                        WHERE d.id = db_agents.id
                          AND d.is_seeded = 0
                          AND COALESCE(i.parent_instance_id, '') = ''
                        ORDER BY i.created_at DESC LIMIT 1
                    ), identity_id),
                    memory_id = COALESCE((
                        SELECT i.memory_id FROM db_agent_instances i
                        INNER JOIN db_agent_definitions d ON i.definition_id = d.id
                        WHERE d.id = db_agents.id
                          AND d.is_seeded = 0
                          AND COALESCE(i.parent_instance_id, '') = ''
                        ORDER BY i.created_at DESC LIMIT 1
                    ), memory_id),
                    working_directory = COALESCE((
                        SELECT i.working_directory FROM db_agent_instances i
                        INNER JOIN db_agent_definitions d ON i.definition_id = d.id
                        WHERE d.id = db_agents.id
                          AND d.is_seeded = 0
                          AND COALESCE(i.parent_instance_id, '') = ''
                        ORDER BY i.created_at DESC LIMIT 1
                    ), working_directory),
                    github_context = COALESCE((
                        SELECT i.github_context FROM db_agent_instances i
                        INNER JOIN db_agent_definitions d ON i.definition_id = d.id
                        WHERE d.id = db_agents.id
                          AND d.is_seeded = 0
                          AND COALESCE(i.parent_instance_id, '') = ''
                        ORDER BY i.created_at DESC LIMIT 1
                    ), github_context),
                    instance_name = COALESCE((
                        SELECT i.instance_name FROM db_agent_instances i
                        INNER JOIN db_agent_definitions d ON i.definition_id = d.id
                        WHERE d.id = db_agents.id
                          AND d.is_seeded = 0
                          AND COALESCE(i.parent_instance_id, '') = ''
                        ORDER BY i.created_at DESC LIMIT 1
                    ), instance_name),
                    definition_id = COALESCE((
                        SELECT i.definition_id FROM db_agent_instances i
                        INNER JOIN db_agent_definitions d ON i.definition_id = d.id
                        WHERE d.id = db_agents.id
                          AND d.is_seeded = 0
                          AND COALESCE(i.parent_instance_id, '') = ''
                        ORDER BY i.created_at DESC LIMIT 1
                    ), definition_id),
                    block_id = COALESCE((
                        SELECT i.block_id FROM db_agent_instances i
                        INNER JOIN db_agent_definitions d ON i.definition_id = d.id
                        WHERE d.id = db_agents.id
                          AND d.is_seeded = 0
                          AND COALESCE(i.parent_instance_id, '') = ''
                        ORDER BY i.created_at DESC LIMIT 1
                    ), block_id),
                    session_id = COALESCE((
                        SELECT i.session_id FROM db_agent_instances i
                        INNER JOIN db_agent_definitions d ON i.definition_id = d.id
                        WHERE d.id = db_agents.id
                          AND d.is_seeded = 0
                          AND COALESCE(i.parent_instance_id, '') = ''
                        ORDER BY i.created_at DESC LIMIT 1
                    ), session_id),
                    status = COALESCE((
                        SELECT i.status FROM db_agent_instances i
                        INNER JOIN db_agent_definitions d ON i.definition_id = d.id
                        WHERE d.id = db_agents.id
                          AND d.is_seeded = 0
                          AND COALESCE(i.parent_instance_id, '') = ''
                        ORDER BY i.created_at DESC LIMIT 1
                    ), status),
                    started_at = COALESCE((
                        SELECT i.started_at FROM db_agent_instances i
                        INNER JOIN db_agent_definitions d ON i.definition_id = d.id
                        WHERE d.id = db_agents.id
                          AND d.is_seeded = 0
                          AND COALESCE(i.parent_instance_id, '') = ''
                        ORDER BY i.created_at DESC LIMIT 1
                    ), started_at),
                    ended_at = COALESCE((
                        SELECT i.ended_at FROM db_agent_instances i
                        INNER JOIN db_agent_definitions d ON i.definition_id = d.id
                        WHERE d.id = db_agents.id
                          AND d.is_seeded = 0
                          AND COALESCE(i.parent_instance_id, '') = ''
                        ORDER BY i.created_at DESC LIMIT 1
                    ), ended_at)
                 WHERE is_template = 0
                   AND EXISTS (
                        SELECT 1 FROM db_agent_instances i
                        INNER JOIN db_agent_definitions d ON i.definition_id = d.id
                        WHERE d.id = db_agents.id
                          AND d.is_seeded = 0
                          AND COALESCE(i.parent_instance_id, '') = ''
                   );",
            )?;
        }
        // Drop the old tables. Order matters because of FKs (still
        // declared on the original tables — only enforcement was
        // disabled), but with `PRAGMA foreign_keys = OFF` the order is
        // a defensive convention rather than a correctness requirement.
        conn.execute_batch(
            "DROP TABLE IF EXISTS db_agent_instances;
             DROP TABLE IF EXISTS db_agent_definitions;",
        )?;
        conn.execute_batch("COMMIT;")?;
        Ok(())
    })();
    // Restore the prior FK setting whatever happened above.
    let _ = conn.execute_batch(if fk_setting != 0 {
        "PRAGMA foreign_keys=ON;"
    } else {
        "PRAGMA foreign_keys=OFF;"
    });
    if result.is_err() {
        // Roll back if the transaction is still alive — ignore the
        // result here so we surface the original error to the caller.
        let _ = conn.execute_batch("ROLLBACK;");
    }
    result
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

    /// Every table the flat `objects.db` schema must contain (schema v5).
    /// `db_agent_definitions` + `db_agent_instances` retired in v5 —
    /// `db_agents` is the canonical agent source.
    const EXPECTED_TABLES: &[&str] = &[
        "db_client",
        "db_window",
        "db_workspace",
        "db_tab",
        "db_layout",
        "db_block",
        "db_temp",
        "db_agent_content",
        "db_agent_skills",
        "db_agent_history",
        "db_identity_accounts",
        "db_agent_identity_links",
        "db_identity_bundles",
        "db_identity_bindings",
        "db_memory_bundles",
        "db_agents",
        "db_drone_definitions",
        "db_drone_runs",
    ];

    /// Tables retired by schema v5. Must NOT exist after
    /// `run_object_schema` runs.
    const RETIRED_TABLES: &[&str] = &[
        "db_agent_definitions",
        "db_agent_instances",
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
        for retired in RETIRED_TABLES {
            assert!(
                !table_exists(&conn, retired),
                "{retired} must not exist post-v5"
            );
        }
        // De-forged + bundle indexes.
        for idx in &[
            "idx_agent_history_agent_date",
            "idx_agent_identity_links_account",
            "idx_identity_accounts_provider",
            "idx_identity_bundles_is_blank",
            "idx_identity_bindings_account",
            "idx_memory_bundles_is_blank",
            "idx_agents_is_template",
            "idx_agents_parent_template_id",
            "idx_agents_is_seeded",
            "idx_agents_block_id",
            "idx_agents_status",
            "idx_agents_definition_id",
            "idx_agents_slug",
            "idx_agents_name_recent",
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
    fn test_adopt_legacy_renames_forge_tables_into_db_agents() {
        // Simulate a pre-flatten (post-v11) dev DB: legacy forge table
        // names + a dead workflow table, with seeded rows. The flatten
        // renames `db_forge_agents` → `db_agent_definitions`; the v5
        // retire step then folds that into `db_agents` and drops the
        // old table. Net effect: the row lands in `db_agents`.
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

        // Old tables gone; row lives in `db_agents`.
        assert!(!table_exists(&conn, "db_forge_agents"));
        assert!(!table_exists(&conn, "db_agent_definitions"));
        assert!(table_exists(&conn, "db_agents"));
        let name: String = conn
            .query_row(
                "SELECT name FROM db_agents WHERE id='a1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(name, "Coder");
        // Forge index dropped, db_agents index present.
        assert!(!index_exists(&conn, "idx_forge_agents_slug"));
        assert!(index_exists(&conn, "idx_agents_slug"));
        // Dead table dropped.
        assert!(!table_exists(&conn, "db_workflow_definitions"));
    }

    #[test]
    fn test_adopt_legacy_is_noop_on_fresh_db() {
        let conn = Connection::open_in_memory().unwrap();
        run_object_schema(&conn).unwrap();
        // Re-running schema on the already-flat-v5 DB leaves db_agents
        // intact and creates no legacy names.
        run_object_schema(&conn).unwrap();
        assert!(table_exists(&conn, "db_agents"));
        assert!(!table_exists(&conn, "db_agent_definitions"));
        assert!(!table_exists(&conn, "db_agent_instances"));
        assert!(!table_exists(&conn, "db_forge_agents"));
    }

    #[test]
    fn test_v5_retires_old_agent_tables_and_carries_data_to_db_agents() {
        // Simulate a v4 dev DB: db_agent_definitions + db_agent_instances
        // populated, db_agents already partially populated (Phase 3a's
        // dual-write). v5 retires the old tables; v5's backfill must NOT
        // clobber existing db_agents rows, just ensure every old-table
        // row has a counterpart.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=OFF;").unwrap();
        // Pre-v5 fixture: definitions + instances + a partial db_agents
        // (one template row already mirrored, the other not).
        conn.execute_batch(
            "CREATE TABLE db_agent_definitions (
                id TEXT PRIMARY KEY, slug TEXT NOT NULL DEFAULT '',
                name TEXT NOT NULL, icon TEXT NOT NULL DEFAULT '✦',
                provider TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                working_directory TEXT NOT NULL DEFAULT '',
                shell TEXT NOT NULL DEFAULT '',
                provider_flags TEXT NOT NULL DEFAULT '',
                auto_start INTEGER NOT NULL DEFAULT 0,
                restart_on_crash INTEGER NOT NULL DEFAULT 0,
                idle_timeout_minutes INTEGER NOT NULL DEFAULT 0,
                agent_type TEXT NOT NULL DEFAULT 'standalone',
                environment TEXT NOT NULL DEFAULT '',
                agent_bus_id TEXT NOT NULL DEFAULT '',
                is_seeded INTEGER NOT NULL DEFAULT 0,
                accounts TEXT NOT NULL DEFAULT '',
                parent_id TEXT NOT NULL DEFAULT '',
                branch_label TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL DEFAULT 0,
                updated_at INTEGER NOT NULL DEFAULT 0,
                user_hidden INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE db_agent_instances (
                id TEXT PRIMARY KEY,
                definition_id TEXT NOT NULL,
                parent_instance_id TEXT NOT NULL DEFAULT '',
                block_id TEXT NOT NULL DEFAULT '',
                session_id TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'running',
                github_context TEXT NOT NULL DEFAULT '',
                identity_id TEXT NOT NULL DEFAULT '',
                memory_id TEXT NOT NULL DEFAULT '',
                instance_name TEXT NOT NULL DEFAULT '',
                working_directory TEXT NOT NULL DEFAULT '',
                display_hidden INTEGER NOT NULL DEFAULT 0,
                started_at INTEGER NOT NULL DEFAULT 0,
                ended_at INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL DEFAULT 0
            );
            INSERT INTO db_agent_definitions (id, slug, name, provider, is_seeded, created_at, updated_at)
                VALUES
                    ('tpl-A', 'coder', 'Coder', 'claude', 1, 1000, 1000),
                    ('tpl-B', 'writer', 'Writer', 'claude', 1, 1100, 1100),
                    ('user-def', 'my-coder', 'My Coder', 'claude', 0, 1500, 1500);
            UPDATE db_agent_definitions SET parent_id = 'tpl-A' WHERE id = 'user-def';
            INSERT INTO db_agent_instances
                (id, definition_id, instance_name, identity_id, memory_id, working_directory,
                 status, block_id, started_at, created_at)
                VALUES
                    ('inst-on-tpl-A', 'tpl-A', 'Maks', 'id-1', 'mem-1', '/wd/maks',
                     'running', 'blk-1', 2000, 2000),
                    ('inst-on-user', 'user-def', 'My Coder v2', 'id-2', 'mem-2', '/wd/userv2',
                     'paused', 'blk-2', 2100, 2100);",
        )
        .unwrap();

        run_object_schema(&conn).unwrap();

        // Old tables gone; db_agents present.
        assert!(!table_exists(&conn, "db_agent_definitions"));
        assert!(!table_exists(&conn, "db_agent_instances"));
        assert!(table_exists(&conn, "db_agents"));

        // Templates landed.
        let tpl_a_template: i64 = conn
            .query_row(
                "SELECT is_template FROM db_agents WHERE id = 'tpl-A'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tpl_a_template, 1);

        // User-def landed (folded with its instance's bindings via the
        // UPDATE pass).
        let user_row_identity: String = conn
            .query_row(
                "SELECT identity_id FROM db_agents WHERE id = 'user-def'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(user_row_identity, "id-2");
        let user_row_status: String = conn
            .query_row(
                "SELECT status FROM db_agents WHERE id = 'user-def'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(user_row_status, "paused");

        // Template-instance projected as its own row keyed by inst.id.
        let inst_a: String = conn
            .query_row(
                "SELECT parent_template_id FROM db_agents WHERE id = 'inst-on-tpl-A'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(inst_a, "tpl-A");
        let inst_a_workdir: String = conn
            .query_row(
                "SELECT working_directory FROM db_agents WHERE id = 'inst-on-tpl-A'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(inst_a_workdir, "/wd/maks");

        // Re-running v5 is idempotent (DROP IF EXISTS + the table-exists
        // gate make the whole step a no-op).
        run_object_schema(&conn).unwrap();
        assert!(!table_exists(&conn, "db_agent_definitions"));
        assert!(!table_exists(&conn, "db_agent_instances"));
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
