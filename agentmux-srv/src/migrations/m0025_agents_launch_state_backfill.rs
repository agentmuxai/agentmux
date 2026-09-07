// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Backfill `db_agents`' latest-launch state (schema v29:
//! `session_id`, `status`, `started_at`, `ended_at`, and `last_block_id`
//! where still empty) from the legacy `db_agent_instances` rows.
//!
//! Agent-concept consolidation, Phase 3b
//! (`SPEC_AGENT_ARCHITECTURE_2026_05_27.md`): these four columns are what
//! kept the legacy table alive — the recent-sessions picker, the pane
//! close/reopen continuity write-back and the status-filtered instance
//! list all read them from `db_agent_instances`. Once every consolidated
//! row carries its own launch state, those reads can flip and the table
//! can go.
//!
//! Rule: for each projection key (the `db_agents` row an instance chain
//! maps to — the chain head's own id for a template launch, the user-clone
//! definition's id for a launch of a user agent) take the most recently
//! created instance in the chain and copy its launch state, but only into
//! a row whose launch state is still entirely default. That makes the
//! migration idempotent and means it never overwrites state the dual-write
//! has written since v29 shipped. Channel-scoped: each channel's
//! `objects.db` has its own rows.
//!
//! Frozen copy of `dual_write.rs::agents_projection_key_for_inst`'s rule
//! (as of v29) — deliberately not a call into it, per this module's "freeze
//! copies of live logic" doc: `dual_write.rs` is deleted when the legacy
//! table is dropped, and this migration must keep producing the same rows
//! on every machine it ever runs on.
//!
//! Tolerates a store that has already dropped `db_agent_instances`
//! (Phase 3c): nothing to backfill, `Ok`.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection};

use crate::backend::storage::store::Store;

use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0025AgentsLaunchStateBackfill;

/// The legacy instance columns the backfill reads.
struct LegacyInstance {
    id: String,
    parent_instance_id: String,
    definition_id: String,
    block_id: String,
    session_id: String,
    status: String,
    started_at: i64,
    ended_at: i64,
    created_at: i64,
}

/// Frozen copy of the v29 projection rule: walk `parent_instance_id` to the
/// chain root; if the root's definition is a user-clone (`is_seeded = 0`)
/// the key is that definition's id (the launch was folded into the def's
/// row), otherwise it is the root instance's own id (a template launch is
/// its own row). A missing definition counts as seeded, matching the
/// `COALESCE(d.is_seeded, 1)` in the live helper. A chain whose parent
/// pointer no longer resolves is rooted at the last instance that does.
fn projection_key(
    inst: &LegacyInstance,
    by_id: &HashMap<String, &LegacyInstance>,
    is_seeded: &HashMap<String, i64>,
) -> String {
    let mut root = inst;
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

/// The backfill proper, on an open connection. `Ok(n)` = rows updated.
pub(super) fn backfill_launch_state(conn: &Connection) -> Result<usize, String> {
    let has_legacy: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'db_agent_instances')",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("probe db_agent_instances: {e}"))?;
    if !has_legacy {
        return Ok(0);
    }

    let mut stmt = conn
        .prepare(
            "SELECT id, parent_instance_id, definition_id, block_id, session_id, status,
                    started_at, ended_at, created_at
             FROM db_agent_instances",
        )
        .map_err(|e| format!("read db_agent_instances: {e}"))?;
    let instances: Vec<LegacyInstance> = stmt
        .query_map([], |row| {
            Ok(LegacyInstance {
                id: row.get(0)?,
                parent_instance_id: row.get(1)?,
                definition_id: row.get(2)?,
                block_id: row.get(3)?,
                session_id: row.get(4)?,
                status: row.get(5)?,
                started_at: row.get(6)?,
                ended_at: row.get(7)?,
                created_at: row.get(8)?,
            })
        })
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(|e| format!("read db_agent_instances: {e}"))?;
    if instances.is_empty() {
        return Ok(0);
    }

    let mut seeded_stmt = conn
        .prepare("SELECT id, is_seeded FROM db_agent_definitions")
        .map_err(|e| format!("read db_agent_definitions: {e}"))?;
    let is_seeded: HashMap<String, i64> = seeded_stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
        .and_then(|rows| rows.collect::<Result<HashMap<_, _>, _>>())
        .map_err(|e| format!("read db_agent_definitions: {e}"))?;

    let by_id: HashMap<String, &LegacyInstance> = instances.iter().map(|i| (i.id.clone(), i)).collect();
    // Latest instance per projection key — `created_at`, then id, so the
    // choice is deterministic when two rows share a timestamp.
    let mut latest: HashMap<String, &LegacyInstance> = HashMap::new();
    for inst in &instances {
        let key = projection_key(inst, &by_id, &is_seeded);
        let replace = match latest.get(&key) {
            None => true,
            Some(cur) => (inst.created_at, inst.id.as_str()) > (cur.created_at, cur.id.as_str()),
        };
        if replace {
            latest.insert(key, inst);
        }
    }

    let mut updated = 0usize;
    for (key, inst) in latest {
        let n = conn
            .execute(
                "UPDATE db_agents SET
                    session_id = ?2,
                    status = ?3,
                    started_at = ?4,
                    ended_at = ?5,
                    last_block_id = CASE WHEN last_block_id = '' THEN ?6 ELSE last_block_id END
                 WHERE id = ?1
                   AND is_template = 0
                   AND session_id = '' AND status = '' AND started_at = 0 AND ended_at = 0",
                params![key, inst.session_id, inst.status, inst.started_at, inst.ended_at, inst.block_id],
            )
            .map_err(|e| format!("update db_agents {key}: {e}"))?;
        updated += n;
    }
    Ok(updated)
}

impl Migration for M0025AgentsLaunchStateBackfill {
    fn id(&self) -> &'static str { "0025_agents_launch_state_backfill" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str {
        "Backfill db_agents launch state (session/status/started/ended) from db_agent_instances"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        // Store::open lays down schema v29 (the columns this writes) before
        // the raw connection below touches the table.
        drop(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("agents_launch_state_backfill: open wstore: {e}")))?,
        );
        let conn = Connection::open(&ctx.channel_store_path)
            .map_err(|e| MigrationError(format!("agents_launch_state_backfill: open: {e}")))?;
        let updated = backfill_launch_state(&conn)
            .map_err(|e| MigrationError(format!("agents_launch_state_backfill: {e}")))?;
        tracing::info!(updated, "m0025_agents_launch_state_backfill: complete");
        Ok(())
    }

    // No `verify()`: the post-condition ("every consolidated row whose
    // legacy chain had launch state now carries it") needs the legacy table
    // to state, and the legacy table is what Phase 3c removes — a check
    // that can only run before its own precondition disappears would report
    // `error` on every post-3c install. `NotVerifiable` is the honest
    // default here; the dual-write and the Phase 3b readers' own tests are
    // what pin the live behaviour.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::store::{AgentDefinition, AgentInstance};

    fn def(id: &str, is_seeded: i64, parent_id: &str) -> AgentDefinition {
        AgentDefinition {
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
            id: id.to_string(),
            slug: id.to_string(),
            name: id.to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: "bash".to_string(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1000,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded,
            accounts: String::new(),
            parent_id: parent_id.to_string(),
            branch_label: String::new(),
            updated_at: 1000,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
        }
    }

    fn inst(id: &str, def_id: &str, parent: &str, block: &str, session: &str, status: &str, created_at: i64) -> AgentInstance {
        AgentInstance {
            id: id.to_string(),
            definition_id: def_id.to_string(),
            parent_instance_id: parent.to_string(),
            block_id: block.to_string(),
            session_id: session.to_string(),
            status: status.to_string(),
            github_context: String::new(),
            started_at: created_at,
            ended_at: 0,
            created_at,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: id.to_string(),
            working_directory: String::new(),
            display_hidden: false,
        }
    }

    fn launch_state(conn: &Connection, id: &str) -> (String, String, i64, String) {
        conn.query_row(
            "SELECT session_id, status, started_at, last_block_id FROM db_agents WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap()
    }

    /// A pre-v29 store: legacy rows in place, db_agents rows present (the
    /// dual-write wrote them) but with the launch-state columns at their
    /// defaults — exactly what an upgraded install looks like the first time
    /// v29's ALTERs run.
    fn pre_v29_store() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("objects.db");
        let store = Store::open(&path).unwrap();
        let mut tpl = def("tpl", 1, "");
        store.agent_def_insert(&mut tpl).unwrap();
        let mut clone = def("clone", 0, "tpl");
        store.agent_def_insert(&mut clone).unwrap();
        // A template launch with one continuation: head → child.
        store.instance_create(&inst("head", "tpl", "", "b1", "s1", "stopped", 2000)).unwrap();
        store.instance_create(&inst("child", "tpl", "head", "b2", "s2", "running", 3000)).unwrap();
        // A launch of the user clone (folds into the clone's own row).
        store.instance_create(&inst("clone-run", "clone", "", "b3", "s3", "paused", 2500)).unwrap();
        drop(store);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("UPDATE db_agents SET session_id = '', status = '', started_at = 0, ended_at = 0, last_block_id = '';")
            .unwrap();
        (dir, path)
    }

    #[test]
    fn backfills_the_latest_launch_of_each_chain_into_its_projection_row() {
        let (_dir, path) = pre_v29_store();
        let conn = Connection::open(&path).unwrap();
        assert_eq!(backfill_launch_state(&conn).unwrap(), 2, "one template chain + one clone");
        // The template chain's head row carries the CHILD's (latest) state.
        assert_eq!(launch_state(&conn, "head"), ("s2".into(), "running".into(), 3000, "b2".into()));
        // The clone's own row carries its launch.
        assert_eq!(launch_state(&conn, "clone"), ("s3".into(), "paused".into(), 2500, "b3".into()));
        // The template row itself is untouched.
        assert_eq!(launch_state(&conn, "tpl"), (String::new(), String::new(), 0, String::new()));
    }

    #[test]
    fn is_idempotent_and_never_overwrites_state_written_since() {
        let (_dir, path) = pre_v29_store();
        let conn = Connection::open(&path).unwrap();
        // Live state already on the head row (the dual-write got there first).
        conn.execute("UPDATE db_agents SET session_id = 'live', status = 'running' WHERE id = 'head'", []).unwrap();
        assert_eq!(backfill_launch_state(&conn).unwrap(), 1, "only the still-default clone row");
        assert_eq!(launch_state(&conn, "head").0, "live");
        assert_eq!(backfill_launch_state(&conn).unwrap(), 0, "second run writes nothing");
    }

    #[test]
    fn tolerates_a_store_without_the_legacy_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("objects.db");
        drop(Store::open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("DROP TABLE db_agent_instances;").unwrap();
        assert_eq!(backfill_launch_state(&conn).unwrap(), 0);
    }

    #[test]
    fn up_runs_end_to_end_against_a_channel_store_and_skips_a_missing_one() {
        let (dir, path) = pre_v29_store();
        let ctx = MigrationContext {
            home: dir.path().to_path_buf(),
            data_dir: dir.path().to_path_buf(),
            shared_store_path: dir.path().join("shared.db"),
            channel_store_path: path.clone(),
        };
        M0025AgentsLaunchStateBackfill.up(&ctx).unwrap();
        let conn = Connection::open(&path).unwrap();
        assert_eq!(launch_state(&conn, "clone").0, "s3");

        let missing = MigrationContext { channel_store_path: dir.path().join("nope.db"), ..ctx };
        M0025AgentsLaunchStateBackfill.up(&missing).unwrap();
        assert!(!missing.channel_store_path.exists(), "a missing channel store is left missing");
    }
}
