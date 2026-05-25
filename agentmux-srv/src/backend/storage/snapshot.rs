// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pre-migration snapshots — Increment B.2 lean cut from
//! `docs/specs/SPEC_DATA_CHANNELS_2026_05_24.md` §3.4.
//!
//! When a launch detects an existing `objects.db` whose `user_version` is
//! behind the binary's `OBJECT_SCHEMA_VERSION`, copy the SQLite DBs into
//! `~/.agentmux/snapshots/<channel>-pre-v<code-version>-<ISO8601>.bak/`
//! BEFORE any migration mutates them, then prune to the newest
//! [`MAX_SNAPSHOTS_PER_CHANNEL`] per channel.
//!
//! Lean-cut scope decisions (vs. the full spec):
//!
//! - SQLite files only — no copy of `agents/`, `config/`, `logs/`, or
//!   `cef-cache/`. Most state worth recovering lives in the DBs; agent
//!   workspaces are typically git-managed; logs and cache are regenerable.
//!   Cuts snapshot size by ~10× vs. a full data-dir copy.
//! - `VACUUM INTO` instead of file copy — produces an atomic, WAL-consistent
//!   single-file snapshot regardless of journal state. No need to coordinate
//!   `.db-wal` and `.db-shm` files separately.
//! - No restore CLI — manual restore is `cp snapshot/*.db <db_dir>/`.
//!   Documented in the spec for now; a CLI can come later if used.
//! - Snapshot failure is logged + non-fatal — the safety lock already
//!   prevents downgrade corruption, so a failed snapshot is a missing
//!   rollback aid, not a data-loss event. Refusing to boot would be worse.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use tracing::{info, warn};

/// Token that separates `<channel>` from the version segment in snapshot
/// dir names: `<channel>-pre-v<code_version>-<ts>.bak`. Used by the prune
/// step to extract the timestamp from the filename (more robust than mtime,
/// which can be rewritten by backup tools).
const NAME_PRE_TOKEN: &str = "-pre-v";
const NAME_SUFFIX: &str = ".bak";

/// Maximum number of snapshots retained per channel. Older ones are pruned
/// after each new snapshot. Spec §3.4 budgets ~2 GB per channel at this
/// retention level; the lean SQLite-only cut runs well under that.
pub const MAX_SNAPSHOTS_PER_CHANNEL: usize = 5;

/// DBs included in a snapshot. Lean-cut: SQLite files only.
const SNAPSHOT_DB_NAMES: &[&str] = &["objects.db", "sagas.db", "filestore.db"];

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("io error during snapshot ({path}): {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("sqlite error during snapshot: {0}")]
    Sql(#[from] rusqlite::Error),

    #[error("invalid channel name for snapshot: {0:?}")]
    InvalidChannel(String),
}

impl SnapshotError {
    fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        SnapshotError::Io {
            path: path.into(),
            source,
        }
    }
}

/// Inspect `objects.db` to decide whether a pre-migration snapshot is
/// warranted. Returns `Some(found_version)` if an existing DB is behind
/// `current`; `None` for fresh installs (no file) or same-version opens.
/// A newer-than-current DB is left alone — the safety lock in `wstore.rs`
/// will refuse to open it before any migration runs.
pub fn needs_snapshot(
    objects_db: &Path,
    current_schema_version: i64,
) -> Result<Option<i64>, SnapshotError> {
    if !objects_db.exists() {
        return Ok(None);
    }
    // Read-only peek. Open in read-only mode so a concurrent write can't
    // be triggered by anything we do here (the connection is dropped
    // before any real open happens).
    let conn = Connection::open_with_flags(
        objects_db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )?;
    let found: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    drop(conn);
    if found < current_schema_version {
        Ok(Some(found))
    } else {
        Ok(None)
    }
}

/// Snapshot the SQLite DBs in `db_dir` to a new directory under
/// `snapshots_dir`. The snapshot dir is named
/// `<channel>-pre-v<code_version>-<ISO8601>.bak`. Uses `VACUUM INTO` for
/// atomic, WAL-consistent copies. Returns the new snapshot dir.
pub fn create_snapshot(
    db_dir: &Path,
    snapshots_dir: &Path,
    channel: &str,
    code_version: &str,
) -> Result<PathBuf, SnapshotError> {
    validate_channel(channel)?;
    let snap_name = snapshot_dir_name(channel, code_version, SystemTime::now());
    let snap_dir = snapshots_dir.join(&snap_name);
    fs::create_dir_all(&snap_dir).map_err(|e| SnapshotError::io(&snap_dir, e))?;
    for db_name in SNAPSHOT_DB_NAMES {
        let src = db_dir.join(db_name);
        if !src.exists() {
            continue;
        }
        let dst = snap_dir.join(db_name);
        let conn = Connection::open(&src)?;
        // VACUUM INTO writes a consistent single-file snapshot of the
        // current DB state including any committed WAL frames. The dst
        // path must not exist. Cannot use parameter binding for the
        // path — SQLite parses VACUUM INTO at planning time before
        // bindings are applied. Path safety is ensured by the snap_dir
        // construction (validate_channel + ISO timestamp).
        let quoted = quote_sql_literal(&dst.to_string_lossy());
        conn.execute_batch(&format!("VACUUM INTO {quoted};"))?;
    }
    info!(
        snapshot = %snap_dir.display(),
        channel,
        code_version,
        "created pre-migration snapshot",
    );
    Ok(snap_dir)
}

/// Delete oldest snapshots for `channel` until at most
/// [`MAX_SNAPSHOTS_PER_CHANNEL`] remain. Returns the number pruned.
/// Pruning errors on a specific entry are logged and skipped rather than
/// aborting the whole sweep (one bad dir shouldn't keep storage growing).
pub fn prune_snapshots(snapshots_dir: &Path, channel: &str) -> Result<usize, SnapshotError> {
    if !snapshots_dir.exists() {
        return Ok(0);
    }
    let prefix = format!("{channel}{NAME_PRE_TOKEN}");
    // Sort by the filename's embedded ISO timestamp rather than filesystem
    // mtime — mtime can be rewritten by backup tools, copies, or restores,
    // and would mis-rank snapshots after a `cp -a`. The filename is what
    // the snapshot creator stamped at the moment it was made.
    let mut matches: Vec<(String, PathBuf)> = Vec::new();
    let entries = fs::read_dir(snapshots_dir).map_err(|e| SnapshotError::io(snapshots_dir, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| SnapshotError::io(snapshots_dir, e))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let fname = entry.file_name().to_string_lossy().into_owned();
        if !fname.starts_with(&prefix) || !fname.ends_with(NAME_SUFFIX) {
            continue;
        }
        let Some(ts) = parse_ts_from_name(&fname, &prefix) else {
            // Malformed name — skip rather than guess at ordering.
            warn!(snapshot = %fname, "snapshot name doesn't match expected layout — ignored for prune");
            continue;
        };
        matches.push((ts, path));
    }
    if matches.len() <= MAX_SNAPSHOTS_PER_CHANNEL {
        return Ok(0);
    }
    // ISO 8601 strings sort lexicographically in chronological order, so
    // a plain string sort gives oldest-first.
    matches.sort_by(|a, b| a.0.cmp(&b.0));
    let to_remove = matches.len() - MAX_SNAPSHOTS_PER_CHANNEL;
    let mut removed = 0;
    for (_, path) in matches.into_iter().take(to_remove) {
        match fs::remove_dir_all(&path) {
            Ok(()) => removed += 1,
            Err(e) => warn!(snapshot = %path.display(), err = %e, "failed to prune snapshot"),
        }
    }
    Ok(removed)
}

/// Extract the timestamp segment from a snapshot dir name.
/// Format: `<channel>-pre-v<code_version>-<ISO8601>.bak`. We already know
/// the name starts with `<channel>-pre-v` (the caller checked); find the
/// last `-` before `.bak` to split off the timestamp from `<code_version>`.
fn parse_ts_from_name(fname: &str, channel_prefix: &str) -> Option<String> {
    let rest = fname.strip_prefix(channel_prefix)?;
    let rest = rest.strip_suffix(NAME_SUFFIX)?;
    // `rest` is now `<code_version>-<ISO8601>`. The ISO timestamp is the
    // last 20 chars (`YYYY-MM-DDTHH-MM-SSZ`); split on the last `-` that
    // separates them. Code versions can contain digits, dots, and dashes
    // (semver pre-release like `0.39.0-rc1`), but our ISO format is fixed
    // width — find the last position where the tail matches the shape.
    const ISO_LEN: usize = 20; // "YYYY-MM-DDTHH-MM-SSZ"
    if rest.len() <= ISO_LEN + 1 {
        return None;
    }
    let sep_idx = rest.len() - ISO_LEN - 1;
    if rest.as_bytes().get(sep_idx) != Some(&b'-') {
        return None;
    }
    Some(rest[sep_idx + 1..].to_string())
}

/// High-level entry: check, snapshot, prune. Returns the snapshot dir on
/// success, or `None` if no snapshot was needed. Errors are surfaced to the
/// caller; production call sites should log-and-continue rather than abort
/// boot (see module docs).
pub fn maybe_snapshot_pre_migration(
    db_dir: &Path,
    snapshots_dir: &Path,
    channel: &str,
    code_version: &str,
    current_schema_version: i64,
) -> Result<Option<PathBuf>, SnapshotError> {
    let objects_db = db_dir.join("objects.db");
    let Some(found_version) = needs_snapshot(&objects_db, current_schema_version)? else {
        return Ok(None);
    };
    info!(
        from_version = found_version,
        to_version = current_schema_version,
        channel,
        "pre-migration snapshot needed",
    );
    fs::create_dir_all(snapshots_dir).map_err(|e| SnapshotError::io(snapshots_dir, e))?;
    let snap = create_snapshot(db_dir, snapshots_dir, channel, code_version)?;
    let pruned = prune_snapshots(snapshots_dir, channel)?;
    if pruned > 0 {
        info!(pruned, channel, "pruned old snapshots");
    }
    Ok(Some(snap))
}

// ── helpers ────────────────────────────────────────────────────────────

/// Reject channel names that would break the filename layout or escape the
/// snapshots dir. Mirrors the reserved-name set in `agentmux-common`'s
/// `sanitize_channel_name`, but operates on the already-resolved channel
/// string this module receives via env.
fn validate_channel(channel: &str) -> Result<(), SnapshotError> {
    if channel.is_empty()
        || channel.contains('/')
        || channel.contains('\\')
        || channel.contains("..")
        || channel.contains('\0')
    {
        return Err(SnapshotError::InvalidChannel(channel.to_string()));
    }
    Ok(())
}

fn snapshot_dir_name(channel: &str, code_version: &str, t: SystemTime) -> String {
    let dt: DateTime<Utc> = t.into();
    // Windows-safe ISO 8601: `:` is illegal in NTFS filenames, so use `-`.
    // Spec §3.4 example uses this same form.
    let ts = dt.format("%Y-%m-%dT%H-%M-%SZ").to_string();
    format!("{channel}-pre-v{code_version}-{ts}.bak")
}

fn quote_sql_literal(s: &str) -> String {
    // Double up any single quotes; wrap in single quotes. VACUUM INTO
    // accepts a string-literal path; binding params is not supported.
    let escaped = s.replace('\'', "''");
    format!("'{escaped}'")
}

// ── tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn make_db_with_version(path: &Path, version: i64) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS t (x INTEGER); INSERT INTO t VALUES (42); PRAGMA user_version = {version};"
        )).unwrap();
    }

    #[test]
    fn needs_snapshot_returns_none_for_fresh_install() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("objects.db");
        assert_eq!(needs_snapshot(&missing, 4).unwrap(), None);
    }

    #[test]
    fn needs_snapshot_returns_none_when_same_version() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("objects.db");
        make_db_with_version(&p, 4);
        assert_eq!(needs_snapshot(&p, 4).unwrap(), None);
    }

    #[test]
    fn needs_snapshot_returns_some_when_db_is_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("objects.db");
        make_db_with_version(&p, 2);
        assert_eq!(needs_snapshot(&p, 4).unwrap(), Some(2));
    }

    #[test]
    fn needs_snapshot_returns_some_for_legacy_db_at_v0() {
        // Pre-flatten DBs never set user_version, so they read 0 even
        // though they contain real data. We snapshot those before the
        // flat schema runs (it's idempotent but still touches state).
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("objects.db");
        let conn = Connection::open(&p).unwrap();
        conn.execute_batch("CREATE TABLE t (x INTEGER); INSERT INTO t VALUES (1);")
            .unwrap();
        assert_eq!(needs_snapshot(&p, 4).unwrap(), Some(0));
    }

    #[test]
    fn create_snapshot_copies_existing_dbs_only() {
        let tmp = tempfile::tempdir().unwrap();
        let db_dir = tmp.path().join("db");
        let snaps_dir = tmp.path().join("snapshots");
        fs::create_dir_all(&db_dir).unwrap();
        make_db_with_version(&db_dir.join("objects.db"), 2);
        // filestore.db intentionally absent — should be skipped without error.
        make_db_with_version(&db_dir.join("sagas.db"), 1);

        let snap = create_snapshot(&db_dir, &snaps_dir, "stable", "0.39.0").unwrap();
        assert!(snap.is_dir());
        assert!(snap.join("objects.db").exists());
        assert!(snap.join("sagas.db").exists());
        assert!(!snap.join("filestore.db").exists());

        // VACUUM INTO preserves user_version.
        let conn = Connection::open(snap.join("objects.db")).unwrap();
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 2);
        // And data.
        let x: i64 = conn.query_row("SELECT x FROM t LIMIT 1", [], |r| r.get(0)).unwrap();
        assert_eq!(x, 42);
    }

    #[test]
    fn snapshot_dir_name_uses_filename_safe_iso8601() {
        let t = UNIX_EPOCH + Duration::from_secs(1_716_634_800); // 2024-05-25T11:00:00Z
        let name = snapshot_dir_name("stable", "0.39.0", t);
        assert_eq!(name, "stable-pre-v0.39.0-2024-05-25T11-00-00Z.bak");
        // Critical for Windows: no colons.
        assert!(!name.contains(':'));
    }

    #[test]
    fn validate_channel_rejects_traversal() {
        assert!(validate_channel("").is_err());
        assert!(validate_channel("..").is_err());
        assert!(validate_channel("a/b").is_err());
        assert!(validate_channel("a\\b").is_err());
        assert!(validate_channel("ok").is_ok());
        assert!(validate_channel("stable").is_ok());
    }

    #[test]
    fn prune_keeps_newest_n_per_channel() {
        let tmp = tempfile::tempdir().unwrap();
        let snaps = tmp.path();
        // Create MAX+3 snapshot dirs with synthetic, spaced-out timestamps
        // in the filename. Prune sorts by the filename's ISO segment, not
        // mtime — so we don't need any sleeps or filetime tweaks.
        let extras = 3;
        let mut created = Vec::new();
        for i in 0..(MAX_SNAPSHOTS_PER_CHANNEL + extras) {
            let t = UNIX_EPOCH + Duration::from_secs(1_700_000_000 + i as u64 * 60);
            let name = snapshot_dir_name("stable", "0.39.0", t);
            let p = snaps.join(&name);
            fs::create_dir_all(&p).unwrap();
            created.push((i, p));
        }
        // Other-channel dir must NOT be pruned even if older.
        let other = snaps.join("beta-pre-v0.39.0-2020-01-01T00-00-00Z.bak");
        fs::create_dir_all(&other).unwrap();

        let pruned = prune_snapshots(snaps, "stable").unwrap();
        assert_eq!(pruned, extras);
        // The MAX_SNAPSHOTS_PER_CHANNEL newest (highest indices) survive.
        for (i, p) in &created {
            let should_exist = *i >= extras;
            assert_eq!(p.exists(), should_exist, "i={i}");
        }
        assert!(other.exists(), "other-channel snapshot must survive");
    }

    #[test]
    fn parse_ts_from_name_extracts_timestamp() {
        let n = "stable-pre-v0.39.0-2024-05-25T11-00-00Z.bak";
        let prefix = "stable-pre-v";
        assert_eq!(
            parse_ts_from_name(n, prefix).as_deref(),
            Some("2024-05-25T11-00-00Z"),
        );
    }

    #[test]
    fn parse_ts_handles_semver_prerelease_in_code_version() {
        let n = "stable-pre-v0.39.0-rc1-2024-05-25T11-00-00Z.bak";
        let prefix = "stable-pre-v";
        assert_eq!(
            parse_ts_from_name(n, prefix).as_deref(),
            Some("2024-05-25T11-00-00Z"),
        );
    }

    #[test]
    fn prune_noop_when_under_limit() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..3 {
            let t = UNIX_EPOCH + Duration::from_secs(1_700_000_000 + i as u64 * 60);
            let p = tmp.path().join(snapshot_dir_name("stable", "0.39.0", t));
            fs::create_dir_all(&p).unwrap();
        }
        assert_eq!(prune_snapshots(tmp.path(), "stable").unwrap(), 0);
    }

    #[test]
    fn maybe_snapshot_returns_none_for_fresh_install() {
        let tmp = tempfile::tempdir().unwrap();
        let db_dir = tmp.path().join("db");
        let snaps = tmp.path().join("snapshots");
        fs::create_dir_all(&db_dir).unwrap();
        let result =
            maybe_snapshot_pre_migration(&db_dir, &snaps, "stable", "0.39.0", 4).unwrap();
        assert!(result.is_none());
        // Snapshots dir is NOT created when no snapshot is needed.
        assert!(!snaps.exists());
    }

    #[test]
    fn maybe_snapshot_runs_when_db_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let db_dir = tmp.path().join("db");
        let snaps = tmp.path().join("snapshots");
        fs::create_dir_all(&db_dir).unwrap();
        make_db_with_version(&db_dir.join("objects.db"), 2);
        let result =
            maybe_snapshot_pre_migration(&db_dir, &snaps, "stable", "0.39.0", 4).unwrap();
        let snap = result.expect("snapshot should be created");
        assert!(snap.is_dir());
        assert!(snap.join("objects.db").exists());
    }
}
