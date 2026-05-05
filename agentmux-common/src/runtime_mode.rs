//! Runtime mode detection — single source of truth.
//!
//! The launcher computes [`RuntimeMode::current`] once at startup and
//! propagates the result to host + srv via the `AGENTMUX_RUNTIME_MODE`
//! environment variable. No binary should call [`current`] more than
//! once per process; downstream binaries read the env var via
//! [`from_env`] instead.
//!
//! Replaces the legacy mix of `cfg!(debug_assertions)`, `env::var
//! ("AGENTMUX_DEV").is_ok()`, and `== Ok("1")` checks across launcher,
//! host, and sidecar — which were each correct in isolation but
//! desynchronized in combination (see docs/specs/
//! SPEC_DATA_DIR_UNIFICATION_2026-05-05.md §2.1).
//!
//! # Detection priority
//!
//! 1. `AGENTMUX_RUNTIME_MODE` env override (testing, CI).
//! 2. Path-based portable detection: `<exe-dir>/runtime/` exists.
//! 3. Path-based dev detection: exe is under a known dev-build dir
//!    (`dist/cef-dev/`, `target/debug/`, `target/release/`).
//! 4. `AGENTMUX_DEV_BRANCH` env override (CI override for dev mode).
//! 5. Default: `Installed`.

use std::path::Path;
use std::process::Command;

/// Where this AgentMux binary is running from. Determines data path
/// layout (see [`crate::DataPaths`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeMode {
    /// Installed via the platform installer (msi/dmg/deb). State lives
    /// in `~/.agentmux/versions/<v>/`.
    Installed,
    /// Running from an extracted portable ZIP. State STILL lives in
    /// `~/.agentmux/versions/<v>/` (not under the portable folder) —
    /// portable binaries are stateless on disk.
    Portable,
    /// Running from a source-tree build. State lives in
    /// `~/.agentmux/dev/<branch>/` so different branches don't share
    /// state.
    Dev { branch: String },
}

impl RuntimeMode {
    /// Detect runtime mode from the launcher's vantage point. Call
    /// ONCE at process startup. Subsequent processes read the
    /// `AGENTMUX_RUNTIME_MODE` env var via [`Self::from_env`].
    ///
    /// `exe_dir` should be `current_exe().parent()` of the binary
    /// doing the detection (typically the launcher).
    pub fn current(exe_dir: &Path) -> Self {
        // 1. Explicit AGENTMUX_RUNTIME_MODE override (tests, CI, ops).
        if let Ok(s) = std::env::var("AGENTMUX_RUNTIME_MODE") {
            if let Some(mode) = parse_mode_string(&s) {
                return mode;
            }
        }

        // 2. Portable: a `runtime/` subdir is bundled next to the
        //    launcher binary in our portable ZIPs.
        if exe_dir.join("runtime").is_dir() {
            return Self::Portable;
        }

        // 3. Path-based dev detection: exe lives under a known
        //    build-output dir. We check by walking parents and
        //    matching path components.
        if exe_dir_is_dev_build(exe_dir) {
            let branch = detect_branch(exe_dir);
            return Self::Dev { branch };
        }

        // 4. AGENTMUX_DEV_BRANCH env override (CI override for dev mode
        //    when the binary isn't in a recognised build dir).
        if std::env::var("AGENTMUX_DEV_BRANCH").is_ok() {
            let branch = detect_branch(exe_dir);
            return Self::Dev { branch };
        }

        // 5. Default.
        Self::Installed
    }

    /// Read mode from the `AGENTMUX_RUNTIME_MODE` env var the launcher
    /// set. Used by host + srv to consume the launcher's decision
    /// without re-detecting (which would re-introduce the desync risk
    /// the legacy code had).
    pub fn from_env() -> Option<Self> {
        std::env::var("AGENTMUX_RUNTIME_MODE")
            .ok()
            .and_then(|s| parse_mode_string(&s))
    }

    /// Encode for the `AGENTMUX_RUNTIME_MODE` env var. Round-trips
    /// with [`parse_mode_string`].
    pub fn to_env_string(&self) -> String {
        match self {
            Self::Installed => "installed".to_string(),
            Self::Portable => "portable".to_string(),
            Self::Dev { branch } => format!("dev:{}", branch),
        }
    }

    /// Slug used inside `~/.agentmux/` to separate state by mode.
    /// Stable across releases of the same major mode + branch.
    pub fn dir_slug(&self) -> String {
        match self {
            // Versioned modes don't include the version here — that's
            // appended separately in DataPaths so callers can re-use
            // RuntimeMode across version queries.
            Self::Installed | Self::Portable => "versions".to_string(),
            // Defense in depth: branch is sanitized at parse time, but
            // a Dev variant constructed directly (e.g. via tests) might
            // still hold an unsafe value. Slug-on-format ensures the
            // returned string is always exactly two segments — `dev/`
            // followed by a single-segment branch slug — so callers
            // splitting on `/` see the expected shape and the resulting
            // filesystem path is always a child of `dev/`.
            Self::Dev { branch } => format!("dev/{}", sanitize_branch_slug(branch)),
        }
    }
}

fn parse_mode_string(s: &str) -> Option<RuntimeMode> {
    let trimmed = s.trim();
    if trimmed.eq_ignore_ascii_case("installed") {
        return Some(RuntimeMode::Installed);
    }
    if trimmed.eq_ignore_ascii_case("portable") {
        return Some(RuntimeMode::Portable);
    }
    if let Some(raw_branch) = trimmed.strip_prefix("dev:") {
        // Slugify at parse time — the branch then flows through
        // DataPaths::resolve into a `~/.agentmux/dev/<branch>/` path,
        // and we must not let `/`, `..`, or shell-meta chars in the
        // env override (`AGENTMUX_RUNTIME_MODE=dev:../versions/x`)
        // escape the dev/ subtree. Codex P1 + Claude P1 on PR #695.
        let slug = sanitize_branch_slug(raw_branch);
        if slug.is_empty() {
            return None;
        }
        return Some(RuntimeMode::Dev { branch: slug });
    }
    if trimmed.eq_ignore_ascii_case("dev") {
        return Some(RuntimeMode::Dev {
            branch: "default".to_string(),
        });
    }
    None
}

/// True when `exe_dir` is one of our known dev-build output dirs.
/// Walks parents to handle nested cases (CEF subprocesses run from
/// `runtime/` even in dev).
fn exe_dir_is_dev_build(exe_dir: &Path) -> bool {
    // Match any ancestor of exe_dir that ends in a known build dir name.
    // We accept: dist/cef-dev/<...>, target/debug/<...>, target/release/<...>
    let mut cur = Some(exe_dir);
    while let Some(p) = cur {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let parent_name = p
            .parent()
            .and_then(|pp| pp.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("");

        if (parent_name == "dist" && name == "cef-dev")
            || (parent_name == "target" && (name == "debug" || name == "release"))
        {
            return true;
        }
        cur = p.parent();
    }
    false
}

/// Detect the current git branch slug for dev mode. Walks up from
/// `exe_dir` to find a git repo, runs `git rev-parse --abbrev-ref
/// HEAD`, and slugifies. Falls back to `"default"` if anything fails.
/// `AGENTMUX_DEV_BRANCH` env override always wins.
fn detect_branch(exe_dir: &Path) -> String {
    if let Ok(b) = std::env::var("AGENTMUX_DEV_BRANCH") {
        let slug = sanitize_branch_slug(&b);
        if !slug.is_empty() {
            return slug;
        }
    }
    // Find a git repo by walking up from exe_dir.
    let mut cur = Some(exe_dir);
    while let Some(p) = cur {
        if p.join(".git").exists() {
            return run_git_branch(p).unwrap_or_else(|| "default".to_string());
        }
        cur = p.parent();
    }
    "default".to_string()
}

fn run_git_branch(repo_dir: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(repo_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8(output.stdout).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() || trimmed == "HEAD" {
        // Detached state; not useful for branch keying.
        return None;
    }
    Some(slugify_branch(trimmed))
}

/// Convert a git branch into a filesystem-safe slug.
/// `agenta/feature-x` → `agenta-feature-x`.
///
/// Use [`sanitize_branch_slug`] for any value that originates from an
/// env var or other untrusted source — it additionally strips `..` and
/// leading dots that could escape the dev/ subtree.
fn slugify_branch(b: &str) -> String {
    b.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "-")
}

/// Stricter sanitization for branch values that came from outside the
/// trusted slugify path (env overrides, CI inputs). Strips parent-dir
/// segments, leading dots, and any whitespace that survived earlier
/// trimming. Returns an empty string if nothing usable remains, which
/// callers should treat as "reject this input."
fn sanitize_branch_slug(b: &str) -> String {
    // Step 1: replace shell + filesystem-meta characters (same as
    // slugify_branch — git-valid chars get a "-").
    let replaced = slugify_branch(b);
    // Step 2: drop `..` segments (now dash-separated, so "-..-"-style
    // sequences too) and any leading/trailing dots/dashes/whitespace
    // that would resolve up out of the dev/ subdir.
    let cleaned: String = replaced
        .split('-')
        .filter(|seg| !seg.is_empty() && *seg != "." && *seg != "..")
        .collect::<Vec<_>>()
        .join("-");
    cleaned
        .trim_matches(|c: char| c == '.' || c == '-' || c.is_whitespace())
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TEST_ENV_LOCK;
    use std::path::PathBuf;

    /// `with_env` does NOT take `TEST_ENV_LOCK` — callers must hold
    /// it. This avoids the re-entrant-locking problem when a test
    /// nests `with_env` calls. The lock is shared across the whole
    /// crate (defined in lib.rs) so tests in this module also
    /// serialize against `data_paths::tests` which touches the same
    /// process-global env vars.
    fn with_env<F: FnOnce()>(key: &str, val: Option<&str>, f: F) {
        let prev = std::env::var(key).ok();
        match val {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
        f();
        match prev {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn parses_env_strings_round_trip() {
        for mode in [
            RuntimeMode::Installed,
            RuntimeMode::Portable,
            RuntimeMode::Dev {
                branch: "main".into(),
            },
            RuntimeMode::Dev {
                branch: "agenta-feature-x".into(),
            },
        ] {
            let s = mode.to_env_string();
            let back = parse_mode_string(&s).expect("round-trip");
            assert_eq!(back, mode);
        }
    }

    #[test]
    fn parse_accepts_bare_dev() {
        assert_eq!(
            parse_mode_string("dev"),
            Some(RuntimeMode::Dev {
                branch: "default".into()
            })
        );
        assert_eq!(
            parse_mode_string("DEV"),
            Some(RuntimeMode::Dev {
                branch: "default".into()
            })
        );
    }

    #[test]
    fn env_override_wins_over_path_detection() {
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Even if exe_dir looks portable (has `runtime/`), env override wins.
        // We simulate by passing a path that doesn't exist (so .is_dir()
        // returns false), but we also force the env override.
        with_env("AGENTMUX_RUNTIME_MODE", Some("dev:main"), || {
            with_env("AGENTMUX_DEV_BRANCH", None, || {
                let mode = RuntimeMode::current(&PathBuf::from("/nonexistent"));
                assert_eq!(
                    mode,
                    RuntimeMode::Dev {
                        branch: "main".into()
                    }
                );
            });
        });
    }

    #[test]
    fn dev_build_path_pattern() {
        // Match dist/cef-dev/...
        assert!(exe_dir_is_dev_build(&PathBuf::from(
            "/c/Systems/agentmux/dist/cef-dev"
        )));
        // Match target/debug/...
        assert!(exe_dir_is_dev_build(&PathBuf::from(
            "/c/Systems/agentmux/target/debug"
        )));
        // Match target/release/...
        assert!(exe_dir_is_dev_build(&PathBuf::from(
            "/c/Systems/agentmux/target/release"
        )));
        // Don't match an installed path.
        assert!(!exe_dir_is_dev_build(&PathBuf::from(
            "/c/Program Files/AgentMux"
        )));
        // Don't match a portable path.
        assert!(!exe_dir_is_dev_build(&PathBuf::from(
            "/Users/me/Desktop/agentmux-portable"
        )));
    }

    #[test]
    fn slugify_branch_replaces_unsafe_chars() {
        assert_eq!(slugify_branch("agenta/feature-x"), "agenta-feature-x");
        assert_eq!(slugify_branch("feat:foo*bar"), "feat-foo-bar");
        assert_eq!(slugify_branch("plain"), "plain");
    }

    #[test]
    fn dir_slug_per_mode() {
        assert_eq!(RuntimeMode::Installed.dir_slug(), "versions");
        assert_eq!(RuntimeMode::Portable.dir_slug(), "versions");
        assert_eq!(
            RuntimeMode::Dev {
                branch: "main".into()
            }
            .dir_slug(),
            "dev/main"
        );
        assert_eq!(
            RuntimeMode::Dev {
                branch: "agenta/x".into()
            }
            .dir_slug(),
            "dev/agenta-x"
        );
    }

    #[test]
    fn invalid_env_string_falls_through() {
        assert!(parse_mode_string("garbage").is_none());
        assert!(parse_mode_string("").is_none());
    }

    #[test]
    fn parse_dev_branch_rejects_traversal_attempts() {
        // `..` resolves out of the dev/ subdir on disk — the slug must
        // either reject it or strip it. We choose strip; if nothing
        // usable remains, parse fails (returns None).
        assert_eq!(parse_mode_string("dev:.."), None);
        assert_eq!(parse_mode_string("dev:."), None);
        assert_eq!(parse_mode_string("dev:"), None);
        // `dev:../versions/x` slugifies to `versions-x` (the slashes
        // are replaced and `..` segment is dropped).
        let m = parse_mode_string("dev:../versions/x").expect("parses");
        match m {
            RuntimeMode::Dev { branch } => {
                assert!(!branch.contains(".."));
                assert!(!branch.contains('/'));
                assert!(!branch.contains('\\'));
            }
            _ => panic!("expected Dev variant"),
        }
    }

    #[test]
    fn sanitize_branch_slug_strips_traversal() {
        assert_eq!(sanitize_branch_slug(".."), "");
        assert_eq!(sanitize_branch_slug("../foo"), "foo");
        assert_eq!(sanitize_branch_slug("foo/../bar"), "foo-bar");
        assert_eq!(sanitize_branch_slug(".hidden"), "hidden");
        assert_eq!(sanitize_branch_slug("ok-branch"), "ok-branch");
    }
}
