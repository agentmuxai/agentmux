// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Runs the build-cache cleanup (`build_cache.rs`) on a schedule and on a finished task, and
//! warns agents when the disk runs low (`docs/specs/SPEC_BUILD_CACHE_CLEANUP_2026_10_26.md`).
//!
//! - Every 5 minutes: the free space of the disk that holds the agents' workspaces. When it is
//!   low, a pass prunes orphans and abandoned clones at once; if it is still low afterwards, the
//!   agents with the largest build folders and their leaders get a note naming the folders and
//!   sizes, at most once an hour on the whole machine (a file in the shared locks folder keeps
//!   the time, so two AgentMux channels do not both warn).
//! - Every hour: the orphan pass over every local workspace.
//! - When an agent completes a Kanban task: its own clones lose their `incremental` folders.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::backend::build_cache::{self as bc, Policy, Why};
use crate::backend::obj::{self, Block};
use crate::backend::storage::store::Store;
use crate::backend::wconfig::ConfigState;

const DISK_EVERY: Duration = Duration::from_secs(5 * 60);
const HOURLY_EVERY: Duration = Duration::from_secs(3600);
/// The first hourly pass waits this long after start, so it never competes with startup.
const FIRST_HOURLY_AFTER: Duration = Duration::from_secs(10 * 60);
/// At most one disk warning per host in this time.
const WARN_EVERY: Duration = Duration::from_secs(3600);
/// After a warning that nobody received, the next attempt (which walks every build folder) waits
/// this long.
const RETRY_AFTER: Duration = Duration::from_secs(30 * 60);
/// Unix seconds before which no warning is tried.
static NEXT_TRY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// How long a finished task's cleanup waits for another srv's pass, and how often it looks.
const TASK_LOCK_WAIT: Duration = Duration::from_secs(15 * 60);
const LOCK_POLL: Duration = Duration::from_millis(500);
/// How many agents with the largest build folders hear of a warning.
const WARN_AGENTS: usize = 3;

/// Where the agents' workspaces are: `~/.agentmux/agents`.
pub fn agents_root() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("AGENTMUX_AGENTS_ROOT").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    Some(dirs::home_dir()?.join(".agentmux").join("agents"))
}

/// How this srv was started, as far as the cleanup cares.
#[derive(Default)]
struct Start<'a> {
    /// `AGENTMUX_HOME_OVERRIDE` is set: a test or capture instance with a home of its own.
    own_home: bool,
    /// `--headless` / `AGENTMUX_HEADLESS`: srv started directly, with no launcher.
    headless: bool,
    /// The binary sits in a source-tree output folder (`target/debug`, `target/release`, `dist/cef-dev*`),
    /// whatever the environment it inherited says.
    built_in_tree: bool,
    /// The launcher's `AGENTMUX_RUNTIME_MODE` is `dev:<branch>`.
    dev_mode: bool,
    /// The data channel (`AGENTMUX_CHANNEL`): `stable` for a release, `local-<branch>` for a local
    /// package, `dev-<branch>` for `task dev`.
    channel: Option<&'a str>,
    /// `AGENTMUX_BUILD_CACHE_CLEAN`: `Some(false)` never cleans, `Some(true)` cleans.
    forced: Option<bool>,
}

/// What `AGENTMUX_BUILD_CACHE_CLEAN` says: `1`/`true`/`on` or `0`/`false`/`off`; anything else, or
/// nothing, leaves the decision to how srv was started.
fn forced_by(value: Option<&str>) -> Option<bool> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "on" => Some(true),
        "0" | "false" | "off" => Some(false),
        _ => None,
    }
}

/// Whether this srv may clean the real agents' folder: one started by its launcher on the real
/// home, whatever its channel is called (the installed release, or a local package on a host that
/// runs one: on such a host nothing else would ever clean). A `task dev` build, a headless srv, a
/// binary run from a source tree, or an instance with a home of its own shares
/// `~/.agentmux/agents` with the real instance but is not the one that should prune it (Kanban
/// #4534). Decided from how srv was started; `AGENTMUX_BUILD_CACHE_CLEAN` overrides it: `0` never
/// cleans, `1` cleans (except an instance with a home of its own, whose cleaning of the real
/// folder is never wanted). Which one srv of several cleans at a time is [`host_lock`].
fn may_clean(s: &Start) -> bool {
    match s.forced {
        Some(false) => false,
        Some(true) => !s.own_home,
        None => {
            !s.own_home
                && !s.headless
                && !s.built_in_tree
                && !s.dev_mode
                && s.channel.is_some_and(|c| !c.starts_with("dev-"))
        }
    }
}

/// The lock that lets one srv on the host clean at a time, whatever channels are running. Held
/// for the length of a pass; `None` when another srv holds it (its pass covers the same folders),
/// when the file can't be opened, or when there is no home.
fn host_lock() -> Option<std::fs::File> {
    let path = dirs::home_dir()?.join(".agentmux").join("shared").join("locks").join("build-cache-clean.lock");
    try_lock_file(&path)
}

/// [`host_lock`], waiting up to `wait` for the srv that holds it: for work only this pass does
/// (an agent's finished task), which the holder's pass does not cover and nothing repeats.
fn host_lock_within(wait: Duration) -> Option<std::fs::File> {
    let path = dirs::home_dir()?.join(".agentmux").join("shared").join("locks").join("build-cache-clean.lock");
    lock_file_within(&path, wait)
}

fn lock_file_within(path: &Path, wait: Duration) -> Option<std::fs::File> {
    let deadline = std::time::Instant::now() + wait;
    loop {
        if let Some(file) = try_lock_file(path) {
            return Some(file);
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(LOCK_POLL);
    }
}

fn try_lock_file(path: &Path) -> Option<std::fs::File> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    #[allow(clippy::suspicious_open_options)]
    let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path).ok()?;
    file.try_lock().ok()?;
    Some(file)
}

/// [`may_clean`] for this process.
pub(crate) fn this_srv_may_clean() -> bool {
    let channel = agentmux_common::DataPaths::from_env().map(|p| p.channel);
    may_clean(&Start {
        own_home: std::env::var_os("AGENTMUX_HOME_OVERRIDE").is_some_and(|v| !v.is_empty()),
        headless: crate::headless::active() || crate::headless::requested(),
        built_in_tree: agentmux_common::is_dev_self(),
        dev_mode: matches!(agentmux_common::RuntimeMode::from_env(), Some(agentmux_common::RuntimeMode::Dev { .. })),
        channel: channel.as_deref(),
        forced: forced_by(std::env::var("AGENTMUX_BUILD_CACHE_CLEAN").ok().as_deref()),
    })
}

/// Every folder directly in the agents' root.
fn workspaces(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

fn same_path(a: &Path, b: &Path) -> bool {
    let n = |p: &Path| p.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_ascii_lowercase();
    n(a) == n(b)
}

/// Whether `path` is a folder under `root`, ignoring case and slash style. The cleanup touches
/// nothing outside the agents' root: an agent may work in a project folder of the user's.
fn inside(root: &Path, path: &Path) -> bool {
    let n = |p: &Path| p.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_ascii_lowercase();
    let (root, path) = (n(root), n(path));
    path.len() > root.len() && path.starts_with(&root) && path.as_bytes()[root.len()] == b'/'
}

/// An agent running here, as the cleanup needs to know it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAgent {
    /// What notes are addressed to (its uid, else its id).
    pub target: String,
    /// The id it works under (`AGENTMUX_AGENT_ID`).
    pub agent_id: String,
    /// The name it is shown with.
    pub name: String,
    /// Its definition id, for the seat's lead.
    pub def_id: Option<String>,
    /// Its workspace.
    pub workspace: PathBuf,
    /// Its leader's definition id, if it has one.
    pub lead_def_id: Option<String>,
}

fn local_agents(mstore: &Store) -> Vec<LocalAgent> {
    let defs = mstore.agent_def_list().unwrap_or_default();
    let mut out: Vec<LocalAgent> = Vec::new();
    for rec in crate::backend::reactive::get_global_handler().list_agents() {
        let Some(block) = mstore.get::<Block>(&rec.block_id).ok().flatten() else { continue };
        let def = defs.iter().find(|d| rec.uid.as_deref() == Some(d.id.as_str()) || d.slug == rec.agent_id || d.id == rec.agent_id);
        let cwd = obj::meta_get_string(&block.meta, "cmd:cwd", "");
        let cwd = if cwd.trim().is_empty() { def.map(|d| d.working_directory.clone()).unwrap_or_default() } else { cwd };
        if cwd.trim().is_empty() {
            continue;
        }
        let workspace = PathBuf::from(crate::backend::blockcontroller::core::expand_home_dir(&cwd));
        if out.iter().any(|a| same_path(&a.workspace, &workspace) && a.target == rec.uid.clone().filter(|u| !u.is_empty()).unwrap_or(rec.agent_id.clone())) {
            continue;
        }
        let lead_def_id = def.and_then(|d| crate::backend::agent_seat::effective_seat(mstore, d)).and_then(|s| s.lead);
        out.push(LocalAgent {
            target: rec.uid.clone().filter(|u| !u.is_empty()).unwrap_or(rec.agent_id.clone()),
            agent_id: rec.agent_id.clone(),
            name: def.map(|d| d.name.clone()).unwrap_or_else(|| rec.agent_id.clone()),
            def_id: def.map(|d| d.id.clone()),
            workspace,
            lead_def_id,
        });
    }
    out
}

/// The agents to tell and what to tell them: the owners of the `WARN_AGENTS` largest
/// workspaces, and each one's leader, each once. `sizes` is `(workspace, bytes)` largest first.
pub fn recipients(sizes: &[(PathBuf, u64)], agents: &[LocalAgent]) -> Vec<LocalAgent> {
    let mut out: Vec<LocalAgent> = Vec::new();
    let add = |a: &LocalAgent, out: &mut Vec<LocalAgent>| {
        if !out.iter().any(|o| o.target == a.target) {
            out.push(a.clone());
        }
    };
    for (workspace, _) in sizes.iter().take(WARN_AGENTS) {
        for owner in agents.iter().filter(|a| same_path(&a.workspace, workspace)) {
            add(owner, &mut out);
            if let Some(lead) = owner.lead_def_id.as_ref().and_then(|l| agents.iter().find(|a| a.def_id.as_deref() == Some(l.as_str()))) {
                add(lead, &mut out);
            }
        }
    }
    out
}

/// The note: how low the disk is, and the largest build folders with sizes.
pub fn warning_text(free: u64, total: u64, sizes: &[bc::TargetSize], policy: &Policy) -> String {
    let pct = if total > 0 { free as f64 * 100.0 / total as f64 } else { 0.0 };
    let mut lines = vec![format!(
        "[AgentMux] This computer's disk is running low: {} free of {} ({pct:.1}%), below the {} / {:.0}% warning level. Largest cargo build folders:",
        bc::human(free),
        bc::human(total),
        bc::human(policy.warn_free_bytes),
        policy.warn_free_pct
    )];
    for s in sizes.iter().take(6) {
        lines.push(format!("- {} : {} ({} of it incremental)", s.clone.display(), bc::human(s.target_bytes), bc::human(s.incremental_bytes)));
    }
    lines.push(
        "AgentMux already clears an agent's incremental cache when it completes a task and prunes old incremental folders every hour. If you own one of these, delete the `target` folders of clones you are done with (only your own, by path)."
            .to_string(),
    );
    lines.join("\n")
}

fn warn_marker() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".agentmux").join("shared").join("locks").join("build-cache-warn"))
}

/// Whether a warning may go out now: none went out in the last hour. The marker file is shared
/// by every AgentMux on the host.
fn warning_due(marker: &Path, now: SystemTime) -> bool {
    match std::fs::metadata(marker).and_then(|m| m.modified()) {
        Ok(last) => now.duration_since(last).unwrap_or_default() >= WARN_EVERY,
        Err(_) => true,
    }
}

/// Records that a warning went out, so none follows for an hour. Only called once a note was
/// delivered: a warning nobody received must not use up the hour.
fn mark_warned(marker: &Path, now: SystemTime) -> bool {
    if let Some(dir) = marker.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(marker, now.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs()).to_string()).is_ok()
}

/// Whether a build runs in `clone`, looking at the machine's processes afresh each time: a pass
/// removes tens of GB, and a build may start in the clone while it does.
fn fresh_busy_check(clone: &Path) -> bool {
    bc::build_running_in(clone, &bc::snapshot_processes())
}

fn policy(config: &ConfigState) -> Policy {
    Policy::from_extra(&config.get_settings().extra)
}

fn all_workspaces_pass(why: Why, policy: &Policy) -> bc::Report {
    let Some(root) = agents_root() else { return bc::Report::default() };
    let Some(_one_at_a_time) = host_lock() else {
        tracing::debug!(why = ?why, "build cache: another srv on this host is cleaning");
        return bc::Report::default();
    };
    pass_over(&root, why, policy)
}

/// A pass over every workspace under `root`; the caller holds the host lock.
fn pass_over(root: &Path, why: Why, policy: &Policy) -> bc::Report {
    let mut total = bc::Report::default();
    let log = bc::log_path();
    let busy = fresh_busy_check;
    for ws in workspaces(root) {
        let r = bc::run_pass(&ws, why, policy, &busy, SystemTime::now(), log.as_deref());
        total.removed.extend(r.removed);
        total.busy.extend(r.busy);
    }
    if !total.removed.is_empty() {
        tracing::info!(why = ?why, removed = total.removed.len(), freed = %bc::human(total.bytes()), "build cache: pass done");
    }
    total
}

/// How long a cleanup that was asked for waits for another srv's pass.
const ASKED_LOCK_WAIT: Duration = Duration::from_secs(5 * 60);

/// The cleanup a `disk_cleanup` request runs once its countdown ended unvetoed: the pass a low disk
/// runs (orphan incremental folders, and the whole `target` of a clone nobody built in for the
/// policy's `stale` days), over every workspace, skipping a clone with a build running in it. Only an
/// srv that cleans the real agents' folder does it, as on a schedule; it blocks, so call it from
/// `spawn_blocking`. Says what it removed, in words, or why it did not run.
pub(crate) fn clean_when_asked(config: &ConfigState) -> Result<String, String> {
    if !this_srv_may_clean() {
        return Err("this AgentMux is not the one that cleans build folders on this computer (a development, headless or test instance, or cleaning is turned off for it)".to_string());
    }
    let policy = policy(config);
    if !policy.enabled {
        return Err("build-folder cleanup is turned off in this AgentMux's settings (buildcache:enabled)".to_string());
    }
    let root = agents_root().ok_or("the agents' folder is not known on this computer")?;
    let Some(_one_at_a_time) = host_lock_within(ASKED_LOCK_WAIT) else {
        return Err("another AgentMux on this computer was cleaning for the whole wait; ask again later".to_string());
    };
    Ok(summary(&pass_over(&root, Why::Asked, &policy)))
}

fn summary(r: &bc::Report) -> String {
    let busy = if r.busy.is_empty() { String::new() } else { format!("; left {} with a build running", r.busy.len()) };
    format!("removed {} build folders ({} freed){busy}", r.removed.len(), bc::human(r.bytes()))
}

/// One look at the disk: clean up when low, and warn if it is still low afterwards.
fn disk_check(mstore: &Store, policy: &Policy) {
    let Some(root) = agents_root() else { return };
    let Some((free, total)) = bc::disk_of(&root) else { return };
    if !policy.is_low(free, total) {
        return;
    }
    tracing::warn!(free = %bc::human(free), total = %bc::human(total), "build cache: the disk is low");
    if policy.enabled {
        all_workspaces_pass(Why::LowDisk, policy);
    }
    let (free, total) = bc::disk_of(&root).unwrap_or((free, total));
    if !policy.is_low(free, total) {
        return;
    }
    let Some(marker) = warn_marker() else { return };
    let now = SystemTime::now();
    if !warning_due(&marker, now) {
        return;
    }
    // The walk below reads every build folder: on a full disk, do not repeat it every check when
    // there was nobody to tell.
    let now_s = now.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if now_s < NEXT_TRY.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    NEXT_TRY.store(now_s + RETRY_AFTER.as_secs(), std::sync::atomic::Ordering::Relaxed);
    let mut per_clone: Vec<bc::TargetSize> = Vec::new();
    let mut per_workspace: Vec<(PathBuf, u64)> = Vec::new();
    for ws in workspaces(&root) {
        let sizes = bc::target_sizes(&ws);
        let sum: u64 = sizes.iter().map(|s| s.target_bytes).sum();
        if sum > 0 {
            per_workspace.push((ws, sum));
        }
        per_clone.extend(sizes);
    }
    per_workspace.sort_by(|a, b| b.1.cmp(&a.1));
    per_clone.sort_by(|a, b| b.target_bytes.cmp(&a.target_bytes));
    let text = warning_text(free, total, &per_clone, policy);
    let handler = crate::backend::reactive::get_global_handler();
    let mut delivered = 0;
    for r in recipients(&per_workspace, &local_agents(mstore)) {
        match handler.deliver_system_note(&r.target, &text) {
            Ok(()) => {
                delivered += 1;
                tracing::info!(agent = %r.name, "build cache: low-disk note sent");
            }
            Err(e) => tracing::warn!(agent = %r.name, error = %e, "build cache: low-disk note not delivered"),
        }
    }
    if delivered > 0 {
        mark_warned(&marker, SystemTime::now());
    } else {
        tracing::warn!("build cache: the disk is low and no agent could be told; trying again in 30 minutes");
    }
}

/// An agent completed a task: clear its own clones' incremental folders. `agent` is the id the
/// task was completed under. Runs in the background; never fails the completion.
pub fn agent_task_done(mstore: Arc<Store>, config: Arc<ConfigState>, agent: String) {
    if !this_srv_may_clean() {
        return;
    }
    tokio::task::spawn_blocking(move || {
        let policy = policy(&config);
        if !policy.enabled {
            return;
        }
        let Some(root) = agents_root() else { return };
        let mine: Vec<LocalAgent> = local_agents(&mstore)
            .into_iter()
            .filter(|a| a.agent_id == agent || a.target == agent || a.name.eq_ignore_ascii_case(&agent))
            // Only workspaces under the agents' root: a project folder is not ours to clean.
            .filter(|a| inside(&root, &a.workspace))
            .collect();
        if mine.is_empty() {
            return;
        }
        let Some(_one_at_a_time) = host_lock_within(TASK_LOCK_WAIT) else {
            tracing::warn!(agent = %agent, "build cache: task cleanup skipped, another srv kept the host lock for too long");
            return;
        };
        let log = bc::log_path();
        let busy = fresh_busy_check;
        let mut seen: Vec<PathBuf> = Vec::new();
        for a in mine {
            if seen.iter().any(|s| same_path(s, &a.workspace)) {
                continue;
            }
            seen.push(a.workspace.clone());
            let r = bc::run_pass(&a.workspace, Why::TaskDone, &policy, &busy, SystemTime::now(), log.as_deref());
            if !r.removed.is_empty() || !r.busy.is_empty() {
                tracing::info!(agent = %a.name, freed = %bc::human(r.bytes()), removed = r.removed.len(), busy = r.busy.len(), "build cache: task done");
            }
        }
    });
}

/// Starts the schedule. Safe to call once at boot.
pub fn install(mstore: Arc<Store>, config: Arc<ConfigState>) {
    // Only an srv started by its launcher on the real home cleans the real workspaces: not a dev,
    // headless or test instance (see `may_clean`).
    if !this_srv_may_clean() {
        tracing::debug!("build cache: not an srv that cleans; no cleanup or disk warning from this srv");
        return;
    }
    tokio::spawn(async move {
        let started = tokio::time::Instant::now();
        let mut disk = tokio::time::interval(DISK_EVERY);
        let mut next_hourly = started + FIRST_HOURLY_AFTER;
        loop {
            disk.tick().await;
            let (store, cfg) = (mstore.clone(), config.clone());
            let hourly_due = tokio::time::Instant::now() >= next_hourly;
            if hourly_due {
                next_hourly = tokio::time::Instant::now() + HOURLY_EVERY;
            }
            let _ = tokio::task::spawn_blocking(move || {
                let policy = policy(&cfg);
                if hourly_due && policy.enabled && policy.orphans {
                    all_workspaces_pass(Why::Hourly, &policy);
                }
                disk_check(&store, &policy);
            })
            .await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(target: &str, def: &str, ws: &str, lead: Option<&str>) -> LocalAgent {
        LocalAgent {
            target: target.into(),
            agent_id: target.into(),
            name: target.into(),
            def_id: Some(def.into()),
            workspace: PathBuf::from(ws),
            lead_def_id: lead.map(str::to_string),
        }
    }

    #[test]
    fn the_largest_workspaces_owners_and_their_leaders_hear_of_it_once() {
        let agents = vec![
            agent("w1", "d-w1", "/a/w1", Some("d-lead")),
            agent("w2", "d-w2", "/a/w2", Some("d-lead")),
            agent("w3", "d-w3", "/a/w3", None),
            agent("w4", "d-w4", "/a/w4", Some("d-lead")),
            agent("lead", "d-lead", "/a/lead", None),
        ];
        let sizes = vec![(PathBuf::from("/a/w1"), 300), (PathBuf::from("/a/w2"), 200), (PathBuf::from("/a/w3"), 100), (PathBuf::from("/a/w4"), 50)];
        let who: Vec<String> = recipients(&sizes, &agents).into_iter().map(|a| a.target).collect();
        assert_eq!(who, ["w1", "lead", "w2", "w3"], "the top three, and their leader once; the fourth is not told");
        assert!(recipients(&[], &agents).is_empty());
        assert!(recipients(&[(PathBuf::from("/elsewhere"), 1)], &agents).is_empty(), "a workspace with no agent here");
    }

    #[test]
    fn the_warning_names_folders_and_sizes() {
        let sizes = vec![
            bc::TargetSize { clone: PathBuf::from("/a/w1/agentmux"), target_bytes: 193 << 30, incremental_bytes: 155 << 30 },
            bc::TargetSize { clone: PathBuf::from("/a/w2/agentmux"), target_bytes: 5 << 30, incremental_bytes: 1 << 30 },
        ];
        let text = warning_text(15 << 30, 1000 << 30, &sizes, &Policy::default());
        assert!(text.starts_with("[AgentMux] This computer's disk is running low: 15.0 GB free of 1000 GB (1.5%)"), "{text}");
        assert!(text.contains("- /a/w1/agentmux : 193 GB (155 GB of it incremental)"), "{text}");
        assert!(text.contains("- /a/w2/agentmux : 5.0 GB (1.0 GB of it incremental)"), "{text}");
    }

    #[test]
    fn a_warning_goes_out_once_an_hour_for_the_whole_host() {
        let t = tempfile::tempdir().unwrap();
        let marker = t.path().join("locks").join("build-cache-warn");
        let now = SystemTime::now();
        assert!(warning_due(&marker, now), "none yet");
        assert!(warning_due(&marker, now), "asking does not use the hour up: a warning nobody received must not");
        assert!(mark_warned(&marker, now));
        assert!(!warning_due(&marker, now + Duration::from_secs(1800)), "half an hour later: no");
        assert!(warning_due(&marker, now + Duration::from_secs(2 * 3600)), "two hours later: yes");
    }

    #[test]
    fn only_a_folder_under_the_agents_root_is_inside_it() {
        let root = Path::new("C:\\Users\\a\\.agentmux\\agents");
        assert!(inside(root, Path::new("c:/users/a/.agentmux/agents/loap-3-0822g")));
        assert!(inside(root, Path::new("C:\\Users\\a\\.agentmux\\agents\\x\\agentmux\\")));
        assert!(!inside(root, Path::new("C:\\Users\\a\\.agentmux\\agents")), "the root itself is not a workspace");
        assert!(!inside(root, Path::new("C:\\Users\\a\\.agentmux\\agents-old\\x")), "a sibling that shares the prefix");
        assert!(!inside(root, Path::new("C:\\Users\\a\\projects\\site")));
    }

    #[test]
    fn paths_compare_without_regard_to_case_slashes_or_a_trailing_one() {
        assert!(same_path(Path::new("C:\\Users\\a\\x\\"), Path::new("c:/users/a/x")));
        assert!(!same_path(Path::new("/a/x"), Path::new("/a/xy")));
    }

    #[test]
    fn a_launcher_started_srv_on_the_real_home_may_clean_whatever_its_channel_is_called() {
        let started = |channel| Start { channel: Some(channel), ..Default::default() };
        assert!(may_clean(&started("stable")), "the installed release cleans");
        assert!(may_clean(&started("local-HEAD-7138a5-bd7e7fdf")), "a local package on a host that runs one");
        assert!(may_clean(&started("local-loap-3-x")));
        // Each way of not being one, alone:
        assert!(!may_clean(&Start { own_home: true, ..started("stable") }), "an instance with a home of its own");
        assert!(!may_clean(&Start { headless: true, ..started("stable") }), "a headless srv");
        assert!(!may_clean(&Start { built_in_tree: true, ..started("stable") }), "a binary from target/debug or target/release");
        assert!(!may_clean(&Start { dev_mode: true, ..started("local-x") }), "a task dev build");
        assert!(!may_clean(&started("dev-main-0123456789abcdef")), "a dev channel");
        assert!(!may_clean(&Start::default()), "no channel at all: not started by a launcher");
    }

    #[test]
    fn the_override_turns_cleaning_off_or_on_but_never_for_an_instance_with_its_own_home() {
        let local = || Start { channel: Some("local-x"), ..Default::default() };
        assert!(!may_clean(&Start { forced: Some(false), channel: Some("stable"), ..Default::default() }), "0 stops the installed release");
        assert!(!may_clean(&Start { forced: Some(false), ..local() }));
        assert!(may_clean(&Start { forced: Some(true), headless: true, ..Default::default() }), "1 lets a headless host clean");
        assert!(may_clean(&Start { forced: Some(true), dev_mode: true, ..local() }));
        assert!(!may_clean(&Start { forced: Some(true), own_home: true, ..local() }), "a home of its own never cleans the real folder");
        assert_eq!(forced_by(None), None);
        assert_eq!(forced_by(Some("1")), Some(true));
        assert_eq!(forced_by(Some(" On ")), Some(true));
        assert_eq!(forced_by(Some("0")), Some(false));
        assert_eq!(forced_by(Some("FALSE")), Some(false));
        assert_eq!(forced_by(Some("")), None, "empty: left to how srv was started");
        assert_eq!(forced_by(Some("maybe")), None);
    }

    #[test]
    fn one_srv_on_the_host_cleans_at_a_time() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("locks").join("build-cache-clean.lock");
        let first = try_lock_file(&path).expect("the first srv takes the lock");
        assert!(try_lock_file(&path).is_none(), "a second srv waits for the next pass");
        drop(first);
        assert!(try_lock_file(&path).is_some(), "free again once the pass is done");
    }

    #[test]
    fn a_finished_tasks_cleanup_waits_for_the_pass_that_holds_the_lock() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("build-cache-clean.lock");
        let held = try_lock_file(&path).unwrap();
        assert!(lock_file_within(&path, Duration::from_millis(50)).is_none(), "gives up when the pass outlasts the wait");
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            drop(held);
        });
        assert!(lock_file_within(&path, Duration::from_secs(10)).is_some(), "takes the lock once the pass is done");
        releaser.join().unwrap();
    }

    #[test]
    fn this_test_binary_does_not_clean() {
        // It runs from target/debug (or release), with no launcher: the real check refuses it.
        assert!(!this_srv_may_clean());
    }

    #[test]
    fn a_cleanup_that_was_asked_for_does_not_run_in_a_srv_that_does_not_clean_and_says_what_it_did() {
        let config = ConfigState::new();
        let err = clean_when_asked(&config).unwrap_err();
        assert!(err.contains("not the one that cleans"), "{err}");

        let removed = |bytes| bc::Removed { path: PathBuf::from("/a/w/agentmux/target"), bytes, what: "stale target" };
        let report = bc::Report { removed: vec![removed(3 << 30), removed(1 << 30)], busy: vec![PathBuf::from("/a/x/agentmux")] };
        assert_eq!(summary(&report), "removed 2 build folders (4.0 GB freed); left 1 with a build running");
        assert_eq!(summary(&bc::Report::default()), "removed 0 build folders (0 MB freed)");
    }
}
