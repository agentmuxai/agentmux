// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Unified data-path resolution for AgentMux.
//!
//! Single source of truth for where state lives on disk. Replaces the
//! launcher / host / sidecar trio of independent path computations
//! (see docs/specs/SPEC_DATA_DIR_UNIFICATION_2026-05-05.md §3) and the
//! per-version isolation pattern it set up (data was keyed on the
//! build version so My Agents reset on every patch bump). The current
//! model keys data on a *channel* — a stable identifier that spans
//! versions within the same compat band, so agents survive rebuilds.
//! See docs/specs/SPEC_DATA_CHANNELS_2026_05_24.md and discussion
//! #1026 for the channel design and rationale.
//!
//! Layout:
//!
//! ```text
//! ~/.agentmux/
//! ├── shared/                       (cookies, credentials, account-wide)
//! ├── channels/<channel>/           (installed + portable + custom)
//! │   ├── data/, config/, logs/, cef-cache/, agents/
//! │   └── runtime/                  (lock + IPC, single instance per channel)
//! └── dev/<branch>/                 (per-branch dev isolation)
//!     └── (same children as channels/<channel>/)
//! ```
//!
//! Channel resolution (via [`DataPaths::resolve`]):
//! - `AGENTMUX_CHANNEL=<name>` env override wins for `Installed` /
//!   `Portable` modes — lets the operator point a released binary at
//!   any channel for parallel-channel testing.
//! - `RuntimeMode::Installed` / `Portable` w/o override → build-time
//!   default from `AGENTMUX_BUILD_CHANNEL_DEFAULT` (set by the
//!   packaging script; defaults to `"stable"` if unset, e.g. for
//!   `cargo run`).
//! - `RuntimeMode::Dev { branch }` → channel name is `dev-<branch>`
//!   for diagnostics; on-disk path stays at `~/.agentmux/dev/<branch>/`
//!   (NOT under `channels/`). Both the host (`agentmux-cef`) and
//!   launcher (`agentmux-launcher`) use [`DataPaths::resolve_path_only`]
//!   for dev builds to ignore `AGENTMUX_CHANNEL` — a dev session
//!   launched from inside a parent agentmux pane mustn't inherit
//!   the parent's channel and break per-branch isolation. Channel
//!   override is intentionally NOT supported in dev mode; if you
//!   want a different channel, use a portable build.

use crate::RuntimeMode;
use std::path::{Path, PathBuf};

/// Build-time default channel for `Installed` / `Portable` modes.
/// Set by the packaging script (`task package` exports
/// `AGENTMUX_BUILD_CHANNEL_DEFAULT=dev-portable`; release CI exports
/// `stable`). Falls back to `"stable"` when the binary is built
/// without the env (e.g. plain `cargo build` / `cargo run` for tests).
const BUILD_CHANNEL_DEFAULT: &str =
    match option_env!("AGENTMUX_BUILD_CHANNEL_DEFAULT") {
        Some(s) => s,
        None => "stable",
    };

/// Channel names that would collide with sibling dirs at
/// `~/.agentmux/` or with reserved subdir names inside a channel.
/// Rejected by [`sanitize_channel_name`].
const RESERVED_CHANNEL_NAMES: &[&str] = &[
    "shared",
    "snapshots",
    "dev",
    "versions",
    "channels",
    "runtime",
];

/// All paths a launcher / host / srv needs. Computed once by the
/// launcher; downstream binaries read paths from env vars set by the
/// launcher rather than recomputing (avoids the legacy desync risk
/// where each binary made its own portable / dev-mode determination).
#[derive(Debug, Clone)]
pub struct DataPaths {
    /// `~/.agentmux/` itself — the resolved root. Account-wide config
    /// that predates the unified layout (e.g. the launcher's
    /// `config.toml`) lives directly here. Honors
    /// `AGENTMUX_HOME_OVERRIDE` for tests.
    pub home_dir: PathBuf,

    /// Top-level dir for this channel+mode. All per-channel paths
    /// below are children. Either `~/.agentmux/channels/<channel>/`
    /// (installed / portable / AGENTMUX_CHANNEL override) or
    /// `~/.agentmux/dev/<branch>/` (dev mode without env override).
    ///
    /// Note: the field name is `instance_dir` for backward compat with
    /// downstream call sites; semantically it's now the *channel* root,
    /// not the *version* root.
    pub instance_dir: PathBuf,

    /// Channel identifier this resolution used (e.g. `"stable"`,
    /// `"dev-portable"`, `"dev-main"`, or a user-specified custom
    /// channel from `AGENTMUX_CHANNEL`). Surfaced for diagnostics,
    /// logging, and the launcher splash; downstream binaries usually
    /// don't need it (paths are passed via env vars).
    pub channel: String,

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
        Self::resolve_internal(version, mode, /* honor_env_channel = */ true)
    }

    /// Like [`Self::resolve`], but ignores the `AGENTMUX_CHANNEL` env
    /// override and uses only the mode-based default channel. Mirror
    /// of [`RuntimeMode::current_path_only`] for path resolution.
    ///
    /// Used by dev-build self-detection paths in `agentmux-cef`'s
    /// `main.rs` and `sidecar.rs`. Those paths run when a dev host
    /// has been launched from inside a parent AgentMux instance (e.g.
    /// `task dev` invoked from inside an agent pane in a portable
    /// build), where the child would otherwise inherit the parent's
    /// `AGENTMUX_*` env — including `AGENTMUX_CHANNEL` — and write
    /// into the parent's channel instead of `dev/<branch>/`. That
    /// cross-contamination would also trip the channel's single-
    /// instance lock and route every "open" back to the parent
    /// window. Path-based mode detection is authoritative for dev
    /// builds; channel resolution here mirrors that discipline.
    /// Codex P1 follow-up on PR #1027.
    pub fn resolve_path_only(version: &str, mode: &RuntimeMode) -> Result<Self, String> {
        Self::resolve_internal(version, mode, /* honor_env_channel = */ false)
    }

    fn resolve_internal(
        version: &str,
        mode: &RuntimeMode,
        honor_env_channel: bool,
    ) -> Result<Self, String> {
        let root = resolve_root()?;
        // `version` is still validated for path safety even though it
        // no longer appears in the on-disk path — it flows into
        // logging, the migration framework (Increment B), and
        // `meta.json` records, so a traversal-laced value mustn't
        // round-trip into a future path build by accident.
        sanitize_path_segment(version)
            .ok_or_else(|| format!("invalid version string for path: {:?}", version))?;

        // Channel resolution: env override > mode default. Dev mode's
        // *channel name* and *path* diverge intentionally — name is
        // `dev-<branch>` for diagnostics; path stays at
        // `~/.agentmux/dev/<branch>/` so per-branch isolation works
        // unchanged from Phase 1.
        let (channel, instance_dir) =
            resolve_channel_and_dir(mode, &root, honor_env_channel)?;

        let data_dir = instance_dir.join("data");
        let config_dir = instance_dir.join("config");
        let logs_dir = instance_dir.join("logs");
        let cef_cache_dir = instance_dir.join("cef-cache");
        let agents_dir = instance_dir.join("agents");
        let instance_runtime_dir = instance_dir.join("runtime");
        let shared_dir = root.join("shared");

        Ok(Self {
            home_dir: root,
            instance_dir,
            channel,
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
    ///
    /// Returns `OsString` (not `String`) so paths with non-UTF-8 bytes
    /// — possible on Linux/macOS for users with exotic home dirs —
    /// round-trip losslessly. `Command::env(k, v)` accepts any
    /// `AsRef<OsStr>`, so the OsString flows through to children
    /// unchanged. The mode value is the only `String`-typed entry
    /// (it's a fixed ASCII vocabulary).
    pub fn to_env_vars(&self) -> Vec<(&'static str, std::ffi::OsString)> {
        use std::ffi::OsString;
        vec![
            ("AGENTMUX_INSTANCE_DIR", self.instance_dir.clone().into_os_string()),
            ("AGENTMUX_DATA_DIR", self.data_dir.clone().into_os_string()),
            ("AGENTMUX_CONFIG_DIR", self.config_dir.clone().into_os_string()),
            ("AGENTMUX_LOG_DIR", self.logs_dir.clone().into_os_string()),
            ("AGENTMUX_CEF_CACHE_DIR", self.cef_cache_dir.clone().into_os_string()),
            ("AGENTMUX_AGENTS_DIR", self.agents_dir.clone().into_os_string()),
            (
                "AGENTMUX_INSTANCE_RUNTIME_DIR",
                self.instance_runtime_dir.clone().into_os_string(),
            ),
            ("AGENTMUX_SHARED_DIR", self.shared_dir.clone().into_os_string()),
            ("AGENTMUX_RUNTIME_MODE", OsString::from(self.mode.to_env_string())),
            // Channel propagated so downstream binaries can log it +
            // surface in diagnostics. NOT used to recompute paths
            // (paths flow through the dir vars above).
            ("AGENTMUX_CHANNEL", OsString::from(self.channel.clone())),
        ]
    }

    /// Reconstruct from env vars set by the launcher. Returns
    /// `None` if any required var is missing — fail-fast vs.
    /// silently falling back to legacy paths the way the old
    /// sidecar.rs did.
    ///
    /// Uses `var_os` (not `var`) so non-UTF-8 path bytes survive.
    pub fn from_env() -> Option<Self> {
        let instance_dir = std::env::var_os("AGENTMUX_INSTANCE_DIR")?;
        let data_dir = std::env::var_os("AGENTMUX_DATA_DIR")?;
        let config_dir = std::env::var_os("AGENTMUX_CONFIG_DIR")?;
        let logs_dir = std::env::var_os("AGENTMUX_LOG_DIR")?;
        let cef_cache_dir = std::env::var_os("AGENTMUX_CEF_CACHE_DIR")?;
        let agents_dir = std::env::var_os("AGENTMUX_AGENTS_DIR")?;
        let instance_runtime_dir = std::env::var_os("AGENTMUX_INSTANCE_RUNTIME_DIR")?;
        let shared_dir = std::env::var_os("AGENTMUX_SHARED_DIR")?;
        let mode = RuntimeMode::from_env()?;
        // Channel is required from the launcher (same fail-fast
        // discipline as every other dir var). Missing AGENTMUX_CHANNEL
        // means the launcher didn't export it — that's a launcher /
        // srv version skew, surface it loudly rather than silently
        // defaulting and risking a wrong-channel write.
        let channel = std::env::var("AGENTMUX_CHANNEL").ok()?;

        // Re-resolve home_dir (the agentmux root) on the consumer
        // side rather than transmitting it via env — it's a function
        // of the AGENTMUX_HOME_OVERRIDE env (test only) and the OS
        // home dir, which are stable across the launcher → host hop.
        let home_dir = resolve_root().ok()?;

        Some(Self {
            home_dir,
            instance_dir: PathBuf::from(instance_dir),
            channel,
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

    /// `~/.agentmux/shared/identities/` — root for per-bundle OAuth
    /// credential directories. Lives under `shared_dir` so it's
    /// account-wide and version-independent: upgrading agentmux does
    /// not move a user's bundle credentials. Per
    /// `SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22.md` §4.1.
    pub fn identities_dir(&self) -> PathBuf {
        self.shared_dir.join("identities")
    }

    /// `~/.agentmux/shared/identities/<bundle_id>/` — a specific
    /// bundle's credential root, when `bundle_id` is a safe path
    /// segment. Returns `None` for empty / `.` / `..` / any segment
    /// containing `/`, `\`, drive-letter colons, or Windows-reserved
    /// characters (same rules as the version/branch sanitizer in
    /// `resolve`).
    ///
    /// Defensive return type: `bundle_id` flows from `auth.start`
    /// request bodies (PR C) into `create_dir_all`, so an
    /// unvalidated `PathBuf::join` would let a crafted id escape the
    /// identities root and write outside the bundle area. codex P1
    /// follow-up on #981.
    ///
    /// Per-provider subdirectories (e.g. `claude/`, `codex/`) hang
    /// off this when the bundle gains an OAuth binding (PR C). The
    /// directory is created lazily by the bundle / OAuth flow that
    /// needs it — `ensure_dirs()` does not pre-create it.
    pub fn identity_dir(&self, bundle_id: &str) -> Option<PathBuf> {
        sanitize_path_segment(bundle_id).map(|safe| self.identities_dir().join(safe))
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

/// Sanitize a string for use as a single filesystem path segment.
/// Rejects empty, `.`, `..`, segments containing path separators, and
/// any character that has filesystem-special meaning on Windows (which
/// is the most restrictive of the platforms we target). Used as belt-
/// and-braces protection in `DataPaths::resolve` to prevent traversal
/// even when callers pass a directly-constructed `RuntimeMode::Dev` or
/// odd version string.
///
/// Why `:` is rejected: on Windows `C:temp` is a drive-relative path,
/// not a literal filename, so `PathBuf::join("versions").join("C:temp")`
/// would resolve OUTSIDE the intended `~/.agentmux/versions/` subtree.
fn sanitize_path_segment(s: &str) -> Option<String> {
    // Reject whitespace padding rather than silently normalizing it
    // away — otherwise distinct caller-supplied ids like "foo" and
    // " foo " would alias to the same directory. bundle_id is a real
    // caller-supplied identifier (passed through RPC payloads), so
    // this matters for credential isolation. codex P2 follow-up on
    // #981. Internally-generated version strings + branch names
    // shouldn't carry padding anyway, so this is no-op for them.
    if s != s.trim() {
        return None;
    }
    if s.is_empty() || s == "." || s == ".." {
        return None;
    }
    // Filesystem separators + Windows-reserved characters + NUL.
    if s
        .chars()
        .any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0'))
    {
        return None;
    }
    Some(s.to_string())
}

/// Sanitize a string for use as a channel name. Same path-segment
/// safety rules as [`sanitize_path_segment`] plus:
/// - Length capped at 64 chars (channel names show up in logs + the
///   launcher splash + may eventually be displayed in a picker, so
///   the cap is for UI sanity, not security).
/// - Rejects names in [`RESERVED_CHANNEL_NAMES`] that would collide
///   with sibling dirs at `~/.agentmux/` or reserved subdir names
///   inside a channel.
/// - The synonym `"default"` maps to `"stable"` (per
///   `SPEC_DATA_CHANNELS_2026_05_24.md` §7.5).
fn sanitize_channel_name(s: &str) -> Option<String> {
    let base = sanitize_path_segment(s)?;
    if base.len() > 64 {
        return None;
    }
    if RESERVED_CHANNEL_NAMES.contains(&base.as_str()) {
        return None;
    }
    if base == "default" {
        return Some("stable".to_string());
    }
    Some(base)
}

/// Resolve the channel name and on-disk channel dir for a given mode.
/// Pure function over (env, mode, root). When `honor_env_channel` is
/// `true`, `AGENTMUX_CHANNEL` overrides the mode default; when `false`,
/// the env is ignored and resolution depends only on `mode` +
/// build-time defaults. The `false` path is used by dev-build self-
/// detection (see [`DataPaths::resolve_path_only`]).
///
/// Resolution order (mirrors `SPEC_DATA_CHANNELS_2026_05_24.md` §2.2):
///   1. (only if `honor_env_channel`) `AGENTMUX_CHANNEL` env override —
///      any mode → path is `<root>/channels/<channel>/`. Lets the
///      operator point any binary at any channel for parallel-channel
///      testing.
///   2. No override (or env-channel disallowed), mode = Dev { branch }
///      → channel name is `dev-<branch>`, path stays at
///      `<root>/dev/<branch>/` (unchanged from Phase 1).
///   3. Same conditions, mode = Installed | Portable → channel name is
///      [`BUILD_CHANNEL_DEFAULT`] (set at build time by the packaging
///      script), path is `<root>/channels/<channel>/`.
fn resolve_channel_and_dir(
    mode: &RuntimeMode,
    root: &Path,
    honor_env_channel: bool,
) -> Result<(String, PathBuf), String> {
    // (1) Explicit env override — only when caller opted in.
    if honor_env_channel {
        if let Ok(raw) = std::env::var("AGENTMUX_CHANNEL") {
            if !raw.is_empty() {
                let channel = sanitize_channel_name(&raw).ok_or_else(|| {
                    format!("invalid AGENTMUX_CHANNEL value: {:?}", raw)
                })?;
                let dir = root.join("channels").join(&channel);
                return Ok((channel, dir));
            }
        }
    }

    // (2) Dev mode default: dev-<branch>, path under dev/<branch>/.
    if let RuntimeMode::Dev { branch } = mode {
        let safe_branch = sanitize_path_segment(branch).ok_or_else(|| {
            format!("invalid dev branch for path: {:?}", branch)
        })?;
        let channel = format!("dev-{}", safe_branch);
        let dir = root.join("dev").join(safe_branch);
        return Ok((channel, dir));
    }

    // (3) Installed / Portable default: build-time channel.
    let channel = sanitize_channel_name(BUILD_CHANNEL_DEFAULT).ok_or_else(|| {
        format!(
            "compile-time AGENTMUX_BUILD_CHANNEL_DEFAULT is invalid: {:?}",
            BUILD_CHANNEL_DEFAULT
        )
    })?;
    let dir = root.join("channels").join(&channel);
    Ok((channel, dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TEST_ENV_LOCK;
    use tempfile::TempDir;

    /// RAII guard that restores process state on drop, even if the
    /// test panics. Without Drop-based cleanup, a panic inside `f`
    /// would leave AGENTMUX_HOME_OVERRIDE set with a stale tempdir
    /// path AND poison the mutex; subsequent tests recover from poison
    /// but inherit the wrong env value.
    struct HomeOverrideGuard {
        _tmp: TempDir,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for HomeOverrideGuard {
        fn drop(&mut self) {
            std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        }
    }

    fn with_home_override<F: FnOnce(&Path)>(f: F) {
        let lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().to_path_buf();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", &path);
        let _guard = HomeOverrideGuard { _tmp: tmp, _lock: lock };
        f(&path);
        // _guard drops here, removing the env var even if f panicked
        // (the panic still propagates after Drop runs).
    }

    /// Helper: clear AGENTMUX_CHANNEL inside an existing
    /// with_home_override block to test pure mode-default resolution.
    /// Channel resolution reads the live env var, so individual tests
    /// must clear it to avoid leakage from sibling tests running
    /// concurrently inside the same process (TEST_ENV_LOCK serializes
    /// HOME_OVERRIDE but the channel var is a separate axis).
    fn clear_channel_env() {
        std::env::remove_var("AGENTMUX_CHANNEL");
    }

    #[test]
    fn installed_paths_under_default_channel() {
        with_home_override(|root| {
            clear_channel_env();
            let p = DataPaths::resolve("0.33.639", &RuntimeMode::Installed).unwrap();
            // Default channel for Installed without env override =
            // BUILD_CHANNEL_DEFAULT (which is "stable" in dev / test
            // builds — `option_env!` falls back when the build env
            // wasn't set by the packaging script).
            assert_eq!(p.channel, "stable");
            assert_eq!(p.instance_dir, root.join("channels").join("stable"));
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
    fn home_dir_resolves_to_root() {
        // The agentmux root (~/.agentmux/ or AGENTMUX_HOME_OVERRIDE)
        // is exposed via DataPaths.home_dir for legacy account-wide
        // state like the launcher's config.toml. Resolve in both
        // installed and dev modes; both should point at the same root.
        with_home_override(|root| {
            clear_channel_env();
            let inst = DataPaths::resolve("0.33.641", &RuntimeMode::Installed).unwrap();
            assert_eq!(inst.home_dir, root);
            let dev = DataPaths::resolve(
                "0.33.641",
                &RuntimeMode::Dev {
                    branch: "main".into(),
                },
            )
            .unwrap();
            assert_eq!(dev.home_dir, root);
        });
    }

    #[test]
    fn portable_paths_match_installed() {
        with_home_override(|root| {
            clear_channel_env();
            let inst = DataPaths::resolve("0.33.639", &RuntimeMode::Installed).unwrap();
            let port = DataPaths::resolve("0.33.639", &RuntimeMode::Portable).unwrap();
            // Portable + Installed share a default channel (no env
            // override → both fall through to BUILD_CHANNEL_DEFAULT),
            // so their data dirs are the same. Multi-instance
            // isolation is now channel-keyed: if you want two
            // independent portables, override AGENTMUX_CHANNEL on one.
            assert_eq!(inst.channel, port.channel);
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
            clear_channel_env();
            let mode = RuntimeMode::Dev {
                branch: "main".into(),
            };
            let p = DataPaths::resolve("0.33.639", &mode).unwrap();
            // Dev mode default: on-disk path stays under dev/<branch>/
            // (unchanged from Phase 1), channel name is "dev-<branch>"
            // for diagnostics. The two diverge intentionally — see
            // resolve_channel_and_dir doc.
            assert_eq!(p.channel, "dev-main");
            assert_eq!(p.instance_dir, root.join("dev").join("main"));
            assert_eq!(p.data_dir, root.join("dev").join("main").join("data"));
            assert_eq!(p.shared_dir, root.join("shared"));
        });
    }

    #[test]
    fn env_override_redirects_any_mode_under_channels() {
        // AGENTMUX_CHANNEL is absolute precedence. Even Dev mode,
        // which would otherwise land at dev/<branch>/, lands under
        // channels/<override>/ when the env is set. This is the
        // "test a hot-fix build against the live stable data" path
        // from SPEC_DATA_CHANNELS_2026_05_24.md §2.2.
        with_home_override(|root| {
            std::env::set_var("AGENTMUX_CHANNEL", "experiment");
            // Cleanup via Drop so a test panic doesn't leak it.
            struct ChannelGuard;
            impl Drop for ChannelGuard {
                fn drop(&mut self) {
                    std::env::remove_var("AGENTMUX_CHANNEL");
                }
            }
            let _g = ChannelGuard;

            let inst = DataPaths::resolve("0.33.639", &RuntimeMode::Installed).unwrap();
            assert_eq!(inst.channel, "experiment");
            assert_eq!(inst.instance_dir, root.join("channels").join("experiment"));

            let port = DataPaths::resolve("0.33.639", &RuntimeMode::Portable).unwrap();
            assert_eq!(port.channel, "experiment");
            assert_eq!(port.instance_dir, root.join("channels").join("experiment"));

            let dev = DataPaths::resolve(
                "0.33.639",
                &RuntimeMode::Dev { branch: "main".into() },
            )
            .unwrap();
            // Override beats the dev/<branch>/ default — channel name
            // matches the override, path lands under channels/, not dev/.
            assert_eq!(dev.channel, "experiment");
            assert_eq!(dev.instance_dir, root.join("channels").join("experiment"));
        });
    }

    #[test]
    fn env_override_rejects_unsafe_or_reserved_names() {
        with_home_override(|_root| {
            // Reserved (would collide with sibling dirs / inner dirs).
            for bad in ["shared", "snapshots", "dev", "versions", "channels", "runtime"] {
                std::env::set_var("AGENTMUX_CHANNEL", bad);
                let r = DataPaths::resolve("0.33.639", &RuntimeMode::Installed);
                std::env::remove_var("AGENTMUX_CHANNEL");
                assert!(
                    r.is_err(),
                    "AGENTMUX_CHANNEL={:?} should be rejected as reserved",
                    bad
                );
            }

            // Path-unsafe (traversal, separators, Windows-reserved).
            // NUL not tested here — Windows' WinAPI rejects NUL in
            // env-var values at the syscall level, so `set_var` would
            // panic before our sanitizer runs. NUL rejection is
            // covered by the direct-call sanitize_path_segment path
            // in identity_dir_rejects_unsafe_segments.
            for bad in ["..", ".", "a/b", "a\\b", "C:foo", "a*b"] {
                std::env::set_var("AGENTMUX_CHANNEL", bad);
                let r = DataPaths::resolve("0.33.639", &RuntimeMode::Installed);
                std::env::remove_var("AGENTMUX_CHANNEL");
                assert!(
                    r.is_err(),
                    "AGENTMUX_CHANNEL={:?} should be rejected as path-unsafe",
                    bad
                );
            }

            // Empty string treated as "not set" — falls through to
            // mode-based default. Documents the behavior so a
            // `.env`-set empty value doesn't surprise.
            std::env::set_var("AGENTMUX_CHANNEL", "");
            let r = DataPaths::resolve("0.33.639", &RuntimeMode::Installed);
            std::env::remove_var("AGENTMUX_CHANNEL");
            assert!(r.is_ok(), "empty AGENTMUX_CHANNEL should fall through to default");
            assert_eq!(r.unwrap().channel, "stable");
        });
    }

    #[test]
    fn env_override_default_is_synonym_for_stable() {
        with_home_override(|root| {
            std::env::set_var("AGENTMUX_CHANNEL", "default");
            let r = DataPaths::resolve("0.33.639", &RuntimeMode::Installed);
            std::env::remove_var("AGENTMUX_CHANNEL");
            let r = r.unwrap();
            // "default" maps to "stable" per spec §7.5; on-disk path
            // is channels/stable/, not channels/default/.
            assert_eq!(r.channel, "stable");
            assert_eq!(r.instance_dir, root.join("channels").join("stable"));
        });
    }

    #[test]
    fn channel_name_length_capped_at_64() {
        with_home_override(|_root| {
            // 64 chars OK, 65 rejected. The cap is for UI sanity, not
            // security — channel names show up in logs and the
            // launcher splash.
            let ok = "a".repeat(64);
            std::env::set_var("AGENTMUX_CHANNEL", &ok);
            let r = DataPaths::resolve("0.33.639", &RuntimeMode::Installed);
            std::env::remove_var("AGENTMUX_CHANNEL");
            assert!(r.is_ok(), "64-char channel should be accepted");

            let too_long = "a".repeat(65);
            std::env::set_var("AGENTMUX_CHANNEL", &too_long);
            let r = DataPaths::resolve("0.33.639", &RuntimeMode::Installed);
            std::env::remove_var("AGENTMUX_CHANNEL");
            assert!(r.is_err(), "65-char channel should be rejected");
        });
    }

    #[test]
    fn dev_branch_traversal_via_runtime_mode_still_rejected() {
        // Dev mode resolution sanitizes the branch via the same
        // sanitize_path_segment as before — channel rename doesn't
        // weaken the traversal-safety guarantees. Reproduces the
        // pre-channel test for parity.
        with_home_override(|_root| {
            clear_channel_env();
            let r = DataPaths::resolve(
                "0.33.639",
                &RuntimeMode::Dev { branch: "..".into() },
            );
            assert!(r.is_err());
            let r = DataPaths::resolve(
                "0.33.639",
                &RuntimeMode::Dev { branch: "foo/bar".into() },
            );
            assert!(r.is_err());
        });
    }

    #[test]
    fn identity_dir_rejects_unsafe_segments() {
        // bundle_id flows from auth.start request bodies into
        // create_dir_all. Without sanitization a crafted id would
        // escape the identities root. The function must return None
        // for traversal attempts, separator-bearing segments, and
        // Windows-reserved characters. codex P1 follow-up on #981.
        with_home_override(|_root| {
            let p = DataPaths::resolve("0.33.639", &RuntimeMode::Installed).unwrap();

            // Happy path — a normal UUID-shaped id resolves.
            assert!(p.identity_dir("abc-123-uuid").is_some());

            // Path traversal.
            assert_eq!(p.identity_dir(".."), None);
            assert_eq!(p.identity_dir("."), None);
            assert_eq!(p.identity_dir("../../../etc"), None);
            assert_eq!(p.identity_dir("a/b"), None);
            assert_eq!(p.identity_dir("a\\b"), None);

            // Empty / whitespace-only.
            assert_eq!(p.identity_dir(""), None);
            assert_eq!(p.identity_dir("   "), None);

            // Windows-reserved characters.
            assert_eq!(p.identity_dir("C:foo"), None);
            assert_eq!(p.identity_dir("foo*bar"), None);
            assert_eq!(p.identity_dir("foo?bar"), None);
            assert_eq!(p.identity_dir("with\0nul"), None);
        });
    }

    #[test]
    fn ensure_dirs_creates_everything() {
        with_home_override(|_root| {
            clear_channel_env();
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
            clear_channel_env();
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
            assert_eq!(p1.channel, p2.channel);
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
            clear_channel_env();
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
            clear_channel_env();
            assert!(DataPaths::resolve("..", &RuntimeMode::Installed).is_err());
            assert!(DataPaths::resolve(
                "0.33.639/etc",
                &RuntimeMode::Installed
            )
            .is_err());
            // Drive-relative on Windows: `PathBuf::join("versions")
            // .join("C:temp")` would resolve outside the intended
            // ~/.agentmux/versions/ subtree because `C:temp` is a
            // drive-relative path, not a literal filename.
            assert!(DataPaths::resolve("C:temp", &RuntimeMode::Installed).is_err());
            // Other Windows-reserved chars also rejected.
            for v in ["a*b", "a?b", "a|b", "a<b", "a>b", "a\"b"] {
                assert!(
                    DataPaths::resolve(v, &RuntimeMode::Installed).is_err(),
                    "should reject version with reserved char: {:?}",
                    v
                );
            }
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
            "AGENTMUX_CHANNEL",
        ] {
            std::env::remove_var(k);
        }
        assert!(DataPaths::from_env().is_none());
    }

    #[test]
    fn from_env_fails_fast_when_channel_missing() {
        // Symmetric to from_env_fails_fast_on_missing_vars but
        // isolates the channel-specific case: a launcher built with
        // the new code will always export AGENTMUX_CHANNEL; a missing
        // value indicates a launcher/srv version skew that must fail
        // loudly rather than silently fall back to a wrong-channel
        // write. Pre-set all other vars to confirm channel is what's
        // gating.
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for (k, v) in [
            ("AGENTMUX_INSTANCE_DIR", "/tmp/x"),
            ("AGENTMUX_DATA_DIR", "/tmp/x/data"),
            ("AGENTMUX_CONFIG_DIR", "/tmp/x/config"),
            ("AGENTMUX_LOG_DIR", "/tmp/x/logs"),
            ("AGENTMUX_CEF_CACHE_DIR", "/tmp/x/cef"),
            ("AGENTMUX_AGENTS_DIR", "/tmp/x/agents"),
            ("AGENTMUX_INSTANCE_RUNTIME_DIR", "/tmp/x/runtime"),
            ("AGENTMUX_SHARED_DIR", "/tmp/x/shared"),
            ("AGENTMUX_RUNTIME_MODE", "installed"),
        ] {
            std::env::set_var(k, v);
        }
        std::env::remove_var("AGENTMUX_CHANNEL");
        assert!(
            DataPaths::from_env().is_none(),
            "from_env() must refuse when AGENTMUX_CHANNEL is missing"
        );
        // Cleanup
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
    }

    #[test]
    fn resolve_path_only_ignores_env_channel() {
        // resolve_path_only is the dev-build self-detection variant —
        // it deliberately ignores AGENTMUX_CHANNEL because a dev host
        // launched from inside a parent agentmux instance would inherit
        // the parent's channel env and cross-contaminate.
        //
        // This is the codex P1 regression test on PR #1027 — without
        // the gate, dev hosts launched via `task dev` from inside an
        // agent pane would redirect to the parent's channel/<channel>/
        // dir instead of dev/<branch>/, tripping the channel's
        // single-instance lock.
        with_home_override(|root| {
            std::env::set_var("AGENTMUX_CHANNEL", "stable");
            struct ChannelGuard;
            impl Drop for ChannelGuard {
                fn drop(&mut self) {
                    std::env::remove_var("AGENTMUX_CHANNEL");
                }
            }
            let _g = ChannelGuard;

            let dev = DataPaths::resolve_path_only(
                "0.33.639",
                &RuntimeMode::Dev { branch: "main".into() },
            )
            .unwrap();
            // Dev mode default holds — channel name is dev-main, path
            // stays under dev/main/. AGENTMUX_CHANNEL=stable does NOT
            // redirect to channels/stable/.
            assert_eq!(dev.channel, "dev-main");
            assert_eq!(dev.instance_dir, root.join("dev").join("main"));

            let inst = DataPaths::resolve_path_only(
                "0.33.639",
                &RuntimeMode::Installed,
            )
            .unwrap();
            // Installed default holds — build-time channel
            // (BUILD_CHANNEL_DEFAULT = "stable" in tests). Path lands
            // at channels/stable/ regardless of the env override.
            assert_eq!(inst.channel, "stable");
            assert_eq!(inst.instance_dir, root.join("channels").join("stable"));

            // Sanity: regular `resolve` DOES honor the env override in
            // both modes — confirms the divergence is solely on the
            // path_only variant.
            let dev_env = DataPaths::resolve(
                "0.33.639",
                &RuntimeMode::Dev { branch: "main".into() },
            )
            .unwrap();
            assert_eq!(dev_env.channel, "stable");
            assert_eq!(dev_env.instance_dir, root.join("channels").join("stable"));
        });
    }

    #[test]
    fn sanitize_channel_name_accepts_normal_names() {
        // The happy path — make sure stable / beta / dev-portable
        // and friends all sanitize cleanly. Catches regressions in
        // case the reserved list grows by mistake.
        assert_eq!(sanitize_channel_name("stable"), Some("stable".into()));
        assert_eq!(sanitize_channel_name("beta"), Some("beta".into()));
        assert_eq!(
            sanitize_channel_name("dev-portable"),
            Some("dev-portable".into())
        );
        assert_eq!(
            sanitize_channel_name("dev-main"),
            Some("dev-main".into())
        );
        assert_eq!(
            sanitize_channel_name("experiment_42"),
            Some("experiment_42".into())
        );
    }
}
