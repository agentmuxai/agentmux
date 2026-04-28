// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Path resolution mirroring `agentmux-cef/src/sidecar.rs` so the
// launcher and host arrive at IDENTICAL data_dir / config_dir / user
// home paths. Phase B's launcher-spawns-srv flow needs the launcher
// to know these paths so it can pass them to both srv (via env) and
// host (via env) — and the host's own copy of the resolution logic
// must agree, since the host falls back to its own computation in
// dev mode (`task dev`) where the launcher isn't in the loop.
//
// Portable detection differs slightly between the two processes:
//   * Launcher's `exe_dir` IS the portable root if `runtime/` exists
//     alongside it (because the launcher binary lives at the top of
//     the portable folder).
//   * Host's `exe_dir` is INSIDE `runtime/`, so the host's portable
//     root is `exe_dir.parent()`.
// Both arrive at the same `<portable-root>/data/` directory.
//
// Installed mode is identical for both: `dirs::data_dir() / "ai.agentmux.cef.v{version_slug}"`.
//
// Spec: specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md §3.2

use std::path::{Path, PathBuf};

/// Resolved per-instance paths shared by host + srv + (after B.1)
/// the launcher itself.
#[derive(Debug, Clone)]
pub struct DataPaths {
    /// Backend data dir — srv reads/writes its DB here (under
    /// `data_dir/db/`) and gets it as `--wavedata`.
    /// Portable: `<portable-root>/data/`.
    /// Installed: `%LOCALAPPDATA%/ai.agentmux.cef.v{ver}/`.
    ///
    /// NOT the CEF cache dir — that's a separate path computed in the
    /// host's main.rs (`<portable-root>/data/cef/` for portable). The
    /// two coincide in installed mode but diverge in portable. Don't
    /// conflate them — doing so silently moves srv DB on upgrade.
    /// (reagent / codex P1 + P2 on PR #571 round-1.)
    pub data_dir: PathBuf,
    /// Per-instance config + settings.
    /// Portable: `<portable-root>/data/config/`.
    /// Installed: `%APPDATA%/ai.agentmux.cef.v{ver}/`.
    pub config_dir: PathBuf,
    /// Per-agent user home (workspaces, GH_CONFIG_DIR, etc.).
    /// Portable: same as `data_dir`.
    /// Installed: `~/.agentmux`.
    /// Overridden by `AGENTMUX_DATA_HOME` env if set.
    pub user_home_dir: PathBuf,
    /// Some(<root>) when running portable, else None.
    pub portable_root: Option<PathBuf>,
}

/// Resolve all paths from the LAUNCHER's vantage point.
///
/// `launcher_exe_dir` should be `current_exe().parent()` — the directory
/// the launcher binary lives in. In portable mode this is the portable
/// root (because the launcher is at the top of the portable folder).
///
/// `version` is the cargo package version (e.g. `0.33.442`).
///
/// `is_dev` mirrors `cfg!(debug_assertions)` from the host so installed-
/// mode dirs match between the two processes.
pub fn resolve_paths(
    launcher_exe_dir: &Path,
    version: &str,
    is_dev: bool,
) -> Result<DataPaths, String> {
    // Portable detection from the launcher's perspective: if a `runtime/`
    // subdir exists alongside us, we're at the top of the portable folder.
    let portable_root: Option<PathBuf> = if launcher_exe_dir.join("runtime").is_dir() {
        Some(launcher_exe_dir.to_path_buf())
    } else {
        None
    };

    let (data_dir, config_dir) = if let Some(ref root) = portable_root {
        // Portable backend layout (must match
        // `agentmux-cef/src/sidecar.rs:54-56` — that's the `task dev`
        // fallback path, both branches must agree on where srv reads
        // and writes its DB):
        //   data_dir   = <root>/data           (srv DB at data/db)
        //   config_dir = <root>/data/config
        //   cef cache  = <root>/data/cef       (host-only, computed in
        //                                       host main.rs from the
        //                                       host's own portable
        //                                       detection — NOT this
        //                                       function's concern)
        //
        // Important: data_dir is NOT data/cef. Earlier B.1 draft
        // had `base.join("cef")` here which would have moved srv DB
        // to data/cef/db on upgrade — silent data loss for existing
        // portable users. (reagent P1 + codex P1 on PR #571 round-1.)
        let base = root.join("data");
        (base.clone(), base.join("config"))
    } else {
        // Installed: version-isolated dirs in platform AppData.
        let dir_name = if is_dev {
            "ai.agentmux.cef.dev".to_string()
        } else {
            let version_slug = version.replace('.', "-");
            format!("ai.agentmux.cef.v{}", version_slug)
        };
        let data = dirs::data_dir()
            .ok_or_else(|| "dirs::data_dir() returned None".to_string())?
            .join(&dir_name);
        let config = dirs::config_dir()
            .ok_or_else(|| "dirs::config_dir() returned None".to_string())?
            .join(&dir_name);
        (data, config)
    };

    // user_home_dir mirrors the host's logic (sidecar.rs):
    //   1. AGENTMUX_DATA_HOME env override wins
    //   2. Portable: same as data_dir
    //   3. Installed: ~/.agentmux
    let user_home_dir = std::env::var("AGENTMUX_DATA_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if portable_root.is_some() {
                data_dir.clone()
            } else {
                dirs::home_dir()
                    .unwrap_or_default()
                    .join(".agentmux")
            }
        });

    Ok(DataPaths {
        data_dir,
        config_dir,
        user_home_dir,
        portable_root,
    })
}

/// Ensure the directories the launcher + srv expect to exist.
///
/// Mirrors host's `spawn_backend` (sidecar.rs:78–81) — creates
/// `data_dir/db` and `config_dir`. Idempotent.
pub fn ensure_dirs(paths: &DataPaths) -> Result<(), String> {
    std::fs::create_dir_all(paths.data_dir.join("db"))
        .map_err(|e| format!("create data_dir/db failed: {}", e))?;
    std::fs::create_dir_all(&paths.config_dir)
        .map_err(|e| format!("create config_dir failed: {}", e))?;
    Ok(())
}
