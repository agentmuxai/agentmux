// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Re-key the named-agent registry from launch ids to agent ids.
//!
//! Agent-concept consolidation, Phase 3b PR 2
//! (`SPEC_AGENT_ARCHITECTURE_2026_05_27.md`): the cross-version/cross-channel
//! registry under `<home>/shared/agents/registry/` names one file per named
//! agent. It used to be keyed by the LAUNCH id (`db_agent_instances.id`);
//! now that an agent is one `db_agents` row, it is keyed by the AGENT id.
//!
//! Without this pass an upgraded install keeps its old files under the old
//! keys, and every one of them is a duplicate the picker shows next to the
//! agent's real entry — and worse, "Forget agent" retires the NEW key while
//! the stale file stays active, so the forgotten agent reappears.
//!
//! Mapping (read-only against the legacy table, which still exists until
//! Phase 3c drops it): a record keyed by launch `L` maps to the agent row
//! that launch folded into — the launch's definition when that definition is
//! a user agent, otherwise the root of `L`'s continuation chain. Records
//! whose launch id is not in THIS channel's `db_agent_instances` are left
//! alone: the registry is shared across channels, so another channel owns
//! them and will re-key them when it upgrades.
//!
//! Global scope: the registry is one host-global tree, and re-keying it twice
//! (once per channel) would be wasted work — but each channel only resolves
//! its own launches, so the pass is written to be safely repeatable and is
//! recorded per install.

use std::collections::HashMap;

use rusqlite::Connection;

use super::{Migration, MigrationContext, MigrationError, MigrationScope};
use crate::registry::Registry;

pub struct M0026RegistryAgentIdRekey;

/// One legacy launch row, reduced to what the mapping needs.
struct LaunchRow {
    definition_id: String,
    parent_instance_id: String,
}

/// The agent id a launch folded into, or `None` when this channel cannot
/// resolve it. Mirrors `instance_create`'s rule: a launch of a USER agent is
/// that agent; a launch of a template is its own agent, and a continuation
/// belongs to the launch at the root of its chain.
fn agent_id_for_launch(
    launch_id: &str,
    launches: &HashMap<String, LaunchRow>,
    user_agent_defs: &dyn Fn(&str) -> bool,
) -> Option<String> {
    let mut id = launch_id.to_string();
    let mut seen = std::collections::HashSet::new();
    loop {
        let row = launches.get(&id)?;
        if user_agent_defs(&row.definition_id) {
            return Some(row.definition_id.clone());
        }
        if row.parent_instance_id.is_empty() || !launches.contains_key(&row.parent_instance_id) {
            return Some(id);
        }
        if !seen.insert(id.clone()) {
            return Some(id); // cyclic chain — stop where we are
        }
        id = row.parent_instance_id.clone();
    }
}

/// How many records moved, and how many were left for another channel.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct RekeyStats {
    pub rekeyed: usize,
    pub already_current: usize,
    pub unresolved: usize,
}

pub(super) fn rekey_registry(conn: &Connection, registry: &Registry) -> Result<RekeyStats, String> {
    let has_legacy: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'db_agent_instances')",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("probe db_agent_instances: {e}"))?;
    if !has_legacy {
        return Ok(RekeyStats::default());
    }

    let mut stmt = conn
        .prepare("SELECT id, definition_id, parent_instance_id FROM db_agent_instances")
        .map_err(|e| format!("read db_agent_instances: {e}"))?;
    let launches: HashMap<String, LaunchRow> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                LaunchRow { definition_id: row.get(1)?, parent_instance_id: row.get(2)? },
            ))
        })
        .and_then(|rows| rows.collect::<Result<HashMap<_, _>, _>>())
        .map_err(|e| format!("read db_agent_instances: {e}"))?;
    drop(stmt);
    if launches.is_empty() {
        return Ok(RekeyStats::default());
    }

    let mut user_stmt = conn
        .prepare("SELECT id FROM db_agents WHERE is_template = 0")
        .map_err(|e| format!("read db_agents: {e}"))?;
    let user_agents: std::collections::HashSet<String> = user_stmt
        .query_map([], |row| row.get::<_, String>(0))
        .and_then(|rows| rows.collect::<Result<std::collections::HashSet<_>, _>>())
        .map_err(|e| format!("read db_agents: {e}"))?;
    drop(user_stmt);
    let is_user_agent = |id: &str| user_agents.contains(id);

    let mut stats = RekeyStats::default();
    let records = registry
        .list_active()
        .map_err(|e| format!("list registry: {e}"))?;
    for mut rec in records {
        let old_id = rec.data.instance_id.clone();
        let Some(agent_id) = agent_id_for_launch(&old_id, &launches, &is_user_agent) else {
            stats.unresolved += 1;
            continue;
        };
        if agent_id == old_id {
            stats.already_current += 1;
            continue;
        }
        // Write the record under the agent id, then drop the old file. An
        // interrupted run leaves both (the next boot re-runs this and the
        // duplicate resolves), never neither.
        rec.data.instance_id = agent_id.clone();
        rec.data.definition_id = agent_id.clone();
        registry
            .upsert(&rec)
            .map_err(|e| format!("write registry record {agent_id}: {e}"))?;
        registry
            .hard_delete(&old_id)
            .map_err(|e| format!("remove stale registry record {old_id}: {e}"))?;
        stats.rekeyed += 1;
    }
    Ok(stats)
}

impl Migration for M0026RegistryAgentIdRekey {
    fn id(&self) -> &'static str { "0026_registry_agent_id_rekey" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str {
        "Re-key the named-agent registry from launch ids to agent ids"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() {
            return Ok(());
        }
        let Some(registry_dir) = crate::registry::resolve_shared_registry_dir() else {
            // No shared root (CI, a bare `cargo run`) — nothing to re-key.
            return Ok(());
        };
        if !registry_dir.exists() {
            return Ok(());
        }
        let registry = Registry::open(registry_dir)
            .map_err(|e| MigrationError(format!("registry_agent_id_rekey: open registry: {e}")))?;
        let conn = Connection::open(&ctx.channel_store_path)
            .map_err(|e| MigrationError(format!("registry_agent_id_rekey: open store: {e}")))?;
        let stats = rekey_registry(&conn, &registry)
            .map_err(|e| MigrationError(format!("registry_agent_id_rekey: {e}")))?;
        tracing::info!(
            rekeyed = stats.rekeyed,
            already_current = stats.already_current,
            unresolved = stats.unresolved,
            "m0026_registry_agent_id_rekey: complete"
        );
        Ok(())
    }

    // No `verify()`: the post-condition is "no active record is keyed by a
    // launch id", which needs the legacy table to state — and that table is
    // what Phase 3c removes, so the check could only ever run before its own
    // precondition disappeared.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::store::Store;
    use crate::registry::{NamedAgentRecord, NamedAgentRecordV1, MAX_SUPPORTED_SCHEMA};

    fn record(instance_id: &str, definition_id: &str, name: &str) -> NamedAgentRecord {
        NamedAgentRecord {
            schema_version: MAX_SUPPORTED_SCHEMA,
            data: NamedAgentRecordV1 {
                instance_id: instance_id.to_string(),
                instance_name: name.to_string(),
                definition_id: definition_id.to_string(),
                identity_id: None,
                memory_id: None,
                session_id: Some("sess".to_string()),
                working_dir: name.to_string(),
                source_agents_base: None,
                created_at_ms: 1,
                last_launched_at_ms: 1,
                created_by_version: "0.55.0".to_string(),
                last_launched_by_version: "0.55.0".to_string(),
            },
        }
    }

    /// A channel store carrying the pre-consolidation launch rows plus the
    /// consolidated agent rows they map onto.
    fn store_with_launches() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("objects.db");
        drop(Store::open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "INSERT INTO db_agents (id, name, provider, is_template, parent_template_id, is_seeded, created_at, updated_at)
             VALUES ('user-agent', 'Maks', 'claude', 0, 'tpl', 0, 1, 1),
                    ('tpl', 'Coder', 'claude', 1, '', 1, 1, 1),
                    ('launch-head', 'FromTemplate', 'claude', 0, 'tpl', 0, 1, 1);
             -- The legacy definition rows the launch rows' FK points at.
             INSERT INTO db_agent_definitions
                (id, slug, name, icon, provider, description, working_directory, shell,
                 provider_flags, auto_start, restart_on_crash, idle_timeout_minutes,
                 created_at, agent_type, environment, agent_bus_id, is_seeded, accounts,
                 parent_id, branch_label, updated_at)
             VALUES ('user-agent', 'maks', 'Maks', '', 'claude', '', '', 'bash', '', 0, 0, 0,
                     1, 'standalone', '', '', 0, '', 'tpl', '', 1),
                    ('tpl', 'coder', 'Coder', '', 'claude', '', '', 'bash', '', 0, 0, 0,
                     1, 'standalone', '', '', 1, '', '', '', 1);
             INSERT INTO db_agent_instances
                (id, definition_id, parent_instance_id, block_id, session_id, status,
                 github_context, started_at, ended_at, created_at,
                 identity_id, memory_id, instance_name, working_directory, display_hidden)
             VALUES ('launch-on-user', 'user-agent', '', '', '', '', '', 1, 0, 1, '', '', 'Maks', '', 0),
                    ('launch-head', 'tpl', '', '', '', '', '', 1, 0, 1, '', '', 'FromTemplate', '', 0),
                    ('launch-cont', 'tpl', 'launch-head', '', '', '', '', 2, 0, 2, '', '', 'FromTemplate', '', 0);",
        )
        .unwrap();
        (dir, conn)
    }

    fn open_registry(dir: &tempfile::TempDir) -> Registry {
        Registry::open(dir.path().join("registry")).unwrap()
    }

    #[test]
    fn a_launch_of_a_user_agent_rekeys_to_that_agent() {
        let (dir, conn) = store_with_launches();
        let reg = open_registry(&dir);
        reg.upsert(&record("launch-on-user", "user-agent", "Maks")).unwrap();

        let stats = rekey_registry(&conn, &reg).unwrap();
        assert_eq!(stats.rekeyed, 1, "{stats:?}");
        assert!(reg.get("launch-on-user").unwrap().is_none(), "the stale file is gone");
        let moved = reg.get("user-agent").unwrap().expect("re-keyed record");
        assert_eq!(moved.data.instance_name, "Maks");
        assert_eq!(moved.data.session_id.as_deref(), Some("sess"), "content survives the move");
    }

    #[test]
    fn a_continuation_rekeys_to_the_chain_head_and_a_head_is_already_current() {
        let (dir, conn) = store_with_launches();
        let reg = open_registry(&dir);
        reg.upsert(&record("launch-cont", "tpl", "FromTemplate")).unwrap();
        let stats = rekey_registry(&conn, &reg).unwrap();
        assert_eq!((stats.rekeyed, stats.already_current), (1, 0), "{stats:?}");
        assert!(reg.get("launch-head").unwrap().is_some());

        // The head's own record is already keyed by the agent id.
        let stats = rekey_registry(&conn, &reg).unwrap();
        assert_eq!((stats.rekeyed, stats.already_current), (0, 1), "idempotent: {stats:?}");
    }

    #[test]
    fn a_record_this_channel_does_not_own_is_left_for_the_channel_that_does() {
        let (dir, conn) = store_with_launches();
        let reg = open_registry(&dir);
        reg.upsert(&record("launch-from-another-channel", "somewhere", "Foreign")).unwrap();
        let stats = rekey_registry(&conn, &reg).unwrap();
        assert_eq!(stats.unresolved, 1, "{stats:?}");
        assert!(
            reg.get("launch-from-another-channel").unwrap().is_some(),
            "another channel's record must survive untouched"
        );
    }

    #[test]
    fn a_store_without_the_legacy_table_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("objects.db");
        drop(Store::open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("DROP TABLE db_agent_instances;").unwrap();
        let reg = open_registry(&dir);
        reg.upsert(&record("whatever", "whatever", "Whatever")).unwrap();
        assert_eq!(rekey_registry(&conn, &reg).unwrap(), RekeyStats::default());
        assert!(reg.get("whatever").unwrap().is_some());
    }
}
