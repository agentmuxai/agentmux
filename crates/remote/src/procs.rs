// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The host's processes for Tower (`fsproto::Request::Procs`,
//! SPEC_TOWER_TASK_MANAGER_PANE_2026_10_08.md §8): sampled here, on the host,
//! with the same readers srv uses locally (`agentmux_procstats`), so only a
//! small frame crosses the link.
//!
//! - **Small:** the `top` busiest by CPU and the `top` largest by memory
//!   (the union, at most `2 × top` rows), plus totals for all of them. A
//!   filter is applied here first, so a search finds a process that isn't in
//!   either top list.
//! - **No command lines.** Names only: a command line can hold a secret, and
//!   never leaves the host through this.
//! - **Rates on the host:** CPU is the change since the previous `Procs` on
//!   this connection, so nothing but the frame is sent and srv keeps no
//!   per-host state.
//!
//! `PaneProcs` (a terminal's stats badge,
//! SPEC_TERMINAL_STATS_BADGE_REMOTE_PANES_2026_10_12.md) sums one AgentMux
//! pane's processes from the same measurement: the tree under a durable
//! session's shell, or, for a WSL or plain SSH pane, every process whose
//! environment holds `AGENTMUX_BLOCKID=<pane>` and what those started. Each
//! process's environment is read once (Linux: `/proc/<pid>/environ`, the
//! user's own processes only) and remembered while it runs.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

use agentmux_procstats::{ProcInfo, ProcKey, RateMeter};

use crate::fsproto::{PaneFound, PaneFrame, ProcFrame, ProcRow};

/// Most rows per list a request may ask for.
pub const MAX_TOP: u32 = 500;

/// A request this soon after the last measurement reuses it (with its own
/// filter): two Tower panes on one host, whatever their filters, must not
/// shrink each other's CPU window to milliseconds.
pub const REUSE_WITHIN: std::time::Duration = std::time::Duration::from_millis(900);

/// Most processes one pane's tree counts (as srv's badge does for a local
/// pane): a fork bomb shouldn't stall a sample.
pub const MAX_PANE_PROCS: usize = 64;

/// The variable that names the AgentMux pane a process belongs to.
const PANE_MARKER: &[u8] = b"AGENTMUX_BLOCKID=";

/// One connection's sampler: the previous round's CPU times, and the last
/// measurement for requests that arrive within [`REUSE_WITHIN`] of it.
#[derive(Default)]
pub struct Sampler {
    rates: RateMeter<ProcKey>,
    last: Option<(Instant, Vec<ProcInfo>, Vec<Option<f64>>)>,
    /// Each running process's pane (`AGENTMUX_BLOCKID`), `None` for none or
    /// unreadable, read once.
    markers: HashMap<ProcKey, Option<String>>,
}

impl Sampler {
    pub fn new() -> Self {
        Self::default()
    }

    /// A frame of the host's processes now (module doc). A host whose
    /// process table can't be read answers with an empty frame.
    pub fn frame(&mut self, top: u32, filter: &str) -> ProcFrame {
        self.measure();
        let (_, procs, rates) = self.last.as_ref().expect("measured above");
        select(procs, rates, top, filter, agentmux_procstats::cpu_count() as u32)
    }

    /// What one pane's processes use (module doc): the tree under
    /// `session_pid` (a durable session's shell, from the daemon) when it is
    /// running, else the processes marked with `block_id` and theirs.
    pub fn pane(&mut self, block_id: &str, session_pid: Option<u32>) -> PaneFrame {
        self.measure();
        let Self { last, markers, .. } = self;
        let (_, procs, rates) = last.as_ref().expect("measured above");
        let cpu_count = agentmux_procstats::cpu_count() as u32;
        if let Some(root) = session_pid.and_then(|pid| procs.iter().position(|p| p.pid == pid)) {
            return pane_frame(procs, rates, &[root], PaneFound::Session, cpu_count);
        }
        let marked: Vec<usize> = if block_id.is_empty() {
            Vec::new()
        } else {
            (0..procs.len())
                .filter(|&i| {
                    let p = &procs[i];
                    markers.entry(p.key()).or_insert_with(|| marker_of(p.pid)).as_deref() == Some(block_id)
                })
                .collect()
        };
        let found = if marked.is_empty() { PaneFound::None } else { PaneFound::Marker };
        pane_frame(procs, rates, &marked, found, cpu_count)
    }

    /// Take a new measurement unless the last is within [`REUSE_WITHIN`].
    fn measure(&mut self) {
        if self.last.as_ref().is_some_and(|(at, _, _)| at.elapsed() < REUSE_WITHIN) {
            return;
        }
        let procs = agentmux_procstats::snapshot().unwrap_or_default();
        let now = Instant::now();
        let rates: Vec<Option<f64>> =
            procs.iter().map(|p| p.cpu_ns.and_then(|ns| self.rates.sample(p.key(), ns, now))).collect();
        self.rates.finish();
        // Forget the markers of processes that have exited.
        let alive: HashSet<ProcKey> = procs.iter().map(ProcInfo::key).collect();
        self.markers.retain(|k, _| alive.contains(k));
        self.last = Some((now, procs, rates));
    }
}

/// The pane a process belongs to: its `AGENTMUX_BLOCKID`. `None` where it has
/// none, can't be read (another user's process), or the OS keeps no
/// environment where this can read it (anything but Linux).
fn marker_of(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let env = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
        marker_in(&env)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        None
    }
}

/// `AGENTMUX_BLOCKID`'s value in a NUL-separated environment.
fn marker_in(env: &[u8]) -> Option<String> {
    env.split(|&b| b == 0)
        .find_map(|kv| kv.strip_prefix(PANE_MARKER))
        .filter(|v| !v.is_empty())
        .map(|v| String::from_utf8_lossy(v).into_owned())
}

/// A pane's frame: `roots` and every process under them (at most
/// [`MAX_PANE_PROCS`]), with the host's totals. The earliest-started root is
/// the pane's shell: its name comes first and its start is the pane's.
pub fn pane_frame(procs: &[ProcInfo], rates: &[Option<f64>], roots: &[usize], found: PaneFound, cpu_count: u32) -> PaneFrame {
    let by_pid: HashMap<u32, usize> = procs.iter().enumerate().map(|(i, p)| (p.pid, i)).collect();
    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, p) in procs.iter().enumerate() {
        if let Some(&parent) = p.ppid.and_then(|pp| by_pid.get(&pp)) {
            if p.is_child_of(&procs[parent]) {
                children.entry(parent).or_default().push(i);
            }
        }
    }
    let mut roots = roots.to_vec();
    roots.sort_by_key(|&i| (procs[i].started_at_ms.unwrap_or(u64::MAX), procs[i].pid));
    let mut tree: Vec<usize> = Vec::new();
    let mut seen: HashSet<usize> = HashSet::new();
    let mut queue: VecDeque<usize> = VecDeque::new();
    for &r in &roots {
        if tree.len() < MAX_PANE_PROCS && seen.insert(r) {
            tree.push(r);
            queue.push_back(r);
        }
    }
    while let Some(i) = queue.pop_front() {
        for &c in children.get(&i).into_iter().flatten() {
            if tree.len() >= MAX_PANE_PROCS {
                break;
            }
            if seen.insert(c) {
                tree.push(c);
                queue.push_back(c);
            }
        }
    }

    let mut names: Vec<(String, u32)> = Vec::new();
    for &i in &tree {
        match names.iter_mut().find(|(n, _)| *n == procs[i].name) {
            Some((_, count)) => *count += 1,
            None => names.push((procs[i].name.clone(), 1)),
        }
    }
    // The shell stays first; the rest, most numerous first.
    if names.len() > 1 {
        names[1..].sort_by(|a, b| b.1.cmp(&a.1));
    }
    let sum = |f: &dyn Fn(usize) -> Option<u64>| tree.iter().filter_map(|&i| f(i)).reduce(|a, b| a + b);
    PaneFrame {
        found,
        processes: tree.len() as u32,
        names,
        started_at_ms: tree.first().and_then(|&i| procs[i].started_at_ms),
        cpu_milli: sum(&|i| rates[i].map(|r| milli(r) as u64)),
        mem: sum(&|i| procs[i].mem_private),
        mem_resident: sum(&|i| procs[i].mem_resident),
        memory_metric: agentmux_procstats::MEMORY_METRIC.to_string(),
        cpu_count,
        host_cpu_milli: rates.iter().flatten().fold(None, |acc: Option<u64>, &r| Some(acc.unwrap_or(0) + milli(r) as u64)),
        host_mem: procs.iter().filter_map(|p| p.mem_private).sum(),
        host_processes: procs.len() as u32,
        host_unmeasured: procs.iter().filter(|p| !p.measured()).count() as u32,
    }
}

fn milli(rate: f64) -> u32 {
    (rate * 1000.0).round().clamp(0.0, (u32::MAX - 1) as f64) as u32
}

/// Whether every word of `filter` is in the name or the PID.
fn matches(p: &ProcInfo, words: &[String]) -> bool {
    let hay = format!("{} {}", p.name.to_lowercase(), p.pid);
    words.iter().all(|w| hay.contains(w.as_str()))
}

/// The frame for `procs` (with each one's CPU rate, fraction of one core):
/// totals over all of them, rows for the busiest and largest that match.
pub fn select(procs: &[ProcInfo], rates: &[Option<f64>], top: u32, filter: &str, cpu_count: u32) -> ProcFrame {
    let top = top.min(MAX_TOP) as usize;
    let words: Vec<String> = filter.to_lowercase().split_whitespace().map(str::to_string).collect();
    let matching: Vec<usize> = (0..procs.len()).filter(|&i| matches(&procs[i], &words)).collect();

    let mut by_cpu = matching.clone();
    by_cpu.sort_by(|&a, &b| rates[b].unwrap_or(-1.0).total_cmp(&rates[a].unwrap_or(-1.0)));
    let mut by_mem = matching.clone();
    by_mem.sort_by_key(|&i| std::cmp::Reverse(procs[i].mem_private.map_or(-1, |m| m as i128)));
    let mut seen = HashSet::new();
    let chosen: Vec<usize> = by_cpu.into_iter().take(top).chain(by_mem.into_iter().take(top)).filter(|i| seen.insert(*i)).collect();

    ProcFrame {
        cpu_count,
        memory_metric: agentmux_procstats::MEMORY_METRIC.to_string(),
        total: procs.len() as u32,
        matched: matching.len() as u32,
        unmeasured: procs.iter().filter(|p| !p.measured()).count() as u32,
        cpu_milli: rates.iter().flatten().fold(None, |acc: Option<u64>, &r| Some(acc.unwrap_or(0) + milli(r) as u64)),
        mem: procs.iter().filter_map(|p| p.mem_private).sum(),
        rows: chosen
            .into_iter()
            .map(|i| {
                let p = &procs[i];
                ProcRow {
                    pid: p.pid,
                    ppid: p.ppid,
                    start_key: p.start_key,
                    started_at_ms: p.started_at_ms,
                    name: p.name.clone(),
                    cpu_milli: rates[i].map(milli),
                    mem: p.mem_private,
                    mem_resident: p.mem_resident,
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, name: &str, mem: Option<u64>) -> ProcInfo {
        ProcInfo {
            pid,
            ppid: Some(1),
            start_key: pid as u64,
            started_at_ms: Some(1),
            name: name.into(),
            cpu_ns: mem.map(|_| 0),
            mem_private: mem,
            mem_resident: mem,
            mem_commit: None,
        }
    }

    fn pids(f: &ProcFrame) -> Vec<u32> {
        let mut v: Vec<u32> = f.rows.iter().map(|r| r.pid).collect();
        v.sort();
        v
    }

    #[test]
    fn the_busiest_and_the_largest_are_sent_with_totals_for_all() {
        let procs = vec![p(1, "init", Some(10)), p(2, "big", Some(9_000)), p(3, "busy", Some(20)), p(4, "idle", Some(30))];
        let rates = vec![Some(0.0), Some(0.01), Some(1.5), None];
        let f = select(&procs, &rates, 1, "", 8);
        assert_eq!(pids(&f), vec![2, 3], "top 1 by CPU (busy) and top 1 by memory (big)");
        assert_eq!((f.total, f.matched, f.cpu_count), (4, 4, 8));
        assert_eq!(f.cpu_milli, Some(1510));
        assert_eq!(f.mem, 9_060);
        let busy = f.rows.iter().find(|r| r.pid == 3).unwrap();
        assert_eq!(busy.cpu_milli, Some(1500));
    }

    #[test]
    fn a_filter_finds_what_neither_top_list_holds() {
        let mut procs: Vec<ProcInfo> = (10..60).map(|i| p(i, "worker", Some(i as u64 * 100))).collect();
        procs.push(p(4242, "sshd", Some(1)));
        let rates = vec![Some(0.1); procs.len()];
        let f = select(&procs, &rates, 5, "SSHD", 2);
        assert_eq!(pids(&f), vec![4242]);
        assert_eq!((f.total, f.matched), (51, 1), "totals still count every process");
        let f = select(&procs, &rates, 5, "4242", 2);
        assert_eq!(pids(&f), vec![4242], "by PID");
    }

    #[test]
    fn an_unmeasured_process_is_counted_and_sent_without_numbers() {
        let procs = vec![p(1, "launchd", None), p(2, "node", Some(5))];
        let f = select(&procs, &[None, Some(0.2)], 10, "", 4);
        assert_eq!(f.unmeasured, 1);
        assert_eq!(select(&procs, &[None, None], 10, "", 4).cpu_milli, None, "no rate yet: unknown, not 0");
        let launchd = f.rows.iter().find(|r| r.pid == 1).unwrap();
        assert_eq!((launchd.cpu_milli, launchd.mem), (None, None));
    }

    fn child(pid: u32, ppid: u32, started: u64, name: &str, mem: u64) -> ProcInfo {
        ProcInfo {
            pid,
            ppid: Some(ppid),
            start_key: started,
            started_at_ms: Some(started),
            name: name.into(),
            cpu_ns: Some(0),
            mem_private: Some(mem),
            mem_resident: Some(mem * 2),
            mem_commit: None,
        }
    }

    /// sshd (1) -> bash (10, the pane's shell) -> node (11) -> node (12);
    /// bash -> git (13); a second marked shell of the same pane (20, a
    /// later `wsl.exe` relaunch); an unrelated process (30).
    fn pane_table() -> Vec<ProcInfo> {
        vec![
            child(1, 0, 100, "sshd", 5),
            child(10, 1, 1_000, "bash", 4),
            child(11, 10, 2_000, "node", 50),
            child(12, 11, 3_000, "node", 30),
            child(13, 10, 2_500, "git", 2),
            child(20, 1, 5_000, "bash", 3),
            child(30, 1, 1_500, "cron", 1),
        ]
    }

    #[test]
    fn a_pane_is_its_roots_and_everything_under_them() {
        let procs = pane_table();
        let rates = vec![Some(0.0), Some(0.01), Some(0.5), Some(0.25), None, Some(0.0), Some(0.0)];
        let f = pane_frame(&procs, &rates, &[5, 1], PaneFound::Marker, 4);
        assert_eq!(f.found, PaneFound::Marker);
        assert_eq!(f.processes, 5, "both shells, two nodes and git; not sshd or cron");
        assert_eq!(f.names, vec![("bash".to_string(), 2), ("node".to_string(), 2), ("git".to_string(), 1)]);
        assert_eq!(f.started_at_ms, Some(1_000), "the earliest root is the shell");
        assert_eq!(f.cpu_milli, Some(10 + 500 + 250));
        assert_eq!(f.mem, Some(4 + 50 + 30 + 2 + 3));
        assert_eq!(f.mem_resident, Some(2 * (4 + 50 + 30 + 2 + 3)));
        assert_eq!((f.cpu_count, f.host_processes, f.host_mem, f.host_unmeasured), (4, 7, 95, 0));
    }

    #[test]
    fn no_roots_is_the_hosts_totals_only() {
        let procs = pane_table();
        let rates = vec![None; procs.len()];
        let f = pane_frame(&procs, &rates, &[], PaneFound::None, 2);
        assert_eq!((f.processes, f.names.len(), f.mem, f.cpu_milli), (0, 0, None, None));
        assert_eq!((f.host_processes, f.host_cpu_milli), (7, None), "no rate yet: unknown, not 0");
        // Another user's process on macOS: listed, but not in the totals.
        let mut procs = pane_table();
        procs[6].cpu_ns = None;
        procs[6].mem_private = None;
        let f = pane_frame(&procs, &rates, &[], PaneFound::None, 2);
        assert_eq!((f.host_processes, f.host_unmeasured, f.host_mem), (7, 1, 94));
    }

    #[test]
    fn a_pane_is_capped() {
        let mut procs = vec![child(1, 0, 1, "bash", 1)];
        procs.extend((2..500).map(|i| child(i, 1, 2, "x", 1)));
        let rates = vec![None; procs.len()];
        assert_eq!(pane_frame(&procs, &rates, &[0], PaneFound::Session, 1).processes, MAX_PANE_PROCS as u32);
    }

    #[test]
    fn the_marker_is_read_from_an_environment() {
        assert_eq!(marker_in(b"HOME=/root\0AGENTMUX_BLOCKID=b-1\0TERM=x\0"), Some("b-1".to_string()));
        assert_eq!(marker_in(b"HOME=/root\0XAGENTMUX_BLOCKID=b-1\0"), None, "a longer name isn't it");
        assert_eq!(marker_in(b"AGENTMUX_BLOCKID=\0"), None);
        assert_eq!(marker_in(b""), None);
    }

    /// The real sampler on Linux: a child started with the marker is the
    /// pane, found by its environment, with what it started; this test
    /// process (unmarked) is not.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_marked_process_and_its_children_are_the_pane() {
        let mut shell = std::process::Command::new("sh")
            .args(["-c", "sleep 30 & sleep 30; true"])
            .env("AGENTMUX_BLOCKID", "test-pane-procs")
            .spawn()
            .expect("spawn sh");
        std::thread::sleep(std::time::Duration::from_millis(300));
        let mut s = Sampler::new();
        let f = s.pane("test-pane-procs", None);
        let _ = shell.kill();
        let _ = shell.wait();
        assert_eq!(f.found, PaneFound::Marker, "{f:?}");
        assert!(f.processes >= 2, "sh and its sleeps: {f:?}");
        assert_eq!(f.names.first().map(|n| n.0.as_str()), Some("sh"), "{f:?}");
        let none = s.pane("no-such-pane", None);
        assert_eq!((none.found, none.processes), (PaneFound::None, 0));
        assert!(none.host_processes > 1);
    }

    #[test]
    fn a_session_shell_is_found_by_its_pid() {
        let mut s = Sampler::new();
        let f = s.pane("", Some(std::process::id()));
        assert_eq!(f.found, PaneFound::Session, "{f:?}");
        assert!(f.processes >= 1);
        let gone = s.pane("", Some(u32::MAX - 3));
        assert_eq!(gone.found, PaneFound::None, "a shell that isn't running");
    }

    #[test]
    fn top_is_capped() {
        let procs: Vec<ProcInfo> = (1..=2000).map(|i| p(i, "x", Some(i as u64))).collect();
        let rates = vec![Some(0.0); procs.len()];
        assert!(select(&procs, &rates, u32::MAX, "", 1).rows.len() <= 2 * MAX_TOP as usize);
    }

    /// The real sampler: this test process is in its own host's frame; a
    /// request within the reuse window, whatever its filter, reuses the
    /// measurement (no new round); a later one has rates.
    #[test]
    fn a_real_frame_and_its_rates() {
        let mut s = Sampler::new();
        let me = std::process::id();
        let first = s.frame(MAX_TOP, &me.to_string());
        assert!(first.rows.iter().any(|r| r.pid == me), "{first:?}");
        let other_filter = s.frame(MAX_TOP, "");
        assert_eq!(other_filter.total, first.total, "the same measurement");
        assert!(other_filter.rows.iter().all(|r| r.cpu_milli.is_none()), "still no previous round");
        std::thread::sleep(REUSE_WITHIN + std::time::Duration::from_millis(50));
        let second = s.frame(MAX_TOP, &me.to_string());
        assert!(second.rows.iter().find(|r| r.pid == me).unwrap().cpu_milli.is_some());
        assert!(second.total > 1);
    }
}
