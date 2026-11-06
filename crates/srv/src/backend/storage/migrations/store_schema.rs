// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The tables, seed and additive columns of the shared store (`store.db`) and the identity store,
//! one function each, for [`run_shared_store_schema`](super::run_shared_store_schema) and
//! [`run_identity_store_schema`](super::run_identity_store_schema). Moved out of them without a
//! change in behaviour. The version notes (v4, v6 and so on) that the SQL comments below refer to are
//! in the docs of `SHARED_STORE_SCHEMA_VERSION` and `IDENTITY_STORE_SCHEMA_VERSION` in `migrations.rs`.

use super::object_schema::apply_additive_columns;
use super::{Connection, StoreError};
use crate::backend::storage::{cron::JOB_TABLES, vault::VAULT_ALTERS};

/// The shared store's table batch.
pub(super) const SHARED_STORE_TABLES_SQL: &str = "CREATE TABLE IF NOT EXISTS db_accounts (
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
        CREATE INDEX IF NOT EXISTS idx_ss_accounts_provider
            ON db_accounts(provider);

        CREATE TABLE IF NOT EXISTS db_agent_identity_links (
            agent_id   TEXT NOT NULL,
            account_id TEXT NOT NULL,
            provider   TEXT NOT NULL,
            PRIMARY KEY (agent_id, provider),
            FOREIGN KEY (account_id) REFERENCES db_accounts(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_ss_agent_identity_links_account
            ON db_agent_identity_links(account_id);

        CREATE TABLE IF NOT EXISTS db_bundles (
            id            TEXT PRIMARY KEY,
            name          TEXT NOT NULL UNIQUE,
            description   TEXT NOT NULL DEFAULT '',
            is_blank      INTEGER NOT NULL DEFAULT 0,
            is_global     INTEGER NOT NULL DEFAULT 0,
            provider      TEXT NOT NULL DEFAULT '',
            model         TEXT NOT NULL DEFAULT '',
            instructions  TEXT NOT NULL DEFAULT '',
            instructions_by_provider TEXT NOT NULL DEFAULT '{}',
            context_files TEXT NOT NULL DEFAULT '[]',
            mcp_servers   TEXT NOT NULL DEFAULT '[]',
            skills        TEXT NOT NULL DEFAULT '[]',
            sort_order    INTEGER NOT NULL DEFAULT 0,
            created_at    INTEGER NOT NULL DEFAULT 0,
            updated_at    INTEGER NOT NULL DEFAULT 0,
            is_system     INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_ss_bundles_is_blank
            ON db_bundles(is_blank);

        CREATE TABLE IF NOT EXISTS db_drone_definitions (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            graph       TEXT NOT NULL DEFAULT '{\"nodes\":[],\"edges\":[]}',
            viewport    TEXT NOT NULL DEFAULT '{\"x\":0,\"y\":0,\"zoom\":1}',
            created_at  INTEGER NOT NULL DEFAULT 0,
            updated_at  INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_ss_drone_definitions_updated
            ON db_drone_definitions(updated_at DESC);

        CREATE TABLE IF NOT EXISTS db_muxbus_credentials (
            id             TEXT PRIMARY KEY DEFAULT 'global',
            cognito_domain TEXT NOT NULL DEFAULT '',
            client_id      TEXT NOT NULL DEFAULT '',
            access_token   TEXT NOT NULL DEFAULT '',
            refresh_token  TEXT NOT NULL DEFAULT '',
            id_token       TEXT NOT NULL DEFAULT '',
            expires_at     INTEGER NOT NULL DEFAULT 0,
            user_email     TEXT NOT NULL DEFAULT '',
            user_sub       TEXT NOT NULL DEFAULT ''
        );

        CREATE TABLE IF NOT EXISTS db_migrations (
            id          TEXT PRIMARY KEY,
            applied_at  TEXT NOT NULL,
            duration_ms INTEGER NOT NULL DEFAULT 0,
            scope       TEXT NOT NULL DEFAULT 'global'
        );

        CREATE TABLE IF NOT EXISTS db_cron_jobs (
            id            TEXT PRIMARY KEY,
            name          TEXT NOT NULL,
            expression    TEXT NOT NULL,
            prompt        TEXT NOT NULL,
            target        TEXT NOT NULL,
            created_by    TEXT NOT NULL DEFAULT '',
            enabled       INTEGER NOT NULL DEFAULT 1,
            last_fired    INTEGER,
            fire_count    INTEGER NOT NULL DEFAULT 0,
            max_fires     INTEGER,
            created_at    INTEGER NOT NULL DEFAULT 0,
            max_age_secs  INTEGER,
            -- v11 (identity M1b): the target's UID (db_agents.id) beside the
            -- slug, resolved ONCE at create time (spec §5.4), dual-written,
            -- not yet read -- a job still fires by `target`. Empty when the
            -- name did not resolve; never guessed.
            target_uid    TEXT NOT NULL DEFAULT '',
            -- v12 (identity M4c-1): the creator's UID from the request's
            -- Caller, beside the created_by name. Empty when unattributed.
            created_by_uid TEXT NOT NULL DEFAULT ''
        );
        CREATE INDEX IF NOT EXISTS idx_ss_cron_jobs_enabled
            ON db_cron_jobs(enabled);

        -- v4: per-agent M2M credential for muxbus Tier-4 binding (see
        -- the v4 note on SHARED_STORE_SCHEMA_VERSION in migrations.rs). client_secret is
        -- a Cognito app client secret, not a user password — same trust
        -- tier as MUXBUS_TOKEN in db_muxbus_credentials above, stored
        -- unencrypted in the same shared store.db for the same reason.
        CREATE TABLE IF NOT EXISTS db_agent_credentials (
            agent_id       TEXT PRIMARY KEY,
            client_id      TEXT NOT NULL DEFAULT '',
            client_secret  TEXT NOT NULL DEFAULT '',
            token_endpoint TEXT NOT NULL DEFAULT '',
            access_token   TEXT NOT NULL DEFAULT '',
            expires_at     INTEGER NOT NULL DEFAULT 0,
            created_at     INTEGER NOT NULL DEFAULT 0
        );

        -- v6: durable mirror of each agent's native (Claude Code) memory
        -- files, keyed by the stable AgentDefinition.id rather than any live
        -- filesystem path — the live path is channel-relative by design
        -- (per-build-channel filesystem isolation), so the same agent opened
        -- from two channels resolves two different on-disk memory dirs. This
        -- table is what makes native memory durable and location-consistent
        -- across channels: agent:memory:list/read_file upsert into it on
        -- every read, and merge the live-FS view with it in the response —
        -- see docs/specs/SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md.
        -- Duplicated into run_object_schema (same both-schemas pattern as
        -- db_agent_credentials above) for id_store's per-channel fallback
        -- before 0011_shared_store_backfill has run.
        CREATE TABLE IF NOT EXISTS db_agent_native_memory (
            agent_id           TEXT NOT NULL,
            filename           TEXT NOT NULL,
            content            TEXT NOT NULL,
            metadata_type      TEXT NOT NULL DEFAULT '',
            size_bytes         INTEGER NOT NULL DEFAULT 0,
            updated_at         INTEGER NOT NULL DEFAULT 0,
            last_seen_path     TEXT NOT NULL DEFAULT '',
            last_seen_mtime_ms INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (agent_id, filename)
        );

        -- v8: append-only version history for native memory content — one
        -- row per agent:memory:write_file call (plus reverts), so a prior
        -- value is never lost the way the mirror above loses it on every
        -- overwrite. See docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md.
        CREATE TABLE IF NOT EXISTS db_agent_native_memory_versions (
            id                 TEXT PRIMARY KEY,
            agent_id           TEXT NOT NULL,
            filename           TEXT NOT NULL,
            content            TEXT NOT NULL,
            content_hash       TEXT NOT NULL,
            parent_version_id  TEXT,
            source             TEXT NOT NULL DEFAULT 'agent_inferred',
            source_detail      TEXT NOT NULL DEFAULT '{}',
            session_id         TEXT NOT NULL DEFAULT '',
            created_at         INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_ss_native_memory_versions_lookup
            ON db_agent_native_memory_versions(agent_id, filename, created_at);

        -- v10: append-only version history for Global Memory entry content
        -- — see SHARED_STORE_SCHEMA_VERSION's own doc comment for the full
        -- rationale (SPEC_AGENT_FACING_GLOBAL_MEMORY_API_2026_09_15.md).
        CREATE TABLE IF NOT EXISTS db_bundle_versions (
            id                 TEXT PRIMARY KEY,
            bundle_id          TEXT NOT NULL,
            name               TEXT NOT NULL,
            instructions       TEXT NOT NULL,
            content_hash       TEXT NOT NULL,
            parent_version_id  TEXT,
            source             TEXT NOT NULL DEFAULT 'agent_inferred',
            source_detail      TEXT NOT NULL DEFAULT '{}',
            written_by         TEXT NOT NULL DEFAULT '',
            created_at         INTEGER NOT NULL DEFAULT 0,
            -- v12 (identity M4c-1): the writer's UID from the request's
            -- Caller, beside the written_by name. Empty when unattributed.
            written_by_uid     TEXT NOT NULL DEFAULT ''
        );
        CREATE INDEX IF NOT EXISTS idx_ss_bundle_versions_lookup
            ON db_bundle_versions(bundle_id, created_at);";

/// The identity store's table batch.
pub(super) const IDENTITY_STORE_TABLES_SQL: &str = "CREATE TABLE IF NOT EXISTS db_agent_identity_links (
            agent_id   TEXT NOT NULL,
            account_id TEXT NOT NULL,
            provider   TEXT NOT NULL,
            PRIMARY KEY (agent_id, provider)
        );
        CREATE INDEX IF NOT EXISTS idx_ids_agent_identity_links_account
            ON db_agent_identity_links(account_id);

        -- v2: fallback mirror only — see the v2 note on
        -- IDENTITY_STORE_SCHEMA_VERSION in migrations.rs. id_store stays the authoritative read/write path.
        CREATE TABLE IF NOT EXISTS db_accounts (
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
        CREATE INDEX IF NOT EXISTS idx_ids_accounts_provider
            ON db_accounts(provider);

        CREATE TABLE IF NOT EXISTS db_bundles (
            id            TEXT PRIMARY KEY,
            name          TEXT NOT NULL UNIQUE,
            description   TEXT NOT NULL DEFAULT '',
            is_blank      INTEGER NOT NULL DEFAULT 0,
            is_global     INTEGER NOT NULL DEFAULT 0,
            provider      TEXT NOT NULL DEFAULT '',
            model         TEXT NOT NULL DEFAULT '',
            instructions  TEXT NOT NULL DEFAULT '',
            instructions_by_provider TEXT NOT NULL DEFAULT '{}',
            context_files TEXT NOT NULL DEFAULT '[]',
            mcp_servers   TEXT NOT NULL DEFAULT '[]',
            skills        TEXT NOT NULL DEFAULT '[]',
            sort_order    INTEGER NOT NULL DEFAULT 0,
            created_at    INTEGER NOT NULL DEFAULT 0,
            updated_at    INTEGER NOT NULL DEFAULT 0,
            is_system     INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_ids_bundles_is_blank
            ON db_bundles(is_blank);

        CREATE TABLE IF NOT EXISTS db_drone_definitions (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            graph       TEXT NOT NULL DEFAULT '{\"nodes\":[],\"edges\":[]}',
            viewport    TEXT NOT NULL DEFAULT '{\"x\":0,\"y\":0,\"zoom\":1}',
            created_at  INTEGER NOT NULL DEFAULT 0,
            updated_at  INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_ids_drone_definitions_updated
            ON db_drone_definitions(updated_at DESC);

        CREATE TABLE IF NOT EXISTS db_muxbus_credentials (
            id             TEXT PRIMARY KEY DEFAULT 'global',
            cognito_domain TEXT NOT NULL DEFAULT '',
            client_id      TEXT NOT NULL DEFAULT '',
            access_token   TEXT NOT NULL DEFAULT '',
            refresh_token  TEXT NOT NULL DEFAULT '',
            id_token       TEXT NOT NULL DEFAULT '',
            expires_at     INTEGER NOT NULL DEFAULT 0,
            user_email     TEXT NOT NULL DEFAULT '',
            user_sub       TEXT NOT NULL DEFAULT ''
        );

        CREATE TABLE IF NOT EXISTS db_migrations (
            id          TEXT PRIMARY KEY,
            applied_at  TEXT NOT NULL,
            duration_ms INTEGER NOT NULL DEFAULT 0,
            scope       TEXT NOT NULL DEFAULT 'global'
        );

        CREATE TABLE IF NOT EXISTS db_cron_jobs (
            id            TEXT PRIMARY KEY,
            name          TEXT NOT NULL,
            expression    TEXT NOT NULL,
            prompt        TEXT NOT NULL,
            target        TEXT NOT NULL,
            created_by    TEXT NOT NULL DEFAULT '',
            enabled       INTEGER NOT NULL DEFAULT 1,
            last_fired    INTEGER,
            fire_count    INTEGER NOT NULL DEFAULT 0,
            max_fires     INTEGER,
            created_at    INTEGER NOT NULL DEFAULT 0,
            max_age_secs  INTEGER,
            -- v10 (identity M1b): UID beside the slug, resolved at create
            -- time, dual-written, not yet read. Parity with the shared-store
            -- copy of this table, which is the one the handlers use today.
            target_uid    TEXT NOT NULL DEFAULT '',
            -- v11 (identity M4c-1): the creator's UID, parity with the
            -- shared-store copy.
            created_by_uid TEXT NOT NULL DEFAULT ''
        );
        CREATE INDEX IF NOT EXISTS idx_ids_cron_jobs_enabled
            ON db_cron_jobs(enabled);

        -- Muxqueue: the universal agent work queue. One row = one unit of work
        -- that ANY agent may claim. See
        -- docs/reports/REPORT_UNIVERSAL_AGENT_WORK_QUEUE_2026_09_01.md.
        --
        -- Deliberately placed directly after db_cron_jobs, because it is that
        -- table's sibling: cron is this row with a TIME trigger, this is the
        -- same row with a READINESS trigger. `not_before` makes the
        -- relationship literal.
        --
        -- It belongs in THIS schema (the always-global identity store) and not
        -- in run_shared_store_schema. A per-channel queue would only ever mean
        -- any agent IN THIS CHANNEL can pick it up, which is close to useless
        -- when every local/dev/portable build is its own channel -- see the
        -- report's section 6.2.
        --
        -- Editing note: this whole schema is ONE Rust string literal passed to
        -- execute_batch, so an UNESCAPED double quote anywhere in it --
        -- including inside an SQL comment like this one -- terminates that
        -- literal and breaks the build with errors that point at prose rather
        -- than at code. Escaped ones are fine and already appear above (see
        -- db_drone_definitions default JSON). Apostrophes are fine too: they
        -- are ordinary characters inside a Rust double-quoted string, and
        -- SQLite does not lex quotes inside a comment.
        CREATE TABLE IF NOT EXISTS db_work_queue (
            id            TEXT PRIMARY KEY,
            title         TEXT NOT NULL,
            payload       TEXT NOT NULL,
            kind          TEXT NOT NULL DEFAULT '',
            target_agent  TEXT NOT NULL DEFAULT '',
            target_group  TEXT NOT NULL DEFAULT '',
            priority      INTEGER NOT NULL DEFAULT 0,
            state         TEXT NOT NULL DEFAULT 'open',
            claimed_by    TEXT NOT NULL DEFAULT '',
            claim_expires INTEGER,
            attempts      INTEGER NOT NULL DEFAULT 0,
            max_attempts  INTEGER NOT NULL DEFAULT 3,
            created_by    TEXT NOT NULL DEFAULT '',
            created_at    INTEGER NOT NULL DEFAULT 0,
            updated_at    INTEGER NOT NULL DEFAULT 0,
            not_before    INTEGER,
            result        TEXT NOT NULL DEFAULT '',
            -- v10 (identity M1b): the UID (db_agents.id) beside each slug
            -- column, dual-written, not yet read. target_agent_uid is
            -- resolved ONCE at enqueue (spec §5.4: resolution happens at
            -- authoring time, when a human or model was present, never at
            -- claim time); claimed_by_uid is CARRIED from the claimer's own
            -- AGENTMUX_AGENT_UID. Empty when unknown -- never guessed.
            target_agent_uid TEXT NOT NULL DEFAULT '',
            claimed_by_uid   TEXT NOT NULL DEFAULT '',
            -- v11 (identity M4c-1): the enqueuer's UID from the request's
            -- Caller, beside the created_by name. Empty when unattributed.
            created_by_uid   TEXT NOT NULL DEFAULT ''
        );
        -- The claim query's exact predicate: state + readiness, ordered by
        -- priority then age. Without this the claim UPDATE degrades to a table
        -- scan under contention, which is when it matters most.
        CREATE INDEX IF NOT EXISTS idx_ids_work_queue_claimable
            ON db_work_queue(state, not_before, priority DESC, created_at);
        -- The reaper's predicate (expired leases).
        CREATE INDEX IF NOT EXISTS idx_ids_work_queue_claim_expires
            ON db_work_queue(claim_expires)
            WHERE claim_expires IS NOT NULL;

        CREATE TABLE IF NOT EXISTS db_agent_credentials (
            agent_id       TEXT PRIMARY KEY,
            client_id      TEXT NOT NULL DEFAULT '',
            client_secret  TEXT NOT NULL DEFAULT '',
            token_endpoint TEXT NOT NULL DEFAULT '',
            access_token   TEXT NOT NULL DEFAULT '',
            expires_at     INTEGER NOT NULL DEFAULT 0,
            created_at     INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS db_agent_native_memory (
            agent_id           TEXT NOT NULL,
            filename           TEXT NOT NULL,
            content            TEXT NOT NULL,
            metadata_type      TEXT NOT NULL DEFAULT '',
            size_bytes         INTEGER NOT NULL DEFAULT 0,
            updated_at         INTEGER NOT NULL DEFAULT 0,
            last_seen_path     TEXT NOT NULL DEFAULT '',
            last_seen_mtime_ms INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (agent_id, filename)
        );

        -- v3: schema parity with the v1 native memory mirror above — see
        -- IDENTITY_STORE_SCHEMA_VERSION's own doc comment for why this
        -- store's copy is not (yet) an actively-written duplicate.
        CREATE TABLE IF NOT EXISTS db_agent_native_memory_versions (
            id                 TEXT PRIMARY KEY,
            agent_id           TEXT NOT NULL,
            filename           TEXT NOT NULL,
            content            TEXT NOT NULL,
            content_hash       TEXT NOT NULL,
            parent_version_id  TEXT,
            source             TEXT NOT NULL DEFAULT 'agent_inferred',
            source_detail      TEXT NOT NULL DEFAULT '{}',
            session_id         TEXT NOT NULL DEFAULT '',
            created_at         INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_ids_native_memory_versions_lookup
            ON db_agent_native_memory_versions(agent_id, filename, created_at);

        -- v9: schema parity with the db_bundle_versions table above (own
        -- store) — see IDENTITY_STORE_SCHEMA_VERSION's own doc comment.
        CREATE TABLE IF NOT EXISTS db_bundle_versions (
            id                 TEXT PRIMARY KEY,
            bundle_id          TEXT NOT NULL,
            name               TEXT NOT NULL,
            instructions       TEXT NOT NULL,
            content_hash       TEXT NOT NULL,
            parent_version_id  TEXT,
            source             TEXT NOT NULL DEFAULT 'agent_inferred',
            source_detail      TEXT NOT NULL DEFAULT '{}',
            written_by         TEXT NOT NULL DEFAULT '',
            created_at         INTEGER NOT NULL DEFAULT 0,
            -- v11 (identity M4c-1): parity with the shared-store copy.
            written_by_uid     TEXT NOT NULL DEFAULT ''
        );
        CREATE INDEX IF NOT EXISTS idx_ids_bundle_versions_lookup
            ON db_bundle_versions(bundle_id, created_at);

        -- v6: authoritative skill/MCP-server catalogs — see the v6 note on
        -- IDENTITY_STORE_SCHEMA_VERSION in migrations.rs. Same column shapes as run_object_schema's
        -- copies; no ref tables here yet (Phase 3/6), so no FK targets exist
        -- to reference them from this store.
        CREATE TABLE IF NOT EXISTS db_skills (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            trigger     TEXT NOT NULL DEFAULT '',
            skill_type  TEXT NOT NULL DEFAULT 'prompt',
            description TEXT NOT NULL DEFAULT '',
            content     TEXT NOT NULL DEFAULT '',
            is_global   INTEGER NOT NULL DEFAULT 0,
            created_at  INTEGER NOT NULL DEFAULT 0,
            updated_at  INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_ids_skills_is_global ON db_skills(is_global);
        -- v8: name-only, matching skill_upsert_unique_global's own dup-check
        -- — see the v8 note on IDENTITY_STORE_SCHEMA_VERSION in migrations.rs. Replaces
        -- idx_ids_skills_global_name_type (dropped just below for an
        -- existing store; a fresh store never creates it in the first
        -- place, since this whole batch runs as one execute_batch).
        CREATE UNIQUE INDEX IF NOT EXISTS idx_ids_skills_global_name
            ON db_skills(name) WHERE is_global = 1;
        DROP INDEX IF EXISTS idx_ids_skills_global_name_type;

        CREATE TABLE IF NOT EXISTS db_mcp_servers (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            transport   TEXT NOT NULL DEFAULT 'stdio',
            config      TEXT NOT NULL DEFAULT '{}',
            is_global   INTEGER NOT NULL DEFAULT 0,
            created_at  INTEGER NOT NULL DEFAULT 0,
            updated_at  INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_ids_mcp_servers_is_global ON db_mcp_servers(is_global);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_ids_mcp_servers_global_name
            ON db_mcp_servers(name) WHERE is_global = 1;
        -- Who a catalog server reaches beyond its binds. Added without a
        -- version bump — see IDENTITY_STORE_SCHEMA_VERSION's doc comment.
        CREATE TABLE IF NOT EXISTS db_mcp_server_reach (
            mcp_id        TEXT PRIMARY KEY,
            all_agents    INTEGER NOT NULL DEFAULT 0,
            in_containers INTEGER NOT NULL DEFAULT 0,
            updated_at    INTEGER NOT NULL DEFAULT 0
        );";

/// The blank Bundle singleton in the shared store.
pub(super) fn seed_shared_blank_bundle(conn: &Connection) -> Result<(), StoreError> {
    // Seed the blank Bundle singleton — same fixed id as objects.db so
    // cross-version reads never see a missing blank row.
    conn.execute_batch(
        "INSERT OR IGNORE INTO db_bundles
            (id, name, description, is_blank, created_at, updated_at)
         VALUES ('blank', '__blank__', 'Vanilla CLI — no instructions, no context', 1, 0, 0);",
    )?;
    Ok(())
}

/// Additive columns on the shared store: the cron job bounds, bundle columns and identity UIDs.
pub(super) fn add_shared_store_columns(conn: &Connection) -> Result<(), StoreError> {
    // ---- Additive column migrations (schema v5+) ----
    // Same idempotent pattern as run_object_schema's additive column steps (`object_schema.rs`):
    // SHARED_STORE_TABLES_SQL covers fresh DBs; these carry an existing
    // shared store forward. "duplicate column" is swallowed so this is safe
    // to run on every startup regardless of whether the column already exists.
    //
    // v5: db_cron_jobs.max_age_secs — optional hard expiry bound (seconds
    //     since created_at), alongside the existing max_fires bound. NULL
    //     for all existing rows = no behavior change for jobs created before
    //     this column existed.
    apply_additive_columns(conn, [
        "ALTER TABLE db_cron_jobs ADD COLUMN max_age_secs INTEGER",
        // v7: provider-scoped bundle instructions (ABF v0.2 §2.2).
        "ALTER TABLE db_bundles ADD COLUMN instructions_by_provider TEXT NOT NULL DEFAULT '{}'",
        // v9: AgentMux-controlled, highest-priority Global Bundle tier —
        // see OBJECT_SCHEMA_VERSION's v27 doc comment in migrations.rs.
        "ALTER TABLE db_bundles ADD COLUMN is_system INTEGER NOT NULL DEFAULT 0",
        // v11: identity M1b — the cron target's UID beside its slug.
        "ALTER TABLE db_cron_jobs ADD COLUMN target_uid TEXT NOT NULL DEFAULT ''",
        // v12: identity M4c-1 — the actor's UID beside each actor name.
        "ALTER TABLE db_cron_jobs ADD COLUMN created_by_uid TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_bundle_versions ADD COLUMN written_by_uid TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_agent_native_memory ADD COLUMN last_seen_mtime_ms INTEGER NOT NULL DEFAULT 0", // a table made before this column
    ].iter().chain(JOB_TABLES.iter()))?;
    Ok(())
}

/// The blank Bundle singleton in the identity store.
pub(super) fn seed_identity_blank_bundle(conn: &Connection) -> Result<(), StoreError> {
    // Seed the blank Bundle singleton — same fixed id as objects.db/store.db
    // so cross-version reads never see a missing blank row.
    conn.execute_batch(
        "INSERT OR IGNORE INTO db_bundles
            (id, name, description, is_blank, created_at, updated_at)
         VALUES ('blank', '__blank__', 'Vanilla CLI — no instructions, no context', 1, 0, 0);",
    )?;
    Ok(())
}

/// Additive columns on the identity store, added after its tables were first created.
pub(super) fn add_identity_store_columns(conn: &Connection) -> Result<(), StoreError> {
    // Additive columns added after this store's tables were first created —
    // same idempotent pattern as add_shared_store_columns: both go through apply_additive_columns.
    apply_additive_columns(conn, [
        // db_bundles.is_system: see OBJECT_SCHEMA_VERSION's v27 doc comment.
        "ALTER TABLE db_bundles ADD COLUMN is_system INTEGER NOT NULL DEFAULT 0",
        // v10: identity M1b — UID columns beside the slug columns, dual-written.
        "ALTER TABLE db_work_queue ADD COLUMN target_agent_uid TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_work_queue ADD COLUMN claimed_by_uid TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_cron_jobs ADD COLUMN target_uid TEXT NOT NULL DEFAULT ''",
        // v11: identity M4c-1 — the actor's UID beside each actor name.
        "ALTER TABLE db_work_queue ADD COLUMN created_by_uid TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_cron_jobs ADD COLUMN created_by_uid TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_bundle_versions ADD COLUMN written_by_uid TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_agent_native_memory ADD COLUMN last_seen_mtime_ms INTEGER NOT NULL DEFAULT 0", // a table made before this column
    ].iter().chain(JOB_TABLES.iter()).chain(VAULT_ALTERS.iter()))?;
    Ok(())
}
