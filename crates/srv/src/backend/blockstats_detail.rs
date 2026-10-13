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

use std::collections::{HashMap, VecDeque};

use agentmux_procstats::ProcInfo;

use crate::backend::rpc_types::{BlockStatsDetail, BlockStatsName};

/// The tooltip's numbers for the process tree under `root_pid`, at most
/// `max` processes (the badge's own cap). `None` when the shell isn't in the
/// snapshot (it exited).
pub fn detail(snap: &[ProcInfo], root_pid: u32, max: usize) -> Option<BlockStatsDetail> {
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

    Some(BlockStatsDetail {
        processes: tree.len() as u32,
        names,
        started_at_ms: snap[root].started_at_ms,
        mem_private: sum(|p| p.mem_private),
        mem_commit: sum(|p| p.mem_commit),
        memory_metric: agentmux_procstats::MEMORY_METRIC.to_string(),
        connection: None,
    })
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
    by_block: HashMap<String, (u32, u64)>,
}

impl Peaks {
    /// Record the badge's `mem` for `block_id`; its peak so far.
    pub fn observe(&mut self, block_id: &str, root_pid: u32, mem: u64) -> u64 {
        let entry = self.by_block.entry(block_id.to_string()).or_insert((root_pid, mem));
        if entry.0 != root_pid {
            *entry = (root_pid, mem);
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

    #[test]
    fn a_peak_holds_until_the_shell_changes() {
        let mut peaks = Peaks::default();
        assert_eq!(peaks.observe("b", 10, 100), 100);
        assert_eq!(peaks.observe("b", 10, 180), 180);
        assert_eq!(peaks.observe("b", 10, 90), 180, "a trim doesn't lower the peak");
        assert_eq!(peaks.observe("b", 11, 50), 50, "a restarted shell starts over");
        peaks.retain(|id| id != "b");
        assert_eq!(peaks.observe("b", 11, 40), 40, "a forgotten pane starts over");
    }
}
