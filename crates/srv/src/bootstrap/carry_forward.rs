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
//!
//! This is the only thing that seeds a version's databases. A channel whose
//! install predates per-version data still has its old `channels/<ch>/data/db/`;
//! it is copied only while no version has data yet. The launcher used to copy
//! that folder into every new version before srv started, so srv found an
//! `objects.db` in place and carried nothing forward: each upgrade started from
//! that old data, without the layout the previous version's quit had saved
//! (`docs/incident/RCA_UPGRADE_LOSES_LAYOUT_2026_10_22.md`). Each copy
//! records where it came from ([`CARRIED_FROM_FILE`]), and the first start
//! after an upgrade checks it ([`continuity_problem`]).

use std::path::{Path, PathBuf};

use crate::backend::storage::db_copy::copy_database;

/// The databases copied, `objects.db` last: its presence is what marks a
/// version as started, so a copy cut short is tried again on the next start.
const DATABASES: [&str; 3] = ["filestore.db", "sagas.db", "objects.db"];

/// In a version's `data/db/`: the version folder its databases were copied
/// from, or [`PRE_VERSIONING`].
pub(crate) const CARRIED_FROM_FILE: &str = "carried-from";

/// What [`CARRIED_FROM_FILE`] says for a copy of the channel's pre-versioning
/// `data/db/`.
const PRE_VERSIONING: &str = "pre-versioning data";

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

/// The channel's pre-versioning `data/db/` (`channels/<ch>/data/db/`, beside
/// `versions_dir`), while no version in `versions_dir` has an `objects.db`:
/// an install from before data was kept per version, on its first versioned
/// start. Never once a version has data: that folder stopped changing when
/// its install moved to per-version data, so it is older than all of it.
fn pre_versioning_db_dir(versions_dir: &Path) -> Option<PathBuf> {
    let a_version_has_data = std::fs::read_dir(versions_dir)
        .map(|d| d.filter_map(|e| e.ok()).any(|e| e.path().join("data").join("db").join("objects.db").is_file()))
        .unwrap_or(false);
    if a_version_has_data {
        return None;
    }
    let db = versions_dir.parent()?.join("data").join("db");
    db.join("objects.db").is_file().then_some(db)
}

/// What [`CARRIED_FROM_FILE`] says for a copy from `source` (a `data/db/`).
fn carried_from_name(source: &Path) -> String {
    let owner = source.parent().and_then(Path::parent);
    match owner {
        Some(dir) if dir.parent().and_then(Path::file_name).is_some_and(|n| n == "versions") => {
            dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        }
        _ => PRE_VERSIONING.to_string(),
    }
}

/// Where the databases in `db_dir` were copied from, if a copy-forward seeded them.
fn carried_from(db_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(db_dir.join(CARRIED_FROM_FILE)).ok()?;
    Some(text.trim().to_string()).filter(|s| !s.is_empty())
}

/// The check on the first start after an upgrade from version `from`: when
/// `from`'s data is in this channel (`versions/<from>/data/db/objects.db`
/// beside this version's), this version's databases (`db_dir`) must have been
/// copied from it, or the windows, tabs and panes `from` saved at its quit
/// are not here. `None` when they were, or when `from` has no data to compare.
pub(crate) fn continuity_problem(db_dir: &Path, from: &str) -> Option<String> {
    let versions_dir = db_dir.parent()?.parent()?.parent()?;
    if versions_dir.file_name()? != "versions" {
        return None;
    }
    let from_db = versions_dir.join(from).join("data").join("db");
    if !from_db.join("objects.db").is_file() {
        return None;
    }
    // Either way the layout `from` quit with isn't here. Which cause it is
    // can't be told from here: this version may have run before (a rollback,
    // then this upgrade again), or something else put its databases in place.
    match carried_from(db_dir) {
        Some(source) if source == from => None,
        Some(source) => Some(format!(
            "this version's data was copied from {source}, not from {from}, the version this upgrade was from \
             (it has run before, or was copied from another version)"
        )),
        None => Some(format!(
            "this version's data wasn't copied from {from}, the version this upgrade was from: \
             its databases were already in {} (it has run before, or something other than srv's \
             copy-forward put them there)",
            db_dir.display()
        )),
    }
}

/// On the first start of `version` (no `objects.db` in `db_dir` yet), copy
/// the databases of the newest older version in the same channel (or the
/// channel's pre-versioning ones, [`source_for`]), and record where they came
/// from ([`CARRIED_FROM_FILE`]). Returns
/// the version directory copied from, or `None` when there was nothing to do
/// or the copy failed (logged; the version then starts fresh).
pub(crate) fn from_previous_version(db_dir: &Path, version: &str) -> Option<PathBuf> {
    match try_from_previous_version(db_dir, version) {
        Ok(from) => from,
        Err(e) => {
            tracing::warn!("carry-forward: {e}; starting fresh");
            None
        }
    }
}

/// [`from_previous_version`], telling "nothing to copy" (`Ok(None)`) apart
/// from "there was data to copy and copying it failed" (`Err`), which
/// `migrate --prepare` must not treat as a fresh start: that would relaunch
/// the new version without the user's data (`migrations/prepare.rs`). Copies
/// that failed are removed again, so a retry starts clean.
pub(crate) fn try_from_previous_version(db_dir: &Path, version: &str) -> Result<Option<PathBuf>, String> {
    let Some(source) = source_for(db_dir, version) else {
        return Ok(None);
    };
    let from = source.parent().and_then(Path::parent).map(Path::to_path_buf).unwrap_or_else(|| source.clone());
    let from_name = carried_from_name(&source);
    std::fs::create_dir_all(db_dir).map_err(|e| format!("can't create {}: {e}", db_dir.display()))?;
    // A large filestore.db takes a while to copy: the same line migrations
    // print moves the launcher's ESTART deadline from 30 s to 30 minutes.
    eprintln!("{}", agentmux_common::srv_stderr::migrating_line("copying-previous-version", &from_name));
    crate::boot_timing::step("copy_previous_version", &format!("Copying from {from_name}"), || {
        let mut copied = Vec::new();
        for name in DATABASES {
            let src = source.join(name);
            let dst = db_dir.join(name);
            // Already here: copied by a start that was cut short before objects.db.
            if !src.is_file() || dst.exists() {
                continue;
            }
            if let Err(e) = copy_database(&src, &dst) {
                for dst in copied {
                    let _ = std::fs::remove_file(dst);
                }
                return Err(format!("copying {name} from {}: {e}", from.display()));
            }
            copied.push(dst);
        }
        Ok(())
    })?;
    // Best-effort: without it the first start after this upgrade reports that
    // the data didn't come from the previous version (`continuity_problem`),
    // which is wrong but harmless; failing the copy over it would not be.
    if let Err(e) = std::fs::write(db_dir.join(CARRIED_FROM_FILE), &from_name) {
        tracing::warn!(error = %e, "carry-forward: couldn't record where the data came from");
    }
    CARRIED_FORWARD.store(true, std::sync::atomic::Ordering::Relaxed);
    tracing::info!(from = %from.display(), "carry-forward: started from the previous version's data");
    Ok(Some(from))
}

/// The `data/db` of the newest older version in the same channel to copy
/// from on the first start of `version`, else the channel's pre-versioning
/// one while no version has data ([`pre_versioning_db_dir`]); `None` once
/// `db_dir` has an `objects.db`, outside a `versions/<v>/data/db` layout, or
/// with nothing to copy.
pub(crate) fn source_for(db_dir: &Path, version: &str) -> Option<PathBuf> {
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
    previous_db_dir(versions_dir, current).or_else(|| pre_versioning_db_dir(versions_dir))
}

static CARRIED_FORWARD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether this process copied a previous version's data forward. The
/// previous version's folder then still holds that data untouched, so the
/// pre-migration snapshot and the backup of `objects.db` would only copy the
/// same data again (`bootstrap/stores.rs`, `migrations/backup.rs`).
pub(crate) fn carried_forward_this_run() -> bool {
    CARRIED_FORWARD.load(std::sync::atomic::Ordering::Relaxed)
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
        seed_dir(&db_dir(root, v), marker)
    }

    /// [`seed`] at any `data/db/`.
    fn seed_dir(dir: &Path, marker: &str) -> Connection {
        std::fs::create_dir_all(dir).unwrap();
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
        assert!(!new.join("objects.db.copying").exists());
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
    fn a_failed_copy_removes_only_what_it_copied() {
        let root = tempfile::tempdir().unwrap();
        let _prev = seed(root.path(), "0.59.26", "previous");
        let old = db_dir(root.path(), "0.59.26");
        Connection::open(old.join("filestore.db")).unwrap().execute_batch("CREATE TABLE f (v)").unwrap();
        // A file that isn't SQLite fails its copy, after filestore.db was copied.
        std::fs::write(old.join("sagas.db"), b"not a database").unwrap();
        let new = db_dir(root.path(), "0.59.27");
        assert!(from_previous_version(&new, "0.59.27").is_none());
        for name in DATABASES {
            assert!(!new.join(name).exists(), "{name} left behind");
        }

        // One already in the new version's folder is never removed or replaced.
        std::fs::write(new.join("filestore.db"), b"already here").unwrap();
        assert!(from_previous_version(&new, "0.59.27").is_none());
        assert_eq!(std::fs::read(new.join("filestore.db")).unwrap(), b"already here");
        assert!(!new.join("objects.db").exists());
    }

    /// The channel's `data/db/` from before data was kept per version.
    fn pre_versioning(root: &Path) -> PathBuf {
        root.join("channels/stable/data/db")
    }

    #[test]
    fn pre_versioning_data_never_shadows_the_previous_version() {
        let root = tempfile::tempdir().unwrap();
        let _legacy = seed_dir(&pre_versioning(root.path()), "legacy");
        let _prev = seed(root.path(), "0.59.38", "previous");
        let new = db_dir(root.path(), "0.59.39");
        assert!(from_previous_version(&new, "0.59.39").unwrap().ends_with("versions/0.59.38"));
        assert_eq!(marker(&new), "previous");
        assert_eq!(carried_from(&new).as_deref(), Some("0.59.38"));
    }

    #[test]
    fn pre_versioning_data_seeds_only_a_channel_without_versioned_data() {
        let root = tempfile::tempdir().unwrap();
        let _legacy = seed_dir(&pre_versioning(root.path()), "legacy");
        let first = db_dir(root.path(), "0.59.39");
        assert!(from_previous_version(&first, "0.59.39").is_some());
        assert_eq!(marker(&first), "legacy");
        assert_eq!(carried_from(&first).as_deref(), Some(PRE_VERSIONING));

        // An older build started after a newer one has data: not that old folder.
        let older = db_dir(root.path(), "0.59.30");
        assert!(from_previous_version(&older, "0.59.30").is_none());
        assert!(!older.join("objects.db").exists());
    }

    /// The layout a quit saved (`session:last_topology` in the client's meta,
    /// `server/service/session_restore.rs`) is in the new version's data, so
    /// its first window restores every window, tab and pane.
    #[test]
    fn the_layout_a_quit_saved_crosses_the_upgrade() {
        use crate::backend::obj::Client;
        use crate::backend::storage::store::Store;
        use crate::server::service::session_restore::{load_last_session_snapshot, save_last_session_snapshot};

        let root = tempfile::tempdir().unwrap();
        let _legacy = seed_dir(&pre_versioning(root.path()), "legacy");
        let old = db_dir(root.path(), "0.59.38");
        std::fs::create_dir_all(&old).unwrap();
        let store = Store::open(&old.join("objects.db")).unwrap();
        store.insert(&mut Client { oid: "client-1".to_string(), ..Default::default() }).unwrap();
        let record = serde_json::json!({
            "saved_for": "quit",
            "windows": [{"name": "Main", "tabs": [{"name": "Tab 1", "blocks": [
                {"meta": {"view": "agent", "agent:sessionid": "session-1"}},
                {"meta": {"view": "agent", "agent:sessionid": "session-2"}},
                {"meta": {"view": "swarm"}}
            ]}]}]
        });
        assert!(save_last_session_snapshot(&store, record.clone()));

        // The old version still holds its store open: the record may be only in its WAL.
        let new = db_dir(root.path(), "0.59.39");
        assert!(from_previous_version(&new, "0.59.39").is_some());
        let carried = Store::open(&new.join("objects.db")).unwrap();
        assert_eq!(load_last_session_snapshot(&carried), Some(record));
        assert_eq!(continuity_problem(&new, "0.59.38"), None);
    }

    #[test]
    fn an_upgrade_whose_data_isnt_the_previous_versions_is_reported() {
        let root = tempfile::tempdir().unwrap();
        let _prev = seed(root.path(), "0.59.38", "previous");

        // Databases put in place before srv's copy-forward: what the launcher did.
        let _put = seed(root.path(), "0.59.39", "legacy");
        let new = db_dir(root.path(), "0.59.39");
        assert!(from_previous_version(&new, "0.59.39").is_none());
        let problem = continuity_problem(&new, "0.59.38").expect("reported");
        assert!(problem.contains("wasn't copied from 0.59.38"), "{problem}");

        // Copied from another version than the one this upgrade was from.
        std::fs::write(new.join(CARRIED_FROM_FILE), "0.59.36").unwrap();
        let problem = continuity_problem(&new, "0.59.38").expect("reported");
        assert!(problem.contains("copied from 0.59.36, not from 0.59.38"), "{problem}");

        std::fs::write(new.join(CARRIED_FROM_FILE), "0.59.38\n").unwrap();
        assert_eq!(continuity_problem(&new, "0.59.38"), None);

        // Nothing to compare with: the version upgraded from has no data here.
        assert_eq!(continuity_problem(&new, "0.59.37"), None);
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
