// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One-shot SQLite → file-registry migration. Runs at most once per
//! `<root>/.migrated_from_sqlite` marker; idempotent and read-only on
//! every SQLite it touches. See SPEC §8.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use super::schema::{NamedAgentRecord, NamedAgentRecordV1, MAX_SUPPORTED_SCHEMA};
use super::store::{Registry, RegistryError};

/// Outcome stats — surfaced in the marker file + the srv log.
#[derive(Debug, Default, Clone, Copy)]
pub struct MigrateStats {
    pub versions_scanned: usize,
    pub rows_seen: usize,
    pub records_written: usize,
    pub records_skipped_existing: usize,
    pub records_skipped_unmappable: usize,
    /// True iff every per-version DB read cleanly. Callers gate
    /// registry attachment on this — partial migrations leave the
    /// registry detached so reads keep falling back to SQLite, and
    /// the next launch retries (no marker written when `false`).
    pub complete: bool,
}

/// Marker filename. Lives in the registry root so the registry's
/// existence implies the migration question has been asked at least
/// once.
const MARKER: &str = ".migrated_from_sqlite";

/// Scan every per-version `data/db/objects.db` and populate the
/// shared registry. Skipped if the marker file exists. Never
/// overwrites an existing registry record (idempotency + respect
/// for newer-written data). The SQLite files are opened **read-only**
/// — never modified.
///
/// On dedup conflicts (same `instance_id` in multiple versions), the
/// row with the latest `started_at` wins.
pub fn migrate_from_sqlite_once(
    shared_home: &Path,
    registry: &Registry,
) -> Result<MigrateStats, RegistryError> {
    let marker_path = registry.root().join(MARKER);
    if marker_path.exists() {
        // Marker present ⇒ a prior run completed; treat as complete
        // so callers attach the registry.
        return Ok(MigrateStats {
            complete: true,
            ..MigrateStats::default()
        });
    }

    let mut stats = MigrateStats::default();
    let agents_root = registry.agents_root().ok_or_else(|| {
        RegistryError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "registry root has no parent",
        ))
    })?;

    let versions_root = shared_home.join("versions");
    if !versions_root.is_dir() {
        stats.complete = true;
        write_marker(&marker_path, &stats)?;
        return Ok(stats);
    }

    let mut latest_by_id: HashMap<String, RowSnapshot> = HashMap::new();
    // True iff any per-version DB threw a non-transient-looking error.
    // We use this to skip writing the marker so the next launch
    // retries — otherwise a brief filesystem hiccup permanently
    // omits those rows from the registry-backed dropdown.
    let mut any_db_failed = false;

    for entry in std::fs::read_dir(&versions_root)? {
        let v_dir = entry?.path();
        if !v_dir.is_dir() {
            continue;
        }
        let db_path = v_dir.join("data").join("db").join("objects.db");
        if !db_path.is_file() {
            continue;
        }
        stats.versions_scanned += 1;

        match read_named_rows(&db_path) {
            Ok(rows) => {
                for row in rows {
                    stats.rows_seen += 1;
                    let key = row.id.clone();
                    match latest_by_id.get_mut(&key) {
                        Some(existing) if existing.started_at >= row.started_at => {
                            // Existing snapshot wins on started_at, but
                            // OR the hidden flag — any version expressing
                            // "forget" intent is preserved as a tombstone.
                            existing.display_hidden =
                                existing.display_hidden || row.display_hidden;
                        }
                        Some(existing) => {
                            let merged_hidden =
                                existing.display_hidden || row.display_hidden;
                            *existing = row;
                            existing.display_hidden = merged_hidden;
                        }
                        None => {
                            latest_by_id.insert(key, row);
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    db = %db_path.display(),
                    error = %e,
                    "registry-migrate: per-version DB unreadable — will retry on next launch"
                );
                any_db_failed = true;
            }
        }
    }

    for (id, row) in latest_by_id {
        // Check active AND retired — a record retired by a newer
        // version's "Forget agent" must NOT be resurrected just
        // because an older version's SQLite still lists it as
        // visible.
        if registry.exists_anywhere(&id) {
            stats.records_skipped_existing += 1;
            continue;
        }
        let display_hidden = row.display_hidden;
        let Some(rec) = row_to_record(&row, agents_root) else {
            stats.records_skipped_unmappable += 1;
            continue;
        };
        if let Err(e) = registry.upsert(&rec) {
            tracing::warn!(
                instance_id = %id,
                error = %e,
                "registry-migrate: upsert failed"
            );
            stats.records_skipped_unmappable += 1;
            continue;
        }
        // Preserve pre-registry "forget" intent: if any version's
        // SQLite had this row hidden, move the freshly-written
        // registry file to retired/ so the dropdown stays consistent
        // with the user's prior soft-delete.
        if display_hidden {
            if let Err(e) = registry.retire(&id) {
                tracing::warn!(
                    instance_id = %id,
                    error = %e,
                    "registry-migrate: failed to retire migrated tombstone — record may surface as active"
                );
            }
        }
        stats.records_written += 1;
    }

    // Only finalize the migration when every DB we encountered was
    // readable. On any per-DB error, defer the marker so a future
    // launch retries the migration AND signal `complete = false` so
    // main.rs leaves the registry detached for this session (reads
    // fall back to SQLite — preferred over serving a partial view).
    stats.complete = !any_db_failed;
    if stats.complete {
        write_marker(&marker_path, &stats)?;
    } else {
        tracing::info!(
            "registry-migrate: deferring marker write; one or more per-version DBs were unreadable and will be retried next launch"
        );
    }
    Ok(stats)
}

fn write_marker(path: &Path, stats: &MigrateStats) -> std::io::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let body = format!(
        "migrated_at: {now}\n\
         versions_scanned: {}\n\
         rows_seen: {}\n\
         records_written: {}\n\
         records_skipped_existing: {}\n\
         records_skipped_unmappable: {}\n",
        stats.versions_scanned,
        stats.rows_seen,
        stats.records_written,
        stats.records_skipped_existing,
        stats.records_skipped_unmappable,
    );
    std::fs::write(path, body)
}

struct RowSnapshot {
    id: String,
    instance_name: String,
    definition_id: String,
    identity_id: String,
    memory_id: String,
    working_directory: String,
    started_at: i64,
    created_at: i64,
    display_hidden: bool,
}

/// True iff the error is SQLite reporting "this column/table doesn't
/// exist in this DB's schema." Distinguishes a pre-v8 DB (skip
/// silently) from corruption (caller logs + continues). Matches on
/// `SqliteFailure` with `Some(msg)` containing the canonical SQLite
/// phrases; the underlying `ExtendedCode` for both is
/// `SQLITE_ERROR` (1), which would also fire for plenty of other
/// real failures, so message inspection is required.
fn is_missing_column_or_table(e: &rusqlite::Error) -> bool {
    match e {
        rusqlite::Error::SqliteFailure(_, Some(msg)) => {
            msg.starts_with("no such column") || msg.starts_with("no such table")
        }
        _ => false,
    }
}

fn read_named_rows(db_path: &Path) -> Result<Vec<RowSnapshot>, rusqlite::Error> {
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    // Older schemas (pre-v8) lack `instance_name` / `working_directory`
    // columns. Suppress ONLY the specific "no such column/table"
    // errors — broader SqliteFailures (corruption, locked, etc.) must
    // surface so the caller can log + continue with the next DB.
    // Include hidden rows — the caller turns them into retired/
    // tombstones so a pre-registry "Forget agent" intent survives
    // migration even if another version still has the row visible.
    let mut stmt = match conn.prepare(
        "SELECT id, instance_name, definition_id, identity_id, memory_id,
                working_directory, started_at, created_at, display_hidden
         FROM db_agent_instances
         WHERE instance_name <> ''
           AND parent_instance_id = ''",
    ) {
        Ok(s) => s,
        Err(e) if is_missing_column_or_table(&e) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let iter = stmt.query_map([], |row| {
        Ok(RowSnapshot {
            id: row.get(0)?,
            instance_name: row.get(1)?,
            definition_id: row.get(2)?,
            identity_id: row.get(3)?,
            memory_id: row.get(4)?,
            working_directory: row.get(5)?,
            started_at: row.get(6)?,
            created_at: row.get(7)?,
            display_hidden: row.get::<_, i64>(8)? != 0,
        })
    })?;
    iter.collect()
}

fn row_to_record(row: &RowSnapshot, agents_root: &Path) -> Option<NamedAgentRecord> {
    let abs = std::path::Path::new(&row.working_directory);
    let rel = abs.strip_prefix(agents_root).ok()?;
    let rel_str = rel.to_string_lossy().to_string();
    if rel_str.is_empty() || rel_str == "." {
        return None;
    }
    Some(NamedAgentRecord {
        schema_version: MAX_SUPPORTED_SCHEMA,
        data: NamedAgentRecordV1 {
            instance_id: row.id.clone(),
            instance_name: row.instance_name.clone(),
            definition_id: row.definition_id.clone(),
            identity_id: empty_to_none(&row.identity_id),
            memory_id: empty_to_none(&row.memory_id),
            working_dir: rel_str,
            created_at_ms: row.created_at,
            last_launched_at_ms: row.started_at,
            // We don't know what version originally inserted these rows.
            // Tag them so post-migration audits can tell. PR C will
            // never overwrite a record so these stay forever.
            created_by_version: "(legacy)".to_string(),
            last_launched_by_version: "(legacy)".to_string(),
        },
    })
}

fn empty_to_none(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    /// Build a per-version SQLite at `<version_dir>/data/db/objects.db`
    /// with a minimal `db_agent_instances` schema and the given rows.
    fn make_version_db(version_dir: &Path, rows: &[(&str, &str, i64, &str)]) {
        let rows: Vec<_> = rows.iter().map(|(a, b, c, d)| (*a, *b, *c, *d, false)).collect();
        make_version_db_with_hidden(version_dir, &rows);
    }

    /// Variant that lets each row carry an explicit `display_hidden`
    /// flag. Used by tests that exercise tombstone propagation.
    fn make_version_db_with_hidden(
        version_dir: &Path,
        rows: &[(&str, &str, i64, &str, bool)],
    ) {
        let db_path = version_dir.join("data").join("db");
        std::fs::create_dir_all(&db_path).unwrap();
        let db_path = db_path.join("objects.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE db_agent_instances (
                id TEXT PRIMARY KEY,
                definition_id TEXT NOT NULL DEFAULT '',
                parent_instance_id TEXT NOT NULL DEFAULT '',
                block_id TEXT NOT NULL DEFAULT '',
                session_id TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'running',
                github_context TEXT NOT NULL DEFAULT '',
                started_at INTEGER NOT NULL DEFAULT 0,
                ended_at INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL DEFAULT 0,
                identity_id TEXT NOT NULL DEFAULT '',
                memory_id TEXT NOT NULL DEFAULT '',
                instance_name TEXT NOT NULL DEFAULT '',
                working_directory TEXT NOT NULL DEFAULT '',
                display_hidden INTEGER NOT NULL DEFAULT 0
            );",
        )
        .unwrap();
        for (id, name, started_at, working_directory, hidden) in rows {
            conn.execute(
                "INSERT INTO db_agent_instances
                    (id, definition_id, instance_name, working_directory, started_at, created_at, display_hidden)
                 VALUES (?1, 'claude-code', ?2, ?3, ?4, ?4, ?5)",
                params![id, name, working_directory, started_at, if *hidden { 1_i64 } else { 0_i64 }],
            )
            .unwrap();
        }
    }

    fn fresh_home() -> (tempfile::TempDir, Registry) {
        let home = tempfile::tempdir().unwrap();
        let reg = Registry::open(home.path().join("agents").join("registry")).unwrap();
        (home, reg)
    }

    #[test]
    fn migrate_with_no_versions_dir_writes_marker_and_no_rows() {
        let (home, reg) = fresh_home();
        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(stats.versions_scanned, 0);
        assert_eq!(stats.records_written, 0);
        assert!(reg.root().join(MARKER).exists());
    }

    #[test]
    fn migrate_is_idempotent() {
        let (home, reg) = fresh_home();
        // Empty home — marker gets written on first call.
        migrate_from_sqlite_once(home.path(), &reg).unwrap();
        // Add a version DB AFTER the marker — second run must NOT pick it up.
        let agents_root = home.path().join("agents");
        let v_dir = home.path().join("versions").join("0.33.821");
        let wd = agents_root.join("demo-1");
        std::fs::create_dir_all(&wd).unwrap();
        make_version_db(
            &v_dir,
            &[("inst-1", "demo", 100, &wd.to_string_lossy())],
        );
        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(
            stats.records_written, 0,
            "marker must short-circuit subsequent runs"
        );
        assert!(reg.list_active().unwrap().is_empty());
    }

    #[test]
    fn migrate_writes_one_record_per_unique_id() {
        let (home, reg) = fresh_home();
        let agents_root = home.path().join("agents");
        std::fs::create_dir_all(&agents_root).unwrap();
        let wd_a = agents_root.join("demo-a");
        let wd_b = agents_root.join("demo-b");
        std::fs::create_dir_all(&wd_a).unwrap();
        std::fs::create_dir_all(&wd_b).unwrap();
        let v_dir = home.path().join("versions").join("0.33.821");
        make_version_db(
            &v_dir,
            &[
                ("inst-a", "demoA", 100, &wd_a.to_string_lossy()),
                ("inst-b", "demoB", 200, &wd_b.to_string_lossy()),
            ],
        );

        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(stats.rows_seen, 2);
        assert_eq!(stats.records_written, 2);
        assert_eq!(reg.list_active().unwrap().len(), 2);
    }

    #[test]
    fn migrate_picks_latest_started_at_on_dedup() {
        let (home, reg) = fresh_home();
        let agents_root = home.path().join("agents");
        std::fs::create_dir_all(&agents_root).unwrap();
        let wd = agents_root.join("demo");
        std::fs::create_dir_all(&wd).unwrap();
        // Same instance_id in two versions, different started_at.
        let v1 = home.path().join("versions").join("0.33.800");
        let v2 = home.path().join("versions").join("0.33.821");
        make_version_db(&v1, &[("inst-1", "demo", 100, &wd.to_string_lossy())]);
        make_version_db(&v2, &[("inst-1", "demo", 200, &wd.to_string_lossy())]);

        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(stats.rows_seen, 2);
        assert_eq!(stats.records_written, 1);
        let recs = reg.list_active().unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].data.last_launched_at_ms, 200);
    }

    #[test]
    fn migrate_skips_when_registry_already_has_record() {
        let (home, reg) = fresh_home();
        let agents_root = home.path().join("agents");
        std::fs::create_dir_all(&agents_root).unwrap();
        let wd = agents_root.join("demo");
        std::fs::create_dir_all(&wd).unwrap();
        // Pre-existing registry record (e.g. PR A already wrote it).
        reg.upsert(&NamedAgentRecord {
            schema_version: MAX_SUPPORTED_SCHEMA,
            data: NamedAgentRecordV1 {
                instance_id: "inst-1".to_string(),
                instance_name: "preexisting".to_string(),
                definition_id: "claude-code".to_string(),
                identity_id: None,
                memory_id: None,
                working_dir: "demo".to_string(),
                created_at_ms: 50,
                last_launched_at_ms: 500,
                created_by_version: "0.33.823".to_string(),
                last_launched_by_version: "0.33.823".to_string(),
            },
        })
        .unwrap();
        // Legacy SQLite row with the SAME instance_id but older data.
        let v_dir = home.path().join("versions").join("0.33.821");
        make_version_db(&v_dir, &[("inst-1", "legacyname", 100, &wd.to_string_lossy())]);

        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(stats.records_skipped_existing, 1);
        assert_eq!(stats.records_written, 0);
        // Pre-existing record stays — name is "preexisting", not "legacyname".
        let recs = reg.list_active().unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].data.instance_name, "preexisting");
    }

    #[test]
    fn migrate_skips_when_record_is_retired() {
        // A user "Forgot" an agent (its registry file is in retired/).
        // Another version's SQLite still has display_hidden=0 for that
        // id. Migration must NOT resurrect the row into active/.
        let (home, reg) = fresh_home();
        let agents_root = home.path().join("agents");
        std::fs::create_dir_all(&agents_root).unwrap();
        let wd = agents_root.join("demo");
        std::fs::create_dir_all(&wd).unwrap();

        // Pre-tombstone a retired record.
        let retired_record = NamedAgentRecord {
            schema_version: MAX_SUPPORTED_SCHEMA,
            data: NamedAgentRecordV1 {
                instance_id: "inst-1".to_string(),
                instance_name: "demo".to_string(),
                definition_id: "claude-code".to_string(),
                identity_id: None,
                memory_id: None,
                working_dir: "demo".to_string(),
                created_at_ms: 50,
                last_launched_at_ms: 50,
                created_by_version: "0.33.823".to_string(),
                last_launched_by_version: "0.33.823".to_string(),
            },
        };
        reg.upsert(&retired_record).unwrap();
        reg.retire("inst-1").unwrap();
        assert!(reg.list_active().unwrap().is_empty());
        assert!(reg.exists_anywhere("inst-1"));

        // Legacy SQLite still has display_hidden=0 for the same id.
        let v_dir = home.path().join("versions").join("0.33.821");
        make_version_db(&v_dir, &[("inst-1", "demo", 100, &wd.to_string_lossy())]);

        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(stats.rows_seen, 1);
        assert_eq!(stats.records_skipped_existing, 1);
        assert_eq!(stats.records_written, 0);
        // Tombstone must remain in retired/, NOT moved to active.
        assert!(reg.list_active().unwrap().is_empty());
        assert!(reg.root().join("retired").join("inst-1.json").exists());
    }

    #[test]
    fn migrate_writes_legacy_hidden_row_as_tombstone() {
        // Pre-registry "Forget agent" intent must survive migration:
        // a single-version row with display_hidden=1 should land in
        // retired/, not active/.
        let (home, reg) = fresh_home();
        let agents_root = home.path().join("agents");
        std::fs::create_dir_all(&agents_root).unwrap();
        let wd = agents_root.join("forgotten");
        std::fs::create_dir_all(&wd).unwrap();
        let v_dir = home.path().join("versions").join("0.33.821");
        make_version_db_with_hidden(
            &v_dir,
            &[("inst-1", "forgotten", 100, &wd.to_string_lossy(), true)],
        );

        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(stats.records_written, 1);
        assert!(reg.list_active().unwrap().is_empty(),
            "hidden legacy row must NOT appear active");
        assert!(reg.root().join("retired").join("inst-1.json").exists(),
            "hidden legacy row must be migrated as retired tombstone");
    }

    #[test]
    fn migrate_preserves_forget_intent_across_versions() {
        // Same id in two versions: one hides it (Forget), the other
        // still has it visible. The "forget" must win — registry
        // tombstone, not active record.
        let (home, reg) = fresh_home();
        let agents_root = home.path().join("agents");
        std::fs::create_dir_all(&agents_root).unwrap();
        let wd = agents_root.join("toggled");
        std::fs::create_dir_all(&wd).unwrap();
        let v1 = home.path().join("versions").join("0.33.800");
        let v2 = home.path().join("versions").join("0.33.821");
        // v1 has it visible; v2 has it hidden (user later Forgot it).
        make_version_db_with_hidden(
            &v1,
            &[("inst-1", "toggled", 100, &wd.to_string_lossy(), false)],
        );
        make_version_db_with_hidden(
            &v2,
            &[("inst-1", "toggled", 200, &wd.to_string_lossy(), true)],
        );

        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(stats.records_written, 1);
        assert!(reg.list_active().unwrap().is_empty(),
            "hidden intent in any version must propagate to registry tombstone");
        assert!(reg.root().join("retired").join("inst-1.json").exists());
    }

    #[test]
    fn migrate_defers_marker_on_unreadable_db() {
        // A briefly-unreadable per-version DB during startup must NOT
        // bake "permanently skip" into the marker. Marker is only
        // written when every DB read succeeded.
        let (home, reg) = fresh_home();
        // Good DB.
        let agents_root = home.path().join("agents");
        std::fs::create_dir_all(&agents_root).unwrap();
        let wd = agents_root.join("demo");
        std::fs::create_dir_all(&wd).unwrap();
        let good_v = home.path().join("versions").join("0.33.821");
        make_version_db(&good_v, &[("inst-good", "demo", 100, &wd.to_string_lossy())]);
        // Bad DB — looks like a SQLite file but is corrupt.
        let bad_v = home.path().join("versions").join("0.33.800");
        let bad_db_dir = bad_v.join("data").join("db");
        std::fs::create_dir_all(&bad_db_dir).unwrap();
        std::fs::write(bad_db_dir.join("objects.db"), b"not actually sqlite").unwrap();

        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(stats.records_written, 1, "good DB still migrated");
        assert!(
            !reg.root().join(MARKER).exists(),
            "marker MUST NOT be written when any DB was unreadable"
        );

        // Next launch retries the migration; the good rows are
        // idempotency-skipped (exists_anywhere), and the bad DB now
        // works (simulate by replacing with a valid file pointing at
        // the same `wd` so we don't need a second working-dir
        // fixture).
        std::fs::remove_file(bad_db_dir.join("objects.db")).unwrap();
        make_version_db(&bad_v, &[("inst-other", "demo", 50, &wd.to_string_lossy())]);
        let stats2 = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert!(stats2.complete, "complete flag set on clean retry");
        assert!(
            reg.root().join(MARKER).exists(),
            "marker written on the retry once all DBs read successfully"
        );
        // Retry: 2 rows seen, 1 new written (inst-other), 1 skipped existing (inst-good).
        assert_eq!(stats2.records_skipped_existing, 1);
        assert_eq!(stats2.records_written, 1);
    }

    #[test]
    fn migrate_skips_unmappable_working_dirs() {
        let (home, reg) = fresh_home();
        // Working dir is OUTSIDE the agents root — unmappable.
        let v_dir = home.path().join("versions").join("0.33.821");
        let outside = home.path().join("not_under_agents").join("foo");
        make_version_db(&v_dir, &[("inst-x", "demo", 100, &outside.to_string_lossy())]);

        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(stats.rows_seen, 1);
        assert_eq!(stats.records_skipped_unmappable, 1);
        assert_eq!(stats.records_written, 0);
    }

    #[test]
    fn migrate_tolerates_missing_or_corrupt_dbs() {
        let (home, reg) = fresh_home();
        // Version dir with no DB file.
        std::fs::create_dir_all(home.path().join("versions").join("0.33.700")).unwrap();
        // Version dir with corrupt DB.
        let bad_v = home.path().join("versions").join("0.33.701");
        let db_dir = bad_v.join("data").join("db");
        std::fs::create_dir_all(&db_dir).unwrap();
        std::fs::write(db_dir.join("objects.db"), b"not a sqlite file").unwrap();

        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        // Both version dirs scanned; only one had a *file*, and it
        // failed to read — no panic, no rows migrated. Marker is
        // deferred so the next launch retries; see the dedicated
        // `migrate_defers_marker_on_unreadable_db` test.
        assert_eq!(stats.versions_scanned, 1);
        assert_eq!(stats.records_written, 0);
        assert!(
            !reg.root().join(MARKER).exists(),
            "marker deferred on unreadable DB"
        );
    }
}
