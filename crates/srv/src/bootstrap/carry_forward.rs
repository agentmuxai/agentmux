// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A new version starts with the data of the version before it
//! (`docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` §6).
//!
//! Installed and portable builds keep their databases per version
//! (`channels/<ch>/versions/<v>/data/db/`), so without this an upgrade
//! starts with an empty `objects.db`: no windows, no layouts, and no pane's
//! `agent:sessionid` to resume. On the first start of a version, before its
//! pre-migration snapshot and migrations, each database is copied from the
//! newest older version in the same channel; migrations then bring it up to
//! date as they would after an in-place upgrade.
//!
//! Each copy is `VACUUM INTO` a temporary file, renamed into place: a
//! consistent snapshot even if the older version is still running (two
//! portable folders side by side), and never a half-written file. Any failure
//! leaves the new version to start fresh, as it did before. Dev builds keep
//! their data per branch, and local `task package` builds get a channel of
//! their own, so neither has an older version to copy from.

use std::path::{Path, PathBuf};

/// The databases copied, `objects.db` last: its presence is what marks a
/// version as started, so a copy cut short is tried again on the next start.
const DATABASES: [&str; 3] = ["filestore.db", "sagas.db", "objects.db"];

/// `major.minor.patch` of a version or version directory name.
fn version_core(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().trim_start_matches('v').split(['+', '-']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// The `data/db/` of the newest version in `versions_dir` older than
/// `current` that has an `objects.db`.
fn previous_db_dir(versions_dir: &Path, current: (u64, u64, u64)) -> Option<PathBuf> {
    std::fs::read_dir(versions_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let v = version_core(e.file_name().to_str()?)?;
            let db = e.path().join("data").join("db");
            (v < current && db.join("objects.db").is_file()).then_some((v, db))
        })
        .max_by_key(|(v, _)| *v)
        .map(|(_, db)| db)
}

/// A consistent copy of the SQLite database `src` at `dst`.
fn copy_database(src: &Path, dst: &Path) -> Result<(), String> {
    let tmp = dst.with_extension("db.carrying");
    let _ = std::fs::remove_file(&tmp);
    let result = (|| {
        let conn = rusqlite::Connection::open_with_flags(
            src,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| format!("open {}: {e}", src.display()))?;
        conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(|e| e.to_string())?;
        let tmp_str = tmp.to_str().ok_or("the data path isn't UTF-8")?;
        conn.execute("VACUUM INTO ?1", [tmp_str])
            .map_err(|e| format!("copy {}: {e}", src.display()))?;
        drop(conn);
        std::fs::rename(&tmp, dst).map_err(|e| format!("rename into {}: {e}", dst.display()))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// On the first start of `version` (no `objects.db` in `db_dir` yet), copy
/// the databases of the newest older version in the same channel. Returns
/// the version directory copied from, or `None` when there was nothing to do
/// or the copy failed (logged; the version then starts fresh).
pub(super) fn from_previous_version(db_dir: &Path, version: &str) -> Option<PathBuf> {
    if db_dir.join("objects.db").exists() {
        return None;
    }
    let current = version_core(version)?;
    // db_dir is channels/<ch>/versions/<v>/data/db.
    let version_dir = db_dir.parent()?.parent()?;
    let versions_dir = version_dir.parent()?;
    if versions_dir.file_name()? != "versions" {
        return None;
    }
    let source = previous_db_dir(versions_dir, current)?;
    let from = source.parent()?.parent()?.to_path_buf();
    if let Err(e) = std::fs::create_dir_all(db_dir) {
        tracing::warn!(error = %e, "carry-forward: can't create {}; starting fresh", db_dir.display());
        return None;
    }
    for name in DATABASES {
        let src = source.join(name);
        if !src.is_file() {
            continue;
        }
        if let Err(e) = copy_database(&src, &db_dir.join(name)) {
            tracing::warn!(error = %e, from = %from.display(), "carry-forward: copy failed; starting fresh");
            for name in DATABASES {
                let _ = std::fs::remove_file(db_dir.join(name));
            }
            return None;
        }
    }
    tracing::info!(from = %from.display(), "carry-forward: started from the previous version's data");
    Some(from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn db_dir(root: &Path, v: &str) -> PathBuf {
        root.join("channels/stable/versions").join(v).join("data/db")
    }

    /// An `objects.db` in WAL mode holding `marker`, left open so its last
    /// write is still only in the WAL.
    fn seed(root: &Path, v: &str, marker: &str) -> Connection {
        let dir = db_dir(root, v);
        std::fs::create_dir_all(&dir).unwrap();
        let conn = Connection::open(dir.join("objects.db")).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        conn.execute_batch("CREATE TABLE t (v TEXT)").unwrap();
        conn.execute("INSERT INTO t VALUES (?1)", [marker]).unwrap();
        conn
    }

    fn marker(dir: &Path) -> String {
        Connection::open(dir.join("objects.db")).unwrap().query_row("SELECT v FROM t", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn a_new_version_starts_from_the_newest_older_one_even_while_it_runs() {
        let root = tempfile::tempdir().unwrap();
        let _old = seed(root.path(), "0.59.9", "old");
        let _prev = seed(root.path(), "0.59.26", "previous");
        let _newer = seed(root.path(), "0.60.0", "newer");
        let new = db_dir(root.path(), "0.59.27");
        let from = from_previous_version(&new, "0.59.27").unwrap();
        assert!(from.ends_with("versions/0.59.26"));
        assert_eq!(marker(&new), "previous");
        assert!(!new.join("objects.db.carrying").exists());
    }

    #[test]
    fn a_version_that_has_started_before_is_left_alone() {
        let root = tempfile::tempdir().unwrap();
        let _prev = seed(root.path(), "0.59.26", "previous");
        let _mine = seed(root.path(), "0.59.27", "mine");
        assert!(from_previous_version(&db_dir(root.path(), "0.59.27"), "0.59.27").is_none());
        assert_eq!(marker(&db_dir(root.path(), "0.59.27")), "mine");
    }

    #[test]
    fn nothing_older_or_not_a_versioned_layout_means_a_fresh_start() {
        let root = tempfile::tempdir().unwrap();
        let _newer = seed(root.path(), "0.60.0", "newer");
        let first = db_dir(root.path(), "0.59.27");
        assert!(from_previous_version(&first, "0.59.27").is_none());
        assert!(!first.join("objects.db").exists());
        // A dev build's data dir isn't under versions/.
        let dev = root.path().join("dev/main/data/db");
        assert!(from_previous_version(&dev, "0.59.27").is_none());
        assert!(from_previous_version(&first, "not-a-version").is_none());
    }

    #[test]
    fn a_failed_copy_leaves_no_partial_databases() {
        let root = tempfile::tempdir().unwrap();
        let _prev = seed(root.path(), "0.59.26", "previous");
        // A file that isn't SQLite fails its copy after filestore.db is skipped.
        std::fs::write(db_dir(root.path(), "0.59.26").join("sagas.db"), b"not a database").unwrap();
        let new = db_dir(root.path(), "0.59.27");
        assert!(from_previous_version(&new, "0.59.27").is_none());
        for name in DATABASES {
            assert!(!new.join(name).exists(), "{name} left behind");
        }
    }

    #[test]
    fn versions_compare_as_numbers() {
        assert_eq!(version_core("0.59.100"), Some((0, 59, 100)));
        assert!(version_core("0.59.100") > version_core("0.59.27"));
        assert_eq!(version_core("v1.2.3+g1a2b"), Some((1, 2, 3)));
        assert_eq!(version_core("1.2"), None);
        assert_eq!(version_core("1.2.3.4"), None);
    }
}
