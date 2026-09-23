// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Identity M3 backfill: fill `db_work_queue.target_agent_uid` (identity
//! store) and `db_cron_jobs.target_uid` (shared store) for rows written before
//! M1b/M3, where the name resolves to exactly one `db_agents` row in THIS
//! channel's object store.
//!
//! Spec §9.1 M3 — "one-time backfill of live rows". §12 left open whether to
//! rewrite live rows or drain them; this rewrites, best-effort, for one
//! reason: a rewritten row keeps working under both keys, a drained one is a
//! task or schedule a human loses. Rows whose name resolves to nothing, or to
//! more than one agent, are left with an empty UID column — they keep
//! resolving by name exactly as before, and the miss is what §9.2's counters
//! will show at fire/claim time.
//!
//! Channel-scoped, because resolution needs this channel's `db_agents`. The
//! queue and cron tables are global, so a row targeting an agent that lives
//! in another channel is skipped here and backfilled when that channel's
//! srv runs this migration. Only rows that can still fire or be claimed are
//! touched: `open`/`claimed` work items, enabled cron jobs.
//!
//! No registry is consulted (nothing is running at migration time), so this
//! resolves through the store alone: exact slug, or a case-insensitive match
//! on `name`/`instance_name`, over non-template, non-hidden rows.

use rusqlite::Connection;

use super::{Migration, MigrationContext, MigrationError, MigrationScope, VerifyOutcome};
use crate::backend::storage::store::Store;
use crate::registry;

pub struct M0035BackfillUidColumnsFromNames;

/// Open a store file read-write, `None` if it does not exist yet (a fresh
/// install has nothing to backfill). Mirrors `runner::open_readonly`'s
/// missing-file semantics; the runner has no read-write counterpart because
/// no earlier migration needed to UPDATE a global store from a channel one.
fn open_rw(path: &std::path::Path) -> Result<Option<Connection>, String> {
    if !path.exists() {
        return Ok(None);
    }
    Connection::open(path)
        .map(Some)
        .map_err(|e| format!("open {}: {e}", path.display()))
}

/// The UID a name backfills to, if exactly one row in `mstore` bears it.
fn unique_uid_for_name(mstore: &Store, name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    // Already a UID? Only a non-template row counts (spec §1.2).
    if let Ok(Some(def)) = mstore.agent_def_get(name) {
        if def.is_seeded == 0 {
            return Some(def.id);
        }
    }
    match mstore.agents_matching_name(name) {
        Ok(rows) if rows.len() == 1 => Some(rows[0].id.clone()),
        _ => None,
    }
}

/// Fill `uid_col` from `name_col` on every row matching `predicate` that has
/// an empty UID. Returns (backfilled, skipped).
fn backfill_table(
    conn: &Connection,
    mstore: &Store,
    table: &str,
    name_col: &str,
    uid_col: &str,
    predicate: &str,
) -> Result<(usize, usize), String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT id, {name_col} FROM {table} WHERE {uid_col} = '' AND {name_col} <> '' AND ({predicate})"
        ))
        .map_err(|e| format!("select {table}: {e}"))?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| format!("query {table}: {e}"))?
        .collect::<Result<_, _>>()
        .map_err(|e| format!("read {table}: {e}"))?;
    let mut done = 0usize;
    let mut skipped = 0usize;
    for (id, name) in rows {
        match unique_uid_for_name(mstore, &name) {
            Some(uid) => {
                conn.execute(
                    &format!("UPDATE {table} SET {uid_col} = ?1 WHERE id = ?2 AND {uid_col} = ''"),
                    rusqlite::params![uid, id],
                )
                .map_err(|e| format!("update {table} {id}: {e}"))?;
                done += 1;
            }
            None => skipped += 1,
        }
    }
    Ok((done, skipped))
}

impl Migration for M0035BackfillUidColumnsFromNames {
    fn id(&self) -> &'static str {
        "0035_backfill_uid_columns_from_names"
    }
    fn scope(&self) -> MigrationScope {
        MigrationScope::Channel
    }
    fn description(&self) -> &'static str {
        "Identity M3: backfill db_work_queue.target_agent_uid and db_cron_jobs.target_uid where the name resolves to exactly one agent in this channel"
    }
    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        let mstore = Store::open(&ctx.channel_store_path)
            .map_err(|e| MigrationError(format!("open channel store: {e}")))?;

        // Work queue lives in the identity store.
        let mut work = (0usize, 0usize);
        if let Some(path) = registry::resolve_identity_store_path() {
            if let Some(conn) = open_rw(&path).map_err(MigrationError)? {
                work = backfill_table(
                    &conn,
                    &mstore,
                    "db_work_queue",
                    "target_agent",
                    "target_agent_uid",
                    "state IN ('open','claimed')",
                )
                .map_err(MigrationError)?;
            }
        }
        // Cron jobs live in the shared store (the copy the handlers use).
        let mut cron = (0usize, 0usize);
        if let Some(conn) = open_rw(&ctx.shared_store_path).map_err(MigrationError)? {
            cron = backfill_table(
                &conn,
                &mstore,
                "db_cron_jobs",
                "target",
                "target_uid",
                "enabled = 1",
            )
            .map_err(MigrationError)?;
        }
        tracing::info!(
            work_backfilled = work.0,
            work_skipped = work.1,
            cron_backfilled = cron.0,
            cron_skipped = cron.1,
            "m0035_backfill_uid_columns_from_names: complete"
        );
        Ok(())
    }
    fn verify(&self, _ctx: &MigrationContext) -> VerifyOutcome {
        // Best-effort by design: a skipped row is a legitimate outcome
        // (unknown or ambiguous name), so there is no invariant to check
        // beyond "ran without error", which the framework records.
        VerifyOutcome::Ok("best-effort backfill; skipped rows are expected".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::agents::test_agent_def;

    fn mstore_with(rows: &[(&str, &str, &str)]) -> Store {
        let store = Store::open_in_memory().unwrap();
        for (id, name, slug) in rows {
            let mut def = test_agent_def(id, name, "claude", "agent", 1, "");
            def.slug = slug.to_string();
            store.agent_def_insert(&mut def).unwrap();
        }
        store
    }

    fn queue_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE db_work_queue (id TEXT PRIMARY KEY, target_agent TEXT NOT NULL DEFAULT '',
             target_agent_uid TEXT NOT NULL DEFAULT '', state TEXT NOT NULL DEFAULT 'open');
             INSERT INTO db_work_queue (id, target_agent, state) VALUES
               ('w-unique', 'agenty', 'open'),
               ('w-display', 'AgentY', 'open'),
               ('w-ambiguous', 'agentz', 'open'),
               ('w-done', 'agenty', 'done'),
               ('w-unknown', 'nobody', 'open'),
               ('w-untargeted', '', 'open');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn backfills_unique_names_and_leaves_ambiguous_unknown_and_finished_rows_alone() {
        let mstore = mstore_with(&[
            ("uid-y", "AgentY", "agenty"),
            ("uid-z1", "AgentZ", "agentz"),
            ("uid-z2", "agentz", "agentz-2"),
        ]);
        let conn = queue_conn();
        let (done, skipped) = backfill_table(
            &conn,
            &mstore,
            "db_work_queue",
            "target_agent",
            "target_agent_uid",
            "state IN ('open','claimed')",
        )
        .unwrap();
        assert_eq!(
            (done, skipped),
            (2, 2),
            "unique slug + unique display name; ambiguous + unknown skipped"
        );
        let uid_of = |id: &str| -> String {
            conn.query_row(
                "SELECT target_agent_uid FROM db_work_queue WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(uid_of("w-unique"), "uid-y");
        assert_eq!(uid_of("w-display"), "uid-y");
        assert_eq!(
            uid_of("w-ambiguous"),
            "",
            "two rows match 'agentz' (slug of one, name of another): never guessed"
        );
        assert_eq!(uid_of("w-unknown"), "");
        assert_eq!(uid_of("w-done"), "", "finished rows are not touched");
        assert_eq!(uid_of("w-untargeted"), "");
    }

    #[test]
    fn running_twice_is_a_no_op() {
        let mstore = mstore_with(&[("uid-y", "AgentY", "agenty")]);
        let conn = queue_conn();
        let first = backfill_table(
            &conn,
            &mstore,
            "db_work_queue",
            "target_agent",
            "target_agent_uid",
            "1=1",
        )
        .unwrap();
        let second = backfill_table(
            &conn,
            &mstore,
            "db_work_queue",
            "target_agent",
            "target_agent_uid",
            "1=1",
        )
        .unwrap();
        assert_eq!(first.0, 3);
        assert_eq!(
            second,
            (0, 2),
            "already-filled rows are not selected again; the two misses stay misses"
        );
    }
}
