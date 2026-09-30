// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Global migration: backfill user-agent DEFINITIONS (with their content +
//! skills) from every channel's per-version `objects.db` — and every `dev/`
//! branch DB — into the GLOBAL definition store, so EXISTING agents become
//! cross-channel without waiting for the next edit. Read-only on every scanned
//! SQLite.
//!
//! **Version-gated, re-runnable marker** (not strictly one-shot): the
//! `.migrated_definitions` marker in the store root records the
//! [`MIGRATION_VERSION`] that last ran. A marker older than the current version
//! — including the legacy `migrated` text, which parses as version 0 — re-runs
//! the scan ONCE, so users whose earlier pass was incomplete recover
//! automatically on the next launch. Bump [`MIGRATION_VERSION`] whenever the
//! scan logic changes to trigger a one-time re-run for everyone.
//!
//! On a re-run, an agent already in the global store is refreshed only when the
//! scanned copy is strictly fresher than the global record (so a broken earlier
//! pass's stale/content-less copy is corrected) — never downgraded below what
//! the live write-mirror has since advanced, and never resurrected if
//! tombstoned. (codex P1 / reagent P2 #1391.)
//!
//! Cross-channel agent persistence, P0.2d
//! (`docs/specs/SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md`).
//!
//! Resilience mirrors `scripts/import-agents.sh`: a single unreadable / locked
//! / old-schema `objects.db` is skipped with a warning, never aborting the
//! pass. The marker is written after a pass even if some DBs were skipped — a
//! transiently-skipped DB's agents still go global on their next edit via the
//! live write-mirror (P0.2b), and a future [`MIGRATION_VERSION`] bump re-scans
//! everything — so nothing is permanently lost and the migration never loops
//! forever on a permanent old-schema failure.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use super::def_schema::{
    DefContentBlob, DefSkillBlob, DefinitionRecord, DefinitionRecordV1, DEF_MAX_SUPPORTED_SCHEMA,
};
use super::def_store::{DefStoreError, DefinitionStore};

const MARKER: &str = ".migrated_definitions";

/// Migration-logic version. Bump whenever the scan changes (new roots, schema
/// handling, etc.) so existing users re-run the migration ONCE: the marker
/// stores this number; a marker older than this re-runs. The pre-v2 marker
/// held the literal text `migrated`, which parses as version 0 → re-runs,
/// recovering everyone whose first pass was incomplete.
///
/// v2 — schema-resilient column handling (missing columns no longer skip a
/// whole DB), scans `dev/` branches too, and a recoverable versioned marker.
/// See `docs/analysis/ANALYSIS_CROSS_CHANNEL_AGENT_RETENTION_2026_06_13.md`.
const MIGRATION_VERSION: u32 = 2;

/// `db_agent_definitions` columns in the order the row mapper expects, paired
/// with a default SQL literal used when a column is absent on an older DB.
/// Substituting `<default> AS <col>` keeps the SELECT's column count + order
/// fixed (so index-based `row.get` stays valid) while tolerating schema drift.
const DEF_COLUMNS: &[(&str, &str)] = &[
    ("id", "''"),
    ("slug", "''"),
    ("name", "''"),
    ("icon", "''"),
    ("provider", "''"),
    ("description", "''"),
    ("working_directory", "''"),
    ("shell", "''"),
    ("provider_flags", "''"),
    ("auto_start", "0"),
    ("restart_on_crash", "0"),
    ("idle_timeout_minutes", "0"),
    ("created_at", "0"),
    ("agent_type", "'host'"),
    ("environment", "''"),
    ("agent_bus_id", "''"),
    ("is_seeded", "0"),
    ("accounts", "''"),
    ("parent_id", "''"),
    ("branch_label", "''"),
    ("updated_at", "0"),
    ("user_hidden", "0"),
    ("container_image", "''"),
    ("container_volumes", "'[]'"),
    ("container_name", "''"),
    ("use_ambient_login", "0"),
    ("auto_continue_enabled", "0"),
];

/// Outcome stats — surfaced in the srv log.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefMigrateStats {
    pub dbs_scanned: usize,
    pub dbs_skipped: usize,
    pub rows_seen: usize,
    pub records_written: usize,
    pub records_skipped_existing: usize,
}

/// Scan every channel AND dev-branch `objects.db` under `home` for user agents
/// and backfill them into the global definition `store`. Re-runs once per
/// [`MIGRATION_VERSION`] bump (marker in `store.root()`); read-only on every
/// scanned SQLite, and tolerant of older schemas (missing columns).
pub fn migrate_definitions_global_once(
    home: &Path,
    store: &DefinitionStore,
) -> Result<DefMigrateStats, DefStoreError> {
    let mut stats = DefMigrateStats::default();
    let marker = store.root().join(MARKER);
    if marker_version(&marker) >= MIGRATION_VERSION {
        return Ok(stats);
    }

    // Dedup across channels/versions/dev-branches: keep the copy with the
    // highest FRESHNESS = max(def.updated_at, content.updated_at,
    // skill.created_at). `agent_content_set` bumps the content row's timestamp
    // without touching the definition row, so comparing definition timestamps
    // alone could keep a stale content/skills snapshot. (codex P1.)
    let mut best: HashMap<String, (DefinitionRecord, i64)> = HashMap::new();
    for db in collect_scan_dbs(home) {
        stats.dbs_scanned += 1;
        match read_user_definitions(&db) {
            Ok(recs) => {
                for (rec, freshness) in recs {
                    stats.rows_seen += 1;
                    let id = rec.data.id.clone();
                    let keep = match best.get(&id) {
                        Some((_, f)) => freshness > *f,
                        None => true,
                    };
                    if keep {
                        best.insert(id, (rec, freshness));
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
    for (id, (rec, freshness)) in best {
        // Decide whether to write based on freshness vs any existing global
        // record:
        //   - absent              → backfill (new agent).
        //   - present & OLDER      → refresh. A broken earlier pass may have
        //     written a stale or content-less copy globally while a fresher
        //     channel/dev DB holds the real one; the recovery re-run must
        //     correct it. (codex P1.)
        //   - present & not-older  → skip. The live write-mirror advances the
        //     global record without touching any channel's SQLite, so a
        //     channel-DB copy can be staler than what's already global — never
        //     downgrade it. (reagent P2.)
        // `upsert` additionally refuses to resurrect a tombstoned id.
        //
        // Freshness comparison: the scanned side is max(def, content, skill)
        // timestamps; the in-memory global record only carries `updated_at`
        // (its content/skill blobs are timestamp-less), so we compare against
        // that. The asymmetry biases toward refreshing on a recovery run, which
        // is the safe direction, and still never downgrades a genuinely-newer
        // global `updated_at`.
        match store.get(&id) {
            Ok(Some(existing)) => {
                if freshness <= existing.data.updated_at {
                    stats.records_skipped_existing += 1;
                    continue;
                }
                // else: scanned copy is strictly fresher → fall through to upsert.
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(def_id = %id, error = %e, "def migrate: get failed; skipping");
                continue;
            }
        }
        match store.upsert(&rec) {
            Ok(()) => stats.records_written += 1,
            Err(e) => {
                tracing::warn!(error = %e, "def migrate: upsert into global store failed")
            }
        }
    }

    // Record the migration version. A DB skipped this pass (genuinely
    // corrupt/locked — now rare since v2 tolerates schema drift) still goes
    // global on its next edit via the live write-mirror (P0.2b), and a future
    // MIGRATION_VERSION bump re-scans everything — so nothing is permanently
    // lost and we never loop on a permanent failure.
    write_marker(&marker, MIGRATION_VERSION)?;
    Ok(stats)
}

/// Migration version recorded in the marker; `0` when absent or unparseable
/// (the legacy `migrated` text → 0 → re-run).
fn marker_version(marker: &Path) -> u32 {
    std::fs::read_to_string(marker)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(0)
}

/// Every `…/data/db/objects.db` under `home/channels/*/versions/*` AND
/// `home/dev/<branch>[/<sub>]`. The instance migration scans both
/// (`migrate.rs::enumerate_sources`); the definition migration must too, or
/// dev-branch agents (e.g. agents created via `task dev`) never go global.
fn collect_scan_dbs(home: &Path) -> Vec<PathBuf> {
    let mut dbs = Vec::new();
    let mut add = |dir: &Path| {
        let db = dir.join("data").join("db").join("objects.db");
        if db.is_file() {
            dbs.push(db);
        }
    };
    // Installed/portable: channels/<ch>/versions/<v>/data/db/objects.db
    for ch in dir_subdirs(&home.join("channels")) {
        for v in dir_subdirs(&ch.join("versions")) {
            add(&v);
        }
    }
    // Dev: dev/<branch>[/<sub>]/data/db/objects.db (both layouts).
    for br in dir_subdirs(&home.join("dev")) {
        add(&br);
        for sub in dir_subdirs(&br) {
            add(&sub);
        }
    }
    dbs
}

/// Read all `is_seeded=0` definitions from `db` with their content + skills,
/// each paired with a freshness timestamp for cross-copy winner selection.
fn read_user_definitions(db: &Path) -> Result<Vec<(DefinitionRecord, i64)>, rusqlite::Error> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    // Schema-resilient: introspect the columns present and substitute a default
    // literal for any absent on an older DB, so a missing column degrades a
    // single field rather than skipping the whole DB (the original bug — older
    // DBs lacked `container_image`/`_volumes`/`_name`). The column order/count
    // stays fixed, so the index-based row mapper below remains valid.
    let present = present_columns(&conn, "db_agent_definitions")?;
    if present.is_empty() {
        // No db_agent_definitions table here → no user definitions.
        return Ok(Vec::new());
    }
    let select_list = DEF_COLUMNS
        .iter()
        .map(|(name, default)| {
            if present.contains(*name) {
                (*name).to_string()
            } else {
                format!("{default} AS {name}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    // Filter seeded templates only when the column exists (pre-seeded-template
    // DBs have no templates, so every row is a user agent).
    let where_clause = if present.contains("is_seeded") {
        " WHERE is_seeded = 0"
    } else {
        ""
    };
    let sql = format!("SELECT {select_list} FROM db_agent_definitions{where_clause}");
    let defs: Vec<DefinitionRecordV1> = {
        let mut stmt = conn.prepare(&sql)?;
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
                use_ambient_login: row.get(25)?,
                auto_continue_enabled: row.get(26)?,
                content: Vec::new(),
                skills: Vec::new(),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };

    let mut out = Vec::with_capacity(defs.len());
    for mut d in defs {
        // Content/skills tables are absent on pre-v2 DBs — tolerate THAT, but
        // propagate a genuine error (lock/corrupt) so the whole DB is skipped
        // rather than silently writing an instruction-less record. (codex P2.)
        d.content = tolerate_missing_table(read_content(&conn, &d.id))?;
        d.skills = tolerate_missing_table(read_skills(&conn, &d.id))?;
        let content_max = max_ts(
            &conn,
            "SELECT MAX(updated_at) FROM db_agent_content WHERE agent_id = ?1",
            &d.id,
        )?;
        let skill_max = max_ts(
            &conn,
            "SELECT MAX(created_at) FROM db_agent_skills WHERE agent_id = ?1",
            &d.id,
        )?;
        let freshness = d.updated_at.max(content_max).max(skill_max);
        out.push((
            DefinitionRecord {
                schema_version: DEF_MAX_SUPPORTED_SCHEMA,
                data: d,
            },
            freshness,
        ));
    }
    Ok(out)
}

/// True iff the error is SQLite "no such table" (a pre-v2 DB missing
/// db_agent_content / db_agent_skills) — the only failure we treat as "no
/// rows" rather than a reason to skip the DB.
fn is_missing_table(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(_, Some(msg)) if msg.contains("no such table"))
}

fn tolerate_missing_table<T: Default>(r: Result<T, rusqlite::Error>) -> Result<T, rusqlite::Error> {
    match r {
        Ok(v) => Ok(v),
        Err(ref e) if is_missing_table(e) => Ok(T::default()),
        Err(e) => Err(e),
    }
}

/// `MAX(col)` over a per-agent table; `0` when the table is missing (pre-v2)
/// or there are no rows. Other errors propagate (→ DB skipped).
fn max_ts(conn: &Connection, sql: &str, agent_id: &str) -> Result<i64, rusqlite::Error> {
    match conn.query_row(sql, [agent_id], |row| row.get::<_, Option<i64>>(0)) {
        Ok(v) => Ok(v.unwrap_or(0)),
        Err(ref e) if is_missing_table(e) => Ok(0),
        Err(e) => Err(e),
    }
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

fn write_marker(path: &Path, version: u32) -> Result<(), DefStoreError> {
    std::fs::write(path, version.to_string().as_bytes())?;
    Ok(())
}

/// Column names present in `table` (empty set if the table doesn't exist —
/// `PRAGMA table_info` on a missing table yields no rows, not an error).
fn present_columns(conn: &Connection, table: &str) -> Result<HashSet<String>, rusqlite::Error> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    /// Create the current (v2) schema at `db`.
    fn create_full_schema(db: &Path) {
        let conn = Connection::open(db).unwrap();
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
    }

    fn db_at(dir: PathBuf) -> PathBuf {
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("objects.db");
        create_full_schema(&db);
        db
    }

    fn make_channel_db(home: &Path, channel: &str, version: &str) -> PathBuf {
        db_at(home
            .join("channels")
            .join(channel)
            .join("versions")
            .join(version)
            .join("data")
            .join("db"))
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
    fn skips_seeded_and_dedups_by_latest_freshness() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        // Same id in two channels with different updated_at → freshest wins.
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
            "latest freshness wins across channels"
        );
        assert!(store.get("tpl").unwrap().is_none(), "seeded template not migrated");
        assert_eq!(stats.records_written, 1);
    }

    #[test]
    fn does_not_downgrade_an_existing_global_record() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        // The channel DB has the OLD copy.
        let db = make_channel_db(home, "ch-a", "0.44.1");
        insert_user_agent(&db, "dup", "OldName", 100);
        let store = store_at(home);
        // The global store already has a NEWER copy (e.g. the live mirror
        // advanced it via a cross-channel edit that never touched the DB).
        store
            .upsert(&DefinitionRecord {
                schema_version: DEF_MAX_SUPPORTED_SCHEMA,
                data: DefinitionRecordV1 {
                    id: "dup".to_string(),
                    name: "NewName".to_string(),
                    provider: "claude".to_string(),
                    updated_at: 500,
                    ..Default::default()
                },
            })
            .unwrap();

        let stats = migrate_definitions_global_once(home, &store).unwrap();
        assert_eq!(
            store.get("dup").unwrap().unwrap().data.name,
            "NewName",
            "migration must NOT downgrade the newer global record"
        );
        assert_eq!(stats.records_written, 0);
        assert_eq!(stats.records_skipped_existing, 1);
    }

    #[test]
    fn refreshes_a_stale_global_record_on_rerun() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        // The channel DB holds a FRESHER copy (updated_at=300).
        let db = make_channel_db(home, "ch-a", "0.44.1");
        insert_user_agent(&db, "dup", "FreshName", 300);
        let store = store_at(home);
        // The global store holds a STALE copy a broken earlier pass wrote
        // (older updated_at, e.g. content-less). The recovery re-run must
        // REFRESH it from the fresher scan rather than skip it. (codex P1.)
        store
            .upsert(&DefinitionRecord {
                schema_version: DEF_MAX_SUPPORTED_SCHEMA,
                data: DefinitionRecordV1 {
                    id: "dup".to_string(),
                    name: "StaleName".to_string(),
                    provider: "claude".to_string(),
                    updated_at: 50,
                    ..Default::default()
                },
            })
            .unwrap();

        let stats = migrate_definitions_global_once(home, &store).unwrap();
        let rec = store.get("dup").unwrap().unwrap();
        assert_eq!(rec.data.name, "FreshName", "stale global record must be refreshed from the fresher scan");
        assert_eq!(rec.data.content.len(), 1, "content backfilled on refresh");
        assert_eq!(stats.records_written, 1);
        assert_eq!(stats.records_skipped_existing, 0);
    }

    #[test]
    fn winner_selection_includes_content_freshness() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        // Copy A: newer DEFINITION timestamp, old content.
        let db_a = make_channel_db(home, "ch-a", "0.44.2");
        insert_user_agent(&db_a, "dup", "FromA", 300);
        // Copy B: older definition timestamp, but content edited later
        // (agent_content_set bumps content.updated_at, not the def row).
        let db_b = make_channel_db(home, "ch-b", "0.44.1");
        insert_user_agent(&db_b, "dup", "FromB", 200);
        Connection::open(&db_b)
            .unwrap()
            .execute(
                "UPDATE db_agent_content SET content = 'fresh', updated_at = 900 WHERE agent_id = 'dup'",
                [],
            )
            .unwrap();

        let store = store_at(home);
        migrate_definitions_global_once(home, &store).unwrap();
        let rec = store.get("dup").unwrap().unwrap();
        assert_eq!(rec.data.name, "FromB", "copy with freshest content wins");
        assert_eq!(rec.data.content[0].content, "fresh");
    }

    #[test]
    fn migrates_db_missing_container_columns() {
        // F1: an older DB without container_image/_volumes/_name (and without
        // the content/skills tables) must NOT be skipped — the missing columns
        // degrade to defaults.
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let db_dir = home
            .join("channels")
            .join("old-ch")
            .join("versions")
            .join("0.40.0")
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
                user_hidden INTEGER);",
        )
        .unwrap();
        // All 22 present columns populated (real old DBs are NOT NULL); only
        // the container_* columns are absent from the schema entirely.
        conn.execute(
            "INSERT INTO db_agent_definitions VALUES
             ('old-1','old-1','OldAgent','✦','claude','','','','',0,0,0,1,
              'host','','',0,'','','',50,0)",
            [],
        )
        .unwrap();
        drop(conn);

        let store = store_at(home);
        let stats = migrate_definitions_global_once(home, &store).unwrap();
        assert_eq!(stats.dbs_skipped, 0, "old-schema DB must not be skipped");
        assert_eq!(stats.records_written, 1, "old-schema agent migrated");
        let rec = store.get("old-1").unwrap().unwrap();
        assert_eq!(rec.data.name, "OldAgent");
        assert_eq!(rec.data.container_volumes, "[]", "missing column defaulted");
    }

    #[test]
    fn scans_dev_branch_agents() {
        // F2: agents in dev/<branch>/<sub>/ must be migrated too.
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let db = db_at(
            home.join("dev")
                .join("main")
                .join("abc123")
                .join("data")
                .join("db"),
        );
        insert_user_agent(&db, "dev-1", "DevAgent", 100);

        let store = store_at(home);
        let stats = migrate_definitions_global_once(home, &store).unwrap();
        assert_eq!(stats.records_written, 1, "dev-branch agent migrated");
        assert_eq!(store.get("dev-1").unwrap().unwrap().data.name, "DevAgent");
    }

    #[test]
    fn legacy_text_marker_triggers_rerun_then_settles() {
        // F3/F4: the pre-v2 marker (literal "migrated") parses as version 0 →
        // re-runs once; afterward the versioned marker makes it idempotent.
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let db = make_channel_db(home, "ch", "0.44.1");
        insert_user_agent(&db, "a1", "A", 100);
        let store = store_at(home);
        std::fs::write(store.root().join(MARKER), b"migrated\n").unwrap();

        let stats = migrate_definitions_global_once(home, &store).unwrap();
        assert_eq!(stats.records_written, 1, "legacy text marker must re-run");
        assert!(store.get("a1").unwrap().is_some());

        let stats2 = migrate_definitions_global_once(home, &store).unwrap();
        assert_eq!(stats2.dbs_scanned, 0, "versioned marker → idempotent after");
    }
}
