// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Compatibility shim around `agentmux_common::DataPaths`.
//!
//! Historically the launcher computed its own paths via the
//! launcher-local `resolve_paths()` function. After the data-dir
//! unification (see docs/specs/SPEC_DATA_DIR_UNIFICATION_2026-05-05.md
//! and PR #695), path resolution is centralized in
//! `agentmux_common::DataPaths`. This module keeps the launcher's
//! existing public API surface (`DataPaths` struct with 4 fields,
//! `resolve_paths()`, `ensure_dirs()`) so call sites in main.rs,
//! diag.rs, srv_spawner.rs etc. don't need to be rewritten — they
//! see the same shape, populated from the common implementation.
//!
//! Field mapping:
//! - launcher.data_dir       = common.data_dir
//! - launcher.config_dir     = common.config_dir
//! - launcher.user_home_dir  = common.home_dir   (the agentmux root,
//!                                                e.g. `~/.agentmux/`,
//!                                                where `config.toml`
//!                                                lives)
//! - launcher.portable_root  = exe_dir when mode == Portable, else None
//!
//! The launcher continues to use these field names internally; the
//! env vars it passes to host + srv are switched in main.rs to the
//! canonical AGENTMUX_* names emitted by `DataPaths::to_env_vars`.

use agentmux_common::{DataPaths as CommonDataPaths, RuntimeMode};
use std::path::{Path, PathBuf};

/// Resolved per-instance paths. Compat shape — see module doc.
#[derive(Debug, Clone)]
pub struct DataPaths {
    pub data_dir: PathBuf,
    pub config_dir: PathBuf,
    pub user_home_dir: PathBuf,
    pub portable_root: Option<PathBuf>,
    /// Full common-paths value — exposes the new fields
    /// (`cef_cache_dir`, `agents_dir`, `instance_runtime_dir`,
    /// `logs_dir`, `instance_dir`, `mode`) for the launcher's
    /// env-var passing in main.rs without re-resolving.
    pub common: CommonDataPaths,
}

/// Resolve all paths from the launcher's vantage point.
///
/// Detects [`RuntimeMode`] from `launcher_exe_dir`, resolves the
/// canonical paths via [`agentmux_common::DataPaths::resolve`], and
/// projects them onto the launcher-local field names.
///
/// Mode detection is authoritative here — `cfg!(debug_assertions)` is
/// unreliable across binaries built with different profiles, so we let
/// `RuntimeMode::current` decide and propagate the answer downstream
/// via the `AGENTMUX_RUNTIME_MODE` env var. The legacy `is_dev`
/// parameter from the pre-PR-#695 signature has been removed; callers
/// no longer need to compute it.
pub fn resolve_paths(launcher_exe_dir: &Path, version: &str) -> Result<DataPaths, String> {
    // Path-only when the exe is a dev build — see SPEC_DEV_ENV_ISOLATION.
    // Prevents inheriting AGENTMUX_RUNTIME_MODE from a parent AgentMux
    // process when `task dev` is launched from inside an existing pane.
    let is_dev = agentmux_common::is_dev_build_exe(launcher_exe_dir);
    let mode = if is_dev {
        RuntimeMode::current_path_only(launcher_exe_dir)
    } else {
        RuntimeMode::current(launcher_exe_dir)
    };
    // For dev builds the launcher MUST use `resolve_path_only` too —
    // not just `resolve` — so that AGENTMUX_CHANNEL is ignored
    // symmetrically with the host's dev-build branch in
    // agentmux-cef/src/main.rs and sidecar.rs. Without this, the
    // launcher would honor a leaked `AGENTMUX_CHANNEL` from a parent
    // agentmux pane and write the lockfile + IPC files into
    // `channels/<override>/runtime/`, while the host (running its own
    // path-only resolution) would look for them under
    // `dev/<branch>/runtime/`. Launcher/host disagreement on the
    // single-instance lock breaks dev-mode isolation. Channel
    // override is intentionally an Installed/Portable-only feature in
    // this design (codex P2 follow-up on PR #1027); dev mode keeps
    // its per-branch isolation as the sole identity axis.
    let common = if is_dev {
        CommonDataPaths::resolve_path_only(version, &mode)?
    } else {
        CommonDataPaths::resolve(version, &mode)?
    };

    let portable_root = if mode == RuntimeMode::Portable {
        Some(launcher_exe_dir.to_path_buf())
    } else {
        None
    };

    Ok(DataPaths {
        data_dir: common.data_dir.clone(),
        config_dir: common.config_dir.clone(),
        // The launcher's `config.toml` (saga retention etc.) lives
        // at `~/.agentmux/config.toml` — account-wide, version-
        // independent, predates the unified layout. Map onto the
        // resolved root, NOT shared_dir or any version-keyed subdir.
        user_home_dir: common.home_dir.clone(),
        portable_root,
        common,
    })
}

/// Read-only resolver for the launcher saga log path. Returns
/// whichever location actually holds the data: the canonical
/// `<data-dir>/db/launcher-sagas.db` if present, otherwise the
/// legacy `<data-dir>/launcher-sagas.db` if THAT exists, otherwise
/// the canonical path (for the fresh-install case).
///
/// Does NOT touch the filesystem — safe for read-only callers like
/// `--diag sagas` which document themselves as passive.
pub fn launcher_saga_log_path_read_only(data_dir: &Path) -> PathBuf {
    let new_path = data_dir.join("db").join("launcher-sagas.db");
    if new_path.exists() {
        return new_path;
    }
    let legacy_path = data_dir.join("launcher-sagas.db");
    if legacy_path.exists() {
        return legacy_path;
    }
    new_path
}

/// Canonical path for the launcher saga log
/// (`<data-dir>/db/launcher-sagas.db`). Performs a one-shot back-
/// compat migration: launcher releases prior to this change wrote
/// the saga log directly under `<data-dir>/launcher-sagas.db` (with
/// srv DBs alongside in `<data-dir>/db/`, an inconsistency flagged
/// by AUDIT_SQLITE_SYSTEMS §8.3). If only the legacy path exists,
/// move it into `db/`.
///
/// This variant has WRITE side effects (rename + mkdir). Callers
/// that must stay read-only (e.g. `--diag sagas` is documented as
/// a passive on-disk inspector) should use
/// `launcher_saga_log_path_read_only` instead. The launcher's own
/// startup path uses this one — the rename is welcome there.
///
/// Idempotent + safe to call repeatedly. Returns the canonical
/// (post-migration) path the caller should open.
pub fn launcher_saga_log_path(data_dir: &Path) -> PathBuf {
    let db_dir = data_dir.join("db");
    let new_path = db_dir.join("launcher-sagas.db");
    let legacy_path = data_dir.join("launcher-sagas.db");

    // Migrate iff only the legacy file is present. Don't overwrite a
    // non-empty new file even if both exist — that would be a
    // surprising data loss and likely indicates a multi-process race
    // we'd want to investigate.
    if legacy_path.exists() && !new_path.exists() {
        // Best-effort `mkdir -p`. If this fails the rename will fail
        // and we'll log + fall back to the legacy path below.
        let _ = std::fs::create_dir_all(&db_dir);
        if let Err(e) = std::fs::rename(&legacy_path, &new_path) {
            // Migration failed — keep using the legacy path so we
            // don't drop saga state. The next launch retries.
            eprintln!(
                "[launcher-saga-log] migration of {} → {} failed: {} \
                 (continuing with legacy path)",
                legacy_path.display(),
                new_path.display(),
                e
            );
            return legacy_path;
        }
    }
    new_path
}

/// Create every directory the launcher + srv expect to exist.
/// Idempotent. Delegates to the common implementation.
pub fn ensure_dirs(paths: &DataPaths) -> Result<(), String> {
    // Migrate BEFORE ensure_dirs creates the destination dirs.
    // ensure_dirs creates <new>/data/db/ as an empty directory; if we
    // checked for db dir existence AFTER that, we'd always see it and
    // skip the copy (codex P1 on #1227).
    migrate_legacy_data_dir(paths);
    paths.common.ensure_dirs()
}

/// One-time migration: copy `channels/<ch>/data/db/` into the new
/// `channels/<ch>/versions/<v>/data/db/` when:
///   (a) the new versioned db dir has no DB files yet, AND
///   (b) the old unversioned db dir does exist (pre-Phase-2 install).
///
/// Uses *copy* not *move* so the old data is preserved for older
/// binaries that may still be installed. Silent on any I/O error —
/// a fresh DB is safer than a partially migrated one blocking launch.
fn migrate_legacy_data_dir(paths: &DataPaths) {
    let new_db = paths.data_dir.join("db");
    // Check for an actual DB file, not just the directory — ensure_dirs
    // may have already created the empty dir on a previous failed launch.
    let has_db_files = new_db.is_dir()
        && std::fs::read_dir(&new_db)
            .map(|mut d| d.any(|e| e.map(|e| e.path().extension().and_then(|x| x.to_str()) == Some("db")).unwrap_or(false)))
            .unwrap_or(false);
    if has_db_files {
        return; // already migrated or fresh install with data
    }
    // The legacy dir is the parent of data_dir in the old layout:
    // channels/<ch>/data/db/ (instance_dir/data/db/).
    // After Phase 2, data_dir is channels/<ch>/versions/<v>/data/.
    // The old path was channels/<ch>/data/.  Walk up from data_dir's
    // grandparent (versions/) to reach instance_dir.
    let old_db = match paths.data_dir.parent().and_then(|v| v.parent()) {
        Some(versions_dir) => versions_dir.parent().map(|ch| ch.join("data").join("db")),
        None => None,
    };
    let old_db = match old_db {
        Some(p) if p.is_dir() => p,
        _ => return, // no legacy data — fresh install
    };
    crate::log(&format!(
        "[migrate] copying legacy data dir {} → {}",
        old_db.display(),
        new_db.display()
    ));
    if let Err(e) = copy_dir_recursive(&old_db, &new_db) {
        crate::log(&format!("[migrate] copy failed (fresh db will be used): {}", e));
    }
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path)?;
        } else {
            std::fs::copy(entry.path(), dst_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn launcher_saga_log_path_returns_db_subdir_on_fresh_install() {
        let tmp = tempdir().unwrap();
        let p = launcher_saga_log_path(tmp.path());
        assert_eq!(p, tmp.path().join("db").join("launcher-sagas.db"));
        // No legacy file → no rename attempt; new path simply returned.
        assert!(!tmp.path().join("launcher-sagas.db").exists());
    }

    #[test]
    fn launcher_saga_log_path_migrates_legacy_file() {
        let tmp = tempdir().unwrap();
        let legacy = tmp.path().join("launcher-sagas.db");
        std::fs::write(&legacy, b"legacy bytes").unwrap();

        let p = launcher_saga_log_path(tmp.path());

        assert_eq!(p, tmp.path().join("db").join("launcher-sagas.db"));
        assert!(p.exists());
        assert_eq!(std::fs::read(&p).unwrap(), b"legacy bytes");
        assert!(!legacy.exists(), "legacy path should be removed by rename");
    }

    #[test]
    fn launcher_saga_log_path_does_not_overwrite_existing_new_file() {
        // Both files exist (theoretical race / aborted migration).
        // Keep the new file untouched and leave the legacy alone.
        let tmp = tempdir().unwrap();
        let legacy = tmp.path().join("launcher-sagas.db");
        let new_dir = tmp.path().join("db");
        std::fs::create_dir_all(&new_dir).unwrap();
        let new_path = new_dir.join("launcher-sagas.db");
        std::fs::write(&legacy, b"legacy").unwrap();
        std::fs::write(&new_path, b"newer").unwrap();

        let p = launcher_saga_log_path(tmp.path());

        assert_eq!(p, new_path);
        assert_eq!(std::fs::read(&p).unwrap(), b"newer");
        // Legacy stays (user can clean up manually); we don't trash it.
        assert!(legacy.exists());
    }

    #[test]
    fn read_only_resolver_returns_canonical_when_neither_file_exists() {
        let tmp = tempdir().unwrap();
        let p = launcher_saga_log_path_read_only(tmp.path());
        assert_eq!(p, tmp.path().join("db").join("launcher-sagas.db"));
        // Crucially, no side effects — no `db/` dir created.
        assert!(!tmp.path().join("db").exists());
    }

    #[test]
    fn read_only_resolver_returns_legacy_path_when_only_legacy_exists() {
        let tmp = tempdir().unwrap();
        let legacy = tmp.path().join("launcher-sagas.db");
        std::fs::write(&legacy, b"legacy").unwrap();

        let p = launcher_saga_log_path_read_only(tmp.path());

        assert_eq!(p, legacy);
        assert!(legacy.exists(), "read-only resolver must not migrate");
        assert!(!tmp.path().join("db").exists());
    }

    #[test]
    fn read_only_resolver_returns_canonical_when_canonical_exists() {
        let tmp = tempdir().unwrap();
        let db_dir = tmp.path().join("db");
        std::fs::create_dir_all(&db_dir).unwrap();
        let new_path = db_dir.join("launcher-sagas.db");
        std::fs::write(&new_path, b"current").unwrap();

        let p = launcher_saga_log_path_read_only(tmp.path());
        assert_eq!(p, new_path);
    }

    #[test]
    fn launcher_saga_log_path_is_idempotent() {
        let tmp = tempdir().unwrap();
        let legacy = tmp.path().join("launcher-sagas.db");
        std::fs::write(&legacy, b"data").unwrap();

        let first = launcher_saga_log_path(tmp.path());
        let second = launcher_saga_log_path(tmp.path());
        assert_eq!(first, second);
        assert!(first.exists());
    }
}
