//! Unified data-path resolution for AgentMux.
//!
//! Single source of truth for where state lives on disk. Replaces the
//! launcher / host / sidecar trio of independent path computations
//! (see docs/specs/SPEC_DATA_DIR_UNIFICATION_2026-05-05.md §3).
//!
//! Layout:
//!
//! ```text
//! ~/.agentmux/
//! ├── shared/                       (cookies, credentials, account-wide)
//! ├── versions/<v>/                 (installed + portable; one running instance)
//! │   ├── data/, config/, logs/, cef-cache/, agents/
//! │   └── runtime/                  (lock + IPC, single set per version)
//! └── dev/<branch>/                 (per-branch dev isolation)
//!     └── (same children as versions/<v>/)
//! ```

use crate::RuntimeMode;
use std::path::{Path, PathBuf};

/// All paths a launcher / host / srv needs. Computed once by the
/// launcher; downstream binaries read paths from env vars set by the
/// launcher rather than recomputing (avoids the legacy desync risk
/// where each binary made its own portable / dev-mode determination).
#[derive(Debug, Clone)]
pub struct DataPaths {
    /// Top-level dir for this version+mode. All version-keyed paths
    /// below are children. Either `~/.agentmux/versions/<v>/` (installed/
    /// portable) or `~/.agentmux/dev/<branch>/` (dev).
    pub instance_dir: PathBuf,

    /// `instance_dir/data/` — srv DB (objects.db, sagas.db, …).
    pub data_dir: PathBuf,

    /// `instance_dir/config/` — settings.json, repos.json, etc.
    pub config_dir: PathBuf,

    /// `instance_dir/logs/` — host + srv + launcher logs (rotated).
    pub logs_dir: PathBuf,

    /// `instance_dir/cef-cache/` — Chromium runtime cache (regenerable).
    pub cef_cache_dir: PathBuf,

    /// `instance_dir/agents/` — agent workspace state.
    pub agents_dir: PathBuf,

    /// `instance_dir/runtime/` — single-instance lock + IPC (pid,
    /// lockfile, ipc-port, named-pipe). One set per version+mode.
    pub instance_runtime_dir: PathBuf,

    /// `~/.agentmux/shared/` — version-independent, account-wide
    /// state (cookies, OAuth tokens, API keys, dictionary downloads).
    pub shared_dir: PathBuf,

    /// Snapshot of the [`RuntimeMode`] this resolution used. Helpful
    /// for logging and feature gates.
    pub mode: RuntimeMode,
}

impl DataPaths {
    /// Resolve all paths for the given version + mode. Honors
    /// `AGENTMUX_HOME_OVERRIDE` for tests (replaces `~/.agentmux` root).
    ///
    /// Returns `Err` if the input contains values that cannot be
    /// represented as a safe single-segment subpath — e.g. `..` in the
    /// version string, or a Dev branch that sanitizes to empty. This
    /// is belt-and-braces safety: parse-time sanitization in
    /// [`crate::RuntimeMode`] should already have caught these, but a
    /// `RuntimeMode::Dev { branch }` constructed directly (e.g. by a
    /// test or future caller) is also rejected here.
    pub fn resolve(version: &str, mode: &RuntimeMode) -> Result<Self, String> {
        let root = resolve_root()?;
        let safe_version = sanitize_path_segment(version)
            .ok_or_else(|| format!("invalid version string for path: {:?}", version))?;
        let instance_dir = match mode {
            RuntimeMode::Installed | RuntimeMode::Portable => {
                root.join("versions").join(&safe_version)
            }
            RuntimeMode::Dev { branch } => {
                let safe_branch = sanitize_path_segment(branch).ok_or_else(|| {
                    format!("invalid dev branch for path: {:?}", branch)
                })?;
                root.join("dev").join(safe_branch)
            }
        };

        let data_dir = instance_dir.join("data");
        let config_dir = instance_dir.join("config");
        let logs_dir = instance_dir.join("logs");
        let cef_cache_dir = instance_dir.join("cef-cache");
        let agents_dir = instance_dir.join("agents");
        let instance_runtime_dir = instance_dir.join("runtime");
        let shared_dir = root.join("shared");

        Ok(Self {
            instance_dir,
            data_dir,
            config_dir,
            logs_dir,
            cef_cache_dir,
            agents_dir,
            instance_runtime_dir,
            shared_dir,
            mode: mode.clone(),
        })
    }

    /// Create every directory that may be written to. Idempotent.
    /// Safe to call on every launch.
    pub fn ensure_dirs(&self) -> Result<(), String> {
        for d in [
            &self.instance_dir,
            &self.data_dir,
            &self.config_dir,
            &self.logs_dir,
            &self.cef_cache_dir,
            &self.agents_dir,
            &self.instance_runtime_dir,
            &self.shared_dir,
        ] {
            std::fs::create_dir_all(d)
                .map_err(|e| format!("failed to create {}: {}", d.display(), e))?;
        }
        // The data dir's `db/` subdir is the canonical srv DB home;
        // mirrors legacy ensure_dirs() and lets srv unconditionally
        // open `data_dir/db/objects.db`.
        std::fs::create_dir_all(self.data_dir.join("db"))
            .map_err(|e| format!("failed to create db dir: {}", e))?;
        Ok(())
    }

    /// Env vars to pass to host + srv subprocesses. The launcher
    /// computes `DataPaths` once and exports these; downstream
    /// binaries read them via [`Self::from_env`] instead of
    /// recomputing.
    pub fn to_env_vars(&self) -> Vec<(&'static str, String)> {
        vec![
            (
                "AGENTMUX_INSTANCE_DIR",
                self.instance_dir.display().to_string(),
            ),
            ("AGENTMUX_DATA_DIR", self.data_dir.display().to_string()),
            ("AGENTMUX_CONFIG_DIR", self.config_dir.display().to_string()),
            ("AGENTMUX_LOG_DIR", self.logs_dir.display().to_string()),
            (
                "AGENTMUX_CEF_CACHE_DIR",
                self.cef_cache_dir.display().to_string(),
            ),
            ("AGENTMUX_AGENTS_DIR", self.agents_dir.display().to_string()),
            (
                "AGENTMUX_INSTANCE_RUNTIME_DIR",
                self.instance_runtime_dir.display().to_string(),
            ),
            ("AGENTMUX_SHARED_DIR", self.shared_dir.display().to_string()),
            ("AGENTMUX_RUNTIME_MODE", self.mode.to_env_string()),
        ]
    }

    /// Reconstruct from env vars set by the launcher. Returns
    /// `None` if any required var is missing — fail-fast vs.
    /// silently falling back to legacy paths the way the old
    /// sidecar.rs did.
    pub fn from_env() -> Option<Self> {
        let instance_dir = std::env::var("AGENTMUX_INSTANCE_DIR").ok()?;
        let data_dir = std::env::var("AGENTMUX_DATA_DIR").ok()?;
        let config_dir = std::env::var("AGENTMUX_CONFIG_DIR").ok()?;
        let logs_dir = std::env::var("AGENTMUX_LOG_DIR").ok()?;
        let cef_cache_dir = std::env::var("AGENTMUX_CEF_CACHE_DIR").ok()?;
        let agents_dir = std::env::var("AGENTMUX_AGENTS_DIR").ok()?;
        let instance_runtime_dir = std::env::var("AGENTMUX_INSTANCE_RUNTIME_DIR").ok()?;
        let shared_dir = std::env::var("AGENTMUX_SHARED_DIR").ok()?;
        let mode = RuntimeMode::from_env()?;

        Some(Self {
            instance_dir: PathBuf::from(instance_dir),
            data_dir: PathBuf::from(data_dir),
            config_dir: PathBuf::from(config_dir),
            logs_dir: PathBuf::from(logs_dir),
            cef_cache_dir: PathBuf::from(cef_cache_dir),
            agents_dir: PathBuf::from(agents_dir),
            instance_runtime_dir: PathBuf::from(instance_runtime_dir),
            shared_dir: PathBuf::from(shared_dir),
            mode,
        })
    }
}

/// `~/.agentmux/` root, or the test override via
/// `AGENTMUX_HOME_OVERRIDE`. Falls back to error if no home dir
/// can be resolved (rare — should only happen in stripped CI envs).
fn resolve_root() -> Result<PathBuf, String> {
    if let Ok(s) = std::env::var("AGENTMUX_HOME_OVERRIDE") {
        if !s.is_empty() {
            return Ok(PathBuf::from(s));
        }
    }
    let home = dirs::home_dir().ok_or_else(|| "dirs::home_dir() returned None".to_string())?;
    Ok(home.join(".agentmux"))
}

/// Helper: is `target` inside or equal to `parent`? Returns `Err` if
/// either path cannot be canonicalized — callers that ask "is target
/// inside parent" before the target exists need the precondition
/// (target on disk) before invoking.
///
/// (We canonicalize both sides so that symlinks, `..`, and case
/// differences resolve before the prefix check; mixing canonicalized
/// and raw values silently produced incorrect results in earlier
/// drafts. Claude P2 on PR #695.)
pub fn path_contains(parent: &Path, target: &Path) -> std::io::Result<bool> {
    let parent = parent.canonicalize()?;
    let target = target.canonicalize()?;
    Ok(target.starts_with(&parent))
}

/// Sanitize a string for use as a single filesystem path segment.
/// Rejects empty, `.`, `..`, and segments that contain path separators
/// or other unsafe chars. Used as belt-and-braces protection in
/// `DataPaths::resolve` to prevent traversal even when callers pass a
/// directly-constructed `RuntimeMode::Dev` or odd version string.
fn sanitize_path_segment(s: &str) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains('\0')
    {
        return None;
    }
    Some(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TEST_ENV_LOCK;
    use tempfile::TempDir;

    fn with_home_override<F: FnOnce(&Path)>(f: F) {
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = TempDir::new().expect("tempdir");
        let path_str = tmp.path().display().to_string();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", &path_str);
        f(tmp.path());
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
    }

    #[test]
    fn installed_paths_under_versions() {
        with_home_override(|root| {
            let p = DataPaths::resolve("0.33.639", &RuntimeMode::Installed).unwrap();
            assert_eq!(p.instance_dir, root.join("versions").join("0.33.639"));
            assert_eq!(p.data_dir, p.instance_dir.join("data"));
            assert_eq!(p.config_dir, p.instance_dir.join("config"));
            assert_eq!(p.logs_dir, p.instance_dir.join("logs"));
            assert_eq!(p.cef_cache_dir, p.instance_dir.join("cef-cache"));
            assert_eq!(p.agents_dir, p.instance_dir.join("agents"));
            assert_eq!(p.instance_runtime_dir, p.instance_dir.join("runtime"));
            assert_eq!(p.shared_dir, root.join("shared"));
        });
    }

    #[test]
    fn portable_paths_match_installed() {
        with_home_override(|root| {
            let inst = DataPaths::resolve("0.33.639", &RuntimeMode::Installed).unwrap();
            let port = DataPaths::resolve("0.33.639", &RuntimeMode::Portable).unwrap();
            // §3 design: portable + installed share the same data dir
            // (per the user's multi-instance decision in §5.5).
            assert_eq!(inst.instance_dir, port.instance_dir);
            assert_eq!(inst.data_dir, port.data_dir);
            // shared/ is mode-independent.
            assert_eq!(inst.shared_dir, root.join("shared"));
            assert_eq!(port.shared_dir, root.join("shared"));
        });
    }

    #[test]
    fn dev_paths_under_dev_branch() {
        with_home_override(|root| {
            let mode = RuntimeMode::Dev {
                branch: "main".into(),
            };
            let p = DataPaths::resolve("0.33.639", &mode).unwrap();
            // Dev mode does NOT use the version — branches isolate.
            assert_eq!(p.instance_dir, root.join("dev").join("main"));
            assert_eq!(p.data_dir, root.join("dev").join("main").join("data"));
            assert_eq!(p.shared_dir, root.join("shared"));
        });
    }

    #[test]
    fn ensure_dirs_creates_everything() {
        with_home_override(|_root| {
            let p = DataPaths::resolve("0.33.639", &RuntimeMode::Installed).unwrap();
            p.ensure_dirs().unwrap();
            assert!(p.instance_dir.is_dir());
            assert!(p.data_dir.is_dir());
            assert!(p.data_dir.join("db").is_dir());
            assert!(p.config_dir.is_dir());
            assert!(p.logs_dir.is_dir());
            assert!(p.cef_cache_dir.is_dir());
            assert!(p.agents_dir.is_dir());
            assert!(p.instance_runtime_dir.is_dir());
            assert!(p.shared_dir.is_dir());
        });
    }

    #[test]
    fn env_vars_round_trip() {
        with_home_override(|_root| {
            let p1 = DataPaths::resolve(
                "0.33.639",
                &RuntimeMode::Dev {
                    branch: "main".into(),
                },
            )
            .unwrap();
            // Apply each env var, then read back.
            for (k, v) in p1.to_env_vars() {
                std::env::set_var(k, v);
            }
            let p2 = DataPaths::from_env().expect("round-trip");
            assert_eq!(p1.instance_dir, p2.instance_dir);
            assert_eq!(p1.data_dir, p2.data_dir);
            assert_eq!(p1.shared_dir, p2.shared_dir);
            assert_eq!(p1.mode, p2.mode);
            // Cleanup
            for (k, _) in p1.to_env_vars() {
                std::env::remove_var(k);
            }
        });
    }

    #[test]
    fn resolve_rejects_dev_branch_traversal() {
        // Even if a caller manages to construct a Dev variant with an
        // unsafe branch (bypassing parse_mode_string sanitization),
        // resolve() must catch it.
        with_home_override(|_root| {
            let mode = RuntimeMode::Dev {
                branch: "..".into(),
            };
            assert!(DataPaths::resolve("0.33.639", &mode).is_err());
            let mode = RuntimeMode::Dev {
                branch: "foo/bar".into(),
            };
            assert!(DataPaths::resolve("0.33.639", &mode).is_err());
        });
    }

    #[test]
    fn resolve_rejects_traversal_version() {
        with_home_override(|_root| {
            assert!(DataPaths::resolve("..", &RuntimeMode::Installed).is_err());
            assert!(DataPaths::resolve(
                "0.33.639/etc",
                &RuntimeMode::Installed
            )
            .is_err());
        });
    }

    #[test]
    fn from_env_fails_fast_on_missing_vars() {
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Clear all expected vars.
        for k in [
            "AGENTMUX_INSTANCE_DIR",
            "AGENTMUX_DATA_DIR",
            "AGENTMUX_CONFIG_DIR",
            "AGENTMUX_LOG_DIR",
            "AGENTMUX_CEF_CACHE_DIR",
            "AGENTMUX_AGENTS_DIR",
            "AGENTMUX_INSTANCE_RUNTIME_DIR",
            "AGENTMUX_SHARED_DIR",
            "AGENTMUX_RUNTIME_MODE",
        ] {
            std::env::remove_var(k);
        }
        assert!(DataPaths::from_env().is_none());
    }
}
