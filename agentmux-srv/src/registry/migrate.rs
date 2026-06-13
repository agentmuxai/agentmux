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
//! **Workspace anchoring.** Agent workspaces live GLOBALLY at
//! `<home>/agents/<name>` (verified on disk: real `working_directory` values
//! are `~/.agentmux/agents/<name>`, e.g. `…/agents/mazs-0527n`), independent of
//! channel/version. [`row_to_record`] therefore strips each row's
//! `working_directory` against the global `<home>/agents` root FIRST, then
//! falls back to this row's own source-channel agents dir (`channels/<ch>/agents`,
//! or `<instance_dir>/agents` for dev) for any legacy row that genuinely lived
//! in-channel — each source DB still carries that per-source dir. The two
//! subtrees are disjoint, so the fallback never mis-maps a global workspace.
//! The chosen base is stored absolute in the record, so a reader in ANY channel
//! round-trips `source_agents_base.join(working_dir)` back to the real
//! workspace. See `docs/specs/SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md` §11.5.

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

/// Bumped when the migration's mapping logic changes in a way that must re-run
/// on registries an older build already finalized. **v2** fixes the workspace
/// anchor: agent workspaces live globally at `<home>/agents/<name>`, but v1
/// stripped `working_directory` against the per-channel `channels/<ch>/agents`
/// dir, so every global workspace came back "unmappable" (`row_to_record`
/// returned `None`) and "My Agents" stayed empty in every channel. A legacy
/// marker (no `migration_version:` line) reads as 0 and re-runs exactly once;
/// `exists_anywhere()` keeps the re-run from duplicating already-written
/// records.
const MIGRATION_VERSION: u32 = 2;

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
    if marker_migration_version(&marker_path) >= MIGRATION_VERSION {
        // A prior run AT THE CURRENT logic version completed; treat as complete
        // so callers attach the registry. An older-version (or legacy,
        // no-version) marker falls through and re-runs once — `exists_anywhere`
        // below keeps it from duplicating records already written.
        return Ok(MigrateStats {
            complete: true,
            ..MigrateStats::default()
        });
    }

    let mut stats = MigrateStats::default();
    // Agent workspaces are created GLOBALLY at `<home>/agents/<name>` — verified
    // on disk: instance `working_directory` values are `~/.agentmux/agents/<name>`
    // (e.g. `…/agents/mazs-0527n`), NOT under any per-channel `channels/<ch>/agents`
    // dir, and NOT under the P0.3-re-rooted registry's parent (`<home>/shared/
    // agents`). `home` is `~/.agentmux` (main.rs derives it as the registry root's
    // 3rd ancestor: registry → agents → shared → home). Anchor migrated records on
    // this real workspace root so the reader (`agent_handlers.rs` reconstructs
    // `source_agents_base.join(working_dir)`) resolves the actual workspace in
    // EVERY channel. NB this deliberately differs from the live mirror, which
    // strips against the per-channel `AGENTMUX_AGENTS_DIR` — that anchor never
    // matched these global workspaces (an earlier draft of this fix wrongly used
    // the registry's parent and would have left every row unmappable).
    let global_agents_root = home.join("agents");
    let (sources, enum_incomplete) = enumerate_sources(home);

    let mut latest_by_id: HashMap<String, RowSnapshot> = HashMap::new();
    // True iff any DB threw a non-transient-looking error OR a directory that
    // should have been enumerable was unreadable. We use this to skip writing
    // the marker so the next launch retries — otherwise a brief filesystem
    // hiccup permanently omits those rows from the registry-backed dropdown.
    let mut any_db_failed = enum_incomplete;

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
        let Some(rec) = row_to_record(&row, &global_agents_root) else {
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

/// Marker for the one-shot `source_agents_base` backfill. Separate from
/// [`MARKER`] so it runs exactly once even on registries the main migration
/// already finalized before schema v3 existed.
const SOURCE_BACKFILL_MARKER: &str = ".backfilled_source_bases";

/// Outcome of [`backfill_source_bases_once`].
#[derive(Debug, Default, Clone, Copy)]
pub struct SourceBackfillStats {
    pub dbs_scanned: usize,
    pub records_updated: usize,
    /// Records that lacked a source base but whose source DB wasn't found
    /// (its channel was deleted) — left as-is; a relaunch's live mirror
    /// backfills them.
    pub records_unresolved: usize,
    pub complete: bool,
}

/// One-shot backfill of `source_agents_base` onto registry records written
/// **before** schema v3 — i.e. by P0.3b's global migration (#1389) or any
/// pre-P0.4 live mirror.
///
/// The main [`migrate_from_sqlite_once`] short-circuits on `.migrated_from_sqlite`,
/// so an already-migrated registry never re-runs `row_to_record` and its
/// records keep `source_agents_base: None`. A cross-channel `listnamedagents`
/// read then re-joins `working_dir` under the READER's own channel and resolves
/// the wrong workspace / `--resume` cwd. This pass re-derives each record's
/// source channel from the SQLite sources and sets **only** `source_agents_base`,
/// preserving every live-mirror-enriched field (session_id, identity,
/// timestamps) — it never blind-upserts the SQLite-derived record. Guarded by
/// its own marker; idempotent; read-only on SQLite.
pub fn backfill_source_bases_once(
    home: &Path,
    registry: &Registry,
) -> Result<SourceBackfillStats, RegistryError> {
    let marker = registry.root().join(SOURCE_BACKFILL_MARKER);
    if marker.exists() {
        return Ok(SourceBackfillStats {
            complete: true,
            ..Default::default()
        });
    }

    let mut stats = SourceBackfillStats::default();

    // Active records still missing a source base, keyed by id. We mutate these
    // snapshots in place and re-upsert, so the existing session_id/identity/
    // timestamps survive. (Retired/forgotten records are out of scope — a
    // relaunch re-mirrors them with the current channel base.)
    let mut pending: HashMap<String, NamedAgentRecord> = registry
        .list_active()?
        .into_iter()
        .filter(|r| r.data.source_agents_base.is_none())
        .map(|r| (r.data.instance_id.clone(), r))
        .collect();

    if pending.is_empty() {
        // Fresh registry, or every record is already v3 — nothing to do.
        std::fs::write(&marker, b"backfilled: 0\n")?;
        stats.complete = true;
        return Ok(stats);
    }

    let (sources, mut incomplete) = enumerate_sources(home);

    // Dedup EXACTLY like migrate_from_sqlite_once: for an id present in more
    // than one DB the latest-`started_at` row wins. This anchors
    // source_agents_base on the SAME channel the record's existing working_dir
    // was stripped against (the migration used that same winning row), instead
    // of whichever DB read_dir happened to return first (codex/reagent P2).
    let mut winners: HashMap<String, (i64, PathBuf)> = HashMap::new();
    for src in sources {
        stats.dbs_scanned += 1;
        match read_named_rows(&src.db_path, &src.agents_root) {
            Ok(rows) => {
                for row in rows {
                    if !pending.contains_key(&row.id) {
                        continue;
                    }
                    match winners.get(&row.id) {
                        Some((ts, _)) if *ts >= row.started_at => {}
                        _ => {
                            winners.insert(row.id.clone(), (row.started_at, row.agents_root));
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    db = %src.db_path.display(),
                    error = %e,
                    "source-base backfill: DB unreadable — will retry next launch"
                );
                incomplete = true;
            }
        }
    }

    // Apply the winning channel's agents dir to each record (only the source
    // base; everything else is preserved from the existing record).
    for (id, (_, agents_root)) in winners {
        let Some(mut rec) = pending.remove(&id) else {
            continue;
        };
        rec.data.source_agents_base = Some(agents_root.to_string_lossy().to_string());
        rec.schema_version = rec.data.min_schema_version();
        if let Err(e) = registry.upsert(&rec) {
            tracing::warn!(
                instance_id = %id,
                error = %e,
                "source-base backfill: upsert failed — will retry next launch"
            );
            pending.insert(id, rec);
            incomplete = true;
        } else {
            stats.records_updated += 1;
        }
    }

    // Records still pending have no locatable source DB (channel removed); a
    // relaunch's live mirror backfills them. Count but don't block.
    stats.records_unresolved = pending.len();

    // Only finalize when every DB read cleanly — otherwise a transient failure
    // would permanently strand records that DO have a readable source DB.
    stats.complete = !incomplete;
    if stats.complete {
        std::fs::write(
            &marker,
            format!(
                "backfilled: {}\nunresolved: {}\n",
                stats.records_updated, stats.records_unresolved
            ),
        )?;
    }
    Ok(stats)
}

/// Enumerate every per-(channel,version) and per-dev-branch `objects.db` under
/// `home`, pairing each with the agents dir its rows are anchored on.
///
/// Returns `(sources, incomplete)`. `incomplete` is true if any directory that
/// *should* be enumerable failed to read for a reason other than "doesn't
/// exist" (permissions, a transient FS/network-home hiccup, a non-dir in the
/// way). The caller treats that like an unreadable DB and DEFERS the one-shot
/// marker so a future launch retries — otherwise a transiently-unreadable
/// `versions/` (or `dev/`) would silently finalize the migration and omit that
/// tree's named agents forever (codex P2 on #1389).
fn enumerate_sources(home: &Path) -> (Vec<SqliteSource>, bool) {
    let mut out = Vec::new();
    let mut incomplete = false;

    // Installed/portable: home/channels/<ch>/versions/<v>/data/db/objects.db,
    // anchored on home/channels/<ch>/agents.
    if let Some(rd) = read_dir_tracking(&home.join("channels"), &mut incomplete) {
        for ch in rd.flatten() {
            let ch_dir = ch.path();
            if !ch_dir.is_dir() {
                continue;
            }
            let agents_root = ch_dir.join("agents");
            if let Some(vrd) = read_dir_tracking(&ch_dir.join("versions"), &mut incomplete) {
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
    // <instance_dir>/agents (instance_dir = the dir that holds `data`).
    collect_dev_sources(&home.join("dev"), &mut out, &mut incomplete);

    (out, incomplete)
}

/// `read_dir` that distinguishes "absent" (fine — nothing to scan) from
/// "present but unreadable" (sets `incomplete` so the marker defers). Returns
/// `None` in both error cases; the iterator otherwise.
fn read_dir_tracking(path: &Path, incomplete: &mut bool) -> Option<std::fs::ReadDir> {
    match std::fs::read_dir(path) {
        Ok(rd) => Some(rd),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => {
            *incomplete = true;
            None
        }
    }
}

/// Locate dev instance dirs (those with `data/db/objects.db`) and anchor each
/// on its sibling `agents/` dir. The dev layout is at most two levels under
/// `dev/`: `dev/<branch>/data/...` (older single-level layout) or
/// `dev/<branch>/<sub-hash>/data/...`. We check exactly those depths and never
/// descend into an instance's own subdirs — so an agent workspace
/// (`<instance>/agents/<slug>/`) that holds a nested AgentMux `objects.db` is
/// never mistaken for a source (reagent P2), and a dev branch whose slug
/// happens to equal an internal dir name like `data`/`agents` is still scanned
/// (no name-based skip-list — codex P2).
fn collect_dev_sources(dev_root: &Path, out: &mut Vec<SqliteSource>, incomplete: &mut bool) {
    let Some(branches) = read_dir_tracking(dev_root, incomplete) else {
        return;
    };
    for branch in branches.flatten() {
        let bdir = branch.path();
        if !bdir.is_dir() {
            continue;
        }
        // Depth 1: the branch dir is itself the instance dir (older layout).
        // If so it is a leaf — do NOT descend into its agents/ etc.
        if push_if_instance(&bdir, out) {
            continue;
        }
        // Depth 2: each child (the sub-hash dir) may be the instance dir.
        if let Some(subs) = read_dir_tracking(&bdir, incomplete) {
            for sub in subs.flatten() {
                let sdir = sub.path();
                if sdir.is_dir() {
                    push_if_instance(&sdir, out);
                }
            }
        }
    }
}

/// Push `dir` as a source iff it holds `data/db/objects.db`. Returns whether it
/// did (so the caller can treat an instance dir as a leaf).
fn push_if_instance(dir: &Path, out: &mut Vec<SqliteSource>) -> bool {
    let db = dir.join("data").join("db").join("objects.db");
    if db.is_file() {
        out.push(SqliteSource {
            db_path: db,
            agents_root: dir.join("agents"),
        });
        true
    } else {
        false
    }
}

/// Read the `migration_version:` line from an existing marker. Returns 0 when
/// the marker is absent, unreadable, or predates versioning (a legacy
/// stats-only marker has no such line) — so a logic bump, or any pre-versioning
/// marker, re-runs the migration exactly once.
fn marker_migration_version(path: &Path) -> u32 {
    let Ok(body) = std::fs::read_to_string(path) else {
        return 0;
    };
    for line in body.lines() {
        if let Some(v) = line.strip_prefix("migration_version:") {
            return v.trim().parse().unwrap_or(0);
        }
    }
    0
}

fn write_marker(path: &Path, stats: &MigrateStats) -> std::io::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let body = format!(
        "migration_version: {MIGRATION_VERSION}\n\
         migrated_at: {now}\n\
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

fn row_to_record(row: &RowSnapshot, global_agents_root: &Path) -> Option<NamedAgentRecord> {
    let abs = std::path::Path::new(&row.working_directory);
    // Agent workspaces live GLOBALLY at `<home>/agents/<name>` (verified: real
    // `working_directory` values are `~/.agentmux/agents/<name>`), so anchor on
    // that global workspace root — `base` is stored absolute in the record, so
    // the reader reconstructs `base.join(working_dir)` correctly in any channel.
    // Fall back to THIS row's own source-channel agents dir for any legacy row
    // whose workspace genuinely lived in-channel (`channels/<ch>/agents`). A
    // workspace under NEITHER root is skipped (e.g. a user cwd like
    // `~/projects/foo`), matching the live mirror's relative_workdir.
    let (rel, base): (&Path, &Path) = abs
        .strip_prefix(global_agents_root)
        .ok()
        .map(|r| (r, global_agents_root))
        .or_else(|| {
            abs.strip_prefix(row.agents_root.as_path())
                .ok()
                .map(|r| (r, row.agents_root.as_path()))
        })?;
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
        // The legacy rows don't carry session_id through this consolidation
        // path; live mirroring (registry_mirror.rs) populates it on the next
        // launch/update. The record is still stamped v3 because
        // source_agents_base is set below — so a pre-v3 reader skips it (the
        // only such readers of the global registry are pre-P0.4 builds).
        session_id: None,
        working_dir: rel_str,
        // v3: anchor on the global agents root (or, for a legacy in-channel
        // workspace, that channel's agents dir) so a reader in ANY channel
        // reconstructs the absolute working_directory correctly (P0.4), not by
        // re-joining under its own channel's agents dir.
        source_agents_base: Some(base.to_string_lossy().to_string()),
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
        // P0.4: each record records its OWN source channel agents base, so a
        // reader in any channel reconstructs the absolute path against the
        // right channel (not its own). The record is therefore schema v3.
        assert_eq!(
            recs[0].data.source_agents_base.as_deref(),
            Some(channel_agents(home.path(), "stable").to_string_lossy().as_ref())
        );
        assert_eq!(
            recs[1].data.source_agents_base.as_deref(),
            Some(
                channel_agents(home.path(), "local-main-b28b7a")
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert_eq!(recs[0].schema_version, 3);
    }

    #[test]
    fn migrate_anchors_global_workspace_not_per_channel() {
        // Production reality (verified on disk): agent workspaces live at the
        // GLOBAL `<home>/agents/<name>` — e.g. `~/.agentmux/agents/qooma-0612g`,
        // NOT under any channel's `channels/<ch>/agents` dir, and NOT under the
        // re-rooted registry's parent (`<home>/shared/agents`). v1 stripped
        // against `channels/<ch>/agents` and dropped every such row as
        // "unmappable" → "My Agents" empty everywhere. The fix anchors on
        // `<home>/agents`, so the row migrates and reconstructs in any channel.
        let (home, reg) = fresh_home();
        let global_agents = home.path().join("agents"); // the REAL workspace root
        // Guard against the earlier wrong fix: the workspace root is NOT the
        // registry's parent (here `<home>/shared/agents`). If someone re-anchors
        // on `registry.root().parent()`, this row goes unmappable and the asserts
        // below fail.
        assert_ne!(
            global_agents.as_path(),
            reg.root().parent().unwrap(),
            "workspace root must differ from the re-rooted registry's parent"
        );
        let wd = global_agents.join("qooma-0612g");
        std::fs::create_dir_all(&wd).unwrap();
        // The row lives in a CHANNEL's SQLite, but its working_directory points
        // at the GLOBAL workspace — the actual on-disk shape.
        make_channel_db(
            home.path(),
            "stable",
            "0.44.2",
            &[("inst-q", "Qooma", 100, &wd.to_string_lossy())],
        );

        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(stats.rows_seen, 1);
        assert_eq!(
            stats.records_skipped_unmappable, 0,
            "a global workspace must NOT be unmappable"
        );
        assert_eq!(stats.records_written, 1);
        let recs = reg.list_active().unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].data.working_dir, "qooma-0612g");
        assert_eq!(
            recs[0].data.source_agents_base.as_deref(),
            Some(global_agents.to_string_lossy().as_ref()),
            "anchored on the GLOBAL workspace root <home>/agents, not the channel"
        );
        assert_eq!(recs[0].schema_version, 3);
    }

    #[test]
    fn migrate_legacy_marker_reruns_then_settles() {
        // A v1 build left a stats-only marker (no `migration_version:` line)
        // after writing 0 records for a global workspace it judged unmappable.
        // The fixed build must read that marker as version 0, re-run once, and
        // capture the row — then settle (a second run is a no-op).
        let (home, reg) = fresh_home();
        let global_agents = home.path().join("agents"); // the REAL workspace root
        let wd = global_agents.join("naki");
        std::fs::create_dir_all(&wd).unwrap();
        make_channel_db(
            home.path(),
            "stable",
            "0.44.2",
            &[("inst-n", "Naki", 100, &wd.to_string_lossy())],
        );
        // Simulate the legacy finalized marker: stats-only, no version line.
        std::fs::write(
            reg.root().join(MARKER),
            b"migrated_at: 2026-06-10T00:00:00Z\nrecords_written: 0\n",
        )
        .unwrap();

        // Re-run: legacy marker → version 0 < MIGRATION_VERSION → re-runs.
        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(
            stats.records_written, 1,
            "legacy marker must trigger a one-time re-run"
        );
        assert_eq!(reg.list_active().unwrap().len(), 1);
        assert_eq!(
            marker_migration_version(&reg.root().join(MARKER)),
            MIGRATION_VERSION,
            "marker upgraded to the current version"
        );

        // Settle: second run sees the current-version marker → no-op.
        let again = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(again.records_written, 0, "settles after the single re-run");
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
    fn migrate_dev_branch_named_like_internal_dir_is_scanned() {
        // A git branch can legitimately be named "data"/"agents"/etc. The dev
        // scan must NOT skip it (no name-based filter at the branch level —
        // codex P2 on #1389).
        let (home, reg) = fresh_home();
        let inst_dir = home.path().join("dev").join("data").join("sub");
        let wd = inst_dir.join("agents").join("a1");
        std::fs::create_dir_all(&wd).unwrap();
        make_db_at(
            &inst_dir,
            &[("inst-d", "D", 100, &wd.to_string_lossy(), false)],
        );

        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert_eq!(stats.dbs_scanned, 1, "branch named 'data' must still be scanned");
        assert_eq!(reg.list_active().unwrap().len(), 1);
    }

    #[test]
    fn migrate_defers_marker_when_versions_dir_is_unreadable() {
        // A channel's versions/ that is present but unreadable (here: a FILE in
        // its place, which makes read_dir fail with a non-NotFound error) must
        // defer the marker so the channel is retried — not be silently treated
        // as empty (codex P2 on #1389).
        let (home, reg) = fresh_home();
        let ch = home.path().join("channels").join("stable");
        std::fs::create_dir_all(&ch).unwrap();
        std::fs::write(ch.join("versions"), b"not a directory").unwrap();

        let stats = migrate_from_sqlite_once(home.path(), &reg).unwrap();
        assert!(
            !stats.complete,
            "an unreadable versions/ must defer the marker"
        );
        assert!(
            !reg.root().join(MARKER).exists(),
            "marker deferred so the channel is retried next launch"
        );
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
                source_agents_base: None,
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
                source_agents_base: None,
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

    // ---- source_agents_base backfill (P0.4) ----

    fn seed_pre_v3_record(reg: &Registry, id: &str, session: Option<&str>) {
        reg.upsert(&NamedAgentRecord {
            schema_version: if session.is_some() { 2 } else { 1 },
            data: NamedAgentRecordV1 {
                instance_id: id.to_string(),
                instance_name: "demo".to_string(),
                definition_id: "claude-code".to_string(),
                identity_id: Some("ident-1".to_string()),
                memory_id: None,
                session_id: session.map(|s| s.to_string()),
                working_dir: "demo".to_string(),
                source_agents_base: None,
                created_at_ms: 10,
                last_launched_at_ms: 20,
                created_by_version: "0.43.0".to_string(),
                last_launched_by_version: "0.43.0".to_string(),
            },
        })
        .unwrap();
    }

    #[test]
    fn backfill_sets_source_base_preserving_session_and_identity() {
        let (home, reg) = fresh_home();
        // Simulate a registry the P0.3b migration already finalized.
        std::fs::write(reg.root().join(MARKER), b"x").unwrap();
        seed_pre_v3_record(&reg, "inst-1", Some("sess-keep"));
        // Its source channel's SQLite still exists.
        let wd = channel_agents(home.path(), "stable").join("demo");
        std::fs::create_dir_all(&wd).unwrap();
        make_channel_db(
            home.path(),
            "stable",
            "0.44.2",
            &[("inst-1", "demo", 100, &wd.to_string_lossy())],
        );

        let stats = backfill_source_bases_once(home.path(), &reg).unwrap();
        assert_eq!(stats.records_updated, 1);
        assert!(stats.complete);
        let recs = reg.list_active().unwrap();
        assert_eq!(recs.len(), 1);
        let r = &recs[0].data;
        // Source base now points at the SOURCE channel agents dir...
        assert_eq!(
            r.source_agents_base.as_deref(),
            Some(channel_agents(home.path(), "stable").to_string_lossy().as_ref())
        );
        // ...and the live-mirror-enriched fields survived (not clobbered).
        assert_eq!(r.session_id.as_deref(), Some("sess-keep"));
        assert_eq!(r.identity_id.as_deref(), Some("ident-1"));
        assert_eq!(recs[0].schema_version, 3);
        assert!(reg.root().join(SOURCE_BACKFILL_MARKER).exists());
    }

    #[test]
    fn backfill_is_idempotent_via_marker() {
        let (home, reg) = fresh_home();
        // Empty registry → nothing pending → marker written.
        let s1 = backfill_source_bases_once(home.path(), &reg).unwrap();
        assert_eq!(s1.records_updated, 0);
        assert!(s1.complete);
        assert!(reg.root().join(SOURCE_BACKFILL_MARKER).exists());
        // A record added AFTER the marker must not be picked up (short-circuit).
        seed_pre_v3_record(&reg, "inst-late", None);
        let s2 = backfill_source_bases_once(home.path(), &reg).unwrap();
        assert_eq!(s2.records_updated, 0, "marker short-circuits subsequent runs");
    }

    #[test]
    fn backfill_anchors_on_latest_started_at_channel() {
        // An id present in two channels must anchor source_agents_base on the
        // SAME (latest-started_at) channel the migration's working_dir came
        // from — not whichever DB the filesystem returned first.
        let (home, reg) = fresh_home();
        std::fs::write(reg.root().join(MARKER), b"x").unwrap();
        seed_pre_v3_record(&reg, "inst-dup", None);
        let wda = channel_agents(home.path(), "chan-a").join("demo");
        let wdb = channel_agents(home.path(), "chan-b").join("demo");
        std::fs::create_dir_all(&wda).unwrap();
        std::fs::create_dir_all(&wdb).unwrap();
        // chan-a older (100), chan-b newer (200) — chan-b must win.
        make_channel_db(
            home.path(),
            "chan-a",
            "0.1",
            &[("inst-dup", "demo", 100, &wda.to_string_lossy())],
        );
        make_channel_db(
            home.path(),
            "chan-b",
            "0.1",
            &[("inst-dup", "demo", 200, &wdb.to_string_lossy())],
        );

        let stats = backfill_source_bases_once(home.path(), &reg).unwrap();
        assert_eq!(stats.records_updated, 1);
        let recs = reg.list_active().unwrap();
        assert_eq!(
            recs[0].data.source_agents_base.as_deref(),
            Some(channel_agents(home.path(), "chan-b").to_string_lossy().as_ref()),
            "anchors on the latest-started_at channel (chan-b)"
        );
    }

    #[test]
    fn backfill_counts_unresolved_when_source_db_gone() {
        let (home, reg) = fresh_home();
        std::fs::write(reg.root().join(MARKER), b"x").unwrap();
        seed_pre_v3_record(&reg, "inst-orphan", None);
        // No channels/ at all — the source channel is gone.
        let stats = backfill_source_bases_once(home.path(), &reg).unwrap();
        assert_eq!(stats.records_updated, 0);
        assert_eq!(stats.records_unresolved, 1);
        assert!(stats.complete, "no DB failure → marker written");
        // Record stays None; the handler falls back to the current channel.
        let recs = reg.list_active().unwrap();
        assert!(recs[0].data.source_agents_base.is_none());
    }
}
