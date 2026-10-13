// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a terminal's stats badge tooltip shows beyond the badge's own CPU and
//! memory (`blockstats.detail`), and the badge's peak memory.
//!
//! The badge (`sysinfo::run_sysinfo_loop`) sums its pane's shell and every
//! process under it. The tooltip describes the same processes: their names,
//! when the shell started, and two more memory figures from one
//! `agentmux_procstats` snapshot, private and committed. A snapshot costs
//! tens of milliseconds, so it is taken only while a tooltip is open, never
//! on the badge's every-second tick.
//!
//! A remote pane (SSH, WSL) is measured on the machine it is connected to,
//! by AgentMux's helper there (`PaneProcs`); [`remote`] turns its answer
//! into the badge's figures, with a peak kept here.
//! SPEC_TERMINAL_STATS_BADGE_REMOTE_PANES_2026_10_12.md.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use agentmux_procstats::ProcInfo;
use agentmux_remote::fsproto::{PaneFound, PaneFrame};

use crate::backend::rpc_types::{BlockStatsFigures, BlockStatsHost, BlockStatsName, BlockStatsRemote};

/// The tooltip's numbers for the process tree under `root_pid`, at most
/// `max` processes (the badge's own cap). `None` when the shell isn't in the
/// snapshot (it exited).
pub fn detail(snap: &[ProcInfo], root_pid: u32, max: usize) -> Option<BlockStatsFigures> {
    let root = snap.iter().position(|p| p.pid == root_pid)?;
    let by_pid: HashMap<u32, usize> = snap.iter().enumerate().map(|(i, p)| (p.pid, i)).collect();
    // Parent → children, trusting a parent only if it really is one (a
    // Windows parent PID can name a newer process that reused it).
    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, p) in snap.iter().enumerate() {
        if let Some(&parent) = p.ppid.and_then(|pp| by_pid.get(&pp)) {
            if p.is_child_of(&snap[parent]) {
                children.entry(parent).or_default().push(i);
            }
        }
    }
    let mut tree = vec![root];
    let mut queue = VecDeque::from([root]);
    'walk: while let Some(i) = queue.pop_front() {
        for &c in children.get(&i).into_iter().flatten() {
            if tree.len() >= max {
                break 'walk;
            }
            if !tree.contains(&c) {
                tree.push(c);
                queue.push_back(c);
            }
        }
    }

    let sum = |f: fn(&ProcInfo) -> Option<u64>| -> Option<u64> {
        tree.iter().filter_map(|&i| f(&snap[i])).reduce(|a, b| a + b)
    };
    let mut names: Vec<BlockStatsName> = Vec::new();
    for &i in &tree {
        let name = display_name(&snap[i].name);
        match names.iter_mut().find(|n| n.name == name) {
            Some(n) => n.count += 1,
            None => names.push(BlockStatsName { name: name.to_string(), count: 1 }),
        }
    }
    // The shell stays first; the rest, most numerous first (stable: ties keep
    // the order the walk met them in).
    names[1..].sort_by(|a, b| b.count.cmp(&a.count));

    Some(BlockStatsFigures {
        processes: tree.len() as u32,
        names,
        started_at_ms: snap[root].started_at_ms,
        cpu: None,
        mem_resident: None,
        mem_private: sum(|p| p.mem_private),
        mem_commit: sum(|p| p.mem_commit),
        memory_metric: agentmux_procstats::MEMORY_METRIC.to_string(),
    })
}

/// A remote pane's helper answer as the badge's figures, with its peak.
pub fn remote(block_id: &str, f: PaneFrame) -> BlockStatsRemote {
    let percent = |milli: u64| milli as f64 / 10.0;
    let host = BlockStatsHost {
        cpu_count: f.cpu_count,
        cpu: f.host_cpu_milli.map(percent),
        mem: f.host_mem,
        processes: f.host_processes,
        unmeasured: f.host_unmeasured,
        memory_metric: f.memory_metric.clone(),
    };
    let found = match f.found {
        PaneFound::Session => "session",
        PaneFound::Marker => "marker",
        PaneFound::None => "none",
    };
    let figures = (f.found != PaneFound::None).then(|| BlockStatsFigures {
        processes: f.processes,
        names: f.names.into_iter().map(|(name, count)| BlockStatsName { name, count }).collect(),
        started_at_ms: f.started_at_ms,
        cpu: f.cpu_milli.map(percent),
        mem_resident: f.mem_resident,
        mem_private: f.mem,
        mem_commit: None,
        memory_metric: f.memory_metric,
    });
    let peak = figures.as_ref().and_then(|fig| {
        let mem = fig.mem_resident?;
        Some(remote_peaks().lock().unwrap().observe(block_id, (0, fig.started_at_ms.unwrap_or(0)), mem))
    });
    BlockStatsRemote { found: found.to_string(), figures, host: Some(host), peak, reason: None }
}

/// A remote pane whose machine couldn't be asked, and why.
pub fn remote_unavailable(reason: String) -> BlockStatsRemote {
    BlockStatsRemote { found: "unavailable".to_string(), figures: None, host: None, peak: None, reason: Some(reason) }
}

/// Remote panes' peaks, by pane and shell (its start: the PID is the other
/// machine's). A pane not asked about for [`REMOTE_PEAK_IDLE`] is forgotten.
fn remote_peaks() -> &'static Mutex<RemotePeaks> {
    static PEAKS: OnceLock<Mutex<RemotePeaks>> = OnceLock::new();
    PEAKS.get_or_init(Default::default)
}

const REMOTE_PEAK_IDLE: Duration = Duration::from_secs(600);

#[derive(Default)]
struct RemotePeaks {
    peaks: Peaks,
    seen: HashMap<String, Instant>,
}

impl RemotePeaks {
    fn observe(&mut self, block_id: &str, shell: Shell, mem: u64) -> u64 {
        let now = Instant::now();
        self.seen.insert(block_id.to_string(), now);
        self.seen.retain(|_, at| now.duration_since(*at) < REMOTE_PEAK_IDLE);
        let seen = &self.seen;
        self.peaks.retain(|id| seen.contains_key(id));
        self.peaks.observe(block_id, shell, mem)
    }
}

/// An executable's name as a person calls it: `pwsh`, not `pwsh.exe`.
fn display_name(name: &str) -> &str {
    let n = name.len();
    if n > 4 && name.is_char_boundary(n - 4) && name[n - 4..].eq_ignore_ascii_case(".exe") {
        &name[..n - 4]
    } else {
        name
    }
}

/// The highest memory each pane's badge has shown since AgentMux started
/// watching its shell. A new shell in the same pane (a restart) starts over.
#[derive(Debug, Default)]
pub struct Peaks {
    by_block: HashMap<String, (Shell, u64)>,
}

/// Which shell a peak is for: its PID and start time, so a restarted shell
/// that got the old one's PID back is still a new shell.
pub type Shell = (u32, u64);

impl Peaks {
    /// Record the badge's `mem` for `block_id`, whose shell is `shell`; its
    /// peak so far.
    pub fn observe(&mut self, block_id: &str, shell: Shell, mem: u64) -> u64 {
        let entry = self.by_block.entry(block_id.to_string()).or_insert((shell, mem));
        if entry.0 != shell {
            *entry = (shell, mem);
        }
        entry.1 = entry.1.max(mem);
        entry.1
    }

    /// Forget every pane `keep` says no longer has stats.
    pub fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        self.by_block.retain(|id, _| keep(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: u32, ppid: Option<u32>, started: u64, name: &str, private: u64, commit: Option<u64>) -> ProcInfo {
        ProcInfo {
            pid,
            ppid,
            start_key: started,
            started_at_ms: Some(started),
            name: name.to_string(),
            cpu_ns: Some(0),
            mem_private: Some(private),
            mem_resident: Some(private * 2),
            mem_commit: commit,
        }
    }

    fn table() -> Vec<ProcInfo> {
        vec![
            proc(1, None, 100, "agentmux-srv.exe", 500, Some(900)),
            proc(10, Some(1), 1_000, "pwsh.exe", 10, Some(90)),
            proc(11, Some(10), 2_000, "node.exe", 20, Some(40)),
            proc(12, Some(11), 3_000, "node.exe", 5, Some(8)),
            proc(13, Some(10), 2_500, "git.exe", 1, Some(2)),
            // Claims pwsh as its parent but started before it: a reused PID.
            proc(14, Some(10), 500, "stranger.exe", 999, Some(999)),
            proc(20, Some(1), 1_500, "bash", 7, None),
        ]
    }

    #[test]
    fn sums_the_shell_and_everything_under_it() {
        let d = detail(&table(), 10, 64).unwrap();
        assert_eq!(d.processes, 4);
        assert_eq!(d.started_at_ms, Some(1_000));
        assert_eq!(d.mem_private, Some(10 + 20 + 5 + 1));
        assert_eq!(d.mem_commit, Some(90 + 40 + 8 + 2));
        assert_eq!(d.memory_metric, agentmux_procstats::MEMORY_METRIC);
    }

    #[test]
    fn names_the_shell_first_then_the_most_numerous() {
        let d = detail(&table(), 10, 64).unwrap();
        let names: Vec<(&str, u32)> = d.names.iter().map(|n| (n.name.as_str(), n.count)).collect();
        assert_eq!(names, [("pwsh", 1), ("node", 2), ("git", 1)]);
    }

    #[test]
    fn a_figure_no_process_reports_is_absent_not_zero() {
        let d = detail(&table(), 20, 64).unwrap();
        assert_eq!(d.processes, 1);
        assert_eq!(d.mem_private, Some(7));
        assert_eq!(d.mem_commit, None, "committed memory is Windows-only");
    }

    #[test]
    fn stops_at_the_badges_cap() {
        let d = detail(&table(), 10, 2).unwrap();
        assert_eq!(d.processes, 2);
    }

    #[test]
    fn an_exited_shell_has_no_detail() {
        assert_eq!(detail(&table(), 99, 64), None);
    }

    #[test]
    fn display_name_drops_only_a_trailing_exe() {
        assert_eq!(display_name("pwsh.exe"), "pwsh");
        assert_eq!(display_name("Code.EXE"), "Code");
        assert_eq!(display_name("bash"), "bash");
        assert_eq!(display_name(".exe"), ".exe");
        assert_eq!(display_name("exe"), "exe");
    }

    fn frame(found: PaneFound, started: u64, resident: u64) -> PaneFrame {
        PaneFrame {
            found,
            processes: 3,
            names: vec![("bash".into(), 1), ("node".into(), 2)],
            started_at_ms: Some(started),
            cpu_milli: Some(1_250),
            mem: Some(40 << 20),
            mem_resident: Some(resident),
            memory_metric: "anonymous resident + swap".into(),
            cpu_count: 8,
            host_cpu_milli: Some(3_000),
            host_mem: 6 << 30,
            host_processes: 400,
            host_unmeasured: 0,
        }
    }

    #[test]
    fn a_remote_answer_becomes_the_badges_figures_and_peak() {
        let r = remote("rb-1", frame(PaneFound::Session, 1_000, 90));
        assert_eq!(r.found, "session");
        let fig = r.figures.unwrap();
        assert_eq!((fig.cpu, fig.mem_resident, fig.processes), (Some(125.0), Some(90), 3));
        assert_eq!(fig.names[0].name, "bash");
        let host = r.host.unwrap();
        assert_eq!((host.cpu, host.cpu_count, host.processes), (Some(300.0), 8, 400));
        assert_eq!(r.peak, Some(90));
        assert_eq!(remote("rb-1", frame(PaneFound::Session, 1_000, 60)).peak, Some(90), "the peak holds");
        assert_eq!(remote("rb-1", frame(PaneFound::Marker, 2_000, 50)).peak, Some(50), "a new shell starts over");
    }

    #[test]
    fn a_remote_pane_found_by_neither_way_has_only_the_hosts_figures() {
        let r = remote("rb-2", frame(PaneFound::None, 0, 0));
        assert_eq!((r.found.as_str(), r.figures, r.peak), ("none", None, None));
        assert!(r.host.is_some());
        let u = remote_unavailable("no helper".into());
        assert_eq!((u.found.as_str(), u.reason.as_deref(), u.host), ("unavailable", Some("no helper"), None));
    }

    #[test]
    fn a_peak_holds_until_the_shell_changes() {
        let mut peaks = Peaks::default();
        assert_eq!(peaks.observe("b", (10, 1_000), 100), 100);
        assert_eq!(peaks.observe("b", (10, 1_000), 180), 180);
        assert_eq!(peaks.observe("b", (10, 1_000), 90), 180, "a trim doesn't lower the peak");
        assert_eq!(peaks.observe("b", (11, 2_000), 50), 50, "a restarted shell starts over");
        assert_eq!(peaks.observe("b", (11, 3_000), 40), 40, "even with the old shell's PID");
        peaks.retain(|id| id != "b");
        assert_eq!(peaks.observe("b", (11, 3_000), 30), 30, "a forgotten pane starts over");
    }
}
