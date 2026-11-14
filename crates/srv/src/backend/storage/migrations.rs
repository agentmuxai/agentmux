// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! SQL schema setup for Store, FileStore, and the saga log.
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
//! tripwire (`check_schema_compat` + `stamp_version`).

use rusqlite::Connection;
use tracing::warn;

use super::error::StoreError;

mod object_schema;
mod store_schema;

pub(in crate::backend::storage) use object_schema::apply_additive_columns;

/// `user_version` stamped into `~/.agentmux/shared/store.db` after `run_shared_store_schema`.
/// Versioned independently from `objects.db`. What each version (v1 to v12) added:
/// docs/architecture/STORAGE_SCHEMA_VERSION_HISTORY.md, "Shared store".
pub const SHARED_STORE_SCHEMA_VERSION: i64 = 12;

/// `user_version` stamped into `objects.db` after `run_object_schema`. The flat schema reset the
/// counter to 1 (the pre-flatten chain never set `user_version`, so legacy files read 0). Bumped
/// per additive migration; `check_schema_compat` makes an older binary refuse a file stamped
/// newer than it. What each version (v1 to v45) added:
/// docs/architecture/STORAGE_SCHEMA_VERSION_HISTORY.md, "Objects store".
pub const OBJECT_SCHEMA_VERSION: i64 = 45;
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
/// also subsumes the v11 `db_memories` rename).
const LEGACY_TABLE_RENAMES: &[(&str, &str)] = &[
    ("db_forge_agents", "db_agent_definitions"),
    ("db_forge_content", "db_agent_content"),
    ("db_forge_skills", "db_agent_skills"),
    ("db_forge_history", "db_agent_history"),
    ("db_forge_agent_identities", "db_agent_identity_links"),
    ("db_memories", "db_memory_bundles"),
    // Phase 4 of SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md — the
    // "bundle" naming collision fix (SPEC_ARMORY_PHASE4_STORAGE_RENAME_COMPLETION_2026_07_12.md).
    // Order matters: this entry runs AFTER the `db_memories` rename above
    // in the same pass, so a database old enough to still have `db_memories`
    // chains straight through to `db_bundles` in one migration run instead
    // of stopping at the intermediate name.
    ("db_identity_accounts", "db_accounts"),
    ("db_memory_bundles", "db_bundles"),
];

/// Legacy index names that must be dropped after their table is renamed —
/// `ALTER TABLE … RENAME` keeps indexes attached but under their old names,
/// which would collide with the flat DDL's `CREATE INDEX`. The flat DDL
/// recreates each under the new name.
const LEGACY_INDEX_DROPS: &[&str] = &[
    "idx_forge_agents_slug",
    "idx_forge_history_agent_date",
    "idx_forge_agent_identities_account",
    "idx_memories_is_blank",
    // Phase 4 rename (see LEGACY_TABLE_RENAMES above) — objects.db names,
    // then the shared store.db names (`idx_ss_*` prefix, see
    // run_shared_store_schema).
    "idx_identity_accounts_provider",
    "idx_memory_bundles_is_blank",
    "idx_ss_identity_accounts_provider",
    "idx_ss_memory_bundles_is_blank",
];

/// Tables retained by the old chain only for a downgrade path the flatten
/// abandons. Dropped from any legacy DB by the adopt step; never created
/// by the flat schema. `db_workflow_*` data was already copied into
/// `db_drone_*` by the old v10 migration, so dropping loses nothing.
///
/// `db_identities` (the pre-v11 name), `db_identity_bundles`, and
/// `db_identity_bindings` were dropped outright rather than renamed
/// forward in Phase 4c of SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md —
/// `db_agent_identity_links`/`db_accounts` is the sole credential-
/// resolution path (`identity/resolver/inject.rs::resolve_bindings_for_instance`),
/// confirmed by the direct-link backfill migrations, since retired
/// (see SPEC_ARMORY_PHASE4_STORAGE_RENAME_COMPLETION_2026_07_12.md §6/§8a).
const DEAD_TABLE_DROPS: &[&str] = &[
    "db_workflow_definitions",
    "db_workflow_runs",
    "db_v10_migrated_legacy_defs",
    "db_v10_migrated_legacy_runs",
    "db_identities",
    "db_identity_bundles",
    "db_identity_bindings",
];

/// Initialize (or re-validate) the full `objects.db` schema.
///
/// Idempotent — safe on every srv startup. Steps:
///
/// 1. `adopt_legacy_table_names` — renames any pre-flatten forge/bundle
///    tables found (protects dev databases created before the flatten;
///    see the spec §3/§7) and drops the dead workflow/sentinel tables.
/// 2. The flat `CREATE TABLE IF NOT EXISTS` batch — the canonical schema.
/// 3. Seeds the blank Bundle singleton row.
///
/// A database stuck at a pre-v11 intermediate schema cannot be fully
/// adopted (its tables predate later columns). The adopt step still
/// renames what it finds; the first query referencing a missing column
/// then fails loudly with `no such column` — a hard error, not silent
/// empty state, with the data preserved on disk. Per-version data dirs
/// make that case unreachable for released builds.
pub fn run_object_schema(conn: &Connection) -> Result<(), StoreError> {
    adopt_legacy_table_names(conn)?;

    object_schema::create_generic_tables(conn)?;
    object_schema::create_input_history_tables(conn)?;
    conn.execute_batch(object_schema::OBJECT_TABLES_SQL)?;
    object_schema::add_object_columns(conn)?;
    object_schema::add_legacy_definition_columns(conn)?;
    object_schema::add_agent_columns(conn)?;
    object_schema::seed_blank_bundle(conn)?;
    object_schema::create_channel_migrations(conn)?;

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
        if !crate::migrations::table_exists(conn, legacy)? {
            continue;
        }
        if crate::migrations::table_exists(conn, current)? {
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

/// Initialize (or re-validate) the `~/.agentmux/shared/store.db` schema.
///
/// Contains only the durable user content that must survive across
/// channels/versions: identity accounts, agent→account links, memory
/// bundles, drone definitions, and muxbus credentials. Session state
/// (`db_block`, `db_tab`, etc.) and drone run history stay in the
/// per-channel `objects.db`.
///
/// `db_agent_identity_links` drops the FK to `db_agent_definitions` here
/// (cross-DB FK enforcement is impossible in SQLite); application code
/// enforces referential integrity instead.
///
/// Idempotent — safe to call on every startup.
pub fn run_shared_store_schema(conn: &Connection) -> Result<(), StoreError> {
    // Rename any legacy table names (including the Phase 4 accounts/bundles
    // rename — LEGACY_TABLE_RENAMES entries that don't apply to this
    // connection's tables are safely skipped) before the idempotent
    // CREATE TABLE IF NOT EXISTS batch below, same as run_object_schema.
    adopt_legacy_table_names(conn)?;

    conn.execute_batch(store_schema::SHARED_STORE_TABLES_SQL)?;
    store_schema::seed_shared_blank_bundle(conn)?;
    store_schema::add_shared_store_columns(conn)?;

    Ok(())
}

/// `user_version` stamped into the permanently-global `~/.agentmux/shared/identity-store.db` (a
/// different file from `store.db`; see [`run_identity_store_schema`]). Independent counter from
/// `SHARED_STORE_SCHEMA_VERSION`. What each version (v1 to v11) added:
/// docs/architecture/STORAGE_SCHEMA_VERSION_HISTORY.md, "Identity store".
///
/// Added WITHOUT a bump: `db_mcp_server_reach` (OBJECT_SCHEMA_VERSION v44
/// has the same table). The catalog's reach switches live with the catalog,
/// here, so they survive an upgrade where an agent bind
/// (`db_agent_mcp_ref`, in `objects.db`) does not. This store is one file
/// shared by every AgentMux on the computer, installed and dev builds alike,
/// and `check_schema_compat` makes any build that speaks an older version
/// refuse it. So a bump here locks every older AgentMux on the computer out
/// as soon as one newer build starts: a dev build of a branch with a bump
/// did exactly that on 2026-10-11. A table an older build never reads needs
/// no lock: an older build ignores it, a newer one creates it if missing.
/// Bump only for a change an older build would misread or corrupt.
/// `SPEC_CONNECTORS_MCP_WHAT_AGENTS_RUN_2026_10_10.md` §2.1.
pub const IDENTITY_STORE_SCHEMA_VERSION: i64 = 11;

/// Initialize (or re-validate) the `~/.agentmux/shared/identity-store.db` schema: the
/// permanently-global store from docs/specs/SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md, which
/// fixes conversation continuity across a version switch
/// (docs/specs/REPORT_HISTORY_CONTINUITY_ACROSS_VERSION_UPGRADE_2026_08_17.md).
///
/// A **near-superset** of [`run_shared_store_schema`]'s tables (v2), including `db_accounts`,
/// but there it is a read-through FALLBACK MIRROR, not the authoritative write path. Accounts
/// have one intentional isolation need (Armory's disposable delete-account testing flow) and
/// the per-account creation-time scope split (SPEC §3.2) hasn't shipped, so `id_store` /
/// `resolve_shared_store_path()` stays the primary read/write path for accounts. This copy
/// exists only so a link resolved here (always global) doesn't dead-end on an account row stuck
/// in a fresh, per-channel `id_store`: see `identity::resolver::inject`'s account resolution
/// (falls back to this store's `identity_get` only when `id_store`'s lookup misses) and
/// `migrations::m0022_identity_store_links_backfill`, which backfills the mirror as it does
/// links. `db_agent_identity_links.account_id` has NO foreign key here, as
/// `run_shared_store_schema` does for `db_agent_definitions`: cross-database FKs can't be
/// enforced in SQLite and the mirror isn't guaranteed complete, so application code enforces
/// integrity and tolerates a miss.
///
/// A dedicated store, not a subset of the shared store reached by a new always-global path, is
/// deliberate: "can this table ever be reached from a per-channel-isolated codepath" becomes a
/// structural fact (a separate file and connection) rather than a convention a later call site
/// of `resolve_shared_store_path()` could violate, the failure mode
/// docs/specs/CHECKLIST_AGENT_DATA_SCOPE_ROUTING_2026_08_17.md items 2-4 record as having
/// bitten more than once.
///
/// Idempotent — safe to call on every startup.
pub fn run_identity_store_schema(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(store_schema::IDENTITY_STORE_TABLES_SQL)?;
    conn.execute_batch(super::vault::VAULT_SCHEMA)?; // The Vault's tables, no version bump: storage/vault.rs.
    store_schema::seed_identity_blank_bundle(conn)?;
    store_schema::add_identity_store_columns(conn)?;

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
    // Line counter (Phase 5a-2, SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_
    // 2026_09_23.md §6.3.7; see `filestore/counter.rs`). Additive and
    // nullable, and deliberately WITHOUT a FILESTORE_SCHEMA_VERSION bump: the
    // global transcript store is shared by every srv instance on the machine,
    // and a bump would make older builds refuse to open it. Older builds
    // insert rows without these columns (NULL = not counted) and write
    // without maintaining them, which the counter detects. "duplicate
    // column" is expected when another instance added them first.
    apply_additive_columns(conn, [
        "ALTER TABLE db_wave_file ADD COLUMN gen TEXT",
        "ALTER TABLE db_wave_file ADD COLUMN lines INTEGER",
        "ALTER TABLE db_wave_file ADD COLUMN lines_size INTEGER",
        "ALTER TABLE db_wave_file ADD COLUMN lines_tail INTEGER",
        "ALTER TABLE db_wave_file ADD COLUMN lines_modts INTEGER",
        // Bumped (by the triggers below) by every write that changes bytes
        // already in the file — not by appends — and the value an epoch was
        // counted at. An epoch is valid only while they are equal.
        "ALTER TABLE db_wave_file ADD COLUMN rev INTEGER",
        "ALTER TABLE db_wave_file ADD COLUMN lines_rev INTEGER",
        // A random identity per row, set by the insert trigger below: a file
        // deleted and re-created is a different incarnation even if every
        // other column (and the creation millisecond) comes out the same.
        "ALTER TABLE db_wave_file ADD COLUMN incarnation BLOB",
    ])?;
    // `rev` is maintained by triggers, not by this code's writes, because
    // triggers are part of the database: they fire for every connection,
    // including older builds that know nothing about the counter. Any write
    // that changes existing bytes bumps it:
    //   - a part deleted (write_file's replace, a delete);
    //   - a part written over with bytes that aren't a pure extension of it
    //     (REPLACE INTO / UPDATE). Appending extends the last part, which
    //     keeps it a prefix, so appends — ours or an older build's — don't.
    // REPLACE INTO doesn't fire DELETE triggers (recursive_triggers is off,
    // SQLite's default; nothing here enables it), so an append can't reach
    // the delete trigger. A trigger on a deleted row's parts finds no row.
    conn.execute_batch(
        "-- Versioned name: CREATE TRIGGER IF NOT EXISTS never updates an
         -- existing definition, so a changed rule gets a new name and the
         -- old one is dropped.
         DROP TRIGGER IF EXISTS db_file_data_rev_insert;
         CREATE TRIGGER IF NOT EXISTS db_file_data_rev_insert_v2
         BEFORE INSERT ON db_file_data
         WHEN EXISTS (
             SELECT 1 FROM db_file_data d
             WHERE d.zoneid = NEW.zoneid AND d.name = NEW.name AND d.partidx = NEW.partidx
               AND substr(NEW.data, 1, length(d.data)) IS NOT d.data
         )
         -- A NEW part inside the range the file already claims: bytes the
         -- file declared (and a reader may have read as missing) are being
         -- filled in, not appended. Every writer inserts appended parts
         -- before raising `size`, so an append's new parts always start at
         -- or past it. Older builds' write_file raises `size` first and
         -- inserts after (Codex on #3631). 65536 is PART_DATA_SIZE.
         OR (
             NOT EXISTS (
                 SELECT 1 FROM db_file_data d
                 WHERE d.zoneid = NEW.zoneid AND d.name = NEW.name AND d.partidx = NEW.partidx
             )
             AND NEW.partidx * 65536 < (
                 SELECT COALESCE(MAX(w.size), 0) FROM db_wave_file w
                 WHERE w.zoneid = NEW.zoneid AND w.name = NEW.name
             )
         )
         BEGIN
             UPDATE db_wave_file SET rev = COALESCE(rev, 0) + 1
             WHERE zoneid = NEW.zoneid AND name = NEW.name;
         END;

         CREATE TRIGGER IF NOT EXISTS db_file_data_rev_update
         BEFORE UPDATE OF data ON db_file_data
         WHEN substr(NEW.data, 1, length(OLD.data)) IS NOT OLD.data
         BEGIN
             UPDATE db_wave_file SET rev = COALESCE(rev, 0) + 1
             WHERE zoneid = OLD.zoneid AND name = OLD.name;
         END;

         CREATE TRIGGER IF NOT EXISTS db_file_data_rev_delete
         AFTER DELETE ON db_file_data
         BEGIN
             UPDATE db_wave_file SET rev = COALESCE(rev, 0) + 1
             WHERE zoneid = OLD.zoneid AND name = OLD.name;
         END;

         -- Every row, whoever inserts it (older builds included), gets 64
         -- random bits of identity. A delete + re-create in the same
         -- millisecond with the same size and bytes is still a different
         -- incarnation (Codex on #3631).
         CREATE TRIGGER IF NOT EXISTS db_wave_file_incarnation
         AFTER INSERT ON db_wave_file
         BEGIN
             UPDATE db_wave_file SET incarnation = randomblob(8)
             WHERE zoneid = NEW.zoneid AND name = NEW.name;
         END;

         UPDATE db_wave_file SET incarnation = randomblob(8) WHERE incarnation IS NULL;",
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

/// `PRAGMA user_version` tripwire (AUDIT_SQLITE_SYSTEMS §8.5): compare the file's
/// `user_version` to `current` and refuse to open a database written by a NEWER binary. This is
/// the forward-compat safety lock of the channels design (`SPEC_DATA_CHANNELS_2026_05_24.md`
/// §3.3): within a channel several released AgentMux versions share one data dir, and the lock
/// keeps an older binary from writing into a schema a newer one laid down (as Chrome's
/// profile-too-new check and Postgres's catalog version mismatch do).
///
/// **Must be called BEFORE `run_*_schema` / `run_*_migrations`.** Those include mutating steps
/// (legacy-table renames, seed inserts, `CREATE TABLE` for tables the older binary doesn't
/// have), so checking afterwards would let a downgraded binary alter a newer database before
/// the error fires, breaking the "reject without touching disk" invariant (#1029).
///
/// Read-only: only a `PRAGMA user_version` query. [`stamp_version`] does the corresponding
/// write AFTER migrations succeed.
///
/// On `SchemaTooNew` the user upgrades AgentMux to a version ≥ `found`, or sets
/// `AGENTMUX_CHANNEL=<other>` to land in a different channel dir; the data on disk is preserved
/// either way, since this function never modifies the database on the rejected path.
pub fn check_schema_compat(
    conn: &Connection,
    current: i64,
    db_label: &str,
) -> Result<(), StoreError> {
    let found: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if found > current {
        warn!(
            db = db_label,
            found, expected = current,
            "database user_version is newer than this build — refusing \
             to open. Upgrade AgentMux or switch channels.",
        );
        return Err(StoreError::SchemaTooNew {
            db: db_label.to_string(),
            found,
            expected: current,
        });
    }
    Ok(())
}

/// Stamp the database's `user_version` PRAGMA to `current`. Called
/// AFTER `run_*_schema` succeeds, paired with a prior
/// [`check_schema_compat`] that gated the migrations on the
/// caller-binary speaking a compatible (or newer) schema version.
///
/// Splitting the read from the write makes the safety-lock order
/// explicit at every call site:
///
/// ```ignore
/// check_schema_compat(&conn, OBJECT_SCHEMA_VERSION, "objects.db")?;
/// run_object_schema(&conn)?;
/// stamp_version(&conn, OBJECT_SCHEMA_VERSION)?;
/// ```
pub fn stamp_version(conn: &Connection, current: i64) -> Result<(), StoreError> {
    conn.execute_batch(&format!("PRAGMA user_version = {current};"))?;
    Ok(())
}

#[cfg(test)]
#[path = "migrations_tests.rs"]
mod tests;
