// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where provider CLIs (claude, codex, …) are installed, shared by every
//! resolver: `resolvecli`, `install.start` / `install.check`, `agent.open`'s
//! launch path and the picker's read-only lookup.
//!
//! **Shared, keyed by pinned CLI version:**
//! `<shared_dir>/cli/<provider>/<pinned_version>/`. The install used to live
//! in `<home>/instances/v<AGENTMUX_VERSION>/cli/<provider>/`, i.e. inside the
//! AgentMux *version* dir, so every upgrade, portable or dev build started
//! with no CLI and the first open of each provider ran a blocking
//! `npm install` of the very same pinned CLI (~3 s, 16 times in a week of
//! logs — SPEC_AGENT_OPEN_LATENCY_2026_09_27.md F1). The CLI's identity is its
//! pinned version, not AgentMux's, so that is the key now.
//!
//! **Legacy fallback (read-only):** an existing per-version install still
//! resolves, so nothing breaks for a version that already has one; new
//! installs always go to the shared dir.
//!
//! **Cross-instance safety:** the shared dir is written by every AgentMux
//! instance on the machine, so installs into it serialize on an exclusive
//! lock file (`<provider>/<pinned_version>.lock`, blocking, both platforms —
//! the same primitive the agent registry uses) and re-check for a finished
//! install after acquiring it, so a second instance waits and then reuses
//! the first one's result instead of running npm into the same directory.
//! A shared dir only counts as installed once [`COMPLETE_MARKER`] is written,
//! after npm succeeded and the shim was verified; an unmarked dir (a failed or
//! in-progress install), or a marked one whose shim is gone, is cleared —
//! marker and all — before the next attempt.
//! `install.start` waits for the lock with [`try_lock_install`] so the wait
//! stays cancellable.

use std::path::{Path, PathBuf};

use agentmux_common::DataPaths;

const AGENTMUX_VERSION: &str = env!("CARGO_PKG_VERSION");

/// A pinned version string is used as a path component, so it must be a
/// plain version: letters, digits and `.`, `-`, `_`, `+`, no `..`, bounded.
/// Anything else (including empty — an unpinned provider) has no shared dir
/// and falls back to the legacy per-version location.
pub fn is_safe_version_component(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 64
        && !v.contains("..")
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

/// Same rule for the provider id, which is also a path component.
fn is_safe_provider_component(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 64
        && p.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

/// `<shared_dir>/cli/<provider>/<pinned_version>/`, or None when either
/// component isn't safe to use as a path segment.
pub fn shared_cli_dir(
    paths: &DataPaths,
    provider_id: &str,
    pinned_version: &str,
) -> Option<PathBuf> {
    if !is_safe_provider_component(provider_id) || !is_safe_version_component(pinned_version) {
        return None;
    }
    Some(
        paths
            .shared_dir
            .join("cli")
            .join(provider_id)
            .join(pinned_version),
    )
}

/// The pre-2026-09-27 per-AgentMux-version location, kept for read-only
/// fallback and for unpinned providers.
pub fn legacy_cli_dir(paths: &DataPaths, provider_id: &str) -> PathBuf {
    paths
        .home_dir
        .join("instances")
        .join(format!("v{AGENTMUX_VERSION}"))
        .join("cli")
        .join(provider_id)
}

/// The directory new installs go to: the shared dir when the provider is
/// pinned, else the legacy one.
pub fn install_dir(paths: &DataPaths, provider_id: &str, pinned_version: &str) -> PathBuf {
    shared_cli_dir(paths, provider_id, pinned_version)
        .unwrap_or_else(|| legacy_cli_dir(paths, provider_id))
}

/// `<dir>/node_modules/.bin/<cli>` — `.cmd` on Windows, where npm writes a
/// batch shim.
pub fn npm_bin(dir: &Path, cli_command: &str) -> PathBuf {
    let bin = dir.join("node_modules").join(".bin");
    if cfg!(windows) {
        bin.join(format!("{cli_command}.cmd"))
    } else {
        bin.join(cli_command)
    }
}

/// Written into a shared install dir only after npm succeeded and the shim was
/// verified, under the install lock. npm creates `.bin` shims before its
/// lifecycle scripts and the rest of the install finish, so a shim alone
/// doesn't mean "installed" — a reader could otherwise run a CLI another
/// instance is still installing, and a failed install's leftover shim would
/// poison the shared cache for every later AgentMux version (Codex P2 on #3927).
pub const COMPLETE_MARKER: &str = ".agentmux-install-complete";

/// Whether `dir` holds a finished, verified install.
pub fn is_complete(dir: &Path) -> bool {
    dir.join(COMPLETE_MARKER).is_file()
}

/// Mark `dir` complete (write-then-rename, so a reader never sees a torn
/// marker). Call under the install lock, after verifying the shim.
pub fn mark_complete(dir: &Path, version: &str) -> std::io::Result<()> {
    let tmp = dir.join(format!("{COMPLETE_MARKER}.tmp"));
    std::fs::write(&tmp, version)?;
    std::fs::rename(&tmp, dir.join(COMPLETE_MARKER))
}

/// Whether `dir` holds a usable install: marked complete AND its shim is
/// still there. A marker alone isn't enough — a finished install can lose its
/// shim later (corruption, antivirus cleanup).
pub fn is_valid_install(dir: &Path, cli_command: &str) -> bool {
    is_complete(dir) && npm_bin(dir, cli_command).is_file()
}

/// Under the install lock, before (re)installing into `dir`: unless it holds
/// a valid install, remove it entirely — leftovers of a failed or interrupted
/// install, and a stale completion marker whose shim is gone. Removing the
/// marker is the point: a repair must not run with the old marker visible, or
/// readers would accept the half-repaired install as soon as npm recreates the
/// shim, and a failed repair would leave marker + shim accepted indefinitely
/// (Codex P2 on #3927, second pass). No-op for a missing or valid dir.
pub fn clear_unless_valid(dir: &Path, cli_command: &str) -> std::io::Result<()> {
    if dir.exists() && !is_valid_install(dir, cli_command) {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}

/// The installed CLI binary, if any: the shared dir (only once marked
/// complete), then the legacy per-version dir (which predates the marker, so
/// its shim is enough).
pub fn find_installed(
    paths: &DataPaths,
    provider_id: &str,
    pinned_version: &str,
    cli_command: &str,
) -> Option<PathBuf> {
    let shared = shared_cli_dir(paths, provider_id, pinned_version)
        .filter(|d| is_complete(d))
        .map(|d| npm_bin(&d, cli_command))
        .filter(|b| b.is_file());
    shared.or_else(|| {
        let legacy = npm_bin(&legacy_cli_dir(paths, provider_id), cli_command);
        legacy.is_file().then_some(legacy)
    })
}

/// Same, with the pinned version taken from the backend provider registry
/// (the source of truth) rather than a caller-supplied string.
pub fn find_installed_for_provider(paths: &DataPaths, provider_id: &str) -> Option<PathBuf> {
    let provider = crate::backend::providers::get_provider(provider_id)?;
    find_installed(
        paths,
        provider.id,
        provider.pinned_version,
        provider.cli_command,
    )
}

/// Held for the duration of an install into a shared dir; dropping it
/// releases the lock. `None` inside for the legacy (per-instance) dir,
/// which needs no cross-instance lock.
pub struct InstallGuard {
    _file: Option<std::fs::File>,
}

/// Take the cross-instance install lock for `dir` when it is a shared dir.
/// Blocks until any other instance's install into the same dir finishes —
/// call it from a blocking context (`spawn_blocking`). The lock file sits
/// next to the dir and is never deleted (deleting a locked file races
/// waiters onto a different inode; see `registry::leases::CriticalSection`).
pub fn lock_install(
    paths: &DataPaths,
    provider_id: &str,
    pinned_version: &str,
) -> std::io::Result<InstallGuard> {
    let Some(file) = open_lock_file(paths, provider_id, pinned_version)? else {
        return Ok(InstallGuard { _file: None });
    };
    crate::registry::lock_exclusive(&file)?;
    Ok(InstallGuard { _file: Some(file) })
}

/// Non-blocking [`lock_install`]: `Ok(None)` while another instance holds
/// the lock. For waiters that must stay cancellable (`install.start` polls
/// this and races `install.cancel`, Codex P2 on #3927).
pub fn try_lock_install(
    paths: &DataPaths,
    provider_id: &str,
    pinned_version: &str,
) -> std::io::Result<Option<InstallGuard>> {
    let Some(file) = open_lock_file(paths, provider_id, pinned_version)? else {
        return Ok(Some(InstallGuard { _file: None }));
    };
    if crate::registry::try_lock_exclusive(&file)? {
        Ok(Some(InstallGuard { _file: Some(file) }))
    } else {
        Ok(None)
    }
}

/// The shared dir's lock file, opened (created if missing); None for a
/// provider/pin with no shared dir.
fn open_lock_file(
    paths: &DataPaths,
    provider_id: &str,
    pinned_version: &str,
) -> std::io::Result<Option<std::fs::File>> {
    let Some(dir) = shared_cli_dir(paths, provider_id, pinned_version) else {
        return Ok(None);
    };
    let parent = dir.parent().unwrap_or(&dir).to_path_buf();
    std::fs::create_dir_all(&parent)?;
    let lock_path = parent.join(format!("{pinned_version}.lock"));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;
    Ok(Some(file))
}

// ─── installing ─────────────────────────────────────────────────────────────

/// What to install: `npm_package@pinned_version`, whose shim is `cli_command`.
#[derive(Debug, Clone)]
pub struct NpmInstallRequest {
    pub provider_id: String,
    pub npm_package: String,
    pub pinned_version: String,
    pub cli_command: String,
    /// Run npm below normal priority (the startup warm-up, not an open
    /// someone is waiting on).
    pub background: bool,
}

/// What `npm install` produced.
#[derive(Debug)]
pub struct NpmRun {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug)]
pub enum InstallOutcome {
    /// This call ran npm; the shim is here.
    Installed(PathBuf),
    /// A finished install was already there — typically another instance's,
    /// completed while this one waited for the lock.
    AlreadyInstalled(PathBuf),
}

#[derive(Debug)]
pub enum InstallError {
    Lock(std::io::Error),
    Clear(PathBuf, std::io::Error),
    Spawn(std::io::Error),
    NpmFailed { code: Option<i32> },
    ShimMissing { expected: PathBuf },
    Mark(PathBuf, std::io::Error),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::Lock(e) => write!(f, "cannot take the CLI install lock: {e}"),
            InstallError::Clear(d, e) => write!(
                f,
                "cannot clear an incomplete CLI install at {}: {e}",
                d.display()
            ),
            InstallError::Spawn(e) => write!(f, "failed to run npm install: {e}"),
            InstallError::NpmFailed { code } => {
                write!(f, "npm install exited {}", code.unwrap_or(-1))
            }
            InstallError::ShimMissing { expected } => {
                write!(f, "npm install left no CLI shim at {}", expected.display())
            }
            InstallError::Mark(d, e) => write!(
                f,
                "cannot mark the CLI install complete at {}: {e}",
                d.display()
            ),
        }
    }
}

/// Install `req` into its install dir, **blocking** — call it from
/// `spawn_blocking` or a plain thread. The one install routine for
/// `resolvecli` and the startup warm-up ([`warm_used_providers`]):
///
/// 1. Take the cross-instance lock, and hold it until this returns — so npm
///    never outlives the lock (Codex P2 on #3927: a timed-out RPC used to
///    drop its guard while npm kept writing).
/// 2. A valid install already there (another instance finished while this
///    one waited): reuse it. Otherwise clear leftovers, marker and all.
/// 3. `npm install --prefix <dir> <pkg>@<pin>`; every output line goes to
///    `on_line(stream, line)`.
/// 4. Verify the shim, then publish with [`COMPLETE_MARKER`].
pub fn install_pinned_cli(
    paths: &DataPaths,
    req: &NpmInstallRequest,
    on_line: &dyn Fn(&'static str, &str),
) -> Result<InstallOutcome, InstallError> {
    install_pinned_cli_with(paths, req, on_line, run_npm_install)
}

/// [`install_pinned_cli`] with the npm runner injected (tests).
pub fn install_pinned_cli_with(
    paths: &DataPaths,
    req: &NpmInstallRequest,
    on_line: &dyn Fn(&'static str, &str),
    run_npm: impl FnOnce(&Path, &str, bool) -> std::io::Result<NpmRun>,
) -> Result<InstallOutcome, InstallError> {
    let dir = install_dir(paths, &req.provider_id, &req.pinned_version);
    let bin = npm_bin(&dir, &req.cli_command);
    let _guard =
        lock_install(paths, &req.provider_id, &req.pinned_version).map_err(InstallError::Lock)?;
    let shared = shared_cli_dir(paths, &req.provider_id, &req.pinned_version);
    if let Some(d) = shared.as_deref() {
        if is_valid_install(d, &req.cli_command) {
            return Ok(InstallOutcome::AlreadyInstalled(bin));
        }
        clear_unless_valid(d, &req.cli_command)
            .map_err(|e| InstallError::Clear(d.to_path_buf(), e))?;
    }
    let package = format!("{}@{}", req.npm_package, req.pinned_version);
    tracing::info!(package = %package, prefix = %dir.display(), background = req.background, "running npm install");
    let run = run_npm(&dir, &package, req.background).map_err(InstallError::Spawn)?;
    tracing::info!(
        exit_code = run.code.unwrap_or(-1),
        stdout_bytes = run.stdout.len(),
        stderr_bytes = run.stderr.len(),
        "npm install output collected"
    );
    // stderr first: npm writes progress and errors there.
    for (stream, bytes) in [("stderr", &run.stderr), ("stdout", &run.stdout)] {
        for line in String::from_utf8_lossy(bytes).lines() {
            if !line.trim().is_empty() {
                on_line(stream, line);
            }
        }
    }
    if !run.success {
        return Err(InstallError::NpmFailed { code: run.code });
    }
    if !bin.is_file() {
        return Err(InstallError::ShimMissing { expected: bin });
    }
    if let Some(d) = shared.as_deref() {
        mark_complete(d, &req.pinned_version)
            .map_err(|e| InstallError::Mark(d.to_path_buf(), e))?;
    }
    Ok(InstallOutcome::Installed(bin))
}

/// Run `npm install --prefix <dir> <package>` to completion.
///
/// Output is collected after exit (`.output()`): pipe-based streaming
/// (async IOCP and sync blocking alike) receives nothing from `cmd.exe /C`
/// batch children on Windows until the process exits.
fn run_npm_install(dir: &Path, package: &str, background: bool) -> std::io::Result<NpmRun> {
    let out = {
        #[cfg(windows)]
        {
            // npm on Windows is a .cmd batch script — it must run via
            // `cmd /C`. `raw_arg` passes the command string verbatim: with
            // `.args(["/C", s])` Rust quotes `s` and escapes inner quotes as
            // `\"`, which cmd.exe takes literally, corrupting the path
            // (CWD + \"C:\path\" → ENOENT).
            use std::os::windows::process::CommandExt;
            // No console flash from a GUI process (CREATE_NO_WINDOW); the
            // warm-up also runs below normal priority.
            const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
            let mut flags = agentmux_common::win32::CREATE_NO_WINDOW;
            if background {
                flags |= BELOW_NORMAL_PRIORITY_CLASS;
            }
            let prefix = dir.to_string_lossy().replace('/', "\\");
            let cmdline = format!(
                "npm install --loglevel=http --no-audit --no-fund --no-progress --prefix \"{prefix}\" {package}"
            );
            let mut c = std::process::Command::new("cmd");
            // `npm install` runs arbitrary postinstall scripts — no instance
            // identity for those.
            crate::backend::pane_env::sanitize_external_std_command(&mut c);
            c.arg("/C")
                .raw_arg(&cmdline)
                .creation_flags(flags)
                .env("CI", "true")
                .env("FORCE_COLOR", "0")
                .output()?
        }
        #[cfg(not(windows))]
        {
            let mut c = std::process::Command::new("npm");
            crate::backend::pane_env::sanitize_external_std_command(&mut c);
            c.args([
                "install",
                "--loglevel=http",
                "--no-audit",
                "--no-fund",
                "--no-progress",
                "--prefix",
            ])
            .arg(dir)
            .arg(package)
            .env("CI", "true")
            .env("FORCE_COLOR", "0");
            if background {
                use std::os::unix::process::CommandExt;
                // SAFETY: setpriority is async-signal-safe; nothing else runs
                // between fork and exec here.
                unsafe {
                    c.pre_exec(|| {
                        let _ = libc::setpriority(libc::PRIO_PROCESS, 0, 10);
                        Ok(())
                    });
                }
            }
            c.output()?
        }
    };
    Ok(NpmRun {
        success: out.status.success(),
        code: out.status.code(),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

// ─── install on demand ──────────────────────────────────────────────────────

/// Why [`ensure_installed`] did not hand back a CLI.
#[derive(Debug)]
pub enum EnsureError {
    /// The install ran and failed (no npm, offline, npm exited non-zero, ...).
    Failed(String),
    /// It is still running after the wait. The install is NOT cancelled: it keeps
    /// the cross-instance lock until npm returns, and a later call waits on that
    /// lock and then reuses the result.
    StillInstalling,
}

/// The pinned CLI for a provider whose install is missing: install it, waiting
/// up to `wait`. `install` is injected so tests don't need npm.
///
/// This is what the UI's launch flow does on a pin bump (`ResolveCli` installs
/// and waits). `agent.open` used to refuse instead, with `CLI_NOT_AVAILABLE`,
/// for the minute or two after an upgrade while the new pin downloaded.
pub async fn ensure_installed_with<F>(
    paths: DataPaths,
    req: NpmInstallRequest,
    wait: std::time::Duration,
    install: F,
) -> Result<PathBuf, EnsureError>
where
    F: FnOnce(&DataPaths, &NpmInstallRequest) -> Result<InstallOutcome, InstallError>
        + Send
        + 'static,
{
    let task = tokio::task::spawn_blocking(move || install(&paths, &req));
    match tokio::time::timeout(wait, task).await {
        Ok(Ok(Ok(InstallOutcome::Installed(bin) | InstallOutcome::AlreadyInstalled(bin)))) => {
            Ok(bin)
        }
        Ok(Ok(Err(e))) => Err(EnsureError::Failed(e.to_string())),
        Ok(Err(join)) => Err(EnsureError::Failed(format!(
            "the install task failed: {join}"
        ))),
        Err(_elapsed) => Err(EnsureError::StillInstalling),
    }
}

/// [`ensure_installed_with`] running the real npm install.
pub async fn ensure_installed(
    paths: DataPaths,
    req: NpmInstallRequest,
    wait: std::time::Duration,
) -> Result<PathBuf, EnsureError> {
    ensure_installed_with(paths, req, wait, |p, r| {
        install_pinned_cli(p, r, &|_, _| {})
    })
    .await
}

// ─── startup warm-up ────────────────────────────────────────────────────────

/// How long after srv start the warm-up begins: after the first IPC traffic,
/// well before most users open an agent.
pub const WARM_INSTALL_DELAY: std::time::Duration = std::time::Duration::from_secs(5);

/// The npm-installed providers among `provider_ids` whose pinned CLI has no
/// install yet, deduplicated, in first-seen order. Pure over the registry and
/// the filesystem.
pub fn providers_needing_install(
    paths: &DataPaths,
    provider_ids: &[String],
) -> Vec<NpmInstallRequest> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for id in provider_ids {
        let Some(p) = crate::backend::providers::get_provider(id) else {
            continue;
        };
        if p.npm_package.is_empty() || !seen.insert(p.id) {
            continue;
        }
        if find_installed(paths, p.id, p.pinned_version, p.cli_command).is_some() {
            continue;
        }
        out.push(NpmInstallRequest {
            provider_id: p.id.to_string(),
            npm_package: p.npm_package.to_string(),
            pinned_version: p.pinned_version.to_string(),
            cli_command: p.cli_command.to_string(),
            background: true,
        });
    }
    out
}

/// SPEC_AGENT_OPEN_LATENCY_2026_09_27.md §4.1: install the pinned CLI of every
/// provider the user's agents use, in the background, so the first open
/// after a pin bump doesn't wait on npm. Only providers in `provider_ids`
/// (the user's own agent definitions — not every template's), only pins with
/// no install yet, one at a time, npm below normal priority. An open that
/// arrives mid-install waits on the same lock and then reuses the result.
/// Failures (no npm, offline) are logged and left for the open to report.
pub fn warm_used_providers(
    paths: DataPaths,
    provider_ids: Vec<String>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("cli-install-warm".into())
        .spawn(move || {
            std::thread::sleep(WARM_INSTALL_DELAY);
            let todo = providers_needing_install(&paths, &provider_ids);
            if todo.is_empty() {
                tracing::info!(
                    providers = provider_ids.len(),
                    "cli warm-up: every used provider's pinned CLI is installed"
                );
                return;
            }
            for req in todo {
                let started = std::time::Instant::now();
                let quiet = |_: &'static str, _: &str| {};
                match install_pinned_cli(&paths, &req, &quiet) {
                    Ok(outcome) => tracing::info!(
                        provider = %req.provider_id,
                        pinned_version = %req.pinned_version,
                        duration_ms = started.elapsed().as_millis() as u64,
                        outcome = ?outcome,
                        "cli warm-up: installed"
                    ),
                    Err(e) => tracing::info!(
                        provider = %req.provider_id,
                        pinned_version = %req.pinned_version,
                        error = %e,
                        "cli warm-up: install failed; the first open will retry and report it"
                    ),
                }
            }
        })
        .expect("spawn cli-install-warm thread")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths_in(root: &Path) -> DataPaths {
        let version_dir = root
            .join("channels")
            .join("c")
            .join("versions")
            .join("1.0.0");
        DataPaths {
            home_dir: version_dir.join("data"),
            instance_dir: root.join("channels").join("c"),
            channel: "c".to_string(),
            data_dir: version_dir.join("data"),
            config_dir: root.join("channels").join("c").join("config"),
            logs_dir: version_dir.join("logs"),
            cef_cache_dir: version_dir.join("cef-cache"),
            agents_dir: root.join("channels").join("c").join("agents"),
            instance_runtime_dir: version_dir.join("runtime"),
            shared_dir: root.join("shared"),
            mode: agentmux_common::RuntimeMode::Portable,
        }
    }

    #[test]
    fn shared_dir_is_keyed_by_provider_and_pinned_version_not_agentmux_version() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths_in(tmp.path());
        let d = shared_cli_dir(&p, "claude", "2.1.280").unwrap();
        assert_eq!(
            d,
            tmp.path()
                .join("shared")
                .join("cli")
                .join("claude")
                .join("2.1.280")
        );
        assert!(!d.to_string_lossy().contains(AGENTMUX_VERSION) || AGENTMUX_VERSION == "2.1.280");
    }

    #[test]
    fn unsafe_components_have_no_shared_dir_and_fall_back_to_legacy() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths_in(tmp.path());
        for bad in ["", "..", "../x", "1.0/2", "1.0\\2", "a b"] {
            assert!(shared_cli_dir(&p, "claude", bad).is_none(), "{bad:?}");
            assert_eq!(install_dir(&p, "claude", bad), legacy_cli_dir(&p, "claude"));
        }
        assert!(shared_cli_dir(&p, "../claude", "1.0.0").is_none());
        assert!(is_safe_version_component("2.1.280"));
        assert!(is_safe_version_component("1.0.0-beta.1+build_7"));
    }

    fn touch_bin(dir: &Path, cli: &str) -> PathBuf {
        let b = npm_bin(dir, cli);
        std::fs::create_dir_all(b.parent().unwrap()).unwrap();
        std::fs::write(&b, "").unwrap();
        b
    }

    #[test]
    fn finds_the_shared_install_first_then_the_legacy_one() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths_in(tmp.path());
        assert!(find_installed(&p, "claude", "2.1.280", "claude").is_none());

        let legacy = touch_bin(&legacy_cli_dir(&p, "claude"), "claude");
        assert_eq!(
            find_installed(&p, "claude", "2.1.280", "claude"),
            Some(legacy.clone())
        );

        let shared_dir = shared_cli_dir(&p, "claude", "2.1.280").unwrap();
        let shared = touch_bin(&shared_dir, "claude");
        // A shim alone isn't an install (npm writes it early): still legacy.
        assert_eq!(
            find_installed(&p, "claude", "2.1.280", "claude"),
            Some(legacy.clone())
        );
        mark_complete(&shared_dir, "2.1.280").unwrap();
        assert_eq!(
            find_installed(&p, "claude", "2.1.280", "claude"),
            Some(shared)
        );

        // A different pinned version doesn't see the shared install.
        assert_eq!(
            find_installed(&p, "claude", "2.1.281", "claude"),
            Some(legacy)
        );
    }

    #[test]
    fn an_upgraded_agentmux_version_reuses_the_shared_install() {
        let tmp = tempfile::tempdir().unwrap();
        let old = paths_in(tmp.path());
        let shared_dir = shared_cli_dir(&old, "claude", "2.1.280").unwrap();
        let shared = touch_bin(&shared_dir, "claude");
        mark_complete(&shared_dir, "2.1.280").unwrap();

        // Same machine, a different AgentMux version dir, same pinned CLI.
        let mut new = paths_in(tmp.path());
        new.home_dir = tmp
            .path()
            .join("channels")
            .join("c")
            .join("versions")
            .join("9.9.9")
            .join("data");
        assert_eq!(
            find_installed(&new, "claude", "2.1.280", "claude"),
            Some(shared)
        );
    }

    #[test]
    fn the_install_lock_serializes_two_holders() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths_in(tmp.path());
        let first = lock_install(&p, "claude", "2.1.280").unwrap();
        assert!(tmp
            .path()
            .join("shared")
            .join("cli")
            .join("claude")
            .join("2.1.280.lock")
            .is_file());

        let p2 = paths_in(tmp.path());
        let (tx, rx) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let _second = lock_install(&p2, "claude", "2.1.280").unwrap();
            tx.send(()).unwrap();
        });
        // The second holder is still blocked while the first holds the lock.
        assert!(rx
            .recv_timeout(std::time::Duration::from_millis(300))
            .is_err());
        drop(first);
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("second holder acquires after release");
        waiter.join().unwrap();
    }

    #[test]
    fn the_legacy_dir_needs_no_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths_in(tmp.path());
        let g = lock_install(&p, "claude", "").unwrap();
        assert!(g._file.is_none());
    }

    /// Codex P2 on #3927: a shared shim without the completion marker (an
    /// install in progress in another instance, or one that failed) is never
    /// handed out, and it doesn't hide an absent install either.
    #[test]
    fn a_shim_without_the_completion_marker_is_not_an_install() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths_in(tmp.path());
        let dir = shared_cli_dir(&p, "claude", "2.1.280").unwrap();
        touch_bin(&dir, "claude");
        assert!(!is_complete(&dir));
        assert!(find_installed(&p, "claude", "2.1.280", "claude").is_none());
    }

    #[test]
    fn clear_unless_valid_removes_leftovers_but_keeps_a_finished_install() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths_in(tmp.path());
        let dir = shared_cli_dir(&p, "claude", "2.1.280").unwrap();
        touch_bin(&dir, "claude");
        clear_unless_valid(&dir, "claude").unwrap();
        assert!(!dir.exists(), "a partial install is cleared");

        touch_bin(&dir, "claude");
        mark_complete(&dir, "2.1.280").unwrap();
        clear_unless_valid(&dir, "claude").unwrap();
        assert!(
            is_valid_install(&dir, "claude"),
            "a finished install is kept"
        );
        clear_unless_valid(&tmp.path().join("missing"), "claude").unwrap();
    }

    /// Codex P2 on #3927 (second pass): a completed install that lost its
    /// shim must have its marker removed before the repair, so neither a
    /// repair in progress nor a failed one is ever accepted.
    #[test]
    fn a_completed_install_that_lost_its_shim_is_cleared_marker_and_all() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths_in(tmp.path());
        let dir = shared_cli_dir(&p, "claude", "2.1.280").unwrap();
        let shim = touch_bin(&dir, "claude");
        mark_complete(&dir, "2.1.280").unwrap();
        std::fs::remove_file(&shim).unwrap();
        assert!(is_complete(&dir) && !is_valid_install(&dir, "claude"));

        clear_unless_valid(&dir, "claude").unwrap();
        assert!(!dir.exists(), "stale marker and leftovers are gone");

        // A repair that recreates only the shim (npm mid-install) is not accepted.
        touch_bin(&dir, "claude");
        assert!(find_installed(&p, "claude", "2.1.280", "claude").is_none());
    }

    #[test]
    fn try_lock_install_reports_a_held_lock_instead_of_waiting() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths_in(tmp.path());
        let held = lock_install(&p, "claude", "2.1.280").unwrap();
        let p2 = paths_in(tmp.path());
        // A second instance's attempt, from another thread (flock/LockFileEx
        // are per-handle, so a second handle contends like another process).
        let busy = std::thread::spawn(move || {
            try_lock_install(&p2, "claude", "2.1.280")
                .unwrap()
                .is_none()
        })
        .join()
        .unwrap();
        assert!(
            busy,
            "try_lock_install must report the held lock, not block"
        );
        drop(held);
        let p3 = paths_in(tmp.path());
        let got = std::thread::spawn(move || {
            try_lock_install(&p3, "claude", "2.1.280")
                .unwrap()
                .is_some()
        })
        .join()
        .unwrap();
        assert!(got, "acquired once released");
        assert!(
            try_lock_install(&p, "claude", "").unwrap().is_some(),
            "no lock needed without a shared dir"
        );
    }

    fn claude_req(background: bool) -> NpmInstallRequest {
        NpmInstallRequest {
            provider_id: "claude".into(),
            npm_package: "@anthropic-ai/claude-code".into(),
            pinned_version: "2.1.280".into(),
            cli_command: "claude".into(),
            background,
        }
    }

    /// A fake npm: writes the shim unless told not to, and reports `success`.
    fn fake_npm(
        write_shim: bool,
        success: bool,
    ) -> impl FnOnce(&Path, &str, bool) -> std::io::Result<NpmRun> {
        move |dir: &Path, _pkg: &str, _bg: bool| {
            if write_shim {
                let bin = npm_bin(dir, "claude");
                std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
                std::fs::write(&bin, "").unwrap();
            }
            Ok(NpmRun {
                success,
                code: Some(if success { 0 } else { 1 }),
                stdout: b"added 3 packages\n".to_vec(),
                stderr: b"npm http fetch\n\n".to_vec(),
            })
        }
    }

    #[test]
    fn install_runs_npm_verifies_the_shim_and_marks_it_complete() {
        let _tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(_tmp.path());
        let lines = std::sync::Mutex::new(Vec::new());
        let on_line = |s: &'static str, l: &str| lines.lock().unwrap().push(format!("{s}:{l}"));
        let out =
            install_pinned_cli_with(&paths, &claude_req(false), &on_line, fake_npm(true, true))
                .unwrap();
        let dir = shared_cli_dir(&paths, "claude", "2.1.280").unwrap();
        assert!(matches!(out, InstallOutcome::Installed(ref b) if *b == npm_bin(&dir, "claude")));
        assert!(is_valid_install(&dir, "claude"));
        assert_eq!(
            *lines.lock().unwrap(),
            vec!["stderr:npm http fetch", "stdout:added 3 packages"]
        );
    }

    #[test]
    fn a_finished_install_is_reused_without_running_npm() {
        let _tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(_tmp.path());
        install_pinned_cli_with(&paths, &claude_req(true), &|_, _| {}, fake_npm(true, true))
            .unwrap();
        let out = install_pinned_cli_with(
            &paths,
            &claude_req(false),
            &|_, _| {},
            |_: &Path, _: &str, _: bool| -> std::io::Result<NpmRun> {
                panic!("npm must not run again")
            },
        )
        .unwrap();
        assert!(matches!(out, InstallOutcome::AlreadyInstalled(_)));
    }

    #[test]
    fn a_failed_npm_leaves_no_marker_and_the_next_attempt_starts_clean() {
        let _tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(_tmp.path());
        let dir = shared_cli_dir(&paths, "claude", "2.1.280").unwrap();
        // npm wrote a shim but exited non-zero: not an install.
        let err = install_pinned_cli_with(
            &paths,
            &claude_req(false),
            &|_, _| {},
            fake_npm(true, false),
        )
        .unwrap_err();
        assert!(matches!(err, InstallError::NpmFailed { code: Some(1) }));
        assert!(!is_complete(&dir));
        assert!(find_installed(&paths, "claude", "2.1.280", "claude").is_none());
        // The retry clears the leftover before npm runs.
        let retry = install_pinned_cli_with(
            &paths,
            &claude_req(false),
            &|_, _| {},
            |d: &Path, p: &str, b: bool| {
                assert!(
                    !npm_bin(d, "claude").exists(),
                    "leftover shim must be cleared first"
                );
                fake_npm(true, true)(d, p, b)
            },
        );
        assert!(matches!(retry, Ok(InstallOutcome::Installed(_))));
    }

    #[test]
    fn npm_success_without_a_shim_is_an_error_and_unmarked() {
        let _tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(_tmp.path());
        let err = install_pinned_cli_with(
            &paths,
            &claude_req(false),
            &|_, _| {},
            fake_npm(false, true),
        )
        .unwrap_err();
        assert!(matches!(err, InstallError::ShimMissing { .. }));
        assert!(!is_complete(
            &shared_cli_dir(&paths, "claude", "2.1.280").unwrap()
        ));
    }

    #[test]
    fn the_install_holds_the_lock_until_npm_returns() {
        let _tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(_tmp.path());
        let p2 = paths.clone();
        install_pinned_cli_with(
            &paths,
            &claude_req(false),
            &|_, _| {},
            move |d: &Path, pk: &str, b: bool| {
                // While npm "runs", nobody else can take the lock.
                assert!(try_lock_install(&p2, "claude", "2.1.280")
                    .unwrap()
                    .is_none());
                fake_npm(true, true)(d, pk, b)
            },
        )
        .unwrap();
        assert!(try_lock_install(&paths, "claude", "2.1.280")
            .unwrap()
            .is_some());
    }

    #[test]
    fn warm_up_picks_used_npm_providers_with_no_install_once_each() {
        let _tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(_tmp.path());
        let ids: Vec<String> = ["claude", "claude", "codex", "not-a-provider"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let todo: Vec<String> = providers_needing_install(&paths, &ids)
            .into_iter()
            .map(|r| r.provider_id)
            .collect();
        assert_eq!(todo, vec!["claude", "codex"]);
        assert!(providers_needing_install(&paths, &ids)
            .iter()
            .all(|r| r.background));

        // Installed → no longer needed. `providers_needing_install` checks
        // against the LIVE registry pin (`providers::get_provider`), unlike
        // `claude_req`'s own hardcoded fixture version — install a request
        // built from that live pin, or this only keeps passing by coincidence
        // (it did, silently, until the 2.1.280 -> 2.1.285 bump broke it).
        let live_claude = crate::backend::providers::get_provider("claude").unwrap();
        let live_req = NpmInstallRequest {
            provider_id: "claude".into(),
            npm_package: live_claude.npm_package.into(),
            pinned_version: live_claude.pinned_version.into(),
            cli_command: live_claude.cli_command.into(),
            background: true,
        };
        install_pinned_cli_with(&paths, &live_req, &|_, _| {}, fake_npm(true, true)).unwrap();
        let todo: Vec<String> = providers_needing_install(&paths, &ids)
            .into_iter()
            .map(|r| r.provider_id)
            .collect();
        assert_eq!(todo, vec!["codex"]);
    }

    // ── a pin bump, as a user meets it ─────────────────────────────────────
    //
    // The user upgraded AgentMux, the provider's pin moved, and they open an
    // existing agent. The previous pin is installed and complete; the new one is
    // not. These use the LIVE registry pin (never a hardcoded version — that is
    // what silently broke `warm_up_picks_used_npm_providers…` at 2.1.280 →
    // 2.1.285) and a fake npm.

    /// A completed install of "some earlier pin" of claude, plus the paths.
    fn with_previous_pin_installed() -> (tempfile::TempDir, DataPaths, String, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths_in(tmp.path());
        let live = crate::backend::providers::get_provider("claude").unwrap();
        let previous = "1.0.0-previous".to_string();
        assert_ne!(previous, live.pinned_version);
        let req = NpmInstallRequest {
            provider_id: "claude".into(),
            npm_package: live.npm_package.into(),
            pinned_version: previous.clone(),
            cli_command: live.cli_command.into(),
            background: true,
        };
        install_pinned_cli_with(&paths, &req, &|_, _| {}, fake_npm(true, true)).unwrap();
        let old_bin = npm_bin(
            &shared_cli_dir(&paths, "claude", &previous).unwrap(),
            "claude",
        );
        assert!(old_bin.is_file());
        (tmp, paths, previous, old_bin)
    }

    fn live_claude_req(background: bool) -> NpmInstallRequest {
        let live = crate::backend::providers::get_provider("claude").unwrap();
        NpmInstallRequest {
            provider_id: "claude".into(),
            npm_package: live.npm_package.into(),
            pinned_version: live.pinned_version.into(),
            cli_command: live.cli_command.into(),
            background,
        }
    }

    #[test]
    fn after_a_pin_bump_the_previous_install_is_not_mistaken_for_the_new_pin() {
        let (_tmp, paths, _prev, _old_bin) = with_previous_pin_installed();
        assert_eq!(
            find_installed_for_provider(&paths, "claude"),
            None,
            "an open must resolve the NEW pin (installing it), not run whatever was there"
        );
        // …and the warm-up knows it has work to do, for exactly the live pin.
        let todo = providers_needing_install(&paths, &["claude".to_string()]);
        assert_eq!(todo.len(), 1);
        let live = crate::backend::providers::get_provider("claude").unwrap();
        assert_eq!(todo[0].pinned_version, live.pinned_version);
    }

    #[test]
    fn installing_the_new_pin_leaves_the_previous_one_runnable_for_panes_that_hold_its_path() {
        // A pane restored with the old `cmd` path keeps working until its mount
        // flow rewrites it; nothing may delete the old install out from under it.
        let (_tmp, paths, previous, old_bin) = with_previous_pin_installed();
        install_pinned_cli_with(
            &paths,
            &live_claude_req(false),
            &|_, _| {},
            fake_npm(true, true),
        )
        .unwrap();

        let new_bin = find_installed_for_provider(&paths, "claude").expect("new pin resolves");
        assert_ne!(new_bin, old_bin);
        assert!(is_valid_install(
            &shared_cli_dir(&paths, "claude", &previous).unwrap(),
            "claude"
        ));
        assert!(old_bin.is_file(), "the previous pin's shim is untouched");
        // Resolving again is stable and does not reinstall.
        assert_eq!(find_installed_for_provider(&paths, "claude"), Some(new_bin));
    }

    #[test]
    fn a_failed_install_of_the_new_pin_does_not_fall_back_to_the_previous_one() {
        // DOCUMENTS CURRENT BEHAVIOUR: the pin is exact. Offline (or no npm)
        // right after an upgrade, opening an agent fails with the install error
        // rather than silently running the previous CLI. The previous install is
        // still on disk and valid; nothing selects it.
        let (_tmp, paths, previous, old_bin) = with_previous_pin_installed();
        let out = install_pinned_cli_with(
            &paths,
            &live_claude_req(false),
            &|_, _| {},
            fake_npm(false, false),
        );
        assert!(
            matches!(out, Err(InstallError::NpmFailed { .. })),
            "{out:?}"
        );
        assert_eq!(find_installed_for_provider(&paths, "claude"), None);
        assert!(old_bin.is_file());
        assert!(is_valid_install(
            &shared_cli_dir(&paths, "claude", &previous).unwrap(),
            "claude"
        ));
        // The failed attempt left nothing behind that a later attempt would trip on.
        let live = crate::backend::providers::get_provider("claude").unwrap();
        assert!(!shared_cli_dir(&paths, "claude", live.pinned_version)
            .unwrap()
            .exists());
    }

    // ── installing on demand (agent.open) ──────────────────────────────────

    fn live_req() -> NpmInstallRequest {
        live_claude_req(false)
    }

    #[tokio::test]
    async fn ensure_installed_installs_the_missing_pin_and_returns_the_shim() {
        let (_tmp, paths, _prev, _old) = with_previous_pin_installed();
        let live = crate::backend::providers::get_provider("claude").unwrap();
        let want = npm_bin(
            &shared_cli_dir(&paths, "claude", live.pinned_version).unwrap(),
            "claude",
        );
        let bin = ensure_installed_with(
            paths.clone(),
            live_req(),
            std::time::Duration::from_secs(30),
            |p, r| install_pinned_cli_with(p, r, &|_, _| {}, fake_npm(true, true)),
        )
        .await
        .expect("installs");
        assert_eq!(bin, want);
        assert!(bin.is_file());
        assert_eq!(find_installed_for_provider(&paths, "claude"), Some(bin));
    }

    #[tokio::test]
    async fn ensure_installed_reuses_an_install_that_finished_meanwhile() {
        let (_tmp, paths, _prev, _old) = with_previous_pin_installed();
        install_pinned_cli_with(
            &paths,
            &live_claude_req(true),
            &|_, _| {},
            fake_npm(true, true),
        )
        .unwrap();
        let bin = ensure_installed_with(
            paths,
            live_req(),
            std::time::Duration::from_secs(30),
            |p, r| {
                install_pinned_cli_with(
                    p,
                    r,
                    &|_, _| {},
                    |_: &Path, _: &str, _: bool| -> std::io::Result<NpmRun> {
                        panic!("npm must not run again")
                    },
                )
            },
        )
        .await
        .expect("reuses");
        assert!(bin.is_file());
    }

    #[tokio::test]
    async fn ensure_installed_reports_a_failed_install_with_its_reason() {
        let (_tmp, paths, _prev, _old) = with_previous_pin_installed();
        let err = ensure_installed_with(
            paths.clone(),
            live_req(),
            std::time::Duration::from_secs(30),
            |p, r| install_pinned_cli_with(p, r, &|_, _| {}, fake_npm(false, false)),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, EnsureError::Failed(m) if m.contains("npm install exited 1")),
            "{err:?}"
        );
        assert_eq!(find_installed_for_provider(&paths, "claude"), None);
    }

    #[tokio::test]
    async fn ensure_installed_stops_waiting_but_does_not_cancel_a_slow_install() {
        let (_tmp, paths, _prev, _old) = with_previous_pin_installed();
        let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = finished.clone();
        let err = ensure_installed_with(
            paths.clone(),
            live_req(),
            std::time::Duration::from_millis(50),
            move |p, r| {
                std::thread::sleep(std::time::Duration::from_millis(400));
                let out = install_pinned_cli_with(p, r, &|_, _| {}, fake_npm(true, true));
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                out
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, EnsureError::StillInstalling), "{err:?}");
        // The caller got its answer quickly; the install carries on and lands.
        for _ in 0..100 {
            if finished.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(
            finished.load(std::sync::atomic::Ordering::SeqCst),
            "the install must keep running"
        );
        assert!(
            find_installed_for_provider(&paths, "claude").is_some(),
            "and a later open finds it"
        );
    }
}
