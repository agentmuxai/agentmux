// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Clearing agents' cargo build caches and warning when a disk runs low
//! (`docs/specs/SPEC_BUILD_CACHE_CLEANUP_2026_10_26.md`).
//!
//! Three hosts nearly filled their disks in two days from agents' `target` folders. On one of
//! them 67% of 1 TB of targets was `target/<profile>/incremental`, and 90% of that was the
//! `agentmux_srv` crate: every change to a dependency gives the crate a new incremental
//! directory (about 10 GB) and the old one is never reused or collected.
//!
//! Three things are done here, none of which can break a build (the worst case of a wrong guess is
//! a slower incremental rebuild):
//!
//! - **Orphans**: in each `incremental` folder, the newest few directories per crate are kept and
//!   older ones idle for a day are removed.
//! - **A finished task**: the agent's own clones lose all their `incremental` folders.
//! - **Abandoned clones**: a clone nobody built in for days loses its whole `target` (when a task
//!   finishes, or the disk is low).
//!
//! A clone with a cargo, rustc, linker or test process running in it is never touched. Every
//! removal is logged, naming the workspace. This file is the file-system and policy part;
//! `build_cache_service.rs` runs it on a schedule and sends the warnings.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Settings keys (in the settings file's free-form part).
pub const KEY_ENABLED: &str = "buildcache:enabled";
pub const KEY_ORPHANS: &str = "buildcache:orphans";
pub const KEY_ORPHAN_KEEP: &str = "buildcache:orphan_keep";
pub const KEY_ORPHAN_IDLE_HOURS: &str = "buildcache:orphan_idle_hours";
pub const KEY_STALE_DAYS: &str = "buildcache:stale_days";
pub const KEY_WARN_FREE_GB: &str = "buildcache:warn_free_gb";
pub const KEY_WARN_FREE_PCT: &str = "buildcache:warn_free_pct";

/// What is cleared, and when the disk warning fires.
#[derive(Debug, Clone, PartialEq)]
pub struct Policy {
    /// Everything below, except the warning, is off when this is false.
    pub enabled: bool,
    /// Prune orphan incremental directories (the hourly pass and the low-disk pass).
    pub orphans: bool,
    /// How many incremental directories per crate and profile are always kept.
    pub orphan_keep: usize,
    /// A directory beyond the newest `orphan_keep` goes only when idle this long.
    pub orphan_idle: Duration,
    /// A clone not built in this long loses its `target`; zero turns that off.
    pub stale: Duration,
    /// Warn when the free space is below this many bytes...
    pub warn_free_bytes: u64,
    /// ...or below this percentage of the disk.
    pub warn_free_pct: f64,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            enabled: true,
            orphans: true,
            orphan_keep: 3,
            orphan_idle: Duration::from_secs(24 * 3600),
            stale: Duration::from_secs(3 * 24 * 3600),
            warn_free_bytes: 20 * (1 << 30),
            warn_free_pct: 10.0,
        }
    }
}

impl Policy {
    /// The policy the settings ask for; a missing or mistyped key keeps its default.
    pub fn from_extra(extra: &HashMap<String, serde_json::Value>) -> Self {
        let d = Self::default();
        let num = |key: &str| extra.get(key).and_then(|v| v.as_f64()).filter(|n| n.is_finite() && *n >= 0.0);
        Self {
            enabled: extra.get(KEY_ENABLED).and_then(|v| v.as_bool()).unwrap_or(d.enabled),
            orphans: extra.get(KEY_ORPHANS).and_then(|v| v.as_bool()).unwrap_or(d.orphans),
            // Never fewer than one: the newest directory is the live one.
            orphan_keep: num(KEY_ORPHAN_KEEP).map_or(d.orphan_keep, |n| (n as usize).max(1)),
            orphan_idle: num(KEY_ORPHAN_IDLE_HOURS).map_or(d.orphan_idle, |h| Duration::from_secs_f64(h * 3600.0)),
            stale: num(KEY_STALE_DAYS).map_or(d.stale, |days| Duration::from_secs_f64(days * 86_400.0)),
            warn_free_bytes: num(KEY_WARN_FREE_GB).map_or(d.warn_free_bytes, |g| (g * (1u64 << 30) as f64) as u64),
            warn_free_pct: num(KEY_WARN_FREE_PCT).unwrap_or(d.warn_free_pct),
        }
    }

    /// Whether `free` of `total` bytes is low enough to warn about.
    pub fn is_low(&self, free: u64, total: u64) -> bool {
        total > 0 && (free < self.warn_free_bytes || (free as f64) * 100.0 < self.warn_free_pct * total as f64)
    }
}

/// Every cargo clone in an agent's workspace: the workspace itself or a folder directly in it
/// that has a `Cargo.toml` and a `target` folder.
pub fn clones_in(workspace: &Path) -> Vec<PathBuf> {
    let is_clone = |p: &Path| p.join("Cargo.toml").is_file() && p.join("target").is_dir();
    let mut out = Vec::new();
    if is_clone(workspace) {
        out.push(workspace.to_path_buf());
    }
    if let Ok(entries) = std::fs::read_dir(workspace) {
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || name == "node_modules" || name == "target" {
                continue;
            }
            if e.file_type().is_ok_and(|t| t.is_dir()) && is_clone(&p) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// The `incremental` folders of a `target`: `target/<profile>/incremental` and, for a cross
/// build, `target/<triple>/<profile>/incremental`.
pub fn incremental_roots(target: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let subdirs = |p: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(p)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .collect()
    };
    for a in subdirs(target) {
        if a.join("incremental").is_dir() {
            out.push(a.join("incremental"));
        }
        for b in subdirs(&a) {
            if b.join("incremental").is_dir() {
                out.push(b.join("incremental"));
            }
        }
    }
    out.sort();
    out
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// The newest change in `dir`, looking two levels down: a crate's incremental folder holds
/// session folders, which hold the files rustc rewrites on every build.
fn activity(dir: &Path) -> SystemTime {
    let mut newest = mtime(dir).unwrap_or(SystemTime::UNIX_EPOCH);
    let mut bump = |t: Option<SystemTime>| {
        if let Some(t) = t {
            newest = newest.max(t);
        }
    };
    for sess in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        bump(sess.metadata().and_then(|m| m.modified()).ok());
        if sess.file_type().is_ok_and(|t| t.is_dir()) {
            for f in std::fs::read_dir(sess.path()).into_iter().flatten().flatten() {
                bump(f.metadata().and_then(|m| m.modified()).ok());
            }
        }
    }
    newest
}

/// The size of everything under `p`, in bytes.
pub fn dir_size(p: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(_) => total += e.metadata().map_or(0, |m| m.len()),
                Err(_) => {}
            }
        }
    }
    total
}

/// A crate's incremental directory is named `<crate>-<hash>`; the crate is what comes before
/// the last dash.
fn crate_of(name: &str) -> &str {
    name.rsplit_once('-').map_or(name, |(c, _)| c)
}

/// The incremental directories under `root` to remove: per crate, everything beyond the newest
/// `keep` that has been idle for `idle` as of `now`.
pub fn orphans_in(root: &Path, keep: usize, idle: Duration, now: SystemTime) -> Vec<PathBuf> {
    let mut by_crate: HashMap<String, Vec<(SystemTime, PathBuf)>> = HashMap::new();
    for e in std::fs::read_dir(root).into_iter().flatten().flatten() {
        if !e.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        by_crate.entry(crate_of(&name).to_string()).or_default().push((activity(&e.path()), e.path()));
    }
    let mut out = Vec::new();
    for (_, mut dirs) in by_crate {
        dirs.sort_by(|a, b| b.0.cmp(&a.0));
        out.extend(
            dirs.into_iter()
                .skip(keep.max(1))
                .filter(|(at, _)| now.duration_since(*at).unwrap_or_default() >= idle)
                .map(|(_, p)| p),
        );
    }
    out.sort();
    out
}

/// When something was last built in `target`: the newest change among the target, its profile
/// folders and the folders cargo writes to on every build.
pub fn last_build(target: &Path) -> SystemTime {
    let mut newest = mtime(target).unwrap_or(SystemTime::UNIX_EPOCH);
    for profile in std::fs::read_dir(target).into_iter().flatten().flatten() {
        if !profile.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let p = profile.path();
        for p in [p.clone(), p.join(".fingerprint"), p.join("deps"), p.join("incremental")] {
            if let Some(t) = mtime(&p) {
                newest = newest.max(t);
            }
        }
    }
    newest
}

/// Whether a build seems to be running in `clone`: a process whose working directory,
/// executable or command line is inside it and which is a compiler, linker, cargo or something
/// run from the `target` folder. Looks at the machine's processes now.
pub fn build_running_in(clone: &Path, procs: &sysinfo::System) -> bool {
    let needle = normal(clone);
    procs.processes().values().any(|p| {
        let name = p.name().to_string_lossy().to_ascii_lowercase();
        let stem = name.strip_suffix(".exe").unwrap_or(&name);
        let tool = matches!(stem, "cargo" | "rustc" | "link" | "lld-link" | "ld" | "cc" | "cl" | "clang" | "gcc" | "cmake" | "ninja" | "clippy-driver" | "rustdoc");
        let in_clone = p.cwd().is_some_and(|c| normal(c).starts_with(&needle))
            || p.exe().is_some_and(|e| normal(e).starts_with(&needle))
            || p.cmd().iter().any(|a| normal(Path::new(a)).contains(&needle));
        in_clone && (tool || p.exe().is_some_and(|e| normal(e).starts_with(&needle)))
    })
}

fn normal(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    // `canonicalize` on Windows gives a `\\?\C:\...` path.
    s.strip_prefix("//?/").map_or_else(|| s.clone(), str::to_string)
}

/// A snapshot of the machine's processes, for [`build_running_in`].
pub fn snapshot_processes() -> sysinfo::System {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        sysinfo::ProcessRefreshKind::nothing()
            .with_cwd(sysinfo::UpdateKind::Always)
            .with_exe(sysinfo::UpdateKind::Always)
            .with_cmd(sysinfo::UpdateKind::Always),
    );
    sys
}

/// What a pass is for; it goes in the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// An agent finished a task.
    TaskDone,
    /// The hourly pass.
    Hourly,
    /// The disk is low.
    LowDisk,
}

impl Why {
    fn as_str(self) -> &'static str {
        match self {
            Why::TaskDone => "task-done",
            Why::Hourly => "hourly",
            Why::LowDisk => "low-disk",
        }
    }
}

/// One removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    pub path: PathBuf,
    pub bytes: u64,
    pub what: &'static str,
}

/// What a pass over one workspace did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    pub removed: Vec<Removed>,
    /// Clones left alone because a build was running in them.
    pub busy: Vec<PathBuf>,
}

impl Report {
    pub fn bytes(&self) -> u64 {
        self.removed.iter().map(|r| r.bytes).sum()
    }
}

/// Where removals are logged (`AGENTMUX_BUILD_CACHE_LOG` overrides it, for tests).
pub fn log_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("AGENTMUX_BUILD_CACHE_LOG").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    Some(dirs::home_dir()?.join(".agentmux").join("logs").join("build-cache-cleanup.log"))
}

fn log_removal(log: Option<&Path>, why: Why, workspace: &Path, r: &Removed) {
    tracing::info!(why = why.as_str(), workspace = %workspace.display(), path = %r.path.display(), bytes = r.bytes, what = r.what, "build cache: removed");
    let Some(log) = log else { return };
    if let Some(dir) = log.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
        let _ = writeln!(f, "{now}\t{}\t{}\t{}\t{}\t{}", why.as_str(), workspace.display(), r.what, r.bytes, r.path.display());
    }
}

fn remove(log: Option<&Path>, why: Why, workspace: &Path, path: PathBuf, what: &'static str, report: &mut Report) {
    let bytes = dir_size(&path);
    match std::fs::remove_dir_all(&path) {
        Ok(()) => {
            let r = Removed { path, bytes, what };
            log_removal(log, why, workspace, &r);
            report.removed.push(r);
        }
        Err(e) => tracing::warn!(path = %path.display(), error = %e, "build cache: could not remove"),
    }
}

/// One pass over the clones of `workspace`.
///
/// `TaskDone` clears every `incremental` folder; the other two prune orphans (when the policy
/// allows). `TaskDone` and `LowDisk` also remove a clone's whole `target` when nothing was built
/// in it for the policy's `stale` time; the hourly pass leaves a quiet clone alone. A clone with
/// a build running in it is skipped.
pub fn run_pass(workspace: &Path, why: Why, policy: &Policy, is_busy: &dyn Fn(&Path) -> bool, now: SystemTime, log: Option<&Path>) -> Report {
    let mut report = Report::default();
    if !policy.enabled {
        return report;
    }
    for clone in clones_in(workspace) {
        if is_busy(&clone) {
            report.busy.push(clone);
            continue;
        }
        let target = clone.join("target");
        let stale_applies = matches!(why, Why::TaskDone | Why::LowDisk);
        if stale_applies && !policy.stale.is_zero() && now.duration_since(last_build(&target)).unwrap_or_default() >= policy.stale {
            remove(log, why, workspace, target, "stale target", &mut report);
            continue;
        }
        for root in incremental_roots(&target) {
            if why == Why::TaskDone {
                for e in std::fs::read_dir(&root).into_iter().flatten().flatten() {
                    if e.file_type().is_ok_and(|t| t.is_dir()) {
                        remove(log, why, workspace, e.path(), "incremental", &mut report);
                    }
                }
            } else if policy.orphans {
                for dir in orphans_in(&root, policy.orphan_keep, policy.orphan_idle, now) {
                    remove(log, why, workspace, dir, "orphan incremental", &mut report);
                }
            }
        }
    }
    report
}

/// The free and total bytes of the disk that holds `path`.
pub fn disk_of(path: &Path) -> Option<(u64, u64)> {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let want = normal(&path);
    let disks = sysinfo::Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        .filter(|d| want.starts_with(&normal(d.mount_point())))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| (d.available_space(), d.total_space()))
}

/// A clone's `target` size and how much of it is `incremental`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetSize {
    pub clone: PathBuf,
    pub target_bytes: u64,
    pub incremental_bytes: u64,
}

/// The size of every clone's `target` in `workspace`, largest first.
pub fn target_sizes(workspace: &Path) -> Vec<TargetSize> {
    let mut out: Vec<TargetSize> = clones_in(workspace)
        .into_iter()
        .map(|clone| {
            let target = clone.join("target");
            let incremental_bytes = incremental_roots(&target).iter().map(|r| dir_size(r)).sum();
            TargetSize { target_bytes: dir_size(&target), incremental_bytes, clone }
        })
        .collect();
    out.sort_by(|a, b| b.target_bytes.cmp(&a.target_bytes));
    out
}

/// "193 GB", "4.7 GB", "820 MB".
pub fn human(bytes: u64) -> String {
    const GB: f64 = (1u64 << 30) as f64;
    let b = bytes as f64;
    if b >= 100.0 * GB {
        format!("{:.0} GB", b / GB)
    } else if b >= GB {
        format!("{:.1} GB", b / GB)
    } else {
        format!("{:.0} MB", b / (1u64 << 20) as f64)
    }
}

#[cfg(test)]
#[path = "build_cache_tests.rs"]
mod tests;
