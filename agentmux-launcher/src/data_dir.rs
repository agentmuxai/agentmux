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
    let mode = if agentmux_common::is_dev_build_exe(launcher_exe_dir) {
        RuntimeMode::current_path_only(launcher_exe_dir)
    } else {
        RuntimeMode::current(launcher_exe_dir)
    };
    let common = CommonDataPaths::resolve(version, &mode)?;

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

/// Canonical path for the launcher saga log
/// (`<data-dir>/db/launcher-sagas.db`). Performs a one-shot back-
/// compat migration: launcher releases prior to this change wrote
/// the saga log directly under `<data-dir>/launcher-sagas.db` (with
/// srv DBs alongside in `<data-dir>/db/`, an inconsistency flagged
/// by AUDIT_SQLITE_SYSTEMS §8.3). If only the legacy path exists,
/// move it into `db/`.
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
    paths.common.ensure_dirs()
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
