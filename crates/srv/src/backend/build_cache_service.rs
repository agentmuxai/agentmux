// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Runs the build-cache cleanup (`build_cache.rs`) on a schedule and on a finished task, and
//! warns agents when the disk runs low (`docs/specs/SPEC_BUILD_CACHE_CLEANUP_2026_10_26.md`).
//!
//! - Every 5 minutes: the free space of the disk that holds the agents' workspaces. When it is
//!   low, a pass prunes orphans and abandoned clones at once; if it is still low afterwards, the
//!   agents whose own workspace is large hear of it, about their own folders, and the host's
//!   steward gets the summary for the whole computer. A note goes out again only after free space
//!   falls a further few GB or the level gets worse, never on a timer alone, and only a critical
//!   level starts a turn (`disk_notice.rs`; a file in the shared locks folder keeps the last
//!   note, so two AgentMux channels do not both warn).
//! - Every hour: the orphan pass over every local workspace.
//! - When an agent completes a Kanban task: its own clones lose their `incremental` folders.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::backend::build_cache::{self as bc, Policy, Why};
use crate::backend::disk_notice::{self as dn, Level};
use crate::backend::obj::{self, Block};
use crate::backend::storage::store::Store;
use crate::backend::wconfig::ConfigState;

const DISK_EVERY: Duration = Duration::from_secs(5 * 60);
const HOURLY_EVERY: Duration = Duration::from_secs(3600);
/// The first hourly pass waits this long after start, so it never competes with startup.
const FIRST_HOURLY_AFTER: Duration = Duration::from_secs(10 * 60);
/// After a warning that nobody received, the next attempt (which walks every build folder) waits
/// this long.
const RETRY_AFTER: Duration = Duration::from_secs(30 * 60);
/// Unix seconds before which no warning is tried.
static NEXT_TRY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// How long a finished task's cleanup waits for another srv's pass, and how often it looks.
const TASK_LOCK_WAIT: Duration = Duration::from_secs(15 * 60);
const LOCK_POLL: Duration = Duration::from_millis(500);
/// The key a held low-disk note is kept under: a newer one replaces it.
const HOLD_KEY: &str = "low-disk";

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
    /// Its workspace.
    pub workspace: PathBuf,
    /// Whether its seat says it is this computer's steward, who gets the summary for the host.
    pub steward: bool,
}

/// Whether a seat names its agent a steward: in its qualifier or the line about what it owns.
fn seat_is_steward(seat: &crate::registry::AgentSeat) -> bool {
    [seat.qualifier.as_deref(), seat.area.as_deref()].into_iter().flatten().any(|t| t.to_ascii_lowercase().contains("steward"))
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
        let steward = def.and_then(|d| crate::backend::agent_seat::effective_seat(mstore, d)).is_some_and(|seat| seat_is_steward(&seat));
        out.push(LocalAgent {
            target: rec.uid.clone().filter(|u| !u.is_empty()).unwrap_or(rec.agent_id.clone()),
            agent_id: rec.agent_id.clone(),
            name: def.map(|d| d.name.clone()).unwrap_or_else(|| rec.agent_id.clone()),
            workspace,
            steward,
        });
    }
    out
}

/// One workspace's build folders, as the note needs them.
#[derive(Debug, Clone)]
pub struct WorkspaceSize {
    pub workspace: PathBuf,
    pub target_bytes: u64,
    pub incremental_bytes: u64,
}

/// The agents whose own workspace is large (build folders over `BIG_WORKSPACE`, or incremental
/// caches over `BIG_INCREMENTAL`): they are the ones who can free the space, so they hear of it.
/// A small workspace never does, however few others there are.
pub fn big_owners(sizes: &[WorkspaceSize], agents: &[LocalAgent]) -> Vec<LocalAgent> {
    let mut out: Vec<LocalAgent> = Vec::new();
    for s in sizes.iter().filter(|s| s.target_bytes >= dn::BIG_WORKSPACE || s.incremental_bytes >= dn::BIG_INCREMENTAL) {
        for owner in agents.iter().filter(|a| same_path(&a.workspace, &s.workspace)) {
            if !out.iter().any(|o| o.target == owner.target) {
                out.push(owner.clone());
            }
        }
    }
    out
}

/// Whether `clone` is `workspace` itself or a folder in it.
fn in_workspace(workspace: &Path, clone: &Path) -> bool {
    same_path(workspace, clone) || inside(workspace, clone)
}

fn folder_line(s: &bc::TargetSize, owner: Option<&str>) -> String {
    let who = owner.map_or(String::new(), |o| format!(" [{o}]"));
    format!("- {} : {} ({} of it incremental){who}", s.clone.display(), bc::human(s.target_bytes), bc::human(s.incremental_bytes))
}

/// The note for one big workspace's owner: its own folders, largest first.
fn owner_note(policy: &Policy, level: Level, free: u64, total: u64, agent: &LocalAgent, per_clone: &[bc::TargetSize]) -> String {
    let own: Vec<String> = per_clone.iter().filter(|c| in_workspace(&agent.workspace, &c.clone)).take(6).map(|c| folder_line(c, None)).collect();
    dn::note_text(policy, level, free, total, "Your largest cargo build folders:", &own)
}

/// The note for the host's steward: the largest folders on the computer, each with its owner.
fn steward_note(policy: &Policy, level: Level, free: u64, total: u64, agents: &[LocalAgent], per_clone: &[bc::TargetSize]) -> String {
    let all: Vec<String> = per_clone
        .iter()
        .take(6)
        .map(|c| folder_line(c, agents.iter().find(|a| in_workspace(&a.workspace, &c.clone)).map(|a| a.name.as_str())))
        .collect();
    dn::note_text(policy, level, free, total, "Largest cargo build folders on this computer (owner in brackets):", &all)
}

/// Tells `agent`: a warning is held for its next turn, only a critical level starts one.
fn tell(agent: &LocalAgent, level: Level, text: &str) -> bool {
    let handler = crate::backend::reactive::get_global_handler();
    let sent = if level == Level::Critical { handler.deliver_system_note(&agent.target, text) } else { handler.hold_system_note(&agent.target, HOLD_KEY, text) };
    match sent {
        Ok(()) => {
            tracing::info!(agent = %agent.name, level = ?level, "build cache: low-disk note sent");
            true
        }
        Err(e) => {
            tracing::warn!(agent = %agent.name, error = %e, "build cache: low-disk note not delivered");
            false
        }
    }
}

fn warn_marker() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".agentmux").join("shared").join("locks").join("build-cache-warn"))
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

fn unix_now() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// One look at the disk: clean up when low, and if it is still low and worse than at the last
/// note (`disk_notice::should_send`), tell the agents who can free space.
fn disk_check(mstore: &Store, policy: &Policy) {
    let Some(root) = agents_root() else { return };
    let Some((free, total)) = bc::disk_of(&root) else { return };
    let marker = warn_marker();
    if !policy.is_low(free, total) {
        // Back above the warning level with room to spare: the next drop is a new event.
        if let Some(m) = marker.as_ref().filter(|_| dn::recovered(policy, free, total)) {
            dn::clear_sent(m);
        }
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
    let Some(marker) = marker else { return };
    let level = dn::level(policy, free, total);
    let prev = dn::read_sent(&marker, free);
    if !dn::should_send(prev.as_ref(), level, free, unix_now()) {
        // A better level than the last note's is remembered, so a later worsening is told.
        if let Some(p) = prev.filter(|p| level < p.level) {
            dn::write_sent(&marker, &dn::Sent { level, ..p });
        }
        return;
    }
    // The walk below reads every build folder: on a full disk, do not repeat it every check when
    // there was nobody to tell.
    let now_s = unix_now();
    if now_s < NEXT_TRY.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    NEXT_TRY.store(now_s + RETRY_AFTER.as_secs(), std::sync::atomic::Ordering::Relaxed);
    let mut per_clone: Vec<bc::TargetSize> = Vec::new();
    let mut sizes: Vec<WorkspaceSize> = Vec::new();
    for ws in workspaces(&root) {
        let clones = bc::target_sizes(&ws);
        let target_bytes: u64 = clones.iter().map(|s| s.target_bytes).sum();
        if target_bytes > 0 {
            sizes.push(WorkspaceSize { workspace: ws, target_bytes, incremental_bytes: clones.iter().map(|s| s.incremental_bytes).sum() });
        }
        per_clone.extend(clones);
    }
    sizes.sort_by(|a, b| b.target_bytes.cmp(&a.target_bytes));
    per_clone.sort_by(|a, b| b.target_bytes.cmp(&a.target_bytes));
    let agents = local_agents(mstore);
    let mut delivered = 0;
    // The steward gets the whole computer's summary, so an owner who is the steward is told once.
    for owner in big_owners(&sizes, &agents).iter().filter(|a| !a.steward) {
        if tell(owner, level, &owner_note(policy, level, free, total, owner, &per_clone)) {
            delivered += 1;
        }
    }
    for steward in agents.iter().filter(|a| a.steward) {
        if tell(steward, level, &steward_note(policy, level, free, total, &agents, &per_clone)) {
            delivered += 1;
        }
    }
    if delivered > 0 {
        dn::write_sent(&marker, &dn::Sent { level, free, at: unix_now() });
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

    const GB: u64 = 1 << 30;

    fn agent(target: &str, ws: &str, steward: bool) -> LocalAgent {
        LocalAgent { target: target.into(), agent_id: target.into(), name: target.into(), workspace: PathBuf::from(ws), steward }
    }

    fn ws(path: &str, target_gb: f64, incremental_gb: f64) -> WorkspaceSize {
        WorkspaceSize { workspace: PathBuf::from(path), target_bytes: (target_gb * GB as f64) as u64, incremental_bytes: (incremental_gb * GB as f64) as u64 }
    }

    fn clone_of(path: &str, target_gb: u64, incremental_gb: u64) -> bc::TargetSize {
        bc::TargetSize { clone: PathBuf::from(path), target_bytes: target_gb * GB, incremental_bytes: incremental_gb * GB }
    }

    #[test]
    fn only_an_owner_of_a_large_workspace_hears_of_it() {
        // charlie's case: three agents with folders of 24.8, 14 and 3.5 GB (no incremental left).
        let agents = vec![agent("parlo", "/a/parlo", false), agent("maricon", "/a/maricon", false), agent("opaz", "/a/opaz", false)];
        let sizes = vec![ws("/a/parlo", 24.8, 1.0), ws("/a/maricon", 14.0, 0.5), ws("/a/opaz", 3.5, 0.0)];
        let who: Vec<String> = big_owners(&sizes, &agents).into_iter().map(|a| a.target).collect();
        assert_eq!(who, ["parlo", "maricon"], "the 3.5 GB folder with nothing incremental is not told");
    }

    #[test]
    fn a_small_workspace_with_a_big_incremental_cache_is_told_and_the_line_is_exact() {
        let agents = vec![agent("a", "/a/a", false), agent("b", "/a/b", false), agent("c", "/a/c", false)];
        let sizes = vec![ws("/a/a", 9.9, 1.9), ws("/a/b", 10.0, 0.0), ws("/a/c", 6.0, 2.0)];
        let who: Vec<String> = big_owners(&sizes, &agents).into_iter().map(|a| a.target).collect();
        assert_eq!(who, ["b", "c"], "10 GB of build folders counts, 9.9 GB does not; 2 GB incremental counts, 1.9 does not");
        assert!(big_owners(&[], &agents).is_empty());
        assert!(big_owners(&[ws("/elsewhere", 50.0, 5.0)], &agents).is_empty(), "a workspace with no agent here");
    }

    #[test]
    fn two_agents_in_one_big_workspace_both_hear_of_it_once() {
        let agents = vec![agent("a1", "/a/shared", false), agent("a2", "/a/shared", false), agent("a1", "/a/shared", false)];
        let who: Vec<String> = big_owners(&[ws("/a/shared", 30.0, 0.0)], &agents).into_iter().map(|a| a.target).collect();
        assert_eq!(who, ["a1", "a2"]);
    }

    #[test]
    fn an_owner_is_told_of_its_own_folders_and_the_steward_of_all_with_their_owners() {
        let agents = vec![agent("w1", "/a/w1", false), agent("w2", "/a/w2", false), agent("clamk", "/a/clamk", true)];
        let per_clone = vec![clone_of("/a/w1/agentmux", 24, 3), clone_of("/a/w2/agentmux", 14, 1), clone_of("/a/w2", 3, 0)];
        let p = Policy::default();
        let own = owner_note(&p, Level::Warning, 43 * GB, 440 * GB, &agents[0], &per_clone);
        assert!(own.contains("- /a/w1/agentmux : 24.0 GB (3.0 GB of it incremental)") || own.contains("- /a/w1/agentmux : 24 GB"), "{own}");
        assert!(!own.contains("/a/w2"), "an owner is not told about other agents' folders: {own}");
        let all = steward_note(&p, Level::Warning, 43 * GB, 440 * GB, &agents, &per_clone);
        assert!(all.contains("/a/w1/agentmux") && all.contains("[w1]"), "{all}");
        assert!(all.contains("[w2]"), "{all}");
        assert!(all.contains("owner in brackets"), "{all}");
        let w2 = owner_note(&p, Level::Warning, 43 * GB, 440 * GB, &agents[1], &per_clone);
        assert!(w2.contains("- /a/w2 :"), "a clone that is the workspace itself counts as in it: {w2}");
    }

    #[test]
    fn a_seat_that_names_a_steward_makes_a_steward() {
        use crate::registry::AgentSeat;
        let seat = |q: Option<&str>, area: Option<&str>| AgentSeat { role: "worker".into(), qualifier: q.map(str::to_string), area: area.map(str::to_string), ..Default::default() };
        assert!(seat_is_steward(&seat(Some("Steward"), None)));
        assert!(seat_is_steward(&seat(None, Some("narko's steward (see below); tasks AgentY gives it"))));
        assert!(!seat_is_steward(&seat(Some("product"), Some("agent pane, identity, accounts"))));
        assert!(!seat_is_steward(&seat(None, None)));
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
