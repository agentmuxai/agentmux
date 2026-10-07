// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Finding an agent's processes in the process table, by the env tag they
//! inherit. Two uses:
//!
//! - [`tagged`], on every OS: after a teardown, what still carries the tag got
//!   out of the agent's tracker, and is reported (`agent_teardown`).
//! - [`ScanTracker`]: the best-effort tracker where the OS can't hold the
//!   tree: macOS (no cgroups), Linux without a delegated cgroup, and Windows
//!   when a Job Object can't be created.
//!
//! An agent's processes are:
//!
//! - every descendant of the processes assigned to it (its CLI), and
//! - every process carrying its `AGENTMUX_BLOCKID` in its environment, which
//!   whatever the agent starts inherits, `setsid` daemons and orphans
//!   reparented to init or launchd included, plus their descendants.
//!
//! A process that cleared its environment after leaving the tree is missed;
//! hence `BestEffort`. See
//! `docs/analysis/agent-spawned-process-tracking-2026-10-07.md` §6.3.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::sync::{Arc, Mutex};

use super::{TrackedProcess, TrackerHandle, TrackingConfidence};

/// A process-table snapshot shared by every scan tracker, refreshed at most
/// this often: the 2 s poll of N agents costs one OS scan, not N.
const SNAPSHOT_TTL: std::time::Duration = std::time::Duration::from_millis(500);

struct Snapshot {
    sys: sysinfo::System,
    at: Option<std::time::Instant>,
}

static SNAPSHOT: Mutex<Option<Snapshot>> = Mutex::new(None);

/// Run `f` over a snapshot of the process table, environments and command
/// lines included, taken no earlier than `after` (when given) and no older
/// than [`SNAPSHOT_TTL`]. Callers waiting on the lock share the scan the
/// first of them takes.
fn with_snapshot<T>(after: Option<std::time::Instant>, f: impl FnOnce(&sysinfo::System) -> T) -> T {
    let mut guard = SNAPSHOT.lock().unwrap_or_else(|e| e.into_inner());
    let snap = guard.get_or_insert_with(|| Snapshot { sys: sysinfo::System::new(), at: None });
    let stale = match snap.at {
        None => true,
        Some(t) => after.is_some_and(|a| t < a) || t.elapsed() >= SNAPSHOT_TTL,
    };
    if stale {
        snap.sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::nothing()
                .with_cmd(sysinfo::UpdateKind::Always)
                .with_environ(sysinfo::UpdateKind::Always)
                .with_memory(),
        );
        snap.at = Some(std::time::Instant::now());
    }
    f(&snap.sys)
}

/// Is `p` the process recorded as started at `started_ms`? A PID the OS has
/// reused for another process is not (start times are whole seconds).
fn same_process(started_ms: u64, p: &sysinfo::Process) -> bool {
    started_ms.abs_diff(p.start_time() * 1000) <= 1000
}

fn is_root(roots: &HashMap<u32, u64>, pid: u32, p: &sysinfo::Process) -> bool {
    roots.get(&pid).is_some_and(|started| same_process(*started, p))
}

/// The members of one agent's tree in `sys`: its live `roots` (PID → start
/// time) and every process whose environment holds `tag`
/// (`AGENTMUX_BLOCKID=<block>`), then all their descendants. Never srv itself.
fn members_in(sys: &sysinfo::System, roots: &HashMap<u32, u64>, tag: &OsString) -> Vec<TrackedProcess> {
    let me = std::process::id();
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for (pid, p) in sys.processes() {
        if let Some(parent) = p.parent() {
            children.entry(parent.as_u32()).or_default().push(pid.as_u32());
        }
    }
    let mut set: Vec<u32> = sys
        .processes()
        .iter()
        .filter(|(_, p)| p.status() != sysinfo::ProcessStatus::Zombie)
        .filter(|(pid, p)| is_root(roots, pid.as_u32(), p) || p.environ().iter().any(|e| e == tag))
        .map(|(pid, _)| pid.as_u32())
        .collect();
    let mut seen: HashSet<u32> = set.iter().copied().collect();
    let mut i = 0;
    while i < set.len() {
        for c in children.get(&set[i]).into_iter().flatten() {
            let zombie = sys.process(sysinfo::Pid::from_u32(*c)).is_some_and(|p| p.status() == sysinfo::ProcessStatus::Zombie);
            if !zombie && seen.insert(*c) {
                set.push(*c);
            }
        }
        i += 1;
    }
    set.into_iter()
        .filter(|pid| *pid != me && *pid > 1)
        .filter_map(|pid| sys.process(sysinfo::Pid::from_u32(pid)).map(|p| (pid, p)))
        .map(|(pid, p)| {
            let cmd: Vec<String> = p.cmd().iter().map(|a| a.to_string_lossy().into_owned()).collect();
            TrackedProcess {
                pid,
                command: if cmd.is_empty() { p.name().to_string_lossy().into_owned() } else { cmd.join(" ") },
                rss_bytes: p.memory(),
                started_at_ms: p.start_time() * 1000,
                parent_pid: p.parent().map(|pp| pp.as_u32()),
                exe: cmd.first().cloned().unwrap_or_else(|| p.name().to_string_lossy().into_owned()),
            }
        })
        .collect()
}

/// Ask `pid` to exit. `false` where there is no such signal (Windows: the
/// agent side runs without a console, so no CTRL_BREAK).
#[cfg(unix)]
fn ask_to_exit(pid: u32) -> bool {
    // SAFETY: kill(2) with a pid from the process table and a constant signal.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) == 0 }
}

#[cfg(windows)]
fn ask_to_exit(_pid: u32) -> bool {
    false
}

#[cfg(unix)]
fn kill_now(pid: u32) {
    // SAFETY: kill(2) with a pid from the process table and a constant signal.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
}

#[cfg(windows)]
fn kill_now(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    // SAFETY: a handle opened, used and closed here.
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !h.is_null() {
            TerminateProcess(h, 1);
            CloseHandle(h);
        }
    }
}

/// The env entry that marks a block's processes.
pub fn block_tag(block_id: &str) -> OsString {
    OsString::from(format!("AGENTMUX_BLOCKID={block_id}"))
}

/// Every live process tagged with `block_id` (and its descendants), from a
/// fresh scan. What a teardown reports as having escaped its tracker.
pub fn tagged(block_id: &str) -> Vec<TrackedProcess> {
    with_snapshot(Some(std::time::Instant::now()), |sys| members_in(sys, &HashMap::new(), &block_tag(block_id)))
}

struct Inner {
    tag: OsString,
    /// The processes assigned to the agent (its CLI, one per spawn): PID →
    /// start time (Unix ms). Pruned as they exit, so a reused PID never
    /// counts.
    roots: Mutex<HashMap<u32, u64>>,
}

impl Inner {
    fn members(&self, fresh: bool) -> Vec<TrackedProcess> {
        let after = fresh.then(std::time::Instant::now);
        with_snapshot(after, |sys| {
            let mut roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
            roots.retain(|pid, started| sys.process(sysinfo::Pid::from_u32(*pid)).is_some_and(|p| same_process(*started, p)));
            members_in(sys, &roots, &self.tag)
        })
    }

    /// SIGKILL every member, leaves first, again until none is left (a
    /// member may fork between the scan and the signal).
    fn kill_all(&self) {
        for _ in 0..4 {
            let members = self.members(true);
            if members.is_empty() {
                return;
            }
            for p in members.iter().rev() {
                kill_now(p.pid);
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
    }
}

/// One agent's best-effort tracker.
pub struct ScanTracker {
    inner: Arc<Inner>,
}

impl ScanTracker {
    pub fn new(block_id: &str) -> Self {
        Self { inner: Arc::new(Inner { tag: block_tag(block_id), roots: Mutex::new(HashMap::new()) }) }
    }
}

impl TrackerHandle for ScanTracker {
    fn assign_process(&self, pid: u32) -> Result<(), String> {
        // Its start time, from a fresh scan: recorded with the PID so a later
        // process reusing the PID is not taken for it.
        let started = with_snapshot(Some(std::time::Instant::now()), |sys| {
            sys.process(sysinfo::Pid::from_u32(pid)).map(|p| p.start_time() * 1000)
        });
        let Some(started) = started else {
            return Err(format!("pid {pid} is not running"));
        };
        self.inner.roots.lock().unwrap_or_else(|e| e.into_inner()).insert(pid, started);
        Ok(())
    }

    fn list_members(&self) -> Vec<TrackedProcess> {
        self.inner.members(false)
    }

    fn kill_tree(&self) {
        self.inner.kill_all();
    }

    fn kill_pid(&self, pid: u32) -> bool {
        if !self.inner.members(true).iter().any(|p| p.pid == pid) {
            return false;
        }
        kill_now(pid);
        true
    }

    fn confidence(&self) -> TrackingConfidence {
        TrackingConfidence::BestEffort
    }

    fn terminate(&self) -> bool {
        let mut asked = false;
        for p in self.inner.members(true) {
            asked |= ask_to_exit(p.pid);
        }
        asked
    }

    fn member_count(&self) -> usize {
        self.inner.members(true).len()
    }
}

impl Drop for ScanTracker {
    /// The agent is gone: end what it left. Off the caller's thread, since a
    /// scan takes tens of milliseconds and the caller may be async.
    fn drop(&mut self) {
        let inner = Arc::clone(&self.inner);
        std::thread::spawn(move || inner.kill_all());
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn spawn(script: &str, block_id: Option<&str>) -> std::process::Child {
        let mut cmd = std::process::Command::new("sh");
        cmd.args(["-c", script]);
        if let Some(id) = block_id {
            cmd.env("AGENTMUX_BLOCKID", id);
        }
        cmd.spawn().expect("spawn sh")
    }

    fn wait_for(mut f: impl FnMut() -> bool) -> bool {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !f() {
            if std::time::Instant::now() >= until {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        true
    }

    fn commands(t: &ScanTracker) -> Vec<String> {
        t.inner.members(true).into_iter().map(|p| p.command).collect()
    }

    /// The tag finds a `setsid` daemon even after the shell that started it
    /// exited (reparented to init), and `kill_tree` ends it.
    #[test]
    fn a_detached_tagged_process_is_found_and_killed() {
        let block = format!("scan-test-detached-{}", std::process::id());
        let tracker = ScanTracker::new(&block);
        let mut sh = spawn("setsid sleep 4001 >/dev/null 2>&1 < /dev/null &", Some(&block));
        let _ = sh.wait();
        assert!(wait_for(|| commands(&tracker).iter().any(|c| c == "sleep 4001")), "{:?}", commands(&tracker));
        tracker.kill_tree();
        assert!(wait_for(|| commands(&tracker).is_empty()), "left: {:?}", commands(&tracker));
    }

    /// A root that exited is forgotten: a later process reusing its PID is
    /// not taken for the agent's.
    #[test]
    fn an_exited_root_is_pruned_and_its_pid_not_trusted() {
        let block = format!("scan-test-prune-{}", std::process::id());
        let tracker = ScanTracker::new(&block);
        let mut root = spawn("exec sleep 4031", None);
        tracker.assign_process(root.id()).unwrap();
        assert!(wait_for(|| tracker.member_count() == 1));
        let _ = root.kill();
        let _ = root.wait();
        assert!(wait_for(|| tracker.member_count() == 0));
        assert!(tracker.inner.roots.lock().unwrap().is_empty(), "the exited root was pruned");
        assert!(tracker.assign_process(root.id()).is_err(), "a PID that isn't running can't be assigned");
    }

    /// Descendants of an assigned root count even without the tag (a
    /// process that cleared its environment), and processes of other blocks
    /// don't.
    #[test]
    fn roots_descendants_count_and_other_blocks_do_not() {
        let block = format!("scan-test-roots-{}", std::process::id());
        let tracker = ScanTracker::new(&block);
        let mut root = spawn("env -i sleep 4011 & sleep 4012; wait", None);
        tracker.assign_process(root.id()).unwrap();
        let mut other = spawn("sleep 4013", Some("some-other-block"));
        assert!(wait_for(|| commands(&tracker).iter().any(|c| c == "sleep 4011")), "{:?}", commands(&tracker));
        assert!(!commands(&tracker).iter().any(|c| c == "sleep 4013"), "another block's process is not a member");
        tracker.terminate();
        assert!(wait_for(|| tracker.member_count() == 0), "left: {:?}", commands(&tracker));
        let _ = root.wait();
        let _ = other.kill();
        let _ = other.wait();
    }

    /// Dropping the tracker (the agent's block is gone) ends what it left.
    #[test]
    fn drop_kills_what_is_left() {
        let block = format!("scan-test-drop-{}", std::process::id());
        let mut sh = spawn("trap '' TERM; setsid sleep 4021 >/dev/null 2>&1 < /dev/null &", Some(&block));
        let _ = sh.wait();
        let tracker = ScanTracker::new(&block);
        assert!(wait_for(|| tracker.member_count() == 1));
        drop(tracker);
        assert!(wait_for(|| tagged(&block).is_empty()), "left: {:?}", tagged(&block));
    }
}
