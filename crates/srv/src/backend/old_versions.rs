// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Clears the version folders an upgrade leaves behind, and the portables it unpacked.
//!
//! Every upgrade starts the new version on a copy of the old version's data
//! (`channels/<channel>/versions/<v>/`), and the updater unpacks a new portable beside the running
//! one. Nothing removes the old ones, so a machine that upgrades daily gains 0.5-1 GB of each a day.
//!
//! A folder of this srv's own channel goes only when ALL hold:
//!
//! - it is older than the version this srv runs (a newer one is another build's, or a rollback
//!   already taken: not ours to judge);
//! - it is not one of the newest `keep_previous` older versions (the way back, one by default);
//! - no running process names it: not in its command line, its working folder, its program path or
//!   its environment (every agent's shell carries `AGENTMUX_DATA_DIR`), and the pid its `srv.lock`
//!   records is not a live AgentMux process;
//! - nothing in it was written for `idle` (a day by default);
//! - the machine's processes could be read at all. A scan that fails keeps everything: a guard that
//!   cannot see must not approve.
//!
//! A removal renames the folder first, which Windows refuses while any file in it is open, and only
//! then deletes it, so a crash leaves a `*.removing-<pid>` folder the next pass finishes. Portable
//! folders (`agentmux-<v>-x64-portable`) are the user's: they are listed, and removed only when
//! `oldversions:portables` is on, and only those the updater unpacked for this channel (they record
//! it). Nothing here follows a symlink, and a name that is not a plain `x.y.z` is left alone.
//!
//! [`run`] with `dry_run` only reads. `old_versions_service` calls it daily and the
//! `/agentmux/old-versions` route lets an agent look at the plan, or carry out what it shows.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Serialize;

/// Settings keys (the free-form part of the settings file).
pub const KEY_CLEAN: &str = "oldversions:clean";
pub const KEY_KEEP_PREVIOUS: &str = "oldversions:keep_previous";
pub const KEY_IDLE_HOURS: &str = "oldversions:idle_hours";
pub const KEY_PORTABLES: &str = "oldversions:portables";

/// What goes, and whether the daily pass removes it or only reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct Policy {
    /// The daily pass removes what it finds. Off: it only logs the plan.
    pub clean: bool,
    /// How many of the newest older versions are kept as the way back; never fewer than one.
    pub keep_previous: usize,
    /// A folder written to within this long is in use.
    pub idle: Duration,
    /// Unpacked portable folders are removed too.
    pub portables: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Self { clean: false, keep_previous: 1, idle: Duration::from_secs(24 * 3600), portables: false }
    }
}

impl Policy {
    /// The policy the settings ask for; a missing or mistyped key keeps its default.
    pub fn from_extra(extra: &HashMap<String, serde_json::Value>) -> Self {
        let d = Self::default();
        let num = |key: &str| extra.get(key).and_then(|v| v.as_f64()).filter(|n| n.is_finite() && *n >= 0.0);
        Self {
            clean: extra.get(KEY_CLEAN).and_then(|v| v.as_bool()).unwrap_or(d.clean),
            keep_previous: num(KEY_KEEP_PREVIOUS).map_or(d.keep_previous, |n| (n as usize).max(1)),
            idle: num(KEY_IDLE_HOURS).and_then(|h| Duration::try_from_secs_f64(h * 3600.0).ok()).unwrap_or(d.idle),
            portables: extra.get(KEY_PORTABLES).and_then(|v| v.as_bool()).unwrap_or(d.portables),
        }
    }
}

/// Where the folders are, and which version this srv is.
#[derive(Debug, Clone)]
pub struct Layout {
    /// `channels/<channel>/versions`.
    pub versions_root: PathBuf,
    /// The folder name of the version this srv runs.
    pub running: String,
    /// This srv's data channel, which an unpacked portable records.
    pub channel: String,
    /// Folders holding unpacked portables: beside the running install, and the channel's `updates/<v>/`.
    pub portable_parents: Vec<PathBuf>,
}

/// The layout of this srv, or `None` when it has no per-version folders (a `task dev` build, a
/// test home).
pub fn layout_from_env() -> Option<Layout> {
    let paths = agentmux_common::DataPaths::from_env()?;
    let version_dir = paths.data_dir.parent()?;
    let versions_root = version_dir.parent()?;
    if versions_root.file_name()? != "versions" || versions_root.parent()? != paths.instance_dir {
        return None;
    }
    let mut portable_parents = Vec::new();
    match crate::backend::update_install::detect() {
        crate::backend::update_install::InstallKind::WindowsPortable { root } | crate::backend::update_install::InstallKind::LinuxPortable { root } => {
            portable_parents.extend(root.parent().map(Path::to_path_buf));
        }
        _ => {}
    }
    if let Ok(entries) = std::fs::read_dir(paths.instance_dir.join("updates")) {
        portable_parents.extend(entries.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()));
    }
    Some(Layout {
        versions_root: versions_root.to_path_buf(),
        running: version_dir.file_name()?.to_string_lossy().into_owned(),
        channel: paths.channel,
        portable_parents,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// `versions/<v>/`: an old version's data, logs and caches.
    VersionData,
    /// An unpacked portable of an old version.
    Portable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Keep {
    /// The version this srv runs.
    Running,
    /// Newer than the running version.
    NotOlder,
    /// One of the newest older versions, kept as the way back.
    Rollback,
    /// A running process names it.
    InUse,
    /// The processes could not be read, so nothing can be shown to be unused.
    ScanFailed,
    /// Written to within the idle time.
    RecentlyActive,
    /// A folder that is not an AgentMux version or not one the updater unpacked for this channel,
    /// or a symlink.
    NotOurs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Candidate {
    pub dir: PathBuf,
    pub version: String,
    pub kind: Kind,
    pub bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Plan {
    pub removable: Vec<Candidate>,
    /// `(folder, why)`.
    pub kept: Vec<(PathBuf, Keep)>,
}

impl Plan {
    pub fn reclaimable_bytes(&self) -> u64 {
        self.removable.iter().map(|c| c.bytes).sum()
    }
}

/// One running process, as the guard needs it.
#[derive(Debug, Clone, Default)]
pub struct ProcView {
    pub pid: u32,
    /// Its program name.
    pub name: String,
    /// Its program path, working folder, arguments and environment entries.
    pub strings: Vec<String>,
}

/// What is running right now.
#[derive(Debug, Clone, Default)]
pub struct InUse {
    /// `None` when the processes could not be read.
    pub procs: Option<Vec<ProcView>>,
}

fn normal(s: &str) -> String {
    let s = s.replace('\\', "/").to_ascii_lowercase();
    // `canonicalize` on Windows gives a `\\?\C:\...` path.
    s.strip_prefix("//?/").map_or_else(|| s.clone(), str::to_string)
}

impl InUse {
    /// Everything about the processes this machine runs.
    pub fn scan() -> Self {
        let mut sys = sysinfo::System::new();
        sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::nothing()
                .with_cwd(sysinfo::UpdateKind::Always)
                .with_exe(sysinfo::UpdateKind::Always)
                .with_cmd(sysinfo::UpdateKind::Always)
                .with_environ(sysinfo::UpdateKind::Always),
        );
        let procs: Vec<ProcView> = sys
            .processes()
            .iter()
            .map(|(pid, p)| {
                let mut strings: Vec<String> = Vec::new();
                strings.extend(p.exe().map(|e| e.to_string_lossy().into_owned()));
                strings.extend(p.cwd().map(|c| c.to_string_lossy().into_owned()));
                strings.extend(p.cmd().iter().map(|a| a.to_string_lossy().into_owned()));
                strings.extend(p.environ().iter().map(|e| e.to_string_lossy().into_owned()));
                ProcView { pid: pid.as_u32(), name: p.name().to_string_lossy().into_owned(), strings }
            })
            .collect();
        // Even a nearly idle machine runs dozens: an empty list means the scan did not work.
        Self { procs: (!procs.is_empty()).then_some(procs) }
    }

    /// Whether any process names `dir` or something inside it. A name that continues the last
    /// component (`0.60.10` for `0.60.1`) is not a mention.
    pub fn mentions(&self, dir: &Path) -> bool {
        let needle = normal(&dir.to_string_lossy());
        let needle = needle.trim_end_matches('/');
        if needle.is_empty() {
            return false;
        }
        self.procs.iter().flatten().flat_map(|p| p.strings.iter()).any(|s| {
            let hay = normal(s);
            hay.match_indices(needle).any(|(i, _)| {
                hay[i + needle.len()..].chars().next().is_none_or(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+')))
            })
        })
    }

    /// Whether `pid` is a live AgentMux process.
    fn agentmux_alive(&self, pid: u32) -> bool {
        self.procs.iter().flatten().any(|p| p.pid == pid && p.name.to_ascii_lowercase().contains("agentmux"))
    }
}

/// `x.y.z`, three numbers and nothing else: a local build's `0.60.2+g...` is not a release.
fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let mut it = s.split('.');
    let mut next = || it.next().filter(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())).and_then(|p| p.parse::<u64>().ok());
    let v = (next()?, next()?, next()?);
    it.next().is_none().then_some(v)
}

/// The version in an unpacked portable's folder name.
fn portable_version(name: &str) -> Option<&str> {
    if cfg!(windows) {
        name.strip_prefix("agentmux-")?.strip_suffix("-x64-portable")
    } else {
        name.strip_prefix("AgentMux-")
    }
}

/// `name` without the `.removing-<pid>` a removal renames a folder to.
fn leftover_stem(name: &str) -> Option<&str> {
    let (stem, pid) = name.rsplit_once(".removing-")?;
    (!pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit())).then_some(stem)
}

/// Whether `stem` is the name of a folder of `kind` that this module removes.
fn stem_is_ours(kind: Kind, stem: &str) -> bool {
    match kind {
        Kind::Portable => portable_version(stem).and_then(parse_version).is_some(),
        Kind::VersionData => parse_version(stem).is_some(),
    }
}

/// Total size of the files under `dir`, never following a symlink.
fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for entry in rd.flatten() {
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                total += meta.len();
            }
        }
    }
    total
}

/// The newest write to any file or folder under `dir`, symlinks not followed. A running version
/// writes its databases and logs; one that stopped has been still since. A folder's own time moves
/// only when an entry is added or removed, so the files themselves are looked at.
fn last_activity(dir: &Path) -> Option<SystemTime> {
    let mut newest: Option<SystemTime> = None;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(t) = std::fs::symlink_metadata(&d).and_then(|m| m.modified()) {
            newest = Some(newest.map_or(t, |n| n.max(t)));
        }
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() && !meta.file_type().is_symlink() {
                stack.push(e.path());
            } else if let Ok(t) = meta.modified() {
                newest = Some(newest.map_or(t, |n| n.max(t)));
            }
        }
    }
    newest
}

/// The pid `srv.lock` records, if the folder has one.
fn recorded_srv_pid(dir: &Path) -> Option<u32> {
    std::fs::read_to_string(dir.join("data").join(crate::backend::base::SRV_DATA_LOCK_FILE)).ok()?.trim().parse().ok()
}

/// The reason `dir` must stay although it is old enough and beyond the way back, if there is one.
fn in_use_reason(dir: &Path, policy: &Policy, now: SystemTime, in_use: &InUse) -> Option<Keep> {
    if in_use.procs.is_none() {
        return Some(Keep::ScanFailed);
    }
    if in_use.mentions(dir) || recorded_srv_pid(dir).is_some_and(|p| in_use.agentmux_alive(p)) {
        return Some(Keep::InUse);
    }
    let idle = last_activity(dir).and_then(|t| now.duration_since(t).ok()).unwrap_or(Duration::ZERO); // unreadable or in the future: just used
    (idle < policy.idle).then_some(Keep::RecentlyActive)
}

struct Entry {
    dir: PathBuf,
    version: String,
    key: (u64, u64, u64),
}

/// Folders of `kind` named by version in `parent`, plus what is kept without being judged.
fn scan_dir(parent: &Path, kind: Kind, layout: &Layout, kept: &mut Vec<(PathBuf, Keep)>, leftovers: &mut Vec<(PathBuf, Kind)>) -> Vec<Entry> {
    let Ok(rd) = std::fs::read_dir(parent) else { return Vec::new() };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        if meta.file_type().is_symlink() {
            kept.push((path, Keep::NotOurs));
            continue;
        }
        if !meta.is_dir() {
            continue;
        }
        if name.contains(".removing-") {
            // Only what a removal of ours renamed: `<a version folder's name>.removing-<pid>`. A
            // portable's parent is the user's, and holds other things.
            if leftover_stem(&name).is_some_and(|stem| stem_is_ours(kind, stem)) {
                leftovers.push((path, kind));
            }
            continue;
        }
        let version = match kind {
            Kind::Portable => portable_version(&name).map(str::to_owned),
            _ => Some(name.clone()),
        };
        let Some(key) = version.as_deref().and_then(parse_version) else {
            // Not ours to judge, unless it looks like a portable of some other kind: stay quiet
            // about unrelated folders in a parent the user owns.
            if kind != Kind::Portable || name.to_ascii_lowercase().starts_with("agentmux") {
                kept.push((path, Keep::NotOurs));
            }
            continue;
        };
        let version = version.unwrap_or_default();
        if kind == Kind::Portable
            && !(crate::backend::update_download::is_staged_portable(&path, &version)
                && agentmux_common::runtime_mode::recorded_channel_in(&path).as_deref() == Some(layout.channel.as_str()))
        {
            kept.push((path, Keep::NotOurs));
            continue;
        }
        out.push(Entry { dir: path, version, key });
    }
    out
}

fn judge(plan: &mut Plan, kind: Kind, mut entries: Vec<Entry>, running: (u64, u64, u64), policy: &Policy, now: SystemTime, in_use: &InUse) {
    entries.sort_by(|a, b| b.key.cmp(&a.key));
    let mut older = 0;
    for e in entries {
        let keep = if e.key == running {
            Some(Keep::Running)
        } else if e.key > running {
            Some(Keep::NotOlder)
        } else {
            older += 1;
            if older <= policy.keep_previous {
                Some(Keep::Rollback)
            } else {
                in_use_reason(&e.dir, policy, now, in_use)
            }
        };
        match keep {
            Some(k) => plan.kept.push((e.dir, k)),
            None => plan.removable.push(Candidate { bytes: dir_size(&e.dir), dir: e.dir, version: e.version, kind }),
        }
    }
}

/// Decides what could go. Reads only.
pub fn plan(layout: &Layout, policy: &Policy, now: SystemTime, in_use: &InUse) -> Plan {
    let mut plan = Plan::default();
    let Some(running) = parse_version(&layout.running) else {
        // A running version that is not a plain release (a local build): no basis to judge by.
        return plan;
    };
    let mut leftovers = Vec::new();
    let entries = scan_dir(&layout.versions_root, Kind::VersionData, layout, &mut plan.kept, &mut leftovers);
    judge(&mut plan, Kind::VersionData, entries, running, policy, now, in_use);
    let mut portables = Vec::new();
    for parent in &layout.portable_parents {
        portables.extend(scan_dir(parent, Kind::Portable, layout, &mut plan.kept, &mut leftovers));
    }
    judge(&mut plan, Kind::Portable, portables, running, policy, now, in_use);
    for (dir, kind) in leftovers {
        match in_use_reason(&dir, policy, now, in_use) {
            Some(k) => plan.kept.push((dir, k)),
            None => plan.removable.push(Candidate { bytes: dir_size(&dir), version: String::new(), dir, kind }),
        }
    }
    plan
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub removed: Vec<(PathBuf, u64)>,
    /// Candidates left alone at removal time, and why.
    pub skipped: Vec<(PathBuf, String)>,
}

/// Removes what [`plan`] found, asking again for each folder at the last moment: a process may
/// have started in it since.
pub fn remove(plan: &Plan, policy: &Policy, now: SystemTime, in_use: &dyn Fn() -> InUse) -> Report {
    let mut report = Report::default();
    for c in &plan.removable {
        // A portable's leftover counts as a portable: the user's, so only when turned on.
        if c.kind == Kind::Portable && !policy.portables {
            report.skipped.push((c.dir.clone(), "portable cleanup is off (oldversions:portables)".into()));
            continue;
        }
        if std::fs::symlink_metadata(&c.dir).map_or(true, |m| m.file_type().is_symlink() || !m.is_dir()) {
            report.skipped.push((c.dir.clone(), "no longer a plain folder".into()));
            continue;
        }
        if let Some(why) = in_use_reason(&c.dir, policy, now, &in_use()) {
            report.skipped.push((c.dir.clone(), format!("{why:?}: in use now")));
            continue;
        }
        let target = if c.dir.file_name().is_some_and(|n| n.to_string_lossy().contains(".removing-")) {
            c.dir.clone()
        } else {
            let name = c.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let renamed = c.dir.with_file_name(format!("{name}.removing-{}", std::process::id()));
            // Windows refuses this while any file inside is open: one more look at "in use".
            if let Err(e) = std::fs::rename(&c.dir, &renamed) {
                report.skipped.push((c.dir.clone(), format!("could not be renamed, so something has it open: {e}")));
                continue;
            }
            renamed
        };
        match std::fs::remove_dir_all(&target) {
            Ok(()) => report.removed.push((c.dir.clone(), c.bytes)),
            Err(e) => report.skipped.push((target, format!("removal started, finishing on the next pass: {e}"))),
        }
    }
    report
}

/// The outcome of one [`run`].
#[derive(Debug, Serialize)]
pub struct Run {
    pub plan: Plan,
    /// `None` for a dry run.
    pub report: Option<Report>,
    /// The processes could be read. When they could not, nothing is removable.
    pub scan_ok: bool,
}

/// Plans, and unless `dry_run`, removes. `only` limits a removal to folders an earlier dry run
/// listed: what became removable since is left alone, so a removal is what the caller was shown.
pub fn run(layout: &Layout, policy: &Policy, dry_run: bool, only: Option<&[String]>, now: SystemTime) -> Run {
    let in_use = InUse::scan();
    let scan_ok = in_use.procs.is_some();
    let mut plan = plan(layout, policy, now, &in_use);
    if let Some(only) = only {
        plan.removable.retain(|c| only.iter().any(|d| Path::new(d) == c.dir));
    }
    let report = (!dry_run).then(|| remove(&plan, policy, now, &InUse::scan));
    Run { plan, report, scan_ok }
}

#[cfg(test)]
#[path = "old_versions_tests.rs"]
mod tests;
