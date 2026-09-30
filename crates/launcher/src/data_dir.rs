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

    // A portable/installed build launched from INSIDE another AgentMux pane
    // inherits the parent's AGENTMUX_CHANNEL + AGENTMUX_RUNTIME_MODE (the srv
    // sets AGENTMUX=1 and the full AGENTMUX_* path env for every pane shell —
    // `agentmux-srv/.../blockcontroller/shell.rs`). Honoring that *leaked*
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
    // with the host's dev/nested branch in agentmux-cef/src/main.rs and
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
