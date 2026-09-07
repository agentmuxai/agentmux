// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Backfill `db_agents.working_directory` with the workspace each agent
//! actually ran in, taken from its latest legacy `db_agent_instances` launch.
//!
//! Agent-concept consolidation, Phase 3b
//! (`SPEC_AGENT_ARCHITECTURE_2026_05_27.md`). Phase 3a's dual-write stored
//! the DEFINITION's configured cwd on the consolidated row, on the reasoning
//! that "db_agents holds durable agent config; the per-launch resolved cwd
//! lives on the block". The launch row held the real one — `agent.open`
//! resolves it, including slug-collision suffixing — so on an upgraded store
//! the two disagree for every agent whose workspace was suffixed or
//! overridden.
//!
//! That disagreement was harmless while readers still joined
//! `db_agent_instances`. It stops being harmless the moment they don't
//! (Phase 3b PR 2, this release): continuing such an agent would open the
//! template's default directory instead of the workspace holding its files,
//! and the cross-version JSON registry mirror silently skips any row whose
//! `working_directory` is not under the agents root. Codex P1 on #3080.
//!
//! Why a separate migration rather than a wider `m0025`: 0025 shipped in
//! v0.55.39 and is already applied on upgraded installs, so amending it
//! would fix nobody who most needs it. This runs for everyone, whether or
//! not 0025 ran.
//!
//! Rule: for each projection key (the `db_agents` row an instance chain maps
//! to) take the most recently created instance in the chain and copy its
//! non-empty `working_directory` onto the row. Frozen copy of the same v29
//! projection rule `m0025` uses — deliberately a copy, per that module's
//! "freeze copies of live logic" policy: the live helper is deleted when the
//! legacy table is dropped, and this must keep producing the same result on
//! every machine it ever runs on.
//!
//! Tolerates a store that has already dropped `db_agent_instances`
//! (Phase 3c): nothing to backfill, `Ok`.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection};

use crate::backend::storage::store::Store;

use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0027AgentsWorkspaceBackfill;

/// The legacy instance columns this backfill reads.
struct LegacyLaunch {
    id: String,
    parent_instance_id: String,
    definition_id: String,
    working_directory: String,
    created_at: i64,
}

/// Frozen copy of the v29 projection rule (see `m0025`'s own copy): walk
/// `parent_instance_id` to the chain root; if the root's definition is a
/// user clone (`is_seeded = 0`) the key is that definition's id, otherwise
/// it is the root launch's own id. A missing definition counts as seeded.
fn projection_key(
    launch: &LegacyLaunch,
    by_id: &HashMap<String, &LegacyLaunch>,
    is_seeded: &HashMap<String, i64>,
) -> String {
    let mut root = launch;
    let mut seen: HashSet<&str> = HashSet::new();
    while !root.parent_instance_id.is_empty() && seen.insert(root.id.as_str()) {
        match by_id.get(&root.parent_instance_id) {
            Some(parent) => root = parent,
            None => break,
        }
    }
    let seeded = is_seeded.get(&root.definition_id).copied().unwrap_or(1);
    if seeded == 0 {
        root.definition_id.clone()
    } else {
        root.id.clone()
    }
}

/// Rows whose workspace was corrected.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct WorkspaceStats {
    pub updated: usize,
}

pub(super) fn backfill_workspaces(conn: &Connection) -> Result<WorkspaceStats, String> {
    let has_legacy: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'db_agent_instances')",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("probe db_agent_instances: {e}"))?;
    if !has_legacy {
        return Ok(WorkspaceStats::default());
    }

    let mut stmt = conn
        .prepare(
            "SELECT id, parent_instance_id, definition_id, working_directory, created_at
             FROM db_agent_instances",
        )
        .map_err(|e| format!("read db_agent_instances: {e}"))?;
    let launches: Vec<LegacyLaunch> = stmt
        .query_map([], |row| {
            Ok(LegacyLaunch {
                id: row.get(0)?,
                parent_instance_id: row.get(1)?,
                definition_id: row.get(2)?,
                working_directory: row.get(3)?,
                created_at: row.get(4)?,
            })
        })
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(|e| format!("read db_agent_instances: {e}"))?;
    drop(stmt);
    if launches.is_empty() {
        return Ok(WorkspaceStats::default());
    }

    let mut seeded_stmt = conn
        .prepare("SELECT id, is_seeded FROM db_agent_definitions")
        .map_err(|e| format!("read db_agent_definitions: {e}"))?;
    let is_seeded: HashMap<String, i64> = seeded_stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
        .and_then(|rows| rows.collect::<Result<HashMap<_, _>, _>>())
        .map_err(|e| format!("read db_agent_definitions: {e}"))?;
    drop(seeded_stmt);

    let by_id: HashMap<String, &LegacyLaunch> = launches.iter().map(|l| (l.id.clone(), l)).collect();
    // Latest launch per projection key — `created_at`, then id, so the choice
    // is deterministic when two rows share a timestamp. Same tie-break as
    // `m0025`, so both migrations pick the same launch.
    let mut latest: HashMap<String, &LegacyLaunch> = HashMap::new();
    for launch in &launches {
        let key = projection_key(launch, &by_id, &is_seeded);
        let replace = match latest.get(&key) {
            None => true,
            Some(cur) => (launch.created_at, launch.id.as_str()) > (cur.created_at, cur.id.as_str()),
        };
        if replace {
            latest.insert(key, launch);
        }
    }

    let mut stats = WorkspaceStats::default();
    for (key, launch) in latest {
        // An empty legacy value carries no information — the row keeps
        // whatever it has rather than being blanked.
        if launch.working_directory.is_empty() {
            continue;
        }
        let n = conn
            .execute(
                "UPDATE db_agents SET working_directory = ?2
                 WHERE id = ?1 AND is_template = 0 AND working_directory != ?2",
                params![key, launch.working_directory],
            )
            .map_err(|e| format!("update db_agents {key}: {e}"))?;
        stats.updated += n;
    }
    Ok(stats)
}

impl Migration for M0027AgentsWorkspaceBackfill {
    fn id(&self) -> &'static str { "0027_agents_workspace_backfill" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str {
        "Backfill db_agents.working_directory from the latest legacy launch's resolved workspace"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        // Store::open lays down the schema before the raw connection below
        // touches the table.
        drop(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("agents_workspace_backfill: open wstore: {e}")))?,
        );
        let conn = Connection::open(&ctx.channel_store_path)
            .map_err(|e| MigrationError(format!("agents_workspace_backfill: open: {e}")))?;
        let stats = backfill_workspaces(&conn)
            .map_err(|e| MigrationError(format!("agents_workspace_backfill: {e}")))?;
        tracing::info!(
            updated = stats.updated,
            "m0027_agents_workspace_backfill: complete"
        );
        Ok(())
    }

    // No `verify()`: the post-condition ("every consolidated row carries its
    // latest launch's workspace") needs the legacy table to state, and that
    // table is what Phase 3c removes — a check that can only run before its
    // own precondition disappears would report NotVerifiable forever after.
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Legacy-shaped fixture: the tables as they exist on a Phase 3a store,
    /// with `db_agents` already carrying the DEFINITION's cwd (what the
    /// Phase 3a dual-write wrote) rather than the launch's resolved one.
    fn legacy_store() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE db_agent_definitions (
                id TEXT PRIMARY KEY, is_seeded INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE db_agent_instances (
                id TEXT PRIMARY KEY,
                parent_instance_id TEXT NOT NULL DEFAULT '',
                definition_id TEXT NOT NULL,
                working_directory TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE db_agents (
                id TEXT PRIMARY KEY,
                is_template INTEGER NOT NULL DEFAULT 0,
                working_directory TEXT NOT NULL DEFAULT ''
             );",
        )
        .unwrap();
        conn
    }

    fn workdir(conn: &Connection, id: &str) -> String {
        conn.query_row("SELECT working_directory FROM db_agents WHERE id = ?1", params![id], |r| {
            r.get(0)
        })
        .unwrap()
    }

    /// The case codex P1 named: a template launch whose resolved workspace
    /// was suffixed for a slug collision. The consolidated row holds the
    /// template's configured cwd until this pass corrects it — otherwise
    /// continuing the agent after the read flip opens the wrong directory.
    #[test]
    fn a_template_launch_row_takes_the_launchs_resolved_workspace() {
        let conn = legacy_store();
        conn.execute_batch(
            "INSERT INTO db_agent_definitions (id, is_seeded) VALUES ('tpl', 1);
             INSERT INTO db_agent_instances (id, definition_id, working_directory, created_at)
                VALUES ('launch-1', 'tpl', '/agents/coder-2', 100);
             INSERT INTO db_agents (id, is_template, working_directory)
                VALUES ('tpl', 1, '/agents/coder'),
                       ('launch-1', 0, '/agents/coder');",
        )
        .unwrap();

        assert_eq!(backfill_workspaces(&conn).unwrap(), WorkspaceStats { updated: 1 });
        assert_eq!(workdir(&conn, "launch-1"), "/agents/coder-2");
        assert_eq!(workdir(&conn, "tpl"), "/agents/coder", "a template's own cwd is not a launch's");
    }

    /// A user agent's chain folds onto the DEFINITION's id, and the newest
    /// launch in that chain is the one whose workspace the agent is really
    /// using.
    #[test]
    fn a_user_agents_row_takes_its_newest_launchs_workspace() {
        let conn = legacy_store();
        conn.execute_batch(
            "INSERT INTO db_agent_definitions (id, is_seeded) VALUES ('agent-a', 0);
             INSERT INTO db_agent_instances (id, parent_instance_id, definition_id, working_directory, created_at)
                VALUES ('l1', '', 'agent-a', '/agents/old', 100),
                       ('l2', 'l1', 'agent-a', '/agents/new', 200);
             INSERT INTO db_agents (id, is_template, working_directory)
                VALUES ('agent-a', 0, '/agents/configured');",
        )
        .unwrap();

        assert_eq!(backfill_workspaces(&conn).unwrap(), WorkspaceStats { updated: 1 });
        assert_eq!(workdir(&conn, "agent-a"), "/agents/new");
    }

    /// Idempotent, and never blanks a row: a second run writes nothing, and
    /// a launch with no recorded workspace leaves the row's value alone.
    #[test]
    fn a_second_run_and_an_empty_legacy_workspace_both_write_nothing() {
        let conn = legacy_store();
        conn.execute_batch(
            "INSERT INTO db_agent_definitions (id, is_seeded) VALUES ('tpl', 1);
             INSERT INTO db_agent_instances (id, definition_id, working_directory, created_at)
                VALUES ('launch-1', 'tpl', '/agents/real', 100),
                       ('launch-2', 'tpl', '', 100);
             INSERT INTO db_agents (id, is_template, working_directory)
                VALUES ('launch-1', 0, '/agents/cfg'),
                       ('launch-2', 0, '/agents/kept');",
        )
        .unwrap();

        assert_eq!(backfill_workspaces(&conn).unwrap(), WorkspaceStats { updated: 1 });
        assert_eq!(workdir(&conn, "launch-2"), "/agents/kept");
        assert_eq!(
            backfill_workspaces(&conn).unwrap(),
            WorkspaceStats::default(),
            "re-running must be a no-op"
        );
    }

    /// A Phase 3c store has no legacy table at all — nothing to read, and
    /// no error.
    #[test]
    fn a_store_without_the_legacy_table_is_a_no_op() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE db_agents (id TEXT PRIMARY KEY);").unwrap();
        assert_eq!(backfill_workspaces(&conn).unwrap(), WorkspaceStats::default());
    }
}
