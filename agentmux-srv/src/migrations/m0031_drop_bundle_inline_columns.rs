// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Drop the inline `db_bundles.mcp_servers` / `db_bundles.skills` JSON
//! columns — the ref tables are the single source of truth.
//!
//! Phase 0b of `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md`. The
//! columns and `db_bundle_skills_ref` / `db_bundle_mcp_ref` recorded the same
//! fact in two places and nothing kept them in sync: binding a skill in the
//! Armory wrote a ref row, while ABF export read the column, so a bundle could
//! run with components it did not export and export components it did not run
//! with (§3.4). `m0030` made the refs authoritative and carried the inline data
//! across; this removes the columns so there is no second place left to drift.
//!
//! **Ordering is the whole design.** This MUST run after `m0030` — dropping
//! first would destroy the data `m0030` exists to carry. Both are
//! channel-scoped and `m0031` is registered immediately after `m0030` in
//! `REGISTRY`, which the runner walks in order, so within any one boot m0030
//! always completes first. That ordering is also why this could not live in
//! `run_shared_store_schema` alongside the ADD COLUMN statements it mirrors:
//! those run on every `Store::open()`, which happens before the migration
//! chain gets to `m0030`. The identity store IS handled that way, because its
//! `db_bundles` is a never-written parity copy with no data to carry — see
//! `IDENTITY_STORE_SCHEMA_VERSION`'s v6 note.
//!
//! **Two stores.** `db_bundles` lives in the shared store, but falls back to
//! the channel store when the shared one is unavailable (see
//! `bootstrap.rs`'s `id_store` resolution and `m0030::resolve_bundle_store`).
//! Rather than guess which one a given install used, this drops from both
//! whenever the columns are present — a store that never had them is a no-op.
//!
//! **Idempotent.** Every drop is guarded by a `PRAGMA table_info` check, so
//! re-running finds nothing to do. That guard is not just belt-and-braces: a
//! channel first booted AFTER this shipped runs the whole chain against a
//! shared store that was dropped by some earlier channel, and must treat that
//! as success rather than error.
//!
//! `ALTER TABLE ... DROP COLUMN` needs SQLite 3.35+; the workspace pins
//! `rusqlite` with `bundled` (3.45), so this is available on every platform
//! with no host-SQLite variability. Neither column is a primary key, unique,
//! indexed, generated, or referenced by a CHECK, view, or trigger — the
//! repo defines none of those on `db_bundles` — so a plain DROP COLUMN is
//! legal and no table rebuild (`m0028`'s pattern) is needed.

use std::path::Path;

use rusqlite::Connection;

use super::{Migration, MigrationContext, MigrationError, MigrationScope, VerifyOutcome};

pub struct M0031DropBundleInlineColumns;

/// The two columns this migration retires.
const RETIRED_COLUMNS: [&str; 2] = ["mcp_servers", "skills"];

/// Which of [`RETIRED_COLUMNS`] `db_bundles` still declares.
///
/// `PRAGMA table_info` on a missing table returns zero rows rather than an
/// error, so this doubles as a table-existence check — a channel store that
/// never carried `db_bundles` reports nothing present.
pub(super) fn present_retired_columns(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(db_bundles)")
        .map_err(|e| format!("prepare table_info: {e}"))?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(|e| format!("query table_info: {e}"))?;
    let mut found = Vec::new();
    for name in rows.flatten() {
        if RETIRED_COLUMNS.contains(&name.as_str()) {
            found.push(name);
        }
    }
    Ok(found)
}

/// Drop whichever retired columns `db_bundles` still has. Returns how many
/// were dropped; zero means there was nothing to do, which is a success.
pub(super) fn drop_inline_columns(conn: &Connection) -> Result<usize, String> {
    let present = present_retired_columns(conn)?;
    for column in &present {
        // Interpolated rather than bound because SQLite does not accept a
        // parameter in a DDL identifier position. Safe: `column` came out of
        // `RETIRED_COLUMNS` via the `contains` filter above, so it is one of
        // two compile-time literals, never anything read off the database.
        conn.execute_batch(&format!("ALTER TABLE db_bundles DROP COLUMN {column}"))
            .map_err(|e| format!("drop db_bundles.{column}: {e}"))?;
    }
    Ok(present.len())
}

/// Drop from one store file, if it exists. A missing file is not an error —
/// an install may have only one of the two stores.
fn drop_from_store(path: &Path) -> Result<usize, String> {
    if !path.exists() {
        return Ok(0);
    }
    let conn = Connection::open(path).map_err(|e| format!("open {}: {}", path.display(), e))?;
    drop_inline_columns(&conn)
}

impl Migration for M0031DropBundleInlineColumns {
    fn id(&self) -> &'static str {
        "0031_drop_bundle_inline_columns"
    }

    fn scope(&self) -> MigrationScope {
        MigrationScope::Channel
    }

    fn description(&self) -> &'static str {
        "Drop the inline db_bundles skills/mcp_servers columns — the ref tables are authoritative"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        let mut dropped = 0usize;
        for path in [&ctx.shared_store_path, &ctx.channel_store_path] {
            dropped += drop_from_store(path)
                .map_err(|e| MigrationError(format!("drop_bundle_inline_columns: {e}")))?;
        }
        tracing::info!(
            columns_dropped = dropped,
            "drop_bundle_inline_columns: complete"
        );
        Ok(())
    }

    fn verify(&self, ctx: &MigrationContext) -> VerifyOutcome {
        let mut remaining = Vec::new();
        for path in [&ctx.shared_store_path, &ctx.channel_store_path] {
            match super::runner::open_readonly(path) {
                Ok(Some(conn)) => match present_retired_columns(&conn) {
                    Ok(found) => remaining.extend(found),
                    Err(e) => return VerifyOutcome::Error(e),
                },
                Ok(None) => {}
                Err(e) => return VerifyOutcome::Error(e),
            }
        }
        if remaining.is_empty() {
            VerifyOutcome::Ok("no inline bundle columns remain".to_string())
        } else {
            VerifyOutcome::Mismatch(format!(
                "db_bundles still declares: {}",
                remaining.join(", ")
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `db_bundles` shaped like the pre-drop schema, with one row.
    fn legacy_store(path: &Path) -> Connection {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE db_bundles (
                id            TEXT PRIMARY KEY,
                name          TEXT NOT NULL UNIQUE,
                context_files TEXT NOT NULL DEFAULT '[]',
                mcp_servers   TEXT NOT NULL DEFAULT '[]',
                skills        TEXT NOT NULL DEFAULT '[]',
                sort_order    INTEGER NOT NULL DEFAULT 0
             );
             INSERT INTO db_bundles (id, name, mcp_servers, skills)
             VALUES ('b1', 'one', '[{\"type\":\"stdio\"}]', '[\"s1\"]');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn drops_both_columns_and_keeps_the_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.db");
        let conn = legacy_store(&path);

        assert_eq!(drop_inline_columns(&conn).unwrap(), 2);
        assert!(present_retired_columns(&conn).unwrap().is_empty());

        // The bundle itself must survive — this retires a redundant
        // representation, it does not delete bundles.
        let name: String = conn
            .query_row("SELECT name FROM db_bundles WHERE id = 'b1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "one");
    }

    #[test]
    fn is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.db");
        let conn = legacy_store(&path);

        assert_eq!(drop_inline_columns(&conn).unwrap(), 2);
        assert_eq!(
            drop_inline_columns(&conn).unwrap(),
            0,
            "a second run must find nothing to do, not fail"
        );
    }

    #[test]
    fn a_store_with_no_db_bundles_is_a_no_op() {
        // The normal path for a channel first booted after this shipped: the
        // chain still runs, against a store that never had the table.
        let dir = tempfile::tempdir().unwrap();
        let conn = Connection::open(dir.path().join("store.db")).unwrap();
        assert!(present_retired_columns(&conn).unwrap().is_empty());
        assert_eq!(drop_inline_columns(&conn).unwrap(), 0);
    }

    #[test]
    fn drops_only_one_column_when_only_one_is_present() {
        // A half-migrated store (interrupted between the two ALTERs) must
        // finish the job rather than tripping over the missing column.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.db");
        let conn = legacy_store(&path);
        conn.execute_batch("ALTER TABLE db_bundles DROP COLUMN mcp_servers")
            .unwrap();

        assert_eq!(drop_inline_columns(&conn).unwrap(), 1);
        assert!(present_retired_columns(&conn).unwrap().is_empty());
    }

    #[test]
    fn a_missing_store_file_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(drop_from_store(&dir.path().join("absent.db")).unwrap(), 0);
    }
}
