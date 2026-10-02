// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which installed provider CLIs can go, decided by *disuse* and never by
//! version number (docs/specs/SPEC_RUNTIME_MENU_REMAINING_GAPS_2026_10_01.md §5).
//!
//! `shared/cli/<provider>/<pin>/` is shared by every AgentMux version and
//! channel on the machine, and they can run at the same time: an older build
//! pins an older CLI, so "everything but the newest" would pull a CLI out from
//! under a live agent. A directory is removable only when ALL hold:
//!
//! - it is not the current pin of any provider in this build's registry;
//! - nothing has used it for `max_idle` (see [`record_use`]);
//! - no live controller's command and no running process's command line
//!   mentions it (a scan that FAILS means "in use": see [`InUse`]);
//! - the install lock can be taken without waiting.
//!
//! Legacy per-version folders (`instances/v<version>/cli/<provider>`) go by the
//! same rules, except that their age is their install time and the running
//! version's own folder is never touched.
//!
//! Nothing here runs by itself: [`plan_prune`] only reads, and [`prune`] is
//! called by whoever decides it is time (§5.8 is an open owner decision on
//! automatic vs opt-in).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use agentmux_common::DataPaths;

use super::cli_install::{
    is_safe_provider_component, is_safe_version_component, try_lock_install, COMPLETE_MARKER,
};

/// Touched whenever an install is handed out; only its mtime matters.
pub const LAST_USED_MARKER: &str = ".agentmux-last-used";

/// How often [`record_use`] actually writes: the marker is on the path that
/// resolves a CLI for every spawn, so it must stay cheap.
pub const RECORD_USE_EVERY: Duration = Duration::from_secs(60 * 60);

/// The default line (spec §5.4): 30 days without use.
pub const DEFAULT_MAX_IDLE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// Note that `dir` was just used. Best effort, throttled to [`RECORD_USE_EVERY`]
/// by the marker's own mtime; an error is ignored (a failure to record can only
/// make the directory look older, and the other guards still apply).
pub fn record_use(dir: &Path, now: SystemTime) {
    let marker = dir.join(LAST_USED_MARKER);
    if let Ok(modified) = std::fs::metadata(&marker).and_then(|m| m.modified()) {
        if now
            .duration_since(modified)
            .map_or(true, |d| d < RECORD_USE_EVERY)
        {
            return;
        }
    }
    let _ = std::fs::write(&marker, b"");
}

/// What is running right now, as text to search for install paths in.
#[derive(Debug, Clone, Default)]
pub struct InUse {
    /// The command (program and arguments) each live controller was spawned
    /// with: the primary guard, and platform independent.
    pub live_commands: Vec<String>,
    /// The command line of every running process, or `None` when it could not
    /// be read. `None` makes [`plan_prune`] keep EVERYTHING: a guard that
    /// cannot see must not approve.
    pub process_commands: Option<Vec<String>>,
}

impl InUse {
    /// Whether any known command mentions something under `dir`.
    fn mentions(&self, dir: &Path) -> bool {
        // Trailing separator, so `…/claude/2.1.28` is not "in use" because of
        // `…/claude/2.1.285/…`.
        let needle = format!("{}{}", dir.display(), std::path::MAIN_SEPARATOR);
        self.live_commands
            .iter()
            .chain(self.process_commands.iter().flatten())
            .any(|c| c.contains(&needle))
    }
}

/// The running processes' command lines. Unix: `ps`; Windows is not scanned
/// (`None`, so nothing is pruned there until it is).
pub fn scan_process_commands() -> Option<Vec<String>> {
    if cfg!(windows) {
        return None;
    }
    let mut cmd = std::process::Command::new("ps");
    cmd.args(["-axww", "-o", "args="]);
    crate::backend::pane_env::sanitize_external_std_command(&mut cmd);
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    // Even a nearly idle machine has dozens of processes: an empty listing
    // means the scan did not work.
    (!lines.is_empty()).then_some(lines)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Keep {
    CurrentPin,
    RecentlyUsed,
    InUse,
    /// The process scan failed, so nothing can be shown to be unused.
    ScanFailed,
    RunningVersion,
    /// A symlink, or a name that is not a safe path component: never touched.
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub dir: PathBuf,
    pub provider: String,
    /// The pin for a shared install; the AgentMux version for a legacy one.
    pub version: String,
    pub legacy: bool,
    pub bytes: u64,
    pub idle: Duration,
}

#[derive(Debug, Default)]
pub struct Plan {
    pub prunable: Vec<Candidate>,
    pub kept: Vec<(PathBuf, Keep)>,
}

impl Plan {
    pub fn reclaimable_bytes(&self) -> u64 {
        self.prunable.iter().map(|c| c.bytes).sum()
    }
}

/// Total size of the files under `dir`, never following a symlink.
fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in rd.flatten() {
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                total += meta.len();
            }
        }
    }
    total
}

/// The latest sign of life: the newest of the last-used marker and the install
/// marker; the directory's own mtime when neither exists.
fn last_activity(dir: &Path) -> Option<SystemTime> {
    [LAST_USED_MARKER, COMPLETE_MARKER]
        .iter()
        .filter_map(|m| {
            std::fs::metadata(dir.join(m))
                .and_then(|m| m.modified())
                .ok()
        })
        .max()
        .or_else(|| std::fs::metadata(dir).and_then(|m| m.modified()).ok())
}

/// Immediate subdirectories of `dir` as `(name, path)`, skipping symlinks and
/// unsafe names (reported through `kept`).
fn safe_children(
    dir: &Path,
    is_safe: fn(&str) -> bool,
    kept: &mut Vec<(PathBuf, Keep)>,
) -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() && is_safe(&name) {
            out.push((name, path));
        } else if meta.is_dir() || meta.file_type().is_symlink() {
            kept.push((path, Keep::Skipped));
        }
        // Plain files (lock files, `<pin>.lock`) are not installs.
    }
    out.sort();
    out
}

/// Decide what could be removed. Reads only; nothing is touched.
///
/// `current_pins`: `(provider, pin)` for every provider in this build's registry.
/// `running_version`: this AgentMux's version, whose legacy folder is kept.
pub fn plan_prune(
    paths: &DataPaths,
    now: SystemTime,
    max_idle: Duration,
    running_version: &str,
    current_pins: &[(String, String)],
    in_use: &InUse,
) -> Plan {
    let mut plan = Plan::default();
    let idle_of = |dir: &Path| {
        last_activity(dir)
            .and_then(|t| now.duration_since(t).ok())
            .unwrap_or(Duration::ZERO) // unreadable or in the future: treat as just used
    };
    let decide = |plan: &mut Plan, c: Candidate, current: bool, running: bool| {
        let keep = if current {
            Some(Keep::CurrentPin)
        } else if running {
            Some(Keep::RunningVersion)
        } else if in_use.process_commands.is_none() {
            Some(Keep::ScanFailed)
        } else if in_use.mentions(&c.dir) {
            Some(Keep::InUse)
        } else if c.idle < max_idle {
            Some(Keep::RecentlyUsed)
        } else {
            None
        };
        match keep {
            Some(k) => plan.kept.push((c.dir, k)),
            None => plan.prunable.push(Candidate {
                bytes: dir_size(&c.dir),
                ..c
            }),
        }
    };

    // shared/cli/<provider>/<pin>
    let cli_root = paths.shared_dir.join("cli");
    let mut skipped = Vec::new();
    for (provider, pdir) in safe_children(&cli_root, is_safe_provider_component, &mut skipped) {
        for (pin, dir) in safe_children(&pdir, is_safe_version_component, &mut skipped) {
            let current = current_pins
                .iter()
                .any(|(p, v)| *p == provider && *v == pin);
            let idle = idle_of(&dir);
            let c = Candidate {
                dir,
                provider: provider.clone(),
                version: pin,
                legacy: false,
                bytes: 0,
                idle,
            };
            decide(&mut plan, c, current, false);
        }
    }

    // instances/v<version>/cli/<provider>
    let instances = paths.home_dir.join("instances");
    for (vname, vdir) in safe_children(&instances, is_safe_version_component, &mut skipped) {
        let Some(version) = vname.strip_prefix('v') else {
            continue;
        };
        for (provider, dir) in
            safe_children(&vdir.join("cli"), is_safe_provider_component, &mut skipped)
        {
            let idle = idle_of(&dir);
            let c = Candidate {
                dir,
                provider,
                version: version.to_owned(),
                legacy: true,
                bytes: 0,
                idle,
            };
            decide(&mut plan, c, false, version == running_version);
        }
    }
    plan.kept.extend(skipped);
    plan
}

#[derive(Debug, Default)]
pub struct Report {
    pub removed: Vec<(PathBuf, u64)>,
    /// Candidates left alone at removal time, and why.
    pub skipped: Vec<(PathBuf, String)>,
}

/// Remove what [`plan_prune`] found, re-checking each directory at the last
/// moment: `in_use` is asked again (a CLI may have started since the plan),
/// and a shared install's lock must be free. The completion marker goes FIRST,
/// so a directory half removed by a crash is already "not an install" and the
/// next install into it clears the rest.
pub fn prune(paths: &DataPaths, plan: &Plan, in_use: &dyn Fn() -> InUse) -> Report {
    let mut report = Report::default();
    for c in &plan.prunable {
        let guard = if c.legacy {
            None
        } else {
            match try_lock_install(paths, &c.provider, &c.version) {
                Ok(Some(g)) => Some(g),
                Ok(None) => {
                    report
                        .skipped
                        .push((c.dir.clone(), "install lock is held".into()));
                    continue;
                }
                Err(e) => {
                    report.skipped.push((
                        c.dir.clone(),
                        format!("could not take the install lock: {e}"),
                    ));
                    continue;
                }
            }
        };
        let now = in_use();
        if now.process_commands.is_none() || now.mentions(&c.dir) {
            report
                .skipped
                .push((c.dir.clone(), "in use, or the process scan failed".into()));
            continue;
        }
        if std::fs::symlink_metadata(&c.dir)
            .map_or(true, |m| m.file_type().is_symlink() || !m.is_dir())
        {
            report
                .skipped
                .push((c.dir.clone(), "no longer a plain directory".into()));
            continue;
        }
        let _ = std::fs::remove_file(c.dir.join(COMPLETE_MARKER));
        match std::fs::remove_dir_all(&c.dir) {
            Ok(()) => report.removed.push((c.dir.clone(), c.bytes)),
            Err(e) => report
                .skipped
                .push((c.dir.clone(), format!("remove failed: {e}"))),
        }
        drop(guard);
    }
    report
}

/// `(provider, pin)` of every provider in this build's registry that has a pin.
pub fn current_pins() -> Vec<(String, String)> {
    crate::backend::providers::all_providers()
        .filter(|p| !p.pinned_version.is_empty())
        .map(|p| (p.id.to_string(), p.pinned_version.to_string()))
        .collect()
}

/// What running things mention, for [`plan_prune`] / [`prune`]: the commands of
/// every block that records one (`live_commands`: open panes AND panes restored
/// from a layout, which hold the absolute CLI path until they mount) and every
/// running process's command line.
pub fn gather_in_use(live_commands: &[String]) -> InUse {
    InUse { live_commands: live_commands.to_vec(), process_commands: scan_process_commands() }
}

/// The outcome of one [`run`].
#[derive(Debug)]
pub struct Run {
    pub plan: Plan,
    /// `None` for a dry run.
    pub report: Option<Report>,
    /// The process scan worked. When it did not, nothing is prunable.
    pub scan_ok: bool,
}

/// Plan, and unless `dry_run`, carry out, a prune. `dry_run` is the caller's
/// explicit choice to delete: nothing in AgentMux calls this with `false`
/// except a user action.
///
/// `only`: remove just these directories (the ones an earlier dry run listed),
/// and only if they are still removable now. Anything that became removable
/// since is left alone, so a removal is exactly what the user was shown.
pub fn run(
    paths: &DataPaths,
    live_commands: &[String],
    dry_run: bool,
    only: Option<&[String]>,
    max_idle: Duration,
    now: SystemTime,
) -> Run {
    let in_use = gather_in_use(live_commands);
    let scan_ok = in_use.process_commands.is_some();
    let mut plan = plan_prune(paths, now, max_idle, env!("CARGO_PKG_VERSION"), &current_pins(), &in_use);
    if let Some(only) = only {
        plan.prunable.retain(|c| only.iter().any(|d| Path::new(d) == c.dir));
    }
    let report = (!dry_run).then(|| prune(paths, &plan, &|| gather_in_use(live_commands)));
    Run { plan, report, scan_ok }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};

    const DAY: Duration = Duration::from_secs(24 * 60 * 60);

    fn paths_in(root: &Path) -> DataPaths {
        let version_dir = root
            .join("channels")
            .join("c")
            .join("versions")
            .join("1.0.0");
        DataPaths {
            home_dir: root.to_path_buf(),
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

    fn set_age(path: &Path, now: SystemTime, age: Duration) {
        let f = File::options().write(true).open(path).unwrap();
        f.set_modified(now - age).unwrap();
    }

    /// A finished shared install whose completion marker is `age` old.
    fn shared_install(
        p: &DataPaths,
        provider: &str,
        pin: &str,
        now: SystemTime,
        age: Duration,
    ) -> PathBuf {
        let dir = p.shared_dir.join("cli").join(provider).join(pin);
        fs::create_dir_all(dir.join("node_modules").join(".bin")).unwrap();
        fs::write(
            dir.join("node_modules").join(".bin").join(provider),
            vec![0u8; 100],
        )
        .unwrap();
        fs::write(dir.join(COMPLETE_MARKER), pin).unwrap();
        set_age(&dir.join(COMPLETE_MARKER), now, age);
        dir
    }

    fn legacy_install(
        p: &DataPaths,
        version: &str,
        provider: &str,
        now: SystemTime,
        age: Duration,
    ) -> PathBuf {
        let dir = p
            .home_dir
            .join("instances")
            .join(format!("v{version}"))
            .join("cli")
            .join(provider);
        fs::create_dir_all(dir.join("node_modules")).unwrap();
        fs::write(dir.join("node_modules").join("x"), vec![0u8; 50]).unwrap();
        // Aged through a file inside it: setting a directory's own mtime needs
        // platform-specific handles (opening a directory fails on Windows).
        fs::write(dir.join(COMPLETE_MARKER), "").unwrap();
        set_age(&dir.join(COMPLETE_MARKER), now, age);
        dir
    }

    fn scanned() -> InUse {
        InUse {
            live_commands: vec![],
            process_commands: Some(vec!["/usr/bin/vim".into()]),
        }
    }
    fn pins() -> Vec<(String, String)> {
        vec![("claude".into(), "2.1.285".into())]
    }
    fn plan(p: &DataPaths, now: SystemTime, in_use: &InUse) -> Plan {
        plan_prune(p, now, DEFAULT_MAX_IDLE, "0.59.1", &pins(), in_use)
    }
    fn dirs(c: &[Candidate]) -> Vec<PathBuf> {
        c.iter().map(|c| c.dir.clone()).collect()
    }

    #[test]
    fn the_current_pin_is_never_prunable_however_old() {
        let t = tempfile::tempdir().unwrap();
        let p = paths_in(t.path());
        let now = SystemTime::now();
        let cur = shared_install(&p, "claude", "2.1.285", now, 400 * DAY);
        let pl = plan(&p, now, &scanned());
        assert!(pl.prunable.is_empty());
        assert!(pl.kept.contains(&(cur, Keep::CurrentPin)));
    }

    #[test]
    fn an_old_unused_pin_is_prunable_and_a_recently_used_one_is_not() {
        let t = tempfile::tempdir().unwrap();
        let p = paths_in(t.path());
        let now = SystemTime::now();
        let old = shared_install(&p, "claude", "2.1.200", now, 90 * DAY);
        let used = shared_install(&p, "claude", "2.1.210", now, 90 * DAY);
        // Installed long ago, but used yesterday (another channel still runs it).
        fs::write(used.join(LAST_USED_MARKER), "").unwrap();
        set_age(&used.join(LAST_USED_MARKER), now, DAY);
        let recent = shared_install(&p, "claude", "2.1.220", now, 5 * DAY);

        let pl = plan(&p, now, &scanned());
        assert_eq!(dirs(&pl.prunable), vec![old]);
        assert!(pl.kept.contains(&(used, Keep::RecentlyUsed)));
        assert!(pl.kept.contains(&(recent, Keep::RecentlyUsed)));
        assert!(pl.reclaimable_bytes() > 0);
    }

    #[test]
    fn planning_changes_nothing() {
        let t = tempfile::tempdir().unwrap();
        let p = paths_in(t.path());
        let now = SystemTime::now();
        let old = shared_install(&p, "claude", "2.1.200", now, 90 * DAY);
        let _ = plan(&p, now, &scanned());
        assert!(old.join(COMPLETE_MARKER).is_file());
        assert!(old
            .join("node_modules")
            .join(".bin")
            .join("claude")
            .is_file());
    }

    #[test]
    fn a_live_controller_or_a_running_process_under_the_dir_keeps_it() {
        let t = tempfile::tempdir().unwrap();
        let p = paths_in(t.path());
        let now = SystemTime::now();
        let a = shared_install(&p, "claude", "2.1.200", now, 90 * DAY);
        let b = shared_install(&p, "claude", "2.1.201", now, 90 * DAY);
        let c = shared_install(&p, "claude", "2.1.202", now, 90 * DAY);
        let sep = std::path::MAIN_SEPARATOR;
        let in_use = InUse {
            live_commands: vec![format!(
                "{}{sep}node_modules{sep}.bin{sep}claude -p",
                a.display()
            )],
            process_commands: Some(vec![format!(
                "node {}{sep}node_modules{sep}x.js",
                b.display()
            )]),
        };
        let pl = plan(&p, now, &in_use);
        assert_eq!(dirs(&pl.prunable), vec![c]);
        assert!(pl.kept.contains(&(a, Keep::InUse)));
        assert!(pl.kept.contains(&(b, Keep::InUse)));
    }

    #[test]
    fn a_version_that_is_a_prefix_of_a_running_one_is_not_considered_in_use() {
        let t = tempfile::tempdir().unwrap();
        let p = paths_in(t.path());
        let now = SystemTime::now();
        let short = shared_install(&p, "claude", "2.1.28", now, 90 * DAY);
        let sep = std::path::MAIN_SEPARATOR;
        let longer = p.shared_dir.join("cli").join("claude").join("2.1.285");
        let in_use = InUse {
            live_commands: vec![format!(
                "{}{sep}node_modules{sep}.bin{sep}claude",
                longer.display()
            )],
            process_commands: Some(vec!["x".into()]),
        };
        assert_eq!(dirs(&plan(&p, now, &in_use).prunable), vec![short]);
    }

    #[test]
    fn a_failed_process_scan_keeps_everything() {
        let t = tempfile::tempdir().unwrap();
        let p = paths_in(t.path());
        let now = SystemTime::now();
        let old = shared_install(&p, "claude", "2.1.200", now, 90 * DAY);
        let legacy = legacy_install(&p, "0.50.0", "claude", now, 90 * DAY);
        let pl = plan(
            &p,
            now,
            &InUse {
                live_commands: vec![],
                process_commands: None,
            },
        );
        assert!(pl.prunable.is_empty());
        assert!(pl.kept.contains(&(old, Keep::ScanFailed)));
        assert!(pl.kept.contains(&(legacy, Keep::ScanFailed)));
    }

    #[test]
    fn legacy_folders_go_only_when_old_and_not_the_running_version() {
        let t = tempfile::tempdir().unwrap();
        let p = paths_in(t.path());
        let now = SystemTime::now();
        let old = legacy_install(&p, "0.50.0", "claude", now, 90 * DAY);
        let young = legacy_install(&p, "0.58.0", "claude", now, 3 * DAY);
        let running = legacy_install(&p, "0.59.1", "claude", now, 90 * DAY);
        let pl = plan(&p, now, &scanned());
        assert_eq!(dirs(&pl.prunable), vec![old]);
        assert!(pl.prunable[0].legacy);
        assert!(pl.kept.contains(&(young, Keep::RecentlyUsed)));
        assert!(pl.kept.contains(&(running, Keep::RunningVersion)));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_and_unsafe_names_are_never_touched() {
        let t = tempfile::tempdir().unwrap();
        let p = paths_in(t.path());
        let now = SystemTime::now();
        let outside = t.path().join("precious");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep.txt"), "x").unwrap();
        let provider_dir = p.shared_dir.join("cli").join("claude");
        fs::create_dir_all(&provider_dir).unwrap();
        std::os::unix::fs::symlink(&outside, provider_dir.join("2.1.100")).unwrap();
        fs::create_dir_all(provider_dir.join("not a version")).unwrap();
        let pl = plan(&p, now, &scanned());
        assert!(pl.prunable.is_empty());
        assert!(pl
            .kept
            .iter()
            .any(|(d, k)| d.ends_with("2.1.100") && *k == Keep::Skipped));
        assert!(pl
            .kept
            .iter()
            .any(|(d, k)| d.ends_with("not a version") && *k == Keep::Skipped));
        assert!(outside.join("keep.txt").is_file());
    }

    #[test]
    fn prune_removes_the_marker_first_and_leaves_the_lock_file() {
        let t = tempfile::tempdir().unwrap();
        let p = paths_in(t.path());
        let now = SystemTime::now();
        let old = shared_install(&p, "claude", "2.1.200", now, 90 * DAY);
        let keep = shared_install(&p, "claude", "2.1.285", now, 90 * DAY);
        let pl = plan(&p, now, &scanned());
        let r = prune(&p, &pl, &scanned);
        assert_eq!(r.removed.len(), 1);
        assert!(r.removed[0].1 > 0);
        assert!(!old.exists());
        assert!(keep.join(COMPLETE_MARKER).is_file());
    }

    #[test]
    fn a_held_install_lock_blocks_removal() {
        let t = tempfile::tempdir().unwrap();
        let p = paths_in(t.path());
        let now = SystemTime::now();
        let old = shared_install(&p, "claude", "2.1.200", now, 90 * DAY);
        let pl = plan(&p, now, &scanned());
        let _held = try_lock_install(&p, "claude", "2.1.200").unwrap().unwrap();
        let r = prune(&p, &pl, &scanned);
        assert!(r.removed.is_empty());
        assert!(r.skipped[0].1.contains("lock"));
        assert!(old.join(COMPLETE_MARKER).is_file());
    }

    #[test]
    fn prune_asks_again_at_removal_time() {
        let t = tempfile::tempdir().unwrap();
        let p = paths_in(t.path());
        let now = SystemTime::now();
        let old = shared_install(&p, "claude", "2.1.200", now, 90 * DAY);
        let pl = plan(&p, now, &scanned());
        // A CLI started between the plan and the removal:
        let sep = std::path::MAIN_SEPARATOR;
        let started = || InUse {
            live_commands: vec![format!(
                "{}{sep}node_modules{sep}.bin{sep}claude",
                old.display()
            )],
            process_commands: Some(vec!["x".into()]),
        };
        assert!(prune(&p, &pl, &started).removed.is_empty());
        // ...or the scan broke:
        let broken = || InUse {
            live_commands: vec![],
            process_commands: None,
        };
        assert!(prune(&p, &pl, &broken).removed.is_empty());
        assert!(old.join(COMPLETE_MARKER).is_file());
    }

    #[test]
    fn record_use_writes_once_per_hour() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("d");
        fs::create_dir_all(&dir).unwrap();
        let now = SystemTime::now();
        record_use(&dir, now);
        let first = fs::metadata(dir.join(LAST_USED_MARKER))
            .unwrap()
            .modified()
            .unwrap();
        // 10 minutes later: throttled, mtime unchanged (pretend the file is old to prove it).
        set_age(&dir.join(LAST_USED_MARKER), now, Duration::from_secs(600));
        record_use(&dir, now);
        let still = fs::metadata(dir.join(LAST_USED_MARKER))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(still, now - Duration::from_secs(600));
        // 2 hours later: written.
        set_age(
            &dir.join(LAST_USED_MARKER),
            now,
            Duration::from_secs(2 * 3600),
        );
        record_use(&dir, now);
        let after = fs::metadata(dir.join(LAST_USED_MARKER))
            .unwrap()
            .modified()
            .unwrap();
        assert!(after > now - Duration::from_secs(3600));
        let _ = first;
    }

    #[cfg(unix)]
    #[test]
    fn run_is_a_dry_run_unless_told_otherwise_and_removes_only_on_an_explicit_request() {
        let t = tempfile::tempdir().unwrap();
        let p = paths_in(t.path());
        let now = SystemTime::now();
        let old = shared_install(&p, "claude", "0.0.1", now, 90 * DAY);
        let recent = shared_install(&p, "claude", "0.0.2", now, DAY);

        let dry = run(&p, &[], true, None, DEFAULT_MAX_IDLE, now);
        assert!(dry.scan_ok, "ps works here");
        assert_eq!(dirs(&dry.plan.prunable), vec![old.clone()]);
        assert!(dry.report.is_none());
        assert!(old.join(COMPLETE_MARKER).is_file(), "a dry run removes nothing");

        // a pane restored from a layout names the old install: kept
        let sep = std::path::MAIN_SEPARATOR;
        let named = format!("{}{sep}node_modules{sep}.bin{sep}claude -p", old.display());
        let kept = run(&p, &[named], false, None, DEFAULT_MAX_IDLE, now);
        assert!(kept.plan.prunable.is_empty());
        assert!(old.join(COMPLETE_MARKER).is_file());

        let real = run(&p, &[], false, None, DEFAULT_MAX_IDLE, now);
        let report = real.report.expect("a real run reports");
        assert_eq!(report.removed.len(), 1);
        assert!(!old.exists());
        assert!(recent.join(COMPLETE_MARKER).is_file(), "recently used stays");
    }

    #[cfg(unix)]
    #[test]
    fn a_removal_is_exactly_the_list_the_user_was_shown() {
        let t = tempfile::tempdir().unwrap();
        let p = paths_in(t.path());
        let now = SystemTime::now();
        let shown = shared_install(&p, "claude", "0.0.1", now, 90 * DAY);
        // became removable after the user's Check (e.g. a pane closed meanwhile)
        let later = shared_install(&p, "claude", "0.0.2", now, 95 * DAY);

        let only = vec![shown.to_string_lossy().into_owned()];
        let r = run(&p, &[], false, Some(&only), DEFAULT_MAX_IDLE, now);
        assert_eq!(r.report.unwrap().removed.len(), 1);
        assert!(!shown.exists());
        assert!(later.join(COMPLETE_MARKER).is_file(), "not shown, so not touched");

        // a listed dir that is no longer removable (now in use) is not removed either
        let sep = std::path::MAIN_SEPARATOR;
        let named = format!("{}{sep}node_modules{sep}.bin{sep}claude", later.display());
        let only = vec![later.to_string_lossy().into_owned()];
        let r = run(&p, &[named], false, Some(&only), DEFAULT_MAX_IDLE, now);
        assert!(r.report.unwrap().removed.is_empty());
        assert!(later.join(COMPLETE_MARKER).is_file());
    }

    #[test]
    fn every_registry_pin_is_a_current_pin() {
        let pins = current_pins();
        assert!(pins.iter().any(|(p, v)| p == "claude" && !v.is_empty()));
        assert!(pins.iter().all(|(p, v)| !p.is_empty() && !v.is_empty()));
    }

    #[cfg(unix)]
    #[test]
    fn the_process_scan_sees_this_process() {
        let seen = scan_process_commands().expect("ps works here");
        assert!(!seen.is_empty());
    }
}
