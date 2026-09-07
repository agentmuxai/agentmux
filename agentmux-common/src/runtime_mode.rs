// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

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
//! 2. Marker-based portable detection: an `agentmux-portable.marker` file
//!    written by `scripts/package-portable.sh`. Looked for next to the
//!    detecting exe, one level down in `runtime/` (the packaging puts it
//!    there to keep the extract root clean; the launcher is at the root and
//!    the host/srv run from `runtime/`), and two levels up for macOS .app
//!    bundles.
//! 3. Path-based dev detection: exe is under a known dev-build dir
//!    (`dist/cef-dev*/`, `target/debug/`, `target/release/`).
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
    /// `~/.agentmux/dev/<branch>/<clone_id>/` so different branches
    /// AND different clones of the same branch don't share state.
    ///
    /// `clone_id` is a 16-char hex hash of the clone's workspace-root
    /// path, derived by [`derive_clone_id`] when the launcher detects
    /// Dev mode. `None` is permitted for backward compatibility with
    /// callers that construct `RuntimeMode::Dev` directly (tests, the
    /// `dev:branch` env-string parser that pre-dates this field): in
    /// that case path resolution falls back to the old two-level
    /// `dev/<branch>/` layout. See
    /// `docs/analysis/ANALYSIS_MULTI_CLONE_TASK_DEV_ISOLATION_2026-05-26.md`.
    Dev {
        branch: String,
        clone_id: Option<String>,
    },
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

        // 2. Portable: marker file written next to the launcher by
        //    `scripts/package-portable.sh`. The presence of a
        //    `runtime/` subdir is NOT a discriminator — installed
        //    builds ship it too — so we require the explicit marker.
        if is_portable_marker_present(exe_dir) {
            return Self::Portable;
        }

        // 3. Path-based dev detection: exe lives under a known
        //    build-output dir.
        if exe_dir_is_dev_build(exe_dir) {
            let branch = detect_branch(exe_dir);
            let clone_id = derive_clone_id(exe_dir);
            return Self::Dev { branch, clone_id };
        }

        // 4. AGENTMUX_DEV_BRANCH override. The env-var IS the branch
        //    source at this step; sanitize it directly, do NOT call
        //    detect_branch (which has its own git fallback that would
        //    take over when the env-var sanitizes to empty — and could
        //    then return a real branch name from an unrelated .git
        //    ancestor of `exe_dir`, silently routing installed runs
        //    into Dev when the user only had a typo'd env var).
        //
        //    If the env value is unusable (empty after trim, or
        //    sanitizes to empty via path-traversal stripping), fall
        //    through to Installed instead of inventing a branch.
        if let Ok(b) = std::env::var("AGENTMUX_DEV_BRANCH") {
            let slug = sanitize_branch_slug(&b);
            if !slug.is_empty() {
                let clone_id = derive_clone_id(exe_dir);
                return Self::Dev {
                    branch: slug,
                    clone_id,
                };
            }
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

    /// Path-only detection — skips the `AGENTMUX_RUNTIME_MODE` env step.
    /// Use when the env can't be trusted (e.g., the binary was launched
    /// as a child of a different AgentMux process that set its own
    /// `AGENTMUX_*` vars). Mirrors the rest of [`Self::current`]'s
    /// priority order (portable marker → dev exe path → installed).
    pub fn current_path_only(exe_dir: &Path) -> Self {
        if is_portable_marker_present(exe_dir) {
            return Self::Portable;
        }
        if exe_dir_is_dev_build(exe_dir) {
            let branch = detect_branch(exe_dir);
            let clone_id = derive_clone_id(exe_dir);
            return Self::Dev { branch, clone_id };
        }
        Self::Installed
    }

    /// Encode for the `AGENTMUX_RUNTIME_MODE` env var. Round-trips
    /// with [`parse_mode_string`].
    pub fn to_env_string(&self) -> String {
        match self {
            Self::Installed => "installed".to_string(),
            Self::Portable => "portable".to_string(),
            // clone_id is intentionally NOT encoded here; it round-trips
            // via a dedicated `AGENTMUX_CLONE_ID` env var so the existing
            // `dev:<branch>` wire format stays backward-compatible with
            // older launchers / parsers.
            Self::Dev { branch, .. } => format!("dev:{}", branch),
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
            // Slug includes the clone_id when present so two clones of
            // the same branch land in distinct subdirs. When clone_id
            // is None (env-roundtrip or direct test construction), fall
            // back to the legacy two-level layout for compatibility.
            Self::Dev { branch, clone_id } => {
                let b = sanitize_branch_slug(branch);
                match clone_id.as_deref().map(sanitize_clone_id) {
                    Some(c) if !c.is_empty() => format!("dev/{}/{}", b, c),
                    _ => format!("dev/{}", b),
                }
            }
        }
    }

    /// Read the runtime mode AND clone_id from the env vars exported
    /// by the parent process. Pairs `AGENTMUX_RUNTIME_MODE` (variant +
    /// branch) with `AGENTMUX_CLONE_ID` (Dev-only clone discriminator).
    /// This is the version every child process (host, srv) should
    /// call — [`Self::from_env`] is the legacy single-var form kept
    /// for callers that don't care about clone isolation.
    pub fn from_env_with_clone() -> Option<Self> {
        let mode = Self::from_env()?;
        if let Self::Dev { branch, .. } = mode {
            let clone_id = std::env::var("AGENTMUX_CLONE_ID")
                .ok()
                .map(|s| sanitize_clone_id(&s))
                .filter(|s| !s.is_empty());
            return Some(Self::Dev { branch, clone_id });
        }
        Some(mode)
    }
}

/// True when this binary is running from one of our known dev-build
/// output directories (`dist/cef-dev*/`, `target/debug/`, `target/release/`).
/// Walks ancestors to handle nested cases (CEF subprocesses run from a
/// `runtime/` subdir even in dev). Path-only — does not read env.
pub fn is_dev_build_exe(exe_dir: &Path) -> bool {
    exe_dir_is_dev_build(exe_dir)
}

/// True iff THIS binary is a source-tree / `task dev` build, determined from
/// the binary's path + portable marker via [`RuntimeMode::current_path_only`]
/// — NOT from `AGENTMUX_RUNTIME_MODE`. A running `task dev` AgentMux exports
/// `AGENTMUX_RUNTIME_MODE=dev:<branch>` into its environment, which every
/// descendant process inherits; a packaged build launched from a terminal /
/// agent pane inside a dev instance would otherwise mis-identify as Dev (e.g.
/// the "DEV" status-bar badge on a release build). Build identity is a
/// property of the binary on disk, not of whoever launched it.
///
/// Use for **build-identity** self-checks (the DEV badge, the "AgentMux DEV"
/// menu name, the dev-only frontend fallback). For intra-instance plumbing
/// that must agree with the launcher's already-made decision, use
/// [`RuntimeMode::from_env`] instead.
pub fn is_dev_self() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .map(|d| matches!(RuntimeMode::current_path_only(&d), RuntimeMode::Dev { .. }))
        .unwrap_or(false)
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
        // escape the dev/ subtree.
        let slug = sanitize_branch_slug(raw_branch);
        if slug.is_empty() {
            return None;
        }
        // clone_id is not encoded in this wire format — callers that
        // care about clone isolation should use [`RuntimeMode::from_env_with_clone`]
        // which pairs this with `AGENTMUX_CLONE_ID`.
        return Some(RuntimeMode::Dev {
            branch: slug,
            clone_id: None,
        });
    }
    if trimmed.eq_ignore_ascii_case("dev") {
        return Some(RuntimeMode::Dev {
            branch: "default".to_string(),
            clone_id: None,
        });
    }
    None
}

// ── Clone-id derivation ────────────────────────────────────────────────

/// Walk up from `exe_dir` looking for a workspace-root marker, then
/// hash that absolute (canonical) path with FNV-1a and return a 16-char
/// hex string. Used as the per-clone discriminator in
/// `~/.agentmux/dev/<branch>/<clone_id>/`. Returns `None` if no marker
/// is found (extreme edge case — `task dev` always runs from inside a
/// clone with `.git`/`Cargo.toml`/`Taskfile.yml`).
///
/// Recognized markers (any one suffices, in priority order):
/// `.git` (dir or file — supports git worktrees), `Cargo.toml`,
/// `Taskfile.yml`. We prefer `.git` because it identifies the literal
/// clone, but Cargo.toml is a safe fallback (the workspace root has
/// a `[workspace]` Cargo.toml).
pub fn derive_clone_id(exe_dir: &Path) -> Option<String> {
    let root = find_clone_root(exe_dir)?;
    // Canonicalize to absorb mixed casing on Windows and resolve `..`
    // segments. Falls back to the raw path if canonicalize fails
    // (rare on real filesystems but possible on transient mounts).
    let canonical = root.canonicalize().unwrap_or(root);
    let s = canonical.to_string_lossy().to_lowercase();
    Some(format!("{:016x}", fnv1a_64(s.as_bytes())))
}

fn find_clone_root(start: &Path) -> Option<std::path::PathBuf> {
    let mut cur = Some(start);
    while let Some(p) = cur {
        if p.join(".git").exists()
            || p.join("Cargo.toml").is_file()
            || p.join("Taskfile.yml").is_file()
        {
            return Some(p.to_path_buf());
        }
        cur = p.parent();
    }
    None
}

/// Lenient sanitization for a clone_id received via env. The launcher
/// produces a clean 16-hex string, but env vars survive process hops
/// and could be tampered with — refuse anything that contains path
/// separators or `..` segments before it lands in a filesystem path.
fn sanitize_clone_id(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
        || trimmed.contains('\0')
    {
        return String::new();
    }
    trimmed.to_string()
}

// Tiny FNV-1a-64 kept inline so agentmux-common doesn't have to depend
// on agentmux-launcher. Matches `agentmux-launcher/src/hash.rs`
// byte-for-byte so hashes are interchangeable.
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// True when `exe_dir` (or its parent on macOS app bundles) contains
/// the `agentmux-portable.marker` marker file written by
/// `scripts/package-portable.sh` at packaging time. Installed builds
/// NEVER write this marker.
///
/// Falls back to `false` (i.e., not portable) if the dir isn't readable
/// — installed-mode default is the safer guess when unsure.
fn is_portable_marker_present(exe_dir: &Path) -> bool {
    // Next to the detecting binary. Covers the host/srv (which live in
    // `runtime/`, so `exe_dir` IS `runtime/`) and legacy portables that wrote
    // the marker at the extract root next to the launcher.
    if exe_dir.join("agentmux-portable.marker").is_file() {
        return true;
    }
    // The launcher sits at the extract root with the marker one level down in
    // `runtime/` (the packaging keeps the root clean: just agentmux.exe +
    // README + runtime/). Check there so root-level detection still works.
    if exe_dir.join("runtime").join("agentmux-portable.marker").is_file() {
        return true;
    }
    // On macOS the launcher exe is at <Bundle>.app/Contents/MacOS/<exe>;
    // a portable .app would put the marker at the bundle root, two
    // levels up.
    if let Some(bundle_root) = exe_dir.parent().and_then(|p| p.parent()) {
        if bundle_root.join("agentmux-portable.marker").is_file() {
            return true;
        }
    }
    false
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

        if (parent_name == "dist" && name.starts_with("cef-dev"))
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
    let mut cmd = Command::new("git");
    cmd.args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(repo_dir);
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW: console-flash suppression — std::process::Command
        // needs the CommandExt trait to call creation_flags.
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(crate::win32::CREATE_NO_WINDOW);
    }
    let output = cmd.output().ok()?;
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
    // tempfile is a dev-dep used here for the marker-detection test.

    /// `with_env` does NOT take `TEST_ENV_LOCK` — callers must hold
    /// it. This avoids the re-entrant-locking problem when a test
    /// nests `with_env` calls. The lock is shared across the whole
    /// crate (defined in lib.rs) so tests in this module also
    /// serialize against `data_paths::tests` which touches the same
    /// process-global env vars.
    ///
    /// Uses a Drop guard so the previous env value is restored even
    /// if `f` panics — without it, a panicking test would leave the
    /// env var modified and any subsequent test in the same process
    /// would see the wrong value.
    fn with_env<F: FnOnce()>(key: &str, val: Option<&str>, f: F) {
        struct EnvGuard {
            key: String,
            prev: Option<String>,
        }
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                match &self.prev {
                    Some(v) => std::env::set_var(&self.key, v),
                    None => std::env::remove_var(&self.key),
                }
            }
        }

        let prev = std::env::var(key).ok();
        let _guard = EnvGuard {
            key: key.to_string(),
            prev,
        };
        match val {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
        f();
        // _guard drops here (also runs on panic).
    }

    #[test]
    fn parses_env_strings_round_trip() {
        for mode in [
            RuntimeMode::Installed,
            RuntimeMode::Portable,
            RuntimeMode::Dev {
                branch: "main".into(),
                clone_id: None,
            },
            RuntimeMode::Dev {
                branch: "agenta-feature-x".into(),
                clone_id: None,
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
                branch: "default".into(),
                clone_id: None,
            })
        );
        assert_eq!(
            parse_mode_string("DEV"),
            Some(RuntimeMode::Dev {
                branch: "default".into(),
                clone_id: None,
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
                        branch: "main".into(),
                        clone_id: None,
                    }
                );
            });
        });
    }

    #[test]
    fn dev_build_path_pattern() {
        // Match dist/cef-dev/... (fixed name)
        assert!(exe_dir_is_dev_build(&PathBuf::from(
            "/c/Systems/agentmux/dist/cef-dev"
        )));
        // Match dist/cef-dev-<epoch>/... (timestamp-stamped, Windows re-launch fix)
        assert!(exe_dir_is_dev_build(&PathBuf::from(
            "/c/Systems/agentmux/dist/cef-dev-1719100000"
        )));
        assert!(exe_dir_is_dev_build(&PathBuf::from(
            "/c/Systems/agentmux/dist/cef-dev-1719100000/runtime"
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
                branch: "main".into(),
                clone_id: None,
            }
            .dir_slug(),
            "dev/main"
        );
        assert_eq!(
            RuntimeMode::Dev {
                branch: "agenta/x".into(),
                clone_id: None,
            }
            .dir_slug(),
            "dev/agenta-x"
        );
        // With a clone_id, the slug nests one level deeper so two
        // clones on the same branch land in distinct subdirs.
        assert_eq!(
            RuntimeMode::Dev {
                branch: "main".into(),
                clone_id: Some("abcdef1234567890".into()),
            }
            .dir_slug(),
            "dev/main/abcdef1234567890"
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
            RuntimeMode::Dev { branch, .. } => {
                assert!(!branch.contains(".."));
                assert!(!branch.contains('/'));
                assert!(!branch.contains('\\'));
            }
            _ => panic!("expected Dev variant"),
        }
    }

    #[test]
    fn dev_branch_env_with_unusable_value_falls_through() {
        // AGENTMUX_DEV_BRANCH=`..` sanitizes to empty.
        // With the fix, an unusable value falls through to Installed
        // when no other dev signal applies — even when exe_dir has
        // a .git ancestor that detect_branch could have grabbed.
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::TempDir::new().expect("tempdir");
        // Plant a .git directory so a buggy implementation that called
        // detect_branch as a fallback would find a real branch name and
        // wrongly classify as Dev. Step 4 now sanitizes the env var
        // directly without invoking the git lookup — verifying that.
        std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
        let exe_dir = tmp.path().join("subdir");
        std::fs::create_dir_all(&exe_dir).unwrap();

        std::env::remove_var("AGENTMUX_RUNTIME_MODE");
        std::env::set_var("AGENTMUX_DEV_BRANCH", "..");
        let mode = RuntimeMode::current(&exe_dir);
        std::env::remove_var("AGENTMUX_DEV_BRANCH");

        // Even with the .git ancestor, an unusable env value must
        // fall through to Installed (NOT to Dev with a git-detected
        // branch). The env var is the source-of-truth at step 4.
        assert_eq!(mode, RuntimeMode::Installed);
    }

    #[test]
    fn portable_marker_detection() {
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let exe_dir = tmp.path();

        // No marker → not portable.
        assert!(!is_portable_marker_present(exe_dir));

        // With marker next to the exe → portable (host/srv in runtime/, or a
        // legacy portable with the marker at the root).
        std::fs::write(exe_dir.join("agentmux-portable.marker"), b"").unwrap();
        assert!(is_portable_marker_present(exe_dir));
        std::fs::remove_file(exe_dir.join("agentmux-portable.marker")).unwrap();

        // Marker inside runtime/ → portable (the launcher sits at the root and
        // the marker lives one level down in runtime/). An empty runtime/ with
        // no marker must NOT count (covered by
        // current_with_only_runtime_subdir_is_not_portable).
        std::fs::create_dir_all(exe_dir.join("runtime")).unwrap();
        assert!(!is_portable_marker_present(exe_dir));
        std::fs::write(exe_dir.join("runtime").join("agentmux-portable.marker"), b"").unwrap();
        assert!(is_portable_marker_present(exe_dir));
        std::fs::remove_file(exe_dir.join("runtime").join("agentmux-portable.marker")).unwrap();

        // Marker two levels up (macOS .app bundle case). All markers are
        // already removed above, so the bundle exe dir starts clean.
        let nested = exe_dir.join("Contents/MacOS");
        std::fs::create_dir_all(&nested).unwrap();
        assert!(!is_portable_marker_present(&nested));
        std::fs::write(exe_dir.join("agentmux-portable.marker"), b"").unwrap();
        assert!(is_portable_marker_present(&nested));
    }

    #[test]
    fn current_with_only_runtime_subdir_is_not_portable() {
        // Regression: prior implementation returned Portable whenever
        // <exe>/runtime/ existed, but installed builds also co-locate
        // runtime/ (per the launcher unconditionally requiring it).
        // Without a marker, we must NOT classify as Portable.
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let exe_dir = tmp.path();
        std::fs::create_dir_all(exe_dir.join("runtime")).unwrap();
        // No agentmux-portable.marker marker — must be Installed (or whatever
        // the fall-through is, but specifically NOT Portable).
        std::env::remove_var("AGENTMUX_RUNTIME_MODE");
        std::env::remove_var("AGENTMUX_DEV_BRANCH");
        let mode = RuntimeMode::current(exe_dir);
        assert_ne!(mode, RuntimeMode::Portable);
    }

    #[test]
    fn sanitize_branch_slug_strips_traversal() {
        assert_eq!(sanitize_branch_slug(".."), "");
        assert_eq!(sanitize_branch_slug("../foo"), "foo");
        assert_eq!(sanitize_branch_slug("foo/../bar"), "foo-bar");
        assert_eq!(sanitize_branch_slug(".hidden"), "hidden");
        assert_eq!(sanitize_branch_slug("ok-branch"), "ok-branch");
    }

    // ── derive_clone_id ─────────────────────────────────────────────

    #[test]
    fn derive_clone_id_returns_16_hex_chars_when_marker_present() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        // Plant a Cargo.toml at the root to act as the clone-root marker.
        std::fs::write(tmp.path().join("Cargo.toml"), b"[workspace]").unwrap();
        let nested = tmp.path().join("target/release");
        std::fs::create_dir_all(&nested).unwrap();
        let id = derive_clone_id(&nested).expect("found");
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn derive_clone_id_is_stable_for_same_clone() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        std::fs::write(tmp.path().join("Taskfile.yml"), b"version: 3").unwrap();
        let exe = tmp.path().join("dist/cef-dev");
        std::fs::create_dir_all(&exe).unwrap();
        let a = derive_clone_id(&exe).unwrap();
        let b = derive_clone_id(&exe).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn derive_clone_id_differs_between_clones() {
        let tmp1 = tempfile::TempDir::new().expect("tempdir1");
        let tmp2 = tempfile::TempDir::new().expect("tempdir2");
        for t in [&tmp1, &tmp2] {
            std::fs::write(t.path().join("Cargo.toml"), b"[workspace]").unwrap();
        }
        let id1 = derive_clone_id(tmp1.path()).unwrap();
        let id2 = derive_clone_id(tmp2.path()).unwrap();
        assert_ne!(
            id1, id2,
            "two clones at different paths must hash to different ids"
        );
    }

    #[test]
    fn derive_clone_id_returns_none_without_marker() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        // No marker at all — caller has no clone root to anchor to.
        let id = derive_clone_id(tmp.path());
        assert!(id.is_none());
    }

    #[test]
    fn derive_clone_id_walks_up_to_find_marker() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
        let deep = tmp.path().join("a/b/c/d");
        std::fs::create_dir_all(&deep).unwrap();
        // From a deeply nested subdir, we should still find the .git marker.
        assert!(derive_clone_id(&deep).is_some());
    }

    #[test]
    fn sanitize_clone_id_rejects_traversal() {
        assert_eq!(sanitize_clone_id("../foo"), "");
        assert_eq!(sanitize_clone_id("a/b"), "");
        assert_eq!(sanitize_clone_id("a\\b"), "");
        assert_eq!(sanitize_clone_id("a..b"), "");
        assert_eq!(sanitize_clone_id(""), "");
        assert_eq!(sanitize_clone_id("abcdef1234567890"), "abcdef1234567890");
    }

    #[test]
    fn from_env_with_clone_populates_clone_id_for_dev() {
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        with_env("AGENTMUX_RUNTIME_MODE", Some("dev:main"), || {
            with_env("AGENTMUX_CLONE_ID", Some("deadbeefcafebabe"), || {
                let m = RuntimeMode::from_env_with_clone().unwrap();
                assert_eq!(
                    m,
                    RuntimeMode::Dev {
                        branch: "main".into(),
                        clone_id: Some("deadbeefcafebabe".into()),
                    }
                );
            });
        });
    }

    #[test]
    fn from_env_with_clone_leaves_clone_id_none_when_unset() {
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        with_env("AGENTMUX_RUNTIME_MODE", Some("dev:main"), || {
            with_env("AGENTMUX_CLONE_ID", None, || {
                let m = RuntimeMode::from_env_with_clone().unwrap();
                assert_eq!(
                    m,
                    RuntimeMode::Dev {
                        branch: "main".into(),
                        clone_id: None,
                    }
                );
            });
        });
    }

    /// Regression for "DEV badge on every build": a packaged build launched as
    /// a descendant of a `task dev` AgentMux inherits
    /// `AGENTMUX_RUNTIME_MODE=dev:main`. Build identity (badge, menu name,
    /// dev-frontend fallback) goes through `current_path_only` / `is_dev_self`,
    /// which must ignore that env and classify purely by the binary's path.
    #[test]
    fn current_path_only_ignores_leaked_dev_env() {
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        with_env("AGENTMUX_RUNTIME_MODE", Some("dev:main"), || {
            // A non-dev exe dir: no portable marker, not a dev-build path.
            let installed = std::path::Path::new("/Applications/AgentMux.app/Contents/MacOS");
            assert!(
                matches!(
                    RuntimeMode::current_path_only(installed),
                    RuntimeMode::Installed
                ),
                "current_path_only must ignore a leaked AGENTMUX_RUNTIME_MODE=dev:main"
            );
            // Contrast: current() DOES still honor an explicit env override.
            assert!(
                matches!(RuntimeMode::current(installed), RuntimeMode::Dev { .. }),
                "current() still honors an explicit AGENTMUX_RUNTIME_MODE override"
            );
        });
    }
}
