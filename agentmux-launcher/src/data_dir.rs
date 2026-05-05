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
    let mode = RuntimeMode::current(launcher_exe_dir);
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

/// Create every directory the launcher + srv expect to exist.
/// Idempotent. Delegates to the common implementation.
pub fn ensure_dirs(paths: &DataPaths) -> Result<(), String> {
    paths.common.ensure_dirs()
}
