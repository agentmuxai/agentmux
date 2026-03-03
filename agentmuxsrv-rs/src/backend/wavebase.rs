// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Wave base utilities: directory management, lock files, environment, platform detection.
//! Port of Go's pkg/wavebase/.

#![allow(dead_code)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

// ---- Environment variable names ----

pub const WAVE_CONFIG_HOME_ENV: &str = "AGENTMUX_CONFIG_HOME";
pub const WAVE_DATA_HOME_ENV: &str = "AGENTMUX_DATA_HOME";
pub const WAVE_APP_PATH_ENV: &str = "AGENTMUX_APP_PATH";
pub const WAVE_DEV_ENV: &str = "AGENTMUX_DEV";
pub const WAVE_DEV_VITE_ENV: &str = "AGENTMUX_DEV_VITE";
pub const WAVE_WSH_FORCE_UPDATE_ENV: &str = "AGENTMUX_WSHFORCEUPDATE";
pub const WAVE_JWT_TOKEN_ENV: &str = "AGENTMUX_JWT";
pub const WAVE_SWAP_TOKEN_ENV: &str = "AGENTMUX_SWAPTOKEN";

// ---- File/directory constants ----

pub const WAVE_LOCK_FILE: &str = "wave.lock";
pub const DOMAIN_SOCKET_BASE_NAME: &str = "wave.sock";
pub const REMOTE_DOMAIN_SOCKET_BASE_NAME: &str = "wave-remote.sock";
pub const WAVE_DB_DIR: &str = "db";
pub const CONFIG_DIR: &str = "config";
pub const REMOTE_WAVE_HOME_DIR_NAME: &str = ".agentmux";
pub const REMOTE_WSH_BIN_DIR_NAME: &str = "bin";
pub const REMOTE_FULL_WSH_BIN_PATH: &str = "~/.agentmux/bin/wsh";
pub const REMOTE_FULL_DOMAIN_SOCKET_PATH: &str = "~/.agentmux/wave-remote.sock";

// ---- Version info (set at startup) ----

static WAVE_VERSION: OnceLock<String> = OnceLock::new();
static BUILD_TIME: OnceLock<String> = OnceLock::new();

/// Set the application version (called once at startup).
pub fn set_version(version: &str) {
    let _ = WAVE_VERSION.set(version.to_string());
}

/// Set the build time (called once at startup).
pub fn set_build_time(time: &str) {
    let _ = BUILD_TIME.set(time.to_string());
}

/// Get the application version.
pub fn get_version() -> &'static str {
    WAVE_VERSION.get().map_or("0.0.0", |v| v.as_str())
}

/// Get the build time.
pub fn get_build_time() -> &'static str {
    BUILD_TIME.get().map_or("0", |v| v.as_str())
}

// ---- Directory paths ----

/// Get the user's home directory.
pub fn get_home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// Get the Wave data directory.
/// Uses `AGENTMUX_DATA_HOME` env var, or defaults to `~/.agentmux`.
pub fn get_wave_data_dir() -> PathBuf {
    if let Ok(dir) = env::var(WAVE_DATA_HOME_ENV) {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    get_home_dir().join(".agentmux")
}

/// Migrate data from `~/.waveterm` to `~/.agentmux` if needed.
/// Called once at startup. No-op if `~/.agentmux` already exists.
pub fn migrate_legacy_data_dir() {
    let new_dir = get_wave_data_dir();
    if new_dir.exists() {
        return; // already migrated or freshly created
    }
    let old_dir = get_home_dir().join(".waveterm");
    if !old_dir.exists() {
        return; // nothing to migrate
    }
    tracing::info!(
        "Migrating data directory from {} to {}",
        old_dir.display(),
        new_dir.display()
    );
    if let Err(e) = copy_dir_all(&old_dir, &new_dir) {
        tracing::warn!("Data migration failed (continuing with empty dir): {}", e);
    } else {
        tracing::info!("Migration complete");
    }
}

/// Recursively copy a directory tree.
fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst)
        .map_err(|e| format!("cannot create {}: {}", dst.display(), e))?;
    for entry in fs::read_dir(src)
        .map_err(|e| format!("cannot read {}: {}", src.display(), e))?
    {
        let entry = entry.map_err(|e| format!("read_dir entry error: {}", e))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)
                .map_err(|e| format!("copy {} → {}: {}", src_path.display(), dst_path.display(), e))?;
        }
    }
    Ok(())
}

/// Get the Wave config directory.
/// Uses `AGENTMUX_CONFIG_HOME` env var, or defaults to `~/.agentmux/config`.
pub fn get_wave_config_dir() -> PathBuf {
    if let Ok(dir) = env::var(WAVE_CONFIG_HOME_ENV) {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    get_wave_data_dir().join(CONFIG_DIR)
}

/// Get the Wave DB directory (`~/.agentmux/db`).
pub fn get_wave_db_dir() -> PathBuf {
    get_wave_data_dir().join(WAVE_DB_DIR)
}

/// Get the Wave app path from env.
pub fn get_wave_app_path() -> Option<PathBuf> {
    env::var(WAVE_APP_PATH_ENV).ok().map(PathBuf::from)
}

/// Get the Wave app bin path.
pub fn get_wave_app_bin_path() -> Option<PathBuf> {
    get_wave_app_path().map(|p| p.join("bin"))
}

/// Get the domain socket path.
pub fn get_domain_socket_name() -> PathBuf {
    get_wave_data_dir().join(DOMAIN_SOCKET_BASE_NAME)
}

/// Get the Wave lock file path.
pub fn get_wave_lock_file() -> PathBuf {
    get_wave_data_dir().join(WAVE_LOCK_FILE)
}

// ---- Directory creation ----

/// Ensure a directory exists with the given permissions.
pub fn ensure_dir(dir: &Path) -> Result<(), String> {
    if dir.exists() {
        return Ok(());
    }
    fs::create_dir_all(dir).map_err(|e| format!("cannot create directory {}: {}", dir.display(), e))
}

/// Ensure the Wave data directory exists.
pub fn ensure_wave_data_dir() -> Result<(), String> {
    ensure_dir(&get_wave_data_dir())
}

/// Ensure the Wave DB directory exists.
pub fn ensure_wave_db_dir() -> Result<(), String> {
    ensure_dir(&get_wave_db_dir())
}

/// Ensure the Wave config directory exists.
pub fn ensure_wave_config_dir() -> Result<(), String> {
    ensure_dir(&get_wave_config_dir())
}

/// Ensure the Wave presets directory exists.
pub fn ensure_wave_presets_dir() -> Result<(), String> {
    ensure_dir(&get_wave_config_dir().join("presets"))
}

// ---- Lock file ----

/// File-based lock for single-instance enforcement.
pub struct WaveLock {
    #[allow(dead_code)]
    file: fs::File,
}

impl WaveLock {
    /// Acquire an exclusive lock on the Wave lock file.
    /// Returns error if another instance is already running.
    #[cfg(unix)]
    pub fn acquire() -> Result<Self, String> {
        use std::os::unix::io::AsRawFd;

        let lock_path = get_wave_lock_file();
        ensure_dir(lock_path.parent().unwrap_or(Path::new("/")))?;

        let file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| format!("cannot open lock file {}: {}", lock_path.display(), e))?;

        let fd = file.as_raw_fd();
        let result = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
        if result != 0 {
            return Err("another AgentMux instance is already running".to_string());
        }

        Ok(WaveLock { file })
    }

    /// Non-Unix fallback: just check the file can be created.
    #[cfg(not(unix))]
    pub fn acquire() -> Result<Self, String> {
        let lock_path = get_wave_lock_file();
        ensure_dir(lock_path.parent().unwrap_or(Path::new("/")))?;

        let file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&lock_path)
            .map_err(|e| format!("cannot open lock file {}: {}", lock_path.display(), e))?;

        Ok(WaveLock { file })
    }
}

// ---- Environment helpers ----

/// Check if Wave is in dev mode.
pub fn is_dev_mode() -> bool {
    env::var(WAVE_DEV_ENV)
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

/// Expand `~` at the start of a path to the home directory.
pub fn expand_home_dir(path: &str) -> Result<PathBuf, String> {
    if let Some(rest) = path.strip_prefix('~') {
        let home = get_home_dir();
        if rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\') {
            Ok(home.join(rest.trim_start_matches(['/', '\\'])))
        } else {
            Err(format!("cannot expand ~: path traversal in {}", path))
        }
    } else {
        Ok(PathBuf::from(path))
    }
}

/// Safe version of expand_home_dir that returns the original on error.
pub fn expand_home_dir_safe(path: &str) -> PathBuf {
    expand_home_dir(path).unwrap_or_else(|_| PathBuf::from(path))
}

/// Replace the home directory prefix with `~`.
pub fn replace_home_dir(path: &str) -> String {
    let home = get_home_dir();
    let home_str = home.to_string_lossy();
    if path.starts_with(home_str.as_ref()) {
        format!("~{}", &path[home_str.len()..])
    } else {
        path.to_string()
    }
}

// ---- Platform detection ----

/// Get the client architecture string ("os/arch").
pub fn client_arch() -> String {
    format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// Get a system summary string.
pub fn get_system_summary() -> String {
    format!(
        "{} {} ({})",
        std::env::consts::OS,
        std::env::consts::ARCH,
        whoami::distro()
    )
}

/// Validate that wsh is supported on the given os/arch combination.
pub fn validate_wsh_supported_arch(os: &str, arch: &str) -> Result<(), String> {
    match (os, arch) {
        ("linux", "amd64" | "x86_64" | "arm64" | "aarch64") => Ok(()),
        ("darwin", "amd64" | "x86_64" | "arm64" | "aarch64") => Ok(()),
        ("windows", "amd64" | "x86_64" | "arm64" | "aarch64") => Ok(()),
        _ => Err(format!("unsupported os/arch combination: {}/{}", os, arch)),
    }
}

/// Determine the system language.
pub fn determine_lang() -> String {
    env::var("LANG")
        .or_else(|_| env::var("LC_ALL"))
        .or_else(|_| env::var("LANGUAGE"))
        .unwrap_or_else(|_| "en_US".to_string())
}

// ---- Tests ----

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_var_constants() {
        assert_eq!(WAVE_CONFIG_HOME_ENV, "AGENTMUX_CONFIG_HOME");
        assert_eq!(WAVE_DATA_HOME_ENV, "AGENTMUX_DATA_HOME");
        assert_eq!(WAVE_DEV_ENV, "AGENTMUX_DEV");
    }

    #[test]
    fn test_file_constants() {
        assert_eq!(WAVE_LOCK_FILE, "wave.lock");
        assert_eq!(DOMAIN_SOCKET_BASE_NAME, "wave.sock");
        assert_eq!(WAVE_DB_DIR, "db");
        assert_eq!(CONFIG_DIR, "config");
    }

    #[test]
    fn test_version_management() {
        // Version defaults to "0.0.0" before being set
        // Can't test set_version since OnceLock can only be set once per process
        assert!(!get_version().is_empty());
    }

    #[test]
    fn test_wave_data_dir_default() {
        // When env var is not set, should be ~/.agentmux
        let dir = get_wave_data_dir();
        assert!(dir.to_string_lossy().contains(".agentmux") || dir.to_string_lossy().contains("AGENTMUX"));
    }

    #[test]
    fn test_wave_db_dir() {
        let db_dir = get_wave_db_dir();
        assert!(db_dir.to_string_lossy().ends_with("db"));
    }

    #[test]
    fn test_wave_config_dir() {
        let config_dir = get_wave_config_dir();
        assert!(config_dir.to_string_lossy().contains("config") || config_dir.to_string_lossy().contains("AGENTMUX"));
    }

    #[test]
    fn test_domain_socket_name() {
        let sock = get_domain_socket_name();
        assert!(sock.to_string_lossy().ends_with("wave.sock"));
    }

    #[test]
    fn test_lock_file_path() {
        let lock = get_wave_lock_file();
        assert!(lock.to_string_lossy().ends_with("wave.lock"));
    }

    #[test]
    fn test_expand_home_dir_tilde() {
        let path = expand_home_dir("~/docs").unwrap();
        assert!(path.to_string_lossy().contains("docs"));
        assert!(!path.to_string_lossy().starts_with('~'));
    }

    #[test]
    fn test_expand_home_dir_tilde_only() {
        let path = expand_home_dir("~").unwrap();
        assert!(!path.to_string_lossy().starts_with('~'));
    }

    #[test]
    fn test_expand_home_dir_no_tilde() {
        let path = expand_home_dir("/etc/hosts").unwrap();
        assert_eq!(path, PathBuf::from("/etc/hosts"));
    }

    #[test]
    fn test_expand_home_dir_traversal() {
        // ~user should fail (path traversal)
        let result = expand_home_dir("~otheruser/docs");
        assert!(result.is_err());
    }

    #[test]
    fn test_expand_home_dir_safe() {
        let path = expand_home_dir_safe("~/test");
        assert!(!path.to_string_lossy().starts_with('~'));

        // On error, returns original
        let path = expand_home_dir_safe("~otheruser/test");
        assert_eq!(path, PathBuf::from("~otheruser/test"));
    }

    #[test]
    fn test_replace_home_dir() {
        let home = get_home_dir();
        let path = format!("{}/documents/file.txt", home.display());
        let replaced = replace_home_dir(&path);
        assert!(replaced.starts_with('~'));
        assert!(replaced.contains("documents/file.txt"));
    }

    #[test]
    fn test_replace_home_dir_no_match() {
        let result = replace_home_dir("/etc/hosts");
        assert_eq!(result, "/etc/hosts");
    }

    #[test]
    fn test_client_arch() {
        let arch = client_arch();
        assert!(arch.contains('/'));
    }

    #[test]
    fn test_system_summary() {
        let summary = get_system_summary();
        assert!(!summary.is_empty());
    }

    #[test]
    fn test_validate_wsh_supported_arch() {
        assert!(validate_wsh_supported_arch("linux", "amd64").is_ok());
        assert!(validate_wsh_supported_arch("darwin", "arm64").is_ok());
        assert!(validate_wsh_supported_arch("windows", "x86_64").is_ok());
        assert!(validate_wsh_supported_arch("freebsd", "amd64").is_err());
        assert!(validate_wsh_supported_arch("linux", "mips").is_err());
    }

    #[test]
    fn test_determine_lang() {
        let lang = determine_lang();
        assert!(!lang.is_empty());
    }

    #[test]
    fn test_is_dev_mode() {
        // Default should be false in test environment (unless set)
        let _ = is_dev_mode(); // Just verify it doesn't panic
    }

    #[test]
    fn test_ensure_dir_existing() {
        // /tmp should already exist
        assert!(ensure_dir(Path::new("/tmp")).is_ok());
    }
}
