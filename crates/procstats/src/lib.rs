// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Per-process CPU and memory, read without elevation
//! (SPEC_TOWER_TASK_MANAGER_PANE_2026_10_08.md §3, §4).
//!
//! One [`snapshot`] lists every process this user may see, with what the OS
//! reports for it:
//!
//! | OS | Source | Other users' processes |
//! |---|---|---|
//! | Windows | one `NtQuerySystemInformation(SystemProcessInformation)` call | full (no per-process handle is opened) |
//! | Linux | `/proc/<pid>/stat` + `status` | full, unless `/proc` is mounted `hidepid` |
//! | macOS | `proc_listallpids` + `proc_pid_rusage` | name and parent only: CPU and memory are `None` |
//!
//! Nothing here asks for a privilege: no `SeDebugPrivilege`, no
//! `task_for_pid`, no `CAP_SYS_PTRACE`. A value the OS won't give us is
//! `None`, never 0.
//!
//! CPU is a cumulative time; [`RateMeter`] turns two samples into a rate.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::Instant;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as os;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as os;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as os;

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod os {
    pub fn snapshot() -> std::io::Result<Vec<super::ProcInfo>> {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "no process reader for this OS"))
    }
    pub fn command_line(_pid: u32) -> Option<String> {
        None
    }
    pub fn cpu_count() -> usize {
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
    }
}

/// What [`ProcInfo::mem_private`] measures on this OS — the number the OS's
/// own task manager shows where it has one (spec §4.1).
#[cfg(windows)]
pub const MEMORY_METRIC: &str = "private working set";
#[cfg(target_os = "macos")]
pub const MEMORY_METRIC: &str = "memory footprint";
#[cfg(not(any(windows, target_os = "macos")))]
pub const MEMORY_METRIC: &str = "anonymous resident + swap";

/// One process, as the OS reported it at the moment of the snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcInfo {
    pub pid: u32,
    /// The parent the OS recorded. On Windows it is never updated, so it can
    /// name a process that has exited or a newer one that reused its PID:
    /// check [`ProcInfo::is_child_of`] before trusting it.
    pub ppid: Option<u32>,
    /// With `pid`, identifies this process across snapshots: a new process
    /// that reuses the PID has a different key. Opaque (Windows: creation
    /// time; Linux: start time in clock ticks since boot; macOS: the kernel's
    /// unique id). 0 when unknown.
    pub start_key: u64,
    /// When it started, unix ms.
    pub started_at_ms: Option<u64>,
    /// The executable's name, without directory.
    pub name: String,
    /// User + kernel CPU time used so far, in nanoseconds.
    pub cpu_ns: Option<u64>,
    /// Private memory in bytes, as [`MEMORY_METRIC`] describes.
    pub mem_private: Option<u64>,
    /// Working set / resident set in bytes, shared pages included.
    pub mem_resident: Option<u64>,
    /// Committed private bytes (Windows only).
    pub mem_commit: Option<u64>,
}

impl ProcInfo {
    /// The key that tells this process apart from a later one with its PID.
    pub fn key(&self) -> ProcKey {
        ProcKey { pid: self.pid, start_key: self.start_key }
    }

    /// The OS couldn't tell us its CPU or memory (another user's process on
    /// macOS).
    pub fn measured(&self) -> bool {
        self.cpu_ns.is_some() || self.mem_private.is_some()
    }

    /// Whether `parent` is really this process's parent: the PID matches and,
    /// where both start times are known, the parent started first (a parent
    /// that started later reused the PID of the real one).
    pub fn is_child_of(&self, parent: &ProcInfo) -> bool {
        if self.ppid != Some(parent.pid) || parent.pid == self.pid {
            return false;
        }
        match (parent.started_at_ms, self.started_at_ms) {
            (Some(p), Some(c)) => p <= c,
            _ => true,
        }
    }
}

/// A process's identity across snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProcKey {
    pub pid: u32,
    pub start_key: u64,
}

/// Every process this user may see. Processes that exit mid-read are left out.
pub fn snapshot() -> std::io::Result<Vec<ProcInfo>> {
    os::snapshot()
}

/// A process's full command line, read on demand (it may hold secrets, so it
/// is never part of a [`snapshot`]). `None` when it has exited or the OS
/// won't say.
pub fn command_line(pid: u32) -> Option<String> {
    os::command_line(pid)
}

/// Logical CPUs on the whole machine — what "percent of the machine" divides
/// by (every processor group on Windows).
pub fn cpu_count() -> usize {
    os::cpu_count().max(1)
}

/// Turns cumulative CPU times into rates: the fraction of one core used
/// since the previous round (1.0 = one core fully busy).
///
/// Each round, call [`RateMeter::sample`] for every key still alive, then
/// [`RateMeter::finish`]; keys not sampled in a round are forgotten, so a PID
/// reused by a new process (a new key) starts fresh.
#[derive(Debug)]
pub struct RateMeter<K> {
    prev: HashMap<K, (u64, Instant)>,
    next: HashMap<K, (u64, Instant)>,
}

impl<K> Default for RateMeter<K> {
    fn default() -> Self {
        Self { prev: HashMap::new(), next: HashMap::new() }
    }
}

impl<K: Eq + Hash + Clone> RateMeter<K> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `total_ns` for `key` at `now`; the rate since the previous
    /// round, or `None` on a key's first round (or a counter that went
    /// backwards).
    pub fn sample(&mut self, key: K, total_ns: u64, now: Instant) -> Option<f64> {
        let rate = self.prev.get(&key).and_then(|&(was, at)| {
            let wall = now.checked_duration_since(at)?.as_nanos();
            (wall > 0 && total_ns >= was).then(|| (total_ns - was) as f64 / wall as f64)
        });
        self.next.insert(key, (total_ns, now));
        rate
    }

    /// End a round.
    pub fn finish(&mut self) {
        self.prev = std::mem::take(&mut self.next);
    }

    /// Keys remembered from the last finished round.
    pub fn len(&self) -> usize {
        self.prev.len()
    }

    pub fn is_empty(&self) -> bool {
        self.prev.is_empty()
    }
}

/// The executable name in a path, without directory (either separator).
pub fn base_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn proc(pid: u32, ppid: Option<u32>, started: Option<u64>) -> ProcInfo {
        ProcInfo {
            pid,
            ppid,
            start_key: started.unwrap_or(0),
            started_at_ms: started,
            name: String::new(),
            cpu_ns: None,
            mem_private: None,
            mem_resident: None,
            mem_commit: None,
        }
    }

    #[test]
    fn rate_needs_two_rounds_and_is_a_fraction_of_one_core() {
        let mut m = RateMeter::new();
        let t0 = Instant::now();
        assert_eq!(m.sample("a", 1_000_000_000, t0), None, "first round has no rate");
        m.finish();
        // 1.5 s of CPU over 1 s of wall: one and a half cores.
        let r = m.sample("a", 2_500_000_000, t0 + Duration::from_secs(1)).unwrap();
        assert!((r - 1.5).abs() < 1e-9, "{r}");
    }

    #[test]
    fn a_key_missing_from_a_round_starts_over() {
        let mut m = RateMeter::new();
        let t0 = Instant::now();
        m.sample(1, 100, t0);
        m.finish();
        m.finish(); // key 1 not sampled: forgotten
        assert_eq!(m.sample(1, 200, t0 + Duration::from_secs(2)), None);
        assert!(m.is_empty(), "nothing finished yet this round");
    }

    #[test]
    fn a_counter_that_goes_backwards_has_no_rate() {
        let mut m = RateMeter::new();
        let t0 = Instant::now();
        m.sample(1, 500, t0);
        m.finish();
        assert_eq!(m.sample(1, 100, t0 + Duration::from_secs(1)), None);
    }

    #[test]
    fn a_parent_that_started_after_its_child_is_a_reused_pid() {
        let parent = proc(10, None, Some(2_000));
        let child = proc(11, Some(10), Some(1_000));
        assert!(!child.is_child_of(&parent));
        let real_parent = proc(10, None, Some(500));
        assert!(child.is_child_of(&real_parent));
        assert!(!child.is_child_of(&proc(12, None, Some(0))), "wrong pid");
        assert!(proc(11, Some(10), None).is_child_of(&parent), "unknown start times trust the pid");
    }

    #[test]
    fn base_name_strips_either_separator() {
        assert_eq!(base_name(r"C:\Program Files\nodejs\node.exe"), "node.exe");
        assert_eq!(base_name("/usr/bin/node"), "node");
        assert_eq!(base_name("node"), "node");
    }

    /// The real reader on this machine: it sees this test process, measured,
    /// with a start time and a parent.
    #[test]
    fn snapshot_sees_this_process_with_cpu_and_memory() {
        let me = std::process::id();
        let all = snapshot().expect("snapshot");
        assert!(all.len() > 1, "only {} processes", all.len());
        let p = all.iter().find(|p| p.pid == me).expect("this process is listed");
        assert!(p.cpu_ns.is_some(), "{p:?}");
        assert!(p.mem_private.unwrap_or(0) > 0, "{p:?}");
        assert!(p.mem_resident.unwrap_or(0) >= p.mem_private.unwrap_or(0) / 4, "{p:?}");
        assert!(p.ppid.is_some(), "{p:?}");
        assert!(!p.name.is_empty(), "{p:?}");
        let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
        let started = p.started_at_ms.expect("start time");
        assert!(started <= now_ms + 5_000 && started > now_ms - 24 * 3600 * 1000, "started {started}, now {now_ms}");
        // Two snapshots agree on identity.
        let again = snapshot().unwrap();
        assert_eq!(again.iter().find(|q| q.pid == me).unwrap().key(), p.key());
    }

    #[test]
    fn cpu_time_grows_while_this_process_spins() {
        let me = std::process::id();
        let cpu = || snapshot().unwrap().into_iter().find(|p| p.pid == me).and_then(|p| p.cpu_ns).unwrap();
        let before = cpu();
        let spin_until = Instant::now() + Duration::from_millis(300);
        let mut x = 0u64;
        while Instant::now() < spin_until {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
        }
        std::hint::black_box(x);
        let after = cpu();
        // At least a third of the spin shows up (coarse clocks: 10-16 ms ticks).
        assert!(after >= before + 100_000_000, "before {before} after {after}");
        // And not absurdly more (a units bug, e.g. Mach ticks read as ns).
        assert!(after < before + 30_000_000_000, "before {before} after {after}");
    }

    #[test]
    fn command_line_of_this_process_names_its_executable() {
        let line = command_line(std::process::id()).expect("own command line");
        let exe = std::env::current_exe().unwrap();
        let stem = exe.file_stem().unwrap().to_string_lossy().to_string();
        assert!(line.contains(&stem), "{line:?} lacks {stem:?}");
        assert_eq!(command_line(u32::MAX - 7), None);
    }

    #[test]
    fn cpu_count_is_at_least_what_this_process_may_use() {
        let usable = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        assert!(cpu_count() >= usable, "{} < {usable}", cpu_count());
    }
}
