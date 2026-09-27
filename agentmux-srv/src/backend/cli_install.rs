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
}
