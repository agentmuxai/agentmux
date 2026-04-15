// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tool store: ensures CLI tools (jq, rg, etc.) are available to agent
//! subprocesses. Reads a bundled catalog JSON, checks system/bundled/managed
//! install paths, and can download + verify tools on demand.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// Embedded catalog — path is relative to THIS FILE's location.
// File: agentmux-srv/src/backend/tool_store.rs
// Catalog: agentmux-srv/src/config/tool-catalog.json
const CATALOG_JSON: &str = include_str!("../config/tool-catalog.json");

// ---- Catalog structs ----

#[derive(Debug, Clone, Deserialize)]
pub struct ToolCatalog {
    pub tools: Vec<ToolSpec>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolSpec {
    pub id: String,
    pub display: String,
    pub description: String,
    pub tier: u8,
    pub version: String,
    pub check_cmd: String,
    pub bundled: bool,
    pub platforms: HashMap<String, PlatformSpec>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlatformSpec {
    pub url: String,
    pub sha256: String,
    /// "none" | "zip" | "tar_gz"
    pub extract: String,
    /// Final filename placed in the bin/ directory.
    pub bin: String,
    /// Path of the target file inside the archive (zip/tar_gz only).
    pub bin_in_archive: Option<String>,
}

// ---- Status types ----

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    InstalledSystem,
    InstalledBundled,
    InstalledManaged,
    Missing,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStatusEntry {
    pub id: String,
    pub display: String,
    pub description: String,
    pub tier: u8,
    pub status: ToolStatus,
    pub version: Option<String>,
    pub path: Option<String>,
}

// ---- Directory helpers ----

/// Returns the bundled tools bin dir: `<exe_dir>/tools/bin/`.
///
/// In development (binary lives under `target/debug/` or `target/release/`)
/// we return `None` so dev builds don't accidentally depend on a non-existent
/// bundled dir.
pub fn bundled_tools_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;

    // Skip if we're running from a Cargo target directory.
    let exe_str = exe_dir.to_string_lossy();
    if exe_str.contains("target/debug")
        || exe_str.contains("target/release")
        || exe_str.contains("target\\debug")
        || exe_str.contains("target\\release")
    {
        return None;
    }

    let bundled = exe_dir.join("tools").join("bin");
    Some(bundled)
}

/// Returns `~/.agentmux/tools/bin/`.
pub fn user_tools_dir() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join(".agentmux").join("tools").join("bin"))
}

/// Returns `~/.agentmux/tools/downloads/`.
fn downloads_dir() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join(".agentmux").join("tools").join("downloads"))
}

// ---- Catalog ----

/// Load and parse the embedded catalog.
pub fn load_catalog() -> Result<ToolCatalog, String> {
    serde_json::from_str(CATALOG_JSON).map_err(|e| format!("parse tool-catalog.json: {e}"))
}

// ---- Platform detection ----

/// Returns the platform key for the current compile target.
pub fn current_platform() -> Result<&'static str, String> {
    // Use std::env::consts at runtime so this compiles on every platform
    // without unreachable-expression warnings.
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Ok("windows-x64"),
        ("macos", "aarch64") => Ok("macos-arm64"),
        ("macos", "x86_64") => Ok("macos-x64"),
        ("linux", "x86_64") => Ok("linux-x64"),
        ("linux", "aarch64") => Ok("linux-arm64"),
        (os, arch) => Err(format!("unsupported platform: os={os} arch={arch}")),
    }
}

// ---- System PATH probe ----

/// Returns true if `name` can be found on the system PATH.
fn probe_system_path(name: &str) -> bool {
    #[cfg(windows)]
    {
        std::process::Command::new("where")
            .arg(name)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("which")
            .arg(name)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

/// Returns the system PATH location of `name`, or None.
fn system_path_of(name: &str) -> Option<String> {
    #[cfg(windows)]
    let output = std::process::Command::new("where").arg(name).output().ok()?;
    #[cfg(not(windows))]
    let output = std::process::Command::new("which").arg(name).output().ok()?;

    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if !path.is_empty() {
            return Some(path);
        }
    }
    None
}

// ---- Status query ----

/// Get the install status of every tool in the catalog.
///
/// Priority: system PATH > bundled dir > user-managed dir.
pub fn get_tool_statuses() -> Vec<ToolStatusEntry> {
    let catalog = match load_catalog() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "failed to load tool catalog");
            return Vec::new();
        }
    };

    let platform = current_platform().ok();

    catalog
        .tools
        .iter()
        .map(|spec| {
            // Check platform availability first so we can return Unavailable early.
            let platform_key = match &platform {
                Some(k) => *k,
                None => {
                    return ToolStatusEntry {
                        id: spec.id.clone(),
                        display: spec.display.clone(),
                        description: spec.description.clone(),
                        tier: spec.tier,
                        status: ToolStatus::Unavailable,
                        version: None,
                        path: None,
                    };
                }
            };

            if !spec.platforms.contains_key(platform_key) {
                return ToolStatusEntry {
                    id: spec.id.clone(),
                    display: spec.display.clone(),
                    description: spec.description.clone(),
                    tier: spec.tier,
                    status: ToolStatus::Unavailable,
                    version: None,
                    path: None,
                };
            }

            let bin_name = &spec.platforms[platform_key].bin;

            // 1. System PATH wins.
            if probe_system_path(&spec.check_cmd) {
                let path = system_path_of(&spec.check_cmd);
                return ToolStatusEntry {
                    id: spec.id.clone(),
                    display: spec.display.clone(),
                    description: spec.description.clone(),
                    tier: spec.tier,
                    status: ToolStatus::InstalledSystem,
                    version: Some(spec.version.clone()),
                    path,
                };
            }

            // 2. Bundled dir.
            if let Some(bundled) = bundled_tools_dir() {
                let p = bundled.join(bin_name);
                if p.exists() {
                    return ToolStatusEntry {
                        id: spec.id.clone(),
                        display: spec.display.clone(),
                        description: spec.description.clone(),
                        tier: spec.tier,
                        status: ToolStatus::InstalledBundled,
                        version: Some(spec.version.clone()),
                        path: Some(p.to_string_lossy().into_owned()),
                    };
                }
            }

            // 3. User-managed store.
            if let Some(user_bin) = user_tools_dir() {
                let p = user_bin.join(bin_name);
                if p.exists() {
                    return ToolStatusEntry {
                        id: spec.id.clone(),
                        display: spec.display.clone(),
                        description: spec.description.clone(),
                        tier: spec.tier,
                        status: ToolStatus::InstalledManaged,
                        version: Some(spec.version.clone()),
                        path: Some(p.to_string_lossy().into_owned()),
                    };
                }
            }

            // 4. Not found.
            ToolStatusEntry {
                id: spec.id.clone(),
                display: spec.display.clone(),
                description: spec.description.clone(),
                tier: spec.tier,
                status: ToolStatus::Missing,
                version: None,
                path: None,
            }
        })
        .collect()
}

// ---- Installation ----

/// Download, verify, and install a single tool by ID into the user-managed
/// store (`~/.agentmux/tools/bin/`).
///
/// Returns the installed binary path on success.
pub async fn install_tool(id: &str, http_client: &reqwest::Client) -> Result<String, String> {
    let catalog = load_catalog()?;
    let spec = catalog
        .tools
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| format!("tool '{id}' not found in catalog"))?;

    let platform_key = current_platform()?;
    let platform_spec = spec
        .platforms
        .get(platform_key)
        .ok_or_else(|| format!("tool '{id}' has no entry for platform '{platform_key}'"))?;

    // Ensure directories exist.
    let dl_dir = downloads_dir().ok_or("cannot determine downloads dir")?;
    let bin_dir = user_tools_dir().ok_or("cannot determine user tools dir")?;
    std::fs::create_dir_all(&dl_dir)
        .map_err(|e| format!("create downloads dir: {e}"))?;
    std::fs::create_dir_all(&bin_dir)
        .map_err(|e| format!("create bin dir: {e}"))?;

    // Download to a temp file.
    let dl_path = dl_dir.join(format!("{id}-download"));
    tracing::info!(tool = %id, url = %platform_spec.url, dest = %dl_path.display(), "downloading tool");

    {
        let bytes = http_client
            .get(&platform_spec.url)
            .send()
            .await
            .map_err(|e| format!("download '{id}': {e}"))?
            .error_for_status()
            .map_err(|e| format!("download '{id}' HTTP error: {e}"))?
            .bytes()
            .await
            .map_err(|e| format!("read download body '{id}': {e}"))?;

        std::fs::write(&dl_path, &bytes)
            .map_err(|e| format!("write download file: {e}"))?;
    }

    // Verify SHA-256.
    {
        let data = std::fs::read(&dl_path)
            .map_err(|e| format!("read downloaded file for hash: {e}"))?;
        let mut hasher = Sha256::new();
        hasher.update(&data);
        let digest = hex::encode(hasher.finalize());
        if digest != platform_spec.sha256 {
            let _ = std::fs::remove_file(&dl_path);
            return Err(format!(
                "SHA-256 mismatch for '{id}': expected={} got={digest}",
                platform_spec.sha256,
            ));
        }
        tracing::debug!(tool = %id, "SHA-256 verified");
    }

    // Extract / copy into bin dir.
    let dest_path = bin_dir.join(&platform_spec.bin);

    match platform_spec.extract.as_str() {
        "none" => {
            std::fs::copy(&dl_path, &dest_path)
                .map_err(|e| format!("copy '{id}' to bin: {e}"))?;
        }
        "zip" => {
            let archive_in = platform_spec
                .bin_in_archive
                .as_deref()
                .ok_or_else(|| format!("tool '{id}': extract=zip but bin_in_archive is missing"))?;

            let zip_data = std::fs::read(&dl_path)
                .map_err(|e| format!("read zip file: {e}"))?;
            let cursor = std::io::Cursor::new(zip_data);
            let mut archive = zip::ZipArchive::new(cursor)
                .map_err(|e| format!("open zip archive: {e}"))?;

            let mut entry = archive
                .by_name(archive_in)
                .map_err(|e| format!("zip entry '{archive_in}': {e}"))?;

            let mut out = std::fs::File::create(&dest_path)
                .map_err(|e| format!("create dest file: {e}"))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| format!("extract zip entry: {e}"))?;
        }
        "tar_gz" => {
            let archive_in = platform_spec
                .bin_in_archive
                .as_deref()
                .ok_or_else(|| format!("tool '{id}': extract=tar_gz but bin_in_archive is missing"))?;

            let gz_file = std::fs::File::open(&dl_path)
                .map_err(|e| format!("open tar.gz: {e}"))?;
            let gz_decoder = flate2::read::GzDecoder::new(gz_file);
            let mut archive = tar::Archive::new(gz_decoder);

            let mut found = false;
            for entry in archive.entries().map_err(|e| format!("tar entries: {e}"))? {
                let mut entry = entry.map_err(|e| format!("tar entry: {e}"))?;
                let path = entry.path().map_err(|e| format!("tar entry path: {e}"))?;
                if path.to_string_lossy() == archive_in {
                    entry
                        .unpack(&dest_path)
                        .map_err(|e| format!("unpack tar entry: {e}"))?;
                    found = true;
                    break;
                }
            }
            if !found {
                return Err(format!(
                    "tar archive does not contain expected entry '{archive_in}'"
                ));
            }
        }
        other => {
            return Err(format!("unknown extract mode '{other}' for tool '{id}'"));
        }
    }

    // Set executable bit on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut perms = std::fs::metadata(&dest_path)
            .map_err(|e| format!("stat installed binary: {e}"))?
            .permissions();
        let mode = perms.mode();
        perms.set_mode(mode | 0o111);
        std::fs::set_permissions(&dest_path, perms)
            .map_err(|e| format!("set executable bit: {e}"))?;
    }

    // Clean up download.
    let _ = std::fs::remove_file(&dl_path);

    let dest_str = dest_path.to_string_lossy().into_owned();
    tracing::info!(tool = %id, path = %dest_str, "tool installed successfully");
    Ok(dest_str)
}
