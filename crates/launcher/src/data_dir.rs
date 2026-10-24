// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Compatibility shim around `agentmux_common::DataPaths`.
//!
//! Historically the launcher computed its own paths via the
//! launcher-local `resolve_paths()` function. After the data-dir
//! unification (see docs/specs/archive/SPEC_DATA_DIR_UNIFICATION_2026-05-05.md
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

    // A portable/installed build launched from INSIDE another AgentMux pane
    // inherits the parent's AGENTMUX_CHANNEL + AGENTMUX_RUNTIME_MODE (the srv
    // sets AGENTMUX=1 and the full AGENTMUX_* path env for every pane shell —
    // `crates/srv/.../blockcontroller/shell.rs`). Honoring that *leaked*
    // channel makes the new build adopt the PARENT's data dir + cef-cache, so
    // Chromium's user-data-dir singleton forwards it into the parent and it exits
    // ("Opening in existing browser session", CEF exit 24) — the build you
    // launched never runs. Treat a nested launch like a dev build: ignore the
    // ambient channel/mode and resolve from this binary's BAKED per-build channel
    // (`AGENTMUX_BUILD_CHANNEL_DEFAULT`). An EXPLICIT, *standalone*
    // `AGENTMUX_CHANNEL=… ./agentmux` (not nested) is still honored — that is the
    // intentional parallel-channel override (PR #1027); only the leak is
    // suppressed. `AGENTMUX` is the canonical "inside a pane" sentinel.
    let nested = std::env::var_os("AGENTMUX").is_some();
    let ignore_ambient = is_dev || nested;

    let mode = if ignore_ambient {
        RuntimeMode::current_path_only(launcher_exe_dir)
    } else {
        RuntimeMode::current(launcher_exe_dir)
    };
    // The launcher MUST use `resolve_path_only` (not `resolve`) whenever it
    // ignores the ambient channel — so AGENTMUX_CHANNEL is dropped symmetrically
    // with the host's dev/nested branch in crates/cef/src/main.rs and
    // sidecar.rs. Without this the launcher would honor a leaked `AGENTMUX_CHANNEL`
    // and write the lockfile + IPC files into `channels/<override>/runtime/`,
    // while the host (path-only) looks elsewhere — launcher/host disagreement on
    // the single-instance lock breaks isolation. The launcher's resolved env is
    // authoritative for the host + srv it spawns (`to_env_vars()` overwrites the
    // inherited AGENTMUX_* at every spawn site), so fixing it here fixes the whole
    // chain.
    let common = if ignore_ambient {
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

/// This channel's `settings.json`, the file srv loads (#1903), resolved
/// early and failure-tolerant for the launcher's own startup reads.
pub fn settings_file() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let version = env!("CARGO_PKG_VERSION");
    resolve_paths(exe.parent()?, version).ok().map(|p| p.common.settings_file())
}

/// Create every directory the launcher + srv expect to exist.
/// Idempotent. Delegates to the common implementation.
///
/// It puts no database in them: what a new version's data starts from is
/// srv's to decide, on its first start (`crates/srv/src/bootstrap/carry_forward.rs`:
/// the newest older version's databases, else the channel's pre-versioning
/// `data/db/`). The launcher used to copy that pre-versioning folder here
/// whenever a version's `data/db/` had no database yet, which is every
/// upgrade: srv then found an `objects.db` in place and carried nothing
/// forward, so each upgrade started from that old data instead of the
/// previous version's, without the layout its quit had saved.
pub fn ensure_dirs(paths: &DataPaths) -> Result<(), String> {
    paths.common.ensure_dirs()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The launcher's paths for `version` of the `stable` channel under `root`.
    fn paths(root: &Path, version: &str) -> DataPaths {
        let channel_dir = root.join("channels").join("stable");
        let version_dir = channel_dir.join("versions").join(version);
        let common = CommonDataPaths {
            home_dir: root.to_path_buf(),
            instance_dir: channel_dir.clone(),
            channel: "stable".to_string(),
            data_dir: version_dir.join("data"),
            config_dir: channel_dir.join("config"),
            logs_dir: version_dir.join("logs"),
            cef_cache_dir: version_dir.join("cef-cache"),
            agents_dir: channel_dir.join("agents"),
            instance_runtime_dir: version_dir.join("runtime"),
            shared_dir: root.join("shared"),
            mode: RuntimeMode::Installed,
        };
        DataPaths {
            data_dir: common.data_dir.clone(),
            config_dir: common.config_dir.clone(),
            user_home_dir: common.home_dir.clone(),
            portable_root: None,
            common,
        }
    }

    fn put_db(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("objects.db"), b"not opened").unwrap();
    }

    /// The launcher never seeds a version's databases: srv copies the
    /// previous version's forward on its first start, and finding an
    /// `objects.db` already there it copies nothing. A copy of the channel's
    /// pre-versioning `data/db/` made here started every upgrade from that
    /// months-old data instead, without the layout the quit had saved
    /// (`docs/incident/RCA_UPGRADE_LOSES_LAYOUT_2026_10_22.md`).
    #[test]
    fn ensure_dirs_leaves_a_new_versions_databases_to_srv() {
        let root = tempfile::tempdir().unwrap();
        let channel_dir = root.path().join("channels").join("stable");
        put_db(&channel_dir.join("data").join("db"));
        put_db(&channel_dir.join("versions").join("0.59.38").join("data").join("db"));

        let new = paths(root.path(), "0.59.39");
        ensure_dirs(&new).unwrap();

        let db_dir = new.data_dir.join("db");
        let files: Vec<_> = std::fs::read_dir(&db_dir)
            .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name()).collect())
            .unwrap_or_default();
        assert!(files.is_empty(), "the launcher put {files:?} in {}", db_dir.display());
    }
}
