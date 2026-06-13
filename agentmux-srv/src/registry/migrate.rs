// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One-shot SQLite → file-registry migration for named **instances**.
//!
//! Runs at most once per `<registry_root>/.migrated_from_sqlite` marker;
//! idempotent and read-only on every SQLite it touches.
//!
//! P0.3 re-roots the registry to the GLOBAL `~/.agentmux/shared/agents/
//! registry/`, so this scan is generalized from "the current channel's
//! per-version DBs" to **every channel and every dev branch on the machine**:
//!
//! ```text
//!   <home>/channels/<ch>/versions/<v>/data/db/objects.db   (installed/portable)
//!   <home>/dev/<branch>[/<sub>]/data/db/objects.db         (dev)
//! ```
//!
//! **Landmine #1 — per-source anchoring.** A row's `working_directory` is
//! absolute under *its own* channel's agents dir (`channels/<ch>/agents`, or
//! `<instance_dir>/agents` for dev). Stripping every row against a single
//! global agents root would mark rows from other channels "unmappable", so
//! each source DB carries the agents dir its rows are anchored on, and
//! [`row_to_record`] strips against that. See
//! `docs/specs/SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md` §11.5.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use super::schema::{NamedAgentRecord, NamedAgentRecordV1, MAX_SUPPORTED_SCHEMA};
use super::store::{Registry, RegistryError};

/// Outcome stats — surfaced in the marker file + the srv log.
#[derive(Debug, Default, Clone, Copy)]
pub struct MigrateStats {
    /// Number of per-(channel,version) / per-dev-branch `objects.db` files
    /// scanned (was "versions" when the scan was single-channel).
    pub dbs_scanned: usize,
    /// DBs that existed but couldn't be read (corrupt / locked). Counted and
    /// skipped — they must NOT disable the whole cross-channel registry now
    /// that the scan spans every channel (codex P1 on #1389).
    pub dbs_skipped: usize,
    pub rows_seen: usize,
    pub records_written: usize,
    pub records_skipped_existing: usize,
    pub records_skipped_unmappable: usize,
    /// True iff every DB read cleanly. Controls only whether the one-shot
    /// **marker** is written: on any skipped DB the marker is deferred so a
    /// future launch retries that (possibly transiently-unreadable) DB. It
    /// does NOT gate registry attachment — `main.rs` attaches whenever the
    /// migration runs, so one bad DB in an unrelated channel can't disable
    /// cross-channel My Agents (the readable records are served now; the live
    /// mirror backfills the current channel regardless).
    pub complete: bool,
}

/// Marker filename. Lives in the registry root so the registry's existence
/// implies the migration question has been asked at least once.
const MARKER: &str = ".migrated_from_sqlite";

/// A per-(channel,version) / per-dev-branch SQLite source, paired with the
/// agents dir whose subtree its `working_directory` values are absolute under.
struct SqliteSource {
    db_path: PathBuf,
    agents_root: PathBuf,
}

/// Scan every channel + dev `objects.db` under `home` and populate the shared
/// registry. Skipped if the marker file exists. Never overwrites an existing
/// registry record (idempotency + respect for newer-written data). The SQLite
/// files are opened **read-only** — never modified.
///
/// On dedup conflicts (same `instance_id` in multiple versions/channels), the
/// row with the latest `started_at` wins; any version expressing "forget"
/// intent (`display_hidden`) is preserved as a tombstone.
pub fn migrate_from_sqlite_once(
    home: &Path,
    registry: &Registry,
) -> Result<MigrateStats, RegistryError> {
    let marker_path = registry.root().join(MARKER);
    if marker_path.exists() {
        // Marker present ⇒ a prior run completed; treat as complete so
        // callers attach the registry.
        return Ok(MigrateStats {
            complete: true,
            ..MigrateStats::default()
        });
    }

    let mut stats = MigrateStats::default();
    let sources = enumerate_sources(home);

    let mut latest_by_id: HashMap<String, RowSnapshot> = HashMap::new();
    // True iff any DB threw a non-transient-looking error. We use this to skip
    // writing the marker so the next launch retries — otherwise a brief
    // filesystem hiccup permanently omits those rows from the registry-backed
    // dropdown.
    let mut any_db_failed = false;

    for src in sources {
        stats.dbs_scanned += 1;
        match read_named_rows(&src.db_path, &src.agents_root) {
            Ok(rows) => {
                for row in rows {
                    stats.rows_seen += 1;
                    let key = row.id.clone();
                    match latest_by_id.get_mut(&key) {
                        Some(existing) if existing.started_at >= row.started_at => {
                            // Existing snapshot wins on started_at, but OR the
                            // hidden flag — any version expressing "forget"
                            // intent is preserved as a tombstone.
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
                    db = %src.db_path.display(),
                    error = %e,
                    "registry-migrate: DB unreadable — skipping this source; registry still attaches, marker deferred to retry"
                );
                stats.dbs_skipped += 1;
                any_db_failed = true;
            }
        }
    }

    for (id, row) in latest_by_id {
        // Check active AND retired — a record retired by a newer version's
        // "Forget agent" must NOT be resurrected just because an older
        // version's SQLite still lists it as visible.
        if registry.exists_anywhere(&id) {
            stats.records_skipped_existing += 1;
            continue;
        }
        let display_hidden = row.display_hidden;
        let Some(rec) = row_to_record(&row) else {
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
        // Preserve pre-registry "forget" intent: if any version's SQLite had
        // this row hidden, move the freshly-written registry file to retired/
        // so the dropdown stays consistent with the user's prior soft-delete.
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

    // `complete` is true only when every DB we encountered was readable. It
    // gates ONLY the one-shot marker: on any skipped DB the marker is deferred
    // so a future launch retries that source. It does NOT detach the registry —
    // main.rs attaches whenever the migration returns Ok and serves the records
    // that did read (codex P1 on #1389); see the field doc above.
    stats.complete = !any_db_failed;
    if stats.complete {
        write_marker(&marker_path, &stats)?;
    } else {
        tracing::info!(
            "registry-migrate: deferring marker write; one or more DBs were unreadable and will be retried next launch"
        );
    }
    Ok(stats)
}

/// Enumerate every per-(channel,version) and per-dev-branch `objects.db` under
/// `home`, pairing each with the agents dir its rows are anchored on.
fn enumerate_sources(home: &Path) -> Vec<SqliteSource> {
    let mut out = Vec::new();

    // Installed/portable: home/channels/<ch>/versions/<v>/data/db/objects.db,
    // anchored on home/channels/<ch>/agents.
    if let Ok(rd) = std::fs::read_dir(home.join("channels")) {
        for ch in rd.flatten() {
            let ch_dir = ch.path();
            if !ch_dir.is_dir() {
                continue;
            }
            let agents_root = ch_dir.join("agents");
            if let Ok(vrd) = std::fs::read_dir(ch_dir.join("versions")) {
                for v in vrd.flatten() {
                    let db = v.path().join("data").join("db").join("objects.db");
                    if db.is_file() {
                        out.push(SqliteSource {
                            db_path: db,
                            agents_root: agents_root.clone(),
                        });
                    }
                }
            }
        }
    }

    // Dev: home/dev/<branch>[/<sub>]/data/db/objects.db, anchored on
    // <instance_dir>/agents (instance_dir = the dir that holds `data`). The
    // depth under dev/ varies (branch, sometimes branch/sub-hash), so search
    // a bounded number of levels for a `data/db/objects.db`.
    collect_dev_sources(&home.join("dev"), &mut out, 0);

    out
}

/// Instance-internal subdirectories (`DataPaths::ensure_dirs`) — never branch
/// or sub-hash containers. The dev walk must NOT descend into these: an agent
/// workspace under `<instance>/agents/<slug>/` can itself contain a nested
/// AgentMux `data/db/objects.db` (e.g. an agent that ran its own instance),
/// which must not be mistaken for a migration source (reagent P2 on #1389).
const DEV_INTERNAL_DIRS: &[&str] =
    &["data", "agents", "logs", "cef-cache", "runtime", "config"];

/// Recursively locate dev instance dirs (those with `data/db/objects.db`) and
/// anchor each on its sibling `agents/` dir. An instance dir is a leaf (we stop
/// descending once a DB is found), and descent skips instance-internal dirs so
/// the walk only traverses branch / sub-hash containers. Bounded depth is a
/// backstop against pathological/looping trees.
fn collect_dev_sources(dir: &Path, out: &mut Vec<SqliteSource>, depth: usize) {
    if depth > 4 {
        return;
    }
    let db = dir.join("data").join("db").join("objects.db");
    if db.is_file() {
        out.push(SqliteSource {
            db_path: db,
            agents_root: dir.join("agents"),
        });
        return;
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if !e.path().is_dir() {
                continue;
            }
            // Skip instance internals (esp. `agents/`) so we never walk into an
            // agent's workspace and pick up a nested objects.db as a source.
            if e.file_name()
                .to_str()
                .is_some_and(|n| DEV_INTERNAL_DIRS.contains(&n))
            {
                continue;
            }
            collect_dev_sources(&e.path(), out, depth + 1);
        }
    }
}

fn write_marker(path: &Path, stats: &MigrateStats) -> std::io::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let body = format!(
        "migrated_at: {now}\n\
         dbs_scanned: {}\n\
         dbs_skipped: {}\n\
         rows_seen: {}\n\
         records_written: {}\n\
         records_skipped_existing: {}\n\
         records_skipped_unmappable: {}\n",
        stats.dbs_scanned,
        stats.dbs_skipped,
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
    /// The agents dir this row's `working_directory` is absolute under — the
    /// source channel's `agents/` (landmine #1). Travels with the row through
    /// dedup so the winner is stripped against its own channel.
    agents_root: PathBuf,
    started_at: i64,
    created_at: i64,
    display_hidden: bool,
}

/// True iff the error is SQLite reporting "this column/table doesn't exist in
/// this DB's schema." Distinguishes a pre-v8 DB (skip silently — those agents
/// weren't named, so wouldn't appear in the dropdown anyway) from corruption
/// (caller logs + continues + defers the marker).
///
/// rusqlite reports prepare-time schema mismatches as
/// `Error::SqlInputError { msg, sql, offset }` and runtime errors as
/// `Error::SqliteFailure(_, Some(msg))`. Both shapes carry the canonical
/// SQLite phrases, but only message inspection distinguishes them from other
/// failures with the same error code.
fn is_missing_column_or_table(e: &rusqlite::Error) -> bool {
    let msg = match e {
        rusqlite::Error::SqliteFailure(_, Some(msg)) => msg.as_str(),
        rusqlite::Error::SqlInputError { msg, .. } => msg.as_str(),
        _ => return false,
    };
    msg.starts_with("no such column") || msg.starts_with("no such table")
}

fn read_named_rows(
    db_path: &Path,
    agents_root: &Path,
) -> Result<Vec<RowSnapshot>, rusqlite::Error> {
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    // Older schemas (pre-v8) lack `instance_name` / `working_directory`
    // columns. Suppress ONLY the specific "no such column/table" errors —
    // broader SqliteFailures (corruption, locked, etc.) must surface so the
    // caller can log + continue with the next DB. Include hidden rows — the
    // caller turns them into retired/ tombstones so a pre-registry "Forget
    // agent" intent survives migration even if another version still has the
    // row visible.
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
            agents_root: agents_root.to_path_buf(),
            started_at: row.get(6)?,
            created_at: row.get(7)?,
            display_hidden: row.get::<_, i64>(8)? != 0,
        })
    })?;
    iter.collect()
}

fn row_to_record(row: &RowSnapshot) -> Option<NamedAgentRecord> {
    let abs = std::path::Path::new(&row.working_directory);
    // Strip against THIS row's own source-channel agents dir (landmine #1),
    // not a single global root — else rows from other channels look unmappable.
    let rel = abs.strip_prefix(&row.agents_root).ok()?;
    let rel_str = rel.to_string_lossy().to_string();
    if rel_str.is_empty() || rel_str == "." {
        return None;
    }
    let data = NamedAgentRecordV1 {
        instance_id: row.id.clone(),
        instance_name: row.instance_name.clone(),
        definition_id: row.definition_id.clone(),
        identity_id: empty_to_none(&row.identity_id),
        memory_id: empty_to_none(&row.memory_id),
        // The legacy per-channel rows don't carry session_id through this
        // consolidation path; live mirroring (registry_mirror.rs) populates it
        // on the next launch/update. Until then these stay session-less (v1)
        // and v1-binary-readable.
        session_id: None,
        working_dir: rel_str,
        created_at_ms: row.created_at,
        last_launched_at_ms: row.started_at,
        // We don't know what version originally inserted these rows. Tag them
        // so post-migration audits can tell. The migration never overwrites a
        // record (exists_anywhere skip) so these stay.
        created_by_version: "(legacy)".to_string(),
        last_launched_by_version: "(legacy)".to_string(),
    };
    Some(NamedAgentRecord {
        schema_version: data.min_schema_version(),
        data,
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

    /// Build a per-version SQLite at `<version_dir>/data/db/objects.db` with a
    /// minimal `db_agent_instances` schema and the given rows.
    fn make_db_at(version_dir: &Path, rows: &[(&str, &str, i64, &str, bool)]) {
        let db_dir = version_dir.join("data").join("db");
        std::fs::create_dir_all(&db_dir).unwrap();
        let conn = Connection::open(db_dir.join("objects.db")).unwrap();
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

    /// The agents dir for a channel under `home`.
    fn channel_agents(home: &Path, channel: &str) -> PathBuf {
        home.join("channels").join(channel).join("agents")
    }

    /// Build a channel/version DB with the given (id, name, started_at, wd) rows.
    fn make_channel_db(
        home: &Path,
        channel: &str,
        version: &str,
        rows: &[(&str, &str, i64, &str)],
    ) {
        let rows: Vec<_> = rows.iter().map(|(a, b, c, d)| (*a, *b, *c, *d, false)).collect();
        let v_dir = home
            .join("channels")
            .join(channel)
            .join("versions")
            .join(version);
        make_db_at(&v_dir, &rows);
    }

    fn make_channel_db_with_hidden(
        home: &Path,
        channel: &str,
        version: &str,
        rows: &[(&str, &str, i64, &str, bool)],
    ) {
        let v_dir = home
            .join("channels")
            .join(channel)
            .join("versions")
            .join(version);
        make_db_at(&v_dir, rows);
    }

    fn fresh_home() -> (tempfile::TempDir, Registry) {
        let home = tempfile::tempdir().unwrap();
        // Registry rooted at the GLOBAL shared location, mirroring production.
        let reg = Registry::open(
            home.path()
                .join("shared")
                .join("agents")
                .join("registry"),
        )
        .unwrap();
        (home, reg)
    }

    #[test]
    fn migrate_with_no_channels_writes_marker_and_no_rows() {
        let (home, reg) = fresh_home();
        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(stats.dbs_scanned, 0);
        assert_eq!(stats.records_written, 0);
        assert!(reg.root().join(MARKER).exists());
    }

    #[test]
    fn migrate_is_idempotent() {
        let (home, reg) = fresh_home();
        // Empty home — marker gets written on first call.
        migrate_from_sqlite_once(home.path(), &reg).unwrap();
        // Add a channel DB AFTER the marker — second run must NOT pick it up.
        let wd = channel_agents(home.path(), "stable").join("demo-1");
        std::fs::create_dir_all(&wd).unwrap();
        make_channel_db(
            home.path(),
            "stable",
            "0.33.821",
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
        let agents = channel_agents(home.path(), "stable");
        let wd_a = agents.join("demo-a");
        let wd_b = agents.join("demo-b");
        std::fs::create_dir_all(&wd_a).unwrap();
        std::fs::create_dir_all(&wd_b).unwrap();
        make_channel_db(
            home.path(),
            "stable",
            "0.33.821",
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
    fn migrate_anchors_each_row_on_its_own_channel() {
        // Landmine #1: two agents in two DIFFERENT channels, each
        // working_directory absolute under its OWN channel's agents dir. Both
        // must migrate with the correct relative slug — neither is dropped as
        // "unmappable" just because the other channel's agents root differs.
        let (home, reg) = fresh_home();
        let wd_a = channel_agents(home.path(), "stable").join("alpha");
        let wd_b = channel_agents(home.path(), "local-main-b28b7a").join("beta");
        std::fs::create_dir_all(&wd_a).unwrap();
        std::fs::create_dir_all(&wd_b).unwrap();
        make_channel_db(
            home.path(),
            "stable",
            "0.44.2",
            &[("inst-a", "Alpha", 100, &wd_a.to_string_lossy())],
        );
        make_channel_db(
            home.path(),
            "local-main-b28b7a",
            "0.44.2",
            &[("inst-b", "Beta", 200, &wd_b.to_string_lossy())],
        );

        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(stats.dbs_scanned, 2);
        assert_eq!(stats.rows_seen, 2);
        assert_eq!(stats.records_skipped_unmappable, 0, "per-channel anchoring");
        assert_eq!(stats.records_written, 2);
        let mut recs = reg.list_active().unwrap();
        recs.sort_by(|a, b| a.data.instance_id.cmp(&b.data.instance_id));
        assert_eq!(recs[0].data.working_dir, "alpha");
        assert_eq!(recs[1].data.working_dir, "beta");
    }

    #[test]
    fn migrate_picks_latest_started_at_on_dedup() {
        let (home, reg) = fresh_home();
        let wd = channel_agents(home.path(), "stable").join("demo");
        std::fs::create_dir_all(&wd).unwrap();
        // Same instance_id in two versions of the same channel, different
        // started_at.
        make_channel_db(
            home.path(),
            "stable",
            "0.33.800",
            &[("inst-1", "demo", 100, &wd.to_string_lossy())],
        );
        make_channel_db(
            home.path(),
            "stable",
            "0.33.821",
            &[("inst-1", "demo", 200, &wd.to_string_lossy())],
        );

        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(stats.rows_seen, 2);
        assert_eq!(stats.records_written, 1);
        let recs = reg.list_active().unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].data.last_launched_at_ms, 200);
    }

    #[test]
    fn migrate_handles_dev_layout() {
        // Dev instance lives at home/dev/<branch>/<sub>/data/db/objects.db,
        // anchored on home/dev/<branch>/<sub>/agents. The recursive dev walk
        // must find it and strip against the sibling agents dir.
        let (home, reg) = fresh_home();
        let inst_dir = home.path().join("dev").join("mybranch").join("69d7a34a");
        let wd = inst_dir.join("agents").join("devagent");
        std::fs::create_dir_all(&wd).unwrap();
        make_db_at(
            &inst_dir,
            &[("inst-dev", "DevAgent", 100, &wd.to_string_lossy(), false)],
        );

        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(stats.dbs_scanned, 1);
        assert_eq!(stats.records_written, 1);
        let recs = reg.list_active().unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].data.working_dir, "devagent");
    }

    #[test]
    fn migrate_dev_ignores_nested_agent_workspace_db() {
        // An agent running inside a dev instance can create its OWN nested
        // AgentMux data dir under <instance>/agents/<slug>/. The dev walk must
        // NOT descend into agents/ and pick that nested objects.db up as a
        // migration source (reagent P2 on #1389).
        let (home, reg) = fresh_home();
        let inst_dir = home.path().join("dev").join("mybranch").join("sub");
        // NO instance-level DB — only a nested one inside an agent workspace.
        let nested_inst = inst_dir.join("agents").join("nestedmux");
        let nested_wd = nested_inst.join("agents").join("inner");
        std::fs::create_dir_all(&nested_wd).unwrap();
        make_db_at(
            &nested_inst,
            &[("inst-nested", "Nested", 100, &nested_wd.to_string_lossy(), false)],
        );

        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(
            stats.dbs_scanned, 0,
            "must not descend into agent workspaces under dev/"
        );
        assert!(reg.list_active().unwrap().is_empty());
    }

    #[test]
    fn migrate_dev_picks_instance_db_and_ignores_nested() {
        // With an instance-level DB present, the walk stops at the instance dir
        // (leaf) and never descends into its agents/ — so a nested workspace DB
        // is ignored even when the real instance DB exists.
        let (home, reg) = fresh_home();
        let inst_dir = home.path().join("dev").join("mybranch").join("sub");
        let wd = inst_dir.join("agents").join("realagent");
        std::fs::create_dir_all(&wd).unwrap();
        make_db_at(
            &inst_dir,
            &[("inst-real", "Real", 100, &wd.to_string_lossy(), false)],
        );
        let nested_inst = inst_dir.join("agents").join("nestedmux");
        let nested_wd = nested_inst.join("agents").join("inner");
        std::fs::create_dir_all(&nested_wd).unwrap();
        make_db_at(
            &nested_inst,
            &[("inst-nested", "Nested", 200, &nested_wd.to_string_lossy(), false)],
        );

        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(stats.dbs_scanned, 1, "only the instance-level dev DB is a source");
        let recs = reg.list_active().unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].data.instance_id, "inst-real");
        assert_eq!(recs[0].data.working_dir, "realagent");
    }

    #[test]
    fn migrate_skips_when_registry_already_has_record() {
        let (home, reg) = fresh_home();
        let wd = channel_agents(home.path(), "stable").join("demo");
        std::fs::create_dir_all(&wd).unwrap();
        // Pre-existing registry record (e.g. the live mirror already wrote it).
        reg.upsert(&NamedAgentRecord {
            schema_version: MAX_SUPPORTED_SCHEMA,
            data: NamedAgentRecordV1 {
                instance_id: "inst-1".to_string(),
                instance_name: "preexisting".to_string(),
                definition_id: "claude-code".to_string(),
                identity_id: None,
                memory_id: None,
                session_id: None,
                working_dir: "demo".to_string(),
                created_at_ms: 50,
                last_launched_at_ms: 500,
                created_by_version: "0.33.823".to_string(),
                last_launched_by_version: "0.33.823".to_string(),
            },
        })
        .unwrap();
        // Legacy SQLite row with the SAME instance_id but older data.
        make_channel_db(
            home.path(),
            "stable",
            "0.33.821",
            &[("inst-1", "legacyname", 100, &wd.to_string_lossy())],
        );

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
        // A user "Forgot" an agent (its registry file is in retired/). Another
        // version's SQLite still has display_hidden=0 for that id. Migration
        // must NOT resurrect the row into active/.
        let (home, reg) = fresh_home();
        let wd = channel_agents(home.path(), "stable").join("demo");
        std::fs::create_dir_all(&wd).unwrap();

        let retired_record = NamedAgentRecord {
            schema_version: MAX_SUPPORTED_SCHEMA,
            data: NamedAgentRecordV1 {
                instance_id: "inst-1".to_string(),
                instance_name: "demo".to_string(),
                definition_id: "claude-code".to_string(),
                identity_id: None,
                memory_id: None,
                session_id: None,
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

        make_channel_db(
            home.path(),
            "stable",
            "0.33.821",
            &[("inst-1", "demo", 100, &wd.to_string_lossy())],
        );

        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(stats.rows_seen, 1);
        assert_eq!(stats.records_skipped_existing, 1);
        assert_eq!(stats.records_written, 0);
        assert!(reg.list_active().unwrap().is_empty());
        assert!(reg.root().join("retired").join("inst-1.json").exists());
    }

    #[test]
    fn migrate_silently_skips_pre_v8_schema() {
        // An older SQLite (pre-v8) lacks the `instance_name` column. rusqlite
        // reports this as `Error::SqlInputError` during prepare. The migrator
        // must treat it as "nothing to migrate from this version" — NOT a real
        // DB failure that defers the marker.
        let (home, reg) = fresh_home();
        let db_dir = home
            .path()
            .join("channels")
            .join("stable")
            .join("versions")
            .join("0.33.643")
            .join("data")
            .join("db");
        std::fs::create_dir_all(&db_dir).unwrap();
        let conn = Connection::open(db_dir.join("objects.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE db_agent_instances (
                id TEXT PRIMARY KEY,
                definition_id TEXT NOT NULL DEFAULT '',
                parent_instance_id TEXT NOT NULL DEFAULT '',
                started_at INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL DEFAULT 0
            );",
        )
        .unwrap();
        drop(conn);

        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(stats.dbs_scanned, 1);
        assert_eq!(stats.rows_seen, 0);
        assert!(stats.complete, "pre-v8 schema must not block the marker");
        assert!(reg.root().join(MARKER).exists());
    }

    #[test]
    fn migrate_writes_legacy_hidden_row_as_tombstone() {
        // Pre-registry "Forget agent" intent must survive migration: a
        // single-version row with display_hidden=1 should land in retired/.
        let (home, reg) = fresh_home();
        let wd = channel_agents(home.path(), "stable").join("forgotten");
        std::fs::create_dir_all(&wd).unwrap();
        make_channel_db_with_hidden(
            home.path(),
            "stable",
            "0.33.821",
            &[("inst-1", "forgotten", 100, &wd.to_string_lossy(), true)],
        );

        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(stats.records_written, 1);
        assert!(
            reg.list_active().unwrap().is_empty(),
            "hidden legacy row must NOT appear active"
        );
        assert!(
            reg.root().join("retired").join("inst-1.json").exists(),
            "hidden legacy row must be migrated as retired tombstone"
        );
    }

    #[test]
    fn migrate_preserves_forget_intent_across_versions() {
        // Same id in two versions: one hides it (Forget), the other still has
        // it visible. The "forget" must win — registry tombstone, not active.
        let (home, reg) = fresh_home();
        let wd = channel_agents(home.path(), "stable").join("toggled");
        std::fs::create_dir_all(&wd).unwrap();
        make_channel_db_with_hidden(
            home.path(),
            "stable",
            "0.33.800",
            &[("inst-1", "toggled", 100, &wd.to_string_lossy(), false)],
        );
        make_channel_db_with_hidden(
            home.path(),
            "stable",
            "0.33.821",
            &[("inst-1", "toggled", 200, &wd.to_string_lossy(), true)],
        );

        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(stats.records_written, 1);
        assert!(
            reg.list_active().unwrap().is_empty(),
            "hidden intent in any version must propagate to registry tombstone"
        );
        assert!(reg.root().join("retired").join("inst-1.json").exists());
    }

    #[test]
    fn migrate_defers_marker_on_unreadable_db() {
        // A briefly-unreadable DB during startup must NOT bake "permanently
        // skip" into the marker. Marker is only written when every DB read.
        let (home, reg) = fresh_home();
        let wd = channel_agents(home.path(), "stable").join("demo");
        std::fs::create_dir_all(&wd).unwrap();
        make_channel_db(
            home.path(),
            "stable",
            "0.33.821",
            &[("inst-good", "demo", 100, &wd.to_string_lossy())],
        );
        // Bad DB — looks like a SQLite file but is corrupt.
        let bad_db_dir = home
            .path()
            .join("channels")
            .join("stable")
            .join("versions")
            .join("0.33.800")
            .join("data")
            .join("db");
        std::fs::create_dir_all(&bad_db_dir).unwrap();
        std::fs::write(bad_db_dir.join("objects.db"), b"not actually sqlite").unwrap();

        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(stats.records_written, 1, "good DB still migrated");
        assert_eq!(stats.dbs_skipped, 1, "bad DB counted, not fatal");
        assert!(
            !reg.root().join(MARKER).exists(),
            "marker deferred (retry) when a DB was unreadable — but the registry still attaches (see main.rs); good records are already written"
        );
        // The good record is present regardless of the deferred marker — a bad
        // unrelated DB must not hide cross-channel agents (codex P1 on #1389).
        assert_eq!(reg.list_active().unwrap().len(), 1);

        // Next launch retries; the good row is idempotency-skipped, and the
        // bad DB now reads (replaced with a valid file under the same wd).
        std::fs::remove_file(bad_db_dir.join("objects.db")).unwrap();
        make_channel_db(
            home.path(),
            "stable",
            "0.33.800",
            &[("inst-other", "demo", 50, &wd.to_string_lossy())],
        );
        let stats2 = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert!(stats2.complete, "complete flag set on clean retry");
        assert!(
            reg.root().join(MARKER).exists(),
            "marker written on the retry once all DBs read successfully"
        );
        assert_eq!(stats2.records_skipped_existing, 1);
        assert_eq!(stats2.records_written, 1);
    }

    #[test]
    fn migrate_skips_unmappable_working_dirs() {
        let (home, reg) = fresh_home();
        // Working dir is OUTSIDE any channel agents root — unmappable.
        let outside = home.path().join("not_under_agents").join("foo");
        make_channel_db(
            home.path(),
            "stable",
            "0.33.821",
            &[("inst-x", "demo", 100, &outside.to_string_lossy())],
        );

        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(stats.rows_seen, 1);
        assert_eq!(stats.records_skipped_unmappable, 1);
        assert_eq!(stats.records_written, 0);
    }

    #[test]
    fn migrate_tolerates_missing_or_corrupt_dbs() {
        let (home, reg) = fresh_home();
        // Channel/version dir with no DB file.
        std::fs::create_dir_all(
            home.path()
                .join("channels")
                .join("stable")
                .join("versions")
                .join("0.33.700"),
        )
        .unwrap();
        // Channel/version dir with corrupt DB.
        let db_dir = home
            .path()
            .join("channels")
            .join("stable")
            .join("versions")
            .join("0.33.701")
            .join("data")
            .join("db");
        std::fs::create_dir_all(&db_dir).unwrap();
        std::fs::write(db_dir.join("objects.db"), b"not a sqlite file").unwrap();

        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        // Only one had a *file*, and it failed to read — no panic, no rows.
        // Marker deferred so the next launch retries; the bad DB is counted.
        assert_eq!(stats.dbs_scanned, 1);
        assert_eq!(stats.dbs_skipped, 1);
        assert_eq!(stats.records_written, 0);
        assert!(
            !reg.root().join(MARKER).exists(),
            "marker deferred on unreadable DB"
        );
    }
}
