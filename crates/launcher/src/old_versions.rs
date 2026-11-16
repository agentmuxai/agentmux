// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The old versions' files in a `setup.exe` install's `runtime\` folder.
//!
//! Each version's setup puts `agentmux-<ver>.exe`, `agentmux-<ver>.dll` and
//! `agentmux-srv-<ver>-windows.<arch>.exe` beside the others (a running old
//! version's files are never overwritten), and nothing took the old ones away:
//! about 95 MB a version. Once this version has run for a while, the launcher
//! removes all but the one previous version, which stays as the way back.
//!
//! Only those three names, only older than the running version (compared as
//! numbers, so 0.60.9 is older than 0.60.10), and only in an install with Inno
//! Setup's uninstaller: a portable folder is a single version by design, and
//! the CEF runtime files and the portable marker are never named here. A file
//! still in use (an older srv that agents are still on) cannot be deleted
//! while it runs; it is skipped and the next start tries again.

use std::path::{Path, PathBuf};

/// Versions older than the running one that stay.
pub const KEEP_PREVIOUS: usize = 1;

/// How long the launcher must have been up before it prunes: a version that
/// came up and stayed up is the one to keep.
pub const HEALTHY_AFTER: std::time::Duration = std::time::Duration::from_secs(10 * 60);

type Version = (u64, u64, u64);

/// `major.minor.patch` and nothing else (no prerelease or build suffix).
fn parse_version(v: &str) -> Option<Version> {
    let mut parts = v.split('.').map(|p| (!p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())).then(|| p.parse::<u64>().ok()).flatten());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// The version in one of the three versioned file names, `None` for any other name.
fn version_in_name(name: &str) -> Option<Version> {
    if let Some(rest) = name.strip_prefix("agentmux-srv-") {
        // agentmux-srv-<ver>-windows.<arch>.exe
        let (ver, tail) = rest.split_once("-windows.")?;
        return (tail.ends_with(".exe")).then(|| parse_version(ver)).flatten();
    }
    let rest = name.strip_prefix("agentmux-")?;
    let ver = rest.strip_suffix(".exe").or_else(|| rest.strip_suffix(".dll"))?;
    parse_version(ver)
}

/// The files to remove from `runtime_dir`: those of every version older than `running`
/// except the newest `keep` of them.
pub fn stale_files(runtime_dir: &Path, running: &str, keep: usize) -> Vec<PathBuf> {
    let Some(running) = parse_version(running) else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(runtime_dir) else { return Vec::new() };
    let mut found: Vec<(Version, PathBuf)> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let version = version_in_name(&e.file_name().to_string_lossy())?;
            (version < running).then(|| (version, e.path()))
        })
        .collect();
    let mut versions: Vec<Version> = found.iter().map(|(v, _)| *v).collect();
    versions.sort_unstable_by(|a, b| b.cmp(a));
    versions.dedup();
    let kept: Vec<Version> = versions.into_iter().take(keep).collect();
    found.retain(|(v, _)| !kept.contains(v));
    found.into_iter().map(|(_, p)| p).collect()
}

/// What a prune did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Pruned {
    pub removed: usize,
    pub bytes: u64,
    /// Files that could not be removed, mostly because something still runs from them.
    pub skipped: usize,
}

/// Remove [`stale_files`] of `runtime_dir`.
pub fn prune(runtime_dir: &Path, running: &str, keep: usize) -> Pruned {
    let mut done = Pruned::default();
    for path in stale_files(runtime_dir, running, keep) {
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        match std::fs::remove_file(&path) {
            Ok(()) => {
                done.removed += 1;
                done.bytes += size;
            }
            Err(_) => done.skipped += 1,
        }
    }
    done
}

/// Prune the install's `runtime\` once this launcher has been up for [`HEALTHY_AFTER`], on its own
/// thread. Only an install made by `setup.exe` (it has an uninstaller beside the launcher).
pub fn prune_when_healthy(exe_dir: PathBuf, runtime_dir: PathBuf, running: &'static str) {
    if !crate::upgrade_restart::has_uninstaller(&exe_dir) {
        return;
    }
    let _ = std::thread::Builder::new().name("old-versions".into()).spawn(move || {
        std::thread::sleep(HEALTHY_AFTER);
        let done = prune(&runtime_dir, running, KEEP_PREVIOUS);
        if done.removed > 0 || done.skipped > 0 {
            crate::logging::log(&format!(
                "old versions: removed {} files ({} MB), {} still in use",
                done.removed,
                done.bytes / 1_000_000,
                done.skipped
            ));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime_with(names: &[&str]) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("amux-oldver-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for n in names {
            std::fs::write(dir.join(n), b"x").unwrap();
        }
        dir
    }

    fn names(paths: Vec<PathBuf>) -> Vec<String> {
        let mut v: Vec<String> = paths.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn the_three_names_carry_a_version_and_others_do_not() {
        assert_eq!(version_in_name("agentmux-0.60.28.exe"), Some((0, 60, 28)));
        assert_eq!(version_in_name("agentmux-0.60.28.dll"), Some((0, 60, 28)));
        assert_eq!(version_in_name("agentmux-srv-0.60.28-windows.x64.exe"), Some((0, 60, 28)));
        assert_eq!(version_in_name("agentmux-srv-0.60.28-windows.arm64.exe"), Some((0, 60, 28)));
        for other in [
            "agentmux.exe",
            "agentmux-portable.marker",
            "agentmux-0.60.28.pdb",
            "agentmux-0.60.28-rc1.exe",
            "agentmux-0.60.exe",
            "agentmux-srv-0.60.28-linux.x64",
            "libcef.dll",
            "chrome_elf.dll",
            "agentmux-srv.exe",
        ] {
            assert_eq!(version_in_name(other), None, "{other}");
        }
    }

    #[test]
    fn versions_compare_as_numbers() {
        let dir = runtime_with(&[
            "agentmux-0.60.9.exe",
            "agentmux-0.60.10.exe",
            "agentmux-0.60.11.exe",
            "agentmux-0.60.12.exe",
        ]);
        // Running 0.60.12: 0.60.11 stays, the rest go (as strings, "0.60.9" would sort last and be kept).
        assert_eq!(names(stale_files(&dir, "0.60.12", 1)), ["agentmux-0.60.10.exe", "agentmux-0.60.9.exe"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_running_and_newer_versions_and_other_files_stay() {
        let dir = runtime_with(&[
            "agentmux-0.60.26.exe",
            "agentmux-0.60.26.dll",
            "agentmux-srv-0.60.26-windows.x64.exe",
            "agentmux-0.60.27.exe",
            "agentmux-0.60.27.dll",
            "agentmux-srv-0.60.27-windows.x64.exe",
            "agentmux-0.60.28.exe",
            "agentmux-0.60.28.dll",
            "agentmux-srv-0.60.28-windows.x64.exe",
            "agentmux-0.60.29.exe",
            "agentmux-portable.marker",
            "libcef.dll",
            "chrome_elf.dll",
        ]);
        assert_eq!(
            names(stale_files(&dir, "0.60.28", 1)),
            ["agentmux-0.60.26.dll", "agentmux-0.60.26.exe", "agentmux-srv-0.60.26-windows.x64.exe"]
        );
        // Nothing older than the running version: nothing to do.
        assert!(stale_files(&dir, "0.60.26", 1).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_version_name_that_is_not_a_version_removes_nothing() {
        let dir = runtime_with(&["agentmux-0.60.1.exe"]);
        assert!(stale_files(&dir, "dev", 1).is_empty());
        assert!(stale_files(&dir, "0.60.28-rc1", 1).is_empty());
        assert!(stale_files(&dir.join("missing"), "0.60.28", 1).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_removes_the_stale_files_and_counts_them() {
        let dir = runtime_with(&["agentmux-0.60.20.exe", "agentmux-0.60.20.dll", "agentmux-0.60.27.exe", "agentmux-0.60.28.exe", "libcef.dll"]);
        let done = prune(&dir, "0.60.28", 1);
        assert_eq!(done, Pruned { removed: 2, bytes: 2, skipped: 0 });
        let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["agentmux-0.60.27.exe", "agentmux-0.60.28.exe", "libcef.dll"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An older srv that agents are still on keeps running after the upgrade: its file is skipped, not removed.
    #[cfg(windows)]
    #[test]
    fn a_file_still_running_is_skipped() {
        let dir = runtime_with(&["agentmux-0.60.28.exe", "agentmux-0.60.20.dll"]);
        let running_copy = dir.join("agentmux-srv-0.60.20-windows.x64.exe");
        std::fs::copy(Path::new(&std::env::var("SystemRoot").unwrap()).join("System32").join("PING.EXE"), &running_copy).unwrap();
        let mut child = std::process::Command::new(&running_copy)
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let done = prune(&dir, "0.60.28", 0);
        let _ = child.kill();
        let _ = child.wait();
        assert!(running_copy.exists(), "a file in use must stay");
        assert!(!dir.join("agentmux-0.60.20.dll").exists());
        assert_eq!((done.removed, done.skipped), (1, 1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeping_none_removes_every_older_version() {
        let dir = runtime_with(&["agentmux-0.60.27.exe", "agentmux-0.60.28.exe"]);
        assert_eq!(names(stale_files(&dir, "0.60.28", 0)), ["agentmux-0.60.27.exe"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
