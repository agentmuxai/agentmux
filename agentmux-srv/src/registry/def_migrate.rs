// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One-shot global migration: backfill user-agent DEFINITIONS (with their
//! content + skills) from every channel's per-version `objects.db` into the
//! GLOBAL definition store, so EXISTING agents become cross-channel without
//! waiting for the next edit. Idempotent via a `.migrated_definitions`
//! marker in the store root; read-only on every scanned SQLite.
//!
//! Cross-channel agent persistence, P0.2d
//! (`docs/specs/SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md`).
//!
//! Resilience mirrors `scripts/import-agents.sh`: a single unreadable /
//! locked / old-schema (missing-column) `objects.db` is skipped with a
//! warning, never aborting the whole pass. The marker is written
//! unconditionally after a pass — a transiently-skipped DB's agents still go
//! global on their next edit via the live write-mirror (P0.2b), so nothing
//! is permanently lost and the migration never loops forever on a permanent
//! old-schema failure.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use super::def_schema::{
    DefContentBlob, DefSkillBlob, DefinitionRecord, DefinitionRecordV1, DEF_MAX_SUPPORTED_SCHEMA,
};
use super::def_store::{DefStoreError, DefinitionStore};

const MARKER: &str = ".migrated_definitions";

/// Outcome stats — surfaced in the srv log.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefMigrateStats {
    pub dbs_scanned: usize,
    pub dbs_skipped: usize,
    pub rows_seen: usize,
    pub records_written: usize,
}

/// Scan `<home>/channels/*/versions/*/data/db/objects.db` for user agents and
/// backfill them into the global definition `store`. Runs at most once
/// (marker in `store.root()`).
pub fn migrate_definitions_global_once(
    home: &Path,
    store: &DefinitionStore,
) -> Result<DefMigrateStats, DefStoreError> {
    let mut stats = DefMigrateStats::default();
    let marker = store.root().join(MARKER);
    if marker.exists() {
        return Ok(stats);
    }

    let channels = home.join("channels");
    if channels.is_dir() {
        // Dedup across channels/versions: keep the row with the highest
        // `updated_at` per definition id (the most recent edit wins).
        let mut best: HashMap<String, DefinitionRecord> = HashMap::new();
        for ch in dir_subdirs(&channels) {
            for v in dir_subdirs(&ch.join("versions")) {
                let db = v.join("data").join("db").join("objects.db");
                if !db.is_file() {
                    continue;
                }
                stats.dbs_scanned += 1;
                match read_user_definitions(&db) {
                    Ok(recs) => {
                        for rec in recs {
                            stats.rows_seen += 1;
                            let id = rec.data.id.clone();
                            let keep = match best.get(&id) {
                                Some(existing) => rec.data.updated_at > existing.data.updated_at,
                                None => true,
                            };
                            if keep {
                                best.insert(id, rec);
                            }
                        }
                    }
                    Err(e) => {
                        stats.dbs_skipped += 1;
                        tracing::warn!(
                            db = %db.display(),
                            error = %e,
                            "def migrate: skipping unreadable/incompatible objects.db"
                        );
                    }
                }
            }
        }
        // `upsert` refuses to resurrect a tombstoned id, so an agent the user
        // deleted (in any channel) stays deleted.
        for (_id, rec) in best {
            match store.upsert(&rec) {
                Ok(()) => stats.records_written += 1,
                Err(e) => {
                    tracing::warn!(error = %e, "def migrate: upsert into global store failed")
                }
            }
        }
    }

    // One-shot: write the marker even if some DBs were skipped (the live
    // write-mirror backfills any skipped-but-current agent on its next edit;
    // re-running every launch would loop forever on permanent old-schema DBs).
    write_marker(&marker)?;
    Ok(stats)
}

fn read_user_definitions(db: &Path) -> Result<Vec<DefinitionRecord>, rusqlite::Error> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let defs: Vec<DefinitionRecordV1> = {
        let mut stmt = conn.prepare(
            "SELECT id, slug, name, icon, provider, description, working_directory, shell,
                    provider_flags, auto_start, restart_on_crash, idle_timeout_minutes, created_at,
                    agent_type, environment, agent_bus_id, is_seeded, accounts, parent_id,
                    branch_label, updated_at, user_hidden, container_image, container_volumes,
                    container_name
             FROM db_agent_definitions
             WHERE is_seeded = 0",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(DefinitionRecordV1 {
                id: row.get(0)?,
                slug: row.get(1)?,
                name: row.get(2)?,
                icon: row.get(3)?,
                provider: row.get(4)?,
                description: row.get(5)?,
                working_directory: row.get(6)?,
                shell: row.get(7)?,
                provider_flags: row.get(8)?,
                auto_start: row.get(9)?,
                restart_on_crash: row.get(10)?,
                idle_timeout_minutes: row.get(11)?,
                created_at: row.get(12)?,
                agent_type: row.get(13)?,
                environment: row.get(14)?,
                agent_bus_id: row.get(15)?,
                is_seeded: row.get(16)?,
                accounts: row.get(17)?,
                parent_id: row.get(18)?,
                branch_label: row.get(19)?,
                updated_at: row.get(20)?,
                user_hidden: row.get(21)?,
                container_image: row.get(22)?,
                container_volumes: row.get(23)?,
                container_name: row.get(24)?,
                content: Vec::new(),
                skills: Vec::new(),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };

    let mut out = Vec::with_capacity(defs.len());
    for mut d in defs {
        // Content/skills are best-effort: a pre-v2 DB may lack the tables.
        d.content = read_content(&conn, &d.id).unwrap_or_default();
        d.skills = read_skills(&conn, &d.id).unwrap_or_default();
        out.push(DefinitionRecord {
            schema_version: DEF_MAX_SUPPORTED_SCHEMA,
            data: d,
        });
    }
    Ok(out)
}

fn read_content(conn: &Connection, agent_id: &str) -> Result<Vec<DefContentBlob>, rusqlite::Error> {
    let mut stmt =
        conn.prepare("SELECT content_type, content FROM db_agent_content WHERE agent_id = ?1")?;
    let rows = stmt.query_map([agent_id], |row| {
        Ok(DefContentBlob {
            content_type: row.get(0)?,
            content: row.get(1)?,
        })
    })?;
    rows.collect()
}

fn read_skills(conn: &Connection, agent_id: &str) -> Result<Vec<DefSkillBlob>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, name, trigger, skill_type, description, content
         FROM db_agent_skills WHERE agent_id = ?1",
    )?;
    let rows = stmt.query_map([agent_id], |row| {
        Ok(DefSkillBlob {
            id: row.get(0)?,
            name: row.get(1)?,
            trigger: row.get(2)?,
            skill_type: row.get(3)?,
            description: row.get(4)?,
            content: row.get(5)?,
        })
    })?;
    rows.collect()
}

fn dir_subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect()
}

fn write_marker(path: &Path) -> Result<(), DefStoreError> {
    std::fs::write(path, b"migrated\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn make_channel_db(home: &Path, channel: &str, version: &str) -> PathBuf {
        let db_dir = home
            .join("channels")
            .join(channel)
            .join("versions")
            .join(version)
            .join("data")
            .join("db");
        std::fs::create_dir_all(&db_dir).unwrap();
        let db = db_dir.join("objects.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE db_agent_definitions (
                id TEXT, slug TEXT, name TEXT, icon TEXT, provider TEXT, description TEXT,
                working_directory TEXT, shell TEXT, provider_flags TEXT, auto_start INTEGER,
                restart_on_crash INTEGER, idle_timeout_minutes INTEGER, created_at INTEGER,
                agent_type TEXT, environment TEXT, agent_bus_id TEXT, is_seeded INTEGER,
                accounts TEXT, parent_id TEXT, branch_label TEXT, updated_at INTEGER,
                user_hidden INTEGER, container_image TEXT, container_volumes TEXT, container_name TEXT);
             CREATE TABLE db_agent_content (agent_id TEXT, content_type TEXT, content TEXT, updated_at INTEGER);
             CREATE TABLE db_agent_skills (id TEXT, agent_id TEXT, name TEXT, trigger TEXT, skill_type TEXT, description TEXT, content TEXT, created_at INTEGER);",
        )
        .unwrap();
        db
    }

    fn insert_user_agent(db: &Path, id: &str, name: &str, updated_at: i64) {
        let conn = Connection::open(db).unwrap();
        conn.execute(
            "INSERT INTO db_agent_definitions (id, slug, name, icon, provider, description,
                working_directory, shell, provider_flags, auto_start, restart_on_crash,
                idle_timeout_minutes, created_at, agent_type, environment, agent_bus_id, is_seeded,
                accounts, parent_id, branch_label, updated_at, user_hidden, container_image,
                container_volumes, container_name)
             VALUES (?1, ?1, ?2, '✦', 'claude', '', '', '', '', 0, 0, 0, 1, 'host', '', '', 0,
                     '', '', '', ?3, 0, '', '[]', '')",
            params![id, name, updated_at],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO db_agent_content (agent_id, content_type, content, updated_at)
             VALUES (?1, 'agentmd', 'instructions', 1)",
            params![id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO db_agent_skills (id, agent_id, name, trigger, skill_type, description, content, created_at)
             VALUES ('sk1', ?1, 'greet', '', 'prompt', '', '', 1)",
            params![id],
        )
        .unwrap();
    }

    fn store_at(home: &Path) -> DefinitionStore {
        DefinitionStore::open(home.join("shared").join("agents").join("definitions")).unwrap()
    }

    #[test]
    fn migrates_user_agents_with_content_and_skills_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let db = make_channel_db(home, "local-a-aaa", "0.44.1");
        insert_user_agent(&db, "agent-1", "Maks", 100);
        let store = store_at(home);

        let stats = migrate_definitions_global_once(home, &store).unwrap();
        assert_eq!(stats.records_written, 1);
        let rec = store.get("agent-1").unwrap().unwrap();
        assert_eq!(rec.data.name, "Maks");
        assert_eq!(rec.data.content.len(), 1, "content backfilled");
        assert_eq!(rec.data.skills.len(), 1, "skills backfilled");

        // Second run is a no-op (marker present).
        let stats2 = migrate_definitions_global_once(home, &store).unwrap();
        assert_eq!(stats2.dbs_scanned, 0);
        assert_eq!(stats2.records_written, 0);
    }

    #[test]
    fn skips_seeded_and_dedups_by_latest_updated_at() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        // Same id in two channels with different updated_at → latest wins.
        let db_a = make_channel_db(home, "ch-a", "0.44.1");
        insert_user_agent(&db_a, "dup", "Old", 100);
        let db_b = make_channel_db(home, "ch-b", "0.44.1");
        insert_user_agent(&db_b, "dup", "New", 200);
        // A seeded template — must NOT migrate.
        Connection::open(&db_a)
            .unwrap()
            .execute(
                "INSERT INTO db_agent_definitions (id, slug, name, provider, is_seeded, container_volumes)
                 VALUES ('tpl', 'tpl', 'Claude', 'claude', 1, '[]')",
                [],
            )
            .unwrap();

        let store = store_at(home);
        let stats = migrate_definitions_global_once(home, &store).unwrap();
        assert_eq!(
            store.get("dup").unwrap().unwrap().data.name,
            "New",
            "latest updated_at wins across channels"
        );
        assert!(store.get("tpl").unwrap().is_none(), "seeded template not migrated");
        assert_eq!(stats.records_written, 1);
    }
}
