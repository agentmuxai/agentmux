// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Narrow the global skill uniqueness index from `(name, skill_type)` to
//! `name` alone, matching the invariant `skill_upsert_unique_global`'s own
//! dup-check has always enforced — with a defensive dedup pass for any
//! existing same-name/different-type pair the OLD, wider index let through.
//!
//! Part B of Phase 2's application-layer redirect,
//! `SPEC_DURABLE_BINDINGS_2026_09_10.md` §5.4 (Codex P2, PR #3182).
//! `idx_ids_skills_global_name_type` (`IDENTITY_STORE_SCHEMA_VERSION` v7) is
//! narrower than what the application actually promises ("a global skill's
//! name is unique, full stop, regardless of type" — see
//! `managed_upsert_unique_global`'s error message). MCP servers don't have
//! this mismatch (`idx_ids_mcp_servers_global_name` is already name-only) —
//! this migration is skills-specific.
//!
//! Because `m0031_carry_skills_and_mcp_servers_to_identity_store`'s own dedup
//! key is `(name, skill_type)`, a store that already ran that migration (in
//! an earlier release) may already hold two global `db_skills` rows sharing
//! a name with different types — exactly the pair the OLD index allowed and
//! the NEW one would reject outright. This migration detects any such
//! group (not assuming exactly two), collapses to the earliest-`created_at`
//! row (ties broken by id), rewrites refs, deletes the loser(s), and only
//! THEN creates the new index — in that order, since creating a UNIQUE
//! index over still-colliding data would fail.
//!
//! ## Cross-channel ref cleanup is intentionally partial
//!
//! `db_agent_skills_ref`/`db_bundle_skills_ref` live in EACH CHANNEL's own
//! `wstore`, not in the identity store this migration (Global-scoped) opens.
//! A single global migration run cannot reach every channel's ref rows the
//! way `m0022_identity_store_links_backfill` reads multiple sources —
//! reaching in and WRITING to a sibling channel's live `objects.db` from a
//! process that isn't running that channel is the same unnecessary risk
//! `m0031`'s own doc comment already declines to take. So this migration
//! rewrites only the CURRENT channel's refs (the one whose `objects.db` is
//! in `MigrationContext`); any other channel's ref rows still pointing at a
//! collapsed-away loser id are left dangling until that channel's own next
//! boot runs this same migration and would have to reconcile them itself —
//! except this migration is Global-scoped and runs exactly ONCE across every
//! channel, so a sibling channel never gets a second pass. This mirrors the
//! already-accepted "cross-channel dangling refs on catalog delete" gap
//! §5.4 documents elsewhere (a dangling ref degrades gracefully: `skill_get`
//! against a superseded id returns `None`, which every read path already
//! handles as "this bound skill no longer exists"), not a new regression —
//! but it is a real, acknowledged incompleteness worth a second look.

use rusqlite::Connection;

use crate::backend::storage::store::Store;
use crate::registry;

use super::{Migration, MigrationContext, MigrationError, MigrationScope, VerifyOutcome};

pub struct M0033NarrowSkillGlobalUniquenessIndex;

/// One global skill row's identity, for grouping/collapsing.
struct GlobalSkillRow {
    id: String,
    name: String,
    created_at: i64,
}

/// Collapse every group of global `db_skills` rows sharing a `name` (however
/// large) to the earliest-`created_at` row (ties broken by `id`, for
/// determinism), rewriting `channel_wstore`'s own ref rows (if given) and
/// deleting the loser(s) from `identity_conn`'s `db_skills` — then swaps the
/// uniqueness index. Returns the number of rows collapsed away.
///
/// Takes explicit connections/stores rather than resolving paths itself so
/// it can be exercised directly in tests without depending on
/// `registry::resolve_identity_store_path`'s env-var-driven resolution (the
/// same reason `m0031`'s `carry_skills`/`carry_mcp_servers` take explicit
/// `Store` params instead of opening their own).
fn collapse_and_reindex(identity_conn: &Connection, channel_wstore: Option<&Store>) -> Result<usize, String> {
    let mut stmt = identity_conn
        .prepare("SELECT id, name, created_at FROM db_skills WHERE is_global = 1 ORDER BY name, created_at ASC, id ASC")
        .map_err(|e| format!("prepare global skills scan: {e}"))?;
    let rows: Vec<GlobalSkillRow> = stmt
        .query_map([], |row| {
            Ok(GlobalSkillRow {
                id: row.get(0)?,
                name: row.get(1)?,
                created_at: row.get(2)?,
            })
        })
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(|e| format!("read global skills: {e}"))?;
    drop(stmt);

    // Group by name, preserving the query's own (name, created_at, id)
    // order within each group — first element is already the survivor.
    let mut groups: std::collections::BTreeMap<String, Vec<GlobalSkillRow>> = std::collections::BTreeMap::new();
    for row in rows {
        groups.entry(row.name.clone()).or_default().push(row);
    }

    let mut collapsed = 0usize;
    for (_, mut members) in groups {
        if members.len() < 2 {
            continue;
        }
        // Deterministic survivor: earliest created_at, ties broken by id.
        // The SELECT's ORDER BY already sorted this way, but sort again
        // explicitly so this function's correctness doesn't depend on the
        // query staying in sync with this comment.
        members.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)));
        let survivor_id = members[0].id.clone();
        for loser in &members[1..] {
            if let Some(wstore) = channel_wstore {
                wstore
                    .skill_rewrite_ref_id(&loser.id, &survivor_id)
                    .map_err(|e| format!("rewrite refs for collapsed skill {}: {e}", loser.id))?;
            }
            identity_conn
                .execute("DELETE FROM db_skills WHERE id = ?1", [&loser.id])
                .map_err(|e| format!("delete collapsed skill {}: {e}", loser.id))?;
            collapsed += 1;
        }
    }

    // Only safe to create the narrower index AFTER every collision has been
    // collapsed above — a UNIQUE index over still-colliding data fails.
    identity_conn
        .execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_ids_skills_global_name
                ON db_skills(name) WHERE is_global = 1;
             DROP INDEX IF EXISTS idx_ids_skills_global_name_type;",
        )
        .map_err(|e| format!("swap uniqueness index: {e}"))?;

    Ok(collapsed)
}

impl Migration for M0033NarrowSkillGlobalUniquenessIndex {
    fn id(&self) -> &'static str {
        "0033_narrow_skill_global_uniqueness_index"
    }

    fn scope(&self) -> MigrationScope {
        MigrationScope::Global
    }

    fn description(&self) -> &'static str {
        "Collapse any global skill rows sharing a name across skill_type, then narrow the global-uniqueness index to name alone"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        let Some(path) = registry::resolve_identity_store_path() else {
            // No resolvable identity store (CI / unusual env, same
            // treat-as-no-op posture as m0031's own resolution failures).
            return Ok(());
        };
        if !path.exists() {
            // Nothing carried yet — a fresh store gets the new index
            // directly from run_identity_store_schema, no dedup needed.
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| MigrationError(format!("narrow_skill_global_uniqueness_index: create dir: {e}")))?;
        }
        let conn = Connection::open(&path)
            .map_err(|e| MigrationError(format!("narrow_skill_global_uniqueness_index: open identity store: {e}")))?;
        let has_table: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'db_skills')",
                [],
                |row| row.get(0),
            )
            .map_err(|e| MigrationError(format!("narrow_skill_global_uniqueness_index: probe db_skills: {e}")))?;
        if !has_table {
            // Older-than-v6 store: db_skills doesn't exist yet. Nothing to
            // dedup or reindex; a later boot creates it fresh (new index
            // included) once it's actually populated.
            return Ok(());
        }

        let channel_wstore = if ctx.channel_store_path.exists() {
            Some(
                Store::open(&ctx.channel_store_path)
                    .map_err(|e| MigrationError(format!("narrow_skill_global_uniqueness_index: open wstore: {e}")))?,
            )
        } else {
            None
        };

        let collapsed = collapse_and_reindex(&conn, channel_wstore.as_ref())
            .map_err(|e| MigrationError(format!("narrow_skill_global_uniqueness_index: {e}")))?;
        tracing::info!(collapsed, "m0033_narrow_skill_global_uniqueness_index: complete");
        Ok(())
    }

    fn verify(&self, _ctx: &MigrationContext) -> VerifyOutcome {
        let Some(path) = registry::resolve_identity_store_path() else {
            return VerifyOutcome::Ok("identity store path unresolved".to_string());
        };
        let conn = match super::runner::open_readonly(&path) {
            Ok(Some(c)) => c,
            Ok(None) => return VerifyOutcome::Ok("identity store not yet created".to_string()),
            Err(e) => return VerifyOutcome::Error(e),
        };
        let dupes: i64 = match conn.query_row(
            "SELECT COUNT(*) FROM (
                SELECT name FROM db_skills WHERE is_global = 1 GROUP BY name HAVING COUNT(*) > 1
             )",
            [],
            |row| row.get(0),
        ) {
            Ok(n) => n,
            Err(rusqlite::Error::SqliteFailure(_, Some(msg))) if msg.contains("no such table") => 0,
            Err(e) => return VerifyOutcome::Error(format!("count duplicate global skill names: {e}")),
        };
        if dupes > 0 {
            return VerifyOutcome::Mismatch(format!("{dupes} global skill name(s) still have more than one row"));
        }
        VerifyOutcome::Ok("no duplicate global skill names remain".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pre-v8 identity store shape: `db_skills` with the OLD
    /// `(name, skill_type)` index, not the new name-only one — the state a
    /// real upgrading store is in before this migration runs. Deliberately
    /// NOT built via `Store::open_identity_store` (which would already lay
    /// down the NEW index from this binary's own `run_identity_store_schema`
    /// and refuse to create a colliding fixture in the first place).
    fn legacy_identity_conn() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity-store.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE db_skills (
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
             CREATE UNIQUE INDEX idx_ids_skills_global_name_type
                ON db_skills(name, skill_type) WHERE is_global = 1;",
        )
        .unwrap();
        (dir, conn)
    }

    fn insert_skill(conn: &Connection, id: &str, name: &str, skill_type: &str, created_at: i64) {
        conn.execute(
            "INSERT INTO db_skills (id, name, skill_type, is_global, created_at, updated_at)
             VALUES (?1, ?2, ?3, 1, ?4, ?4)",
            rusqlite::params![id, name, skill_type, created_at],
        )
        .unwrap();
    }

    #[test]
    fn two_colliding_global_skills_are_collapsed_to_the_earliest_created_at_row() {
        let (_dir, conn) = legacy_identity_conn();
        insert_skill(&conn, "newer", "Deploy", "prompt", 200);
        insert_skill(&conn, "older", "Deploy", "agent-skill", 100);

        let collapsed = collapse_and_reindex(&conn, None).unwrap();
        assert_eq!(collapsed, 1);

        let remaining: Vec<String> = {
            let mut stmt = conn.prepare("SELECT id FROM db_skills").unwrap();
            stmt.query_map([], |r| r.get::<_, String>(0)).unwrap().map(|r| r.unwrap()).collect()
        };
        assert_eq!(remaining, vec!["older".to_string()], "the earliest created_at row must survive");
    }

    #[test]
    fn a_larger_group_of_collisions_collapses_to_one_survivor() {
        let (_dir, conn) = legacy_identity_conn();
        insert_skill(&conn, "c", "Deploy", "prompt", 300);
        insert_skill(&conn, "a", "Deploy", "agent-skill", 100);
        insert_skill(&conn, "b", "Deploy", "shell", 200);

        let collapsed = collapse_and_reindex(&conn, None).unwrap();
        assert_eq!(collapsed, 2, "must not assume exactly two colliding rows");

        let remaining: i64 = conn.query_row("SELECT COUNT(*) FROM db_skills", [], |r| r.get(0)).unwrap();
        assert_eq!(remaining, 1);
        let survivor: String = conn.query_row("SELECT id FROM db_skills", [], |r| r.get(0)).unwrap();
        assert_eq!(survivor, "a");
    }

    #[test]
    fn ties_in_created_at_are_broken_deterministically_by_id() {
        let (_dir, conn) = legacy_identity_conn();
        insert_skill(&conn, "zzz", "Deploy", "prompt", 100);
        insert_skill(&conn, "aaa", "Deploy", "agent-skill", 100);

        collapse_and_reindex(&conn, None).unwrap();
        let survivor: String = conn.query_row("SELECT id FROM db_skills", [], |r| r.get(0)).unwrap();
        assert_eq!(survivor, "aaa", "the lexicographically smaller id must win a created_at tie");
    }

    #[test]
    fn the_current_channels_ref_rows_are_rewritten_to_the_survivor() {
        let (_dir, conn) = legacy_identity_conn();
        insert_skill(&conn, "loser", "Deploy", "prompt", 200);
        insert_skill(&conn, "winner", "Deploy", "agent-skill", 100);

        let wstore = Store::open_in_memory().unwrap();
        // Insert the ref row directly rather than through `skill_bind` — the
        // catalog row lives only in the raw identity `conn` above (not
        // wrapped as a `Store`), and `skill_bind` now checks catalog
        // existence (Part C of SPEC_DURABLE_BINDINGS_2026_09_10.md). This
        // test is only exercising `skill_rewrite_ref_id`'s effect on the ref
        // table, same as m0031's own tests seed ref rows directly.
        {
            let raw = wstore.conn().lock().unwrap();
            raw.execute("INSERT INTO db_agents (id, name, provider) VALUES ('agent-1', 'A', 'claude')", [])
                .unwrap();
            raw.execute("INSERT INTO db_agent_skills_ref (agent_id, skill_id) VALUES ('agent-1', 'loser')", [])
                .unwrap();
        }

        collapse_and_reindex(&conn, Some(&wstore)).unwrap();

        assert!(wstore.skill_is_bound_to("agent-1", "winner").unwrap(), "the ref must now point at the survivor");
        assert!(!wstore.skill_is_bound_to("agent-1", "loser").unwrap());
    }

    #[test]
    fn the_new_index_rejects_a_fresh_same_name_insert_after_the_swap() {
        let (_dir, conn) = legacy_identity_conn();
        insert_skill(&conn, "existing", "Deploy", "prompt", 100);

        collapse_and_reindex(&conn, None).unwrap();

        let result = conn.execute(
            "INSERT INTO db_skills (id, name, skill_type, is_global, created_at, updated_at)
             VALUES ('new-one', 'Deploy', 'agent-skill', 1, 200, 200)",
            [],
        );
        assert!(
            result.is_err(),
            "the new name-only index must reject a same-name insert even with a different skill_type"
        );
    }

    #[test]
    fn a_store_with_no_collisions_is_a_clean_no_op() {
        let (_dir, conn) = legacy_identity_conn();
        insert_skill(&conn, "s1", "Deploy", "prompt", 100);
        insert_skill(&conn, "s2", "Review", "prompt", 100);

        let collapsed = collapse_and_reindex(&conn, None).unwrap();
        assert_eq!(collapsed, 0);

        let remaining: i64 = conn.query_row("SELECT COUNT(*) FROM db_skills", [], |r| r.get(0)).unwrap();
        assert_eq!(remaining, 2, "no-collision rows must be untouched");

        // The index swap must still have happened even with nothing to
        // collapse — a fresh-enough store still needs the new index.
        let has_new_index: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name='idx_ids_skills_global_name')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(has_new_index);
    }

    // ── up()-level tests: these resolve a REAL identity store path via
    // registry::resolve_identity_store_path(), so — same as
    // m0022_identity_store_links_backfill's test module — they must pin
    // AGENTMUX_HOME_OVERRIDE to an isolated temp dir under the shared
    // process-wide env lock, never run against whatever the test process's
    // ambient environment happens to resolve to.
    use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_LOCK;

    fn clear_env() {
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        std::env::remove_var("AGENTMUX_SHARED_DIR");
        std::env::remove_var("AGENTMUX_ISOLATED_AUTH");
        std::env::remove_var("AGENTMUX_CHANNEL");
        std::env::remove_var("AGENTMUX_INSTANCE_DIR");
    }

    #[test]
    fn a_non_existent_identity_store_path_is_a_no_op() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", tmp.path());

        let ctx = MigrationContext {
            home: tmp.path().to_path_buf(),
            data_dir: tmp.path().join("data"),
            shared_store_path: tmp.path().join("shared").join("store.db"),
            channel_store_path: tmp.path().join("data").join("db").join("does-not-exist.db"),
        };
        // Nothing has ever created identity-store.db under this fresh home.
        assert!(M0033NarrowSkillGlobalUniquenessIndex.up(&ctx).is_ok());
        clear_env();
    }

    /// End-to-end through `up()` itself: a real identity store (pre-v8
    /// shape, built the same way `legacy_identity_conn` does but at the
    /// resolved production path) with a real colliding pair, plus a real
    /// current-channel `wstore` holding a ref row pointing at the loser —
    /// after `up()`, the identity store has one row, the ref points at the
    /// survivor, and the new index is in place.
    #[test]
    fn up_end_to_end_collapses_rewrites_refs_and_swaps_the_index() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", tmp.path());

        let identity_path = registry::resolve_identity_store_path().unwrap();
        std::fs::create_dir_all(identity_path.parent().unwrap()).unwrap();
        {
            let conn = Connection::open(&identity_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE db_skills (
                    id TEXT PRIMARY KEY, name TEXT NOT NULL, trigger TEXT NOT NULL DEFAULT '',
                    skill_type TEXT NOT NULL DEFAULT 'prompt', description TEXT NOT NULL DEFAULT '',
                    content TEXT NOT NULL DEFAULT '', is_global INTEGER NOT NULL DEFAULT 0,
                    created_at INTEGER NOT NULL DEFAULT 0, updated_at INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE UNIQUE INDEX idx_ids_skills_global_name_type
                    ON db_skills(name, skill_type) WHERE is_global = 1;",
            )
            .unwrap();
            insert_skill(&conn, "loser", "Deploy", "prompt", 200);
            insert_skill(&conn, "winner", "Deploy", "agent-skill", 100);
        }

        let channel_store_path = tmp.path().join("data").join("db").join("objects.db");
        std::fs::create_dir_all(channel_store_path.parent().unwrap()).unwrap();
        {
            let wstore = Store::open(&channel_store_path).unwrap();
            // Direct ref-row insert, not `skill_bind` — see the identical
            // note on `the_current_channels_ref_rows_are_rewritten_to_the_survivor`
            // above.
            let raw = wstore.conn().lock().unwrap();
            raw.execute("INSERT INTO db_agents (id, name, provider) VALUES ('agent-1', 'A', 'claude')", [])
                .unwrap();
            raw.execute("INSERT INTO db_agent_skills_ref (agent_id, skill_id) VALUES ('agent-1', 'loser')", [])
                .unwrap();
        }

        let ctx = MigrationContext {
            home: tmp.path().to_path_buf(),
            data_dir: tmp.path().join("data"),
            shared_store_path: tmp.path().join("shared").join("store.db"),
            channel_store_path,
        };
        M0033NarrowSkillGlobalUniquenessIndex.up(&ctx).unwrap();

        let identity_conn = Connection::open(&identity_path).unwrap();
        let remaining: i64 = identity_conn.query_row("SELECT COUNT(*) FROM db_skills", [], |r| r.get(0)).unwrap();
        assert_eq!(remaining, 1, "the loser row must be gone from the identity store");
        let survivor: String = identity_conn.query_row("SELECT id FROM db_skills", [], |r| r.get(0)).unwrap();
        assert_eq!(survivor, "winner");

        let wstore = Store::open(&ctx.channel_store_path).unwrap();
        assert!(wstore.skill_is_bound_to("agent-1", "winner").unwrap(), "the ref must now point at the survivor");
        assert!(!wstore.skill_is_bound_to("agent-1", "loser").unwrap());

        assert!(
            matches!(M0033NarrowSkillGlobalUniquenessIndex.verify(&ctx), VerifyOutcome::Ok(_)),
            "no duplicate names remain, so verify() must pass"
        );

        clear_env();
    }
}
