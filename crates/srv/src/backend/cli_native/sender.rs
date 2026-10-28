// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who sent a CLI-native message, from the process id the CLI verified for the
//! sender's connection to the receiving session's socket (`SO_PEERCRED`, in the
//! frame's `origin.verifiedPeerPid`).
//!
//! Everything else in the message is the sender's own words: its name, its
//! address, its mode, its text. Any process of the same user can connect to a
//! session's socket and say what it likes, but none can choose the pid the
//! kernel reports. So the sender is the agent whose CLI process that pid
//! belongs to, or descends from, in srv's own table, and a pid outside every
//! agent is an outside session whatever name it gives.

/// What AgentMux knows about the process behind a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sender {
    /// The pid is, or descends from, the CLI process srv runs for this block.
    Agent { block_id: String },
    /// A live process that belongs to no agent pane srv runs.
    Outside { pid: u32 },
    /// No verified pid, a pid that is gone, or one the system has recycled.
    Unverified,
}

/// The parts of the process table the mapping reads.
pub trait ProcView {
    /// The parent of `pid`; `None` when the process does not exist.
    fn parent(&self, pid: u32) -> Option<u32>;
    /// The process's start time in the unit the CLI reports it (Linux: clock
    /// ticks since boot, `/proc/<pid>/stat` field 22); `None` when unknown.
    fn start_time(&self, pid: u32) -> Option<u64>;
}

/// Longest parent chain walked.
const MAX_DEPTH: usize = 64;

/// Maps the CLI-verified connecting process to a sender. `cli_pids` is srv's
/// (block, CLI process) table; `proc_start` the start time the CLI reported.
pub fn resolve(pid: Option<u32>, proc_start: Option<&str>, cli_pids: &[(String, u32)], procs: &dyn ProcView) -> Sender {
    let Some(pid) = pid.filter(|p| *p > 1) else { return Sender::Unverified };
    // A pid is only a name for a process while it keeps the start time it had.
    if let (Some(reported), Some(now)) = (proc_start.and_then(|s| s.parse::<u64>().ok()), procs.start_time(pid)) {
        if reported != now {
            return Sender::Unverified;
        }
    }
    let mut cur = pid;
    for _ in 0..MAX_DEPTH {
        if let Some((block, _)) = cli_pids.iter().find(|(_, p)| *p == cur) {
            return Sender::Agent { block_id: block.clone() };
        }
        match procs.parent(cur) {
            Some(parent) if parent > 1 && parent != cur => cur = parent,
            _ => break,
        }
    }
    // Gone before it could be looked at: nothing to say it was outside.
    if procs.parent(pid).is_none() {
        return Sender::Unverified;
    }
    Sender::Outside { pid }
}

/// The running system's process table.
pub struct SystemProcs;

#[cfg(target_os = "linux")]
impl ProcView for SystemProcs {
    fn parent(&self, pid: u32) -> Option<u32> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        stat_fields(&stat).get(1)?.parse().ok()
    }

    fn start_time(&self, pid: u32) -> Option<u64> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        stat_fields(&stat).get(19)?.parse().ok()
    }
}

/// The fields of a `/proc/<pid>/stat` line after the process name, which can
/// hold spaces and brackets: index 0 is the state, 1 the parent, 19 the start time.
#[cfg(target_os = "linux")]
fn stat_fields(stat: &str) -> Vec<&str> {
    stat.rsplit_once(')').map(|(_, rest)| rest.split_whitespace().collect()).unwrap_or_default()
}

#[cfg(not(target_os = "linux"))]
impl ProcView for SystemProcs {
    fn parent(&self, pid: u32) -> Option<u32> {
        use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
        let mut sys = System::new();
        sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true, ProcessRefreshKind::nothing());
        sys.process(Pid::from_u32(pid)).map(|p| p.parent().map(|pp| pp.as_u32()).unwrap_or(0))
    }

    // The CLI's start-time unit is not known here, so the recycled-pid check is skipped.
    fn start_time(&self, _pid: u32) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Table {
        parents: HashMap<u32, u32>,
        starts: HashMap<u32, u64>,
    }

    impl ProcView for Table {
        fn parent(&self, pid: u32) -> Option<u32> {
            self.parents.get(&pid).copied()
        }
        fn start_time(&self, pid: u32) -> Option<u64> {
            self.starts.get(&pid).copied()
        }
    }

    // agent A's CLI is 100 (child 101, grandchild 102); agent B's CLI is 200; 300 is a
    // terminal session of the operator under init; 400's parent chain ends in a pid that is gone.
    fn table() -> Table {
        Table {
            parents: HashMap::from([(100, 10), (101, 100), (102, 101), (200, 10), (300, 20), (400, 399)]),
            starts: HashMap::from([(100, 5_000), (101, 5_100), (200, 6_000), (300, 7_000), (400, 8_000)]),
        }
    }

    fn cli() -> Vec<(String, u32)> {
        vec![("block-a".into(), 100), ("block-b".into(), 200)]
    }

    #[test]
    fn the_cli_process_maps_to_its_agent() {
        assert_eq!(resolve(Some(100), Some("5000"), &cli(), &table()), Sender::Agent { block_id: "block-a".into() });
    }

    #[test]
    fn a_child_or_grandchild_of_the_cli_process_maps_to_the_same_agent() {
        assert_eq!(resolve(Some(101), Some("5100"), &cli(), &table()), Sender::Agent { block_id: "block-a".into() });
        assert_eq!(resolve(Some(102), None, &cli(), &table()), Sender::Agent { block_id: "block-a".into() });
    }

    #[test]
    fn another_agents_pid_maps_to_that_agent_whatever_name_is_claimed() {
        // The claimed name is not an input: B's process is B, however the message signs itself.
        assert_eq!(resolve(Some(200), Some("6000"), &cli(), &table()), Sender::Agent { block_id: "block-b".into() });
    }

    #[test]
    fn a_pid_outside_every_agent_is_an_outside_session() {
        assert_eq!(resolve(Some(300), Some("7000"), &cli(), &table()), Sender::Outside { pid: 300 });
    }

    #[test]
    fn a_recycled_pid_is_unverified() {
        // The CLI reported start time 5000 for pid 100; the process now at 100 started at 9999.
        let mut t = table();
        t.starts.insert(100, 9_999);
        assert_eq!(resolve(Some(100), Some("5000"), &cli(), &t), Sender::Unverified);
    }

    #[test]
    fn a_missing_a_system_or_a_vanished_pid_is_unverified() {
        assert_eq!(resolve(None, None, &cli(), &table()), Sender::Unverified);
        assert_eq!(resolve(Some(0), None, &cli(), &table()), Sender::Unverified);
        assert_eq!(resolve(Some(1), None, &cli(), &table()), Sender::Unverified);
        assert_eq!(resolve(Some(777), Some("1"), &cli(), &table()), Sender::Unverified);
    }

    #[test]
    fn a_parent_cycle_ends_the_walk() {
        let mut t = table();
        t.parents.insert(500, 501);
        t.parents.insert(501, 500);
        assert_eq!(resolve(Some(500), None, &cli(), &t), Sender::Outside { pid: 500 });
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_running_system_reads_its_own_process() {
        let me = std::process::id();
        let parent = SystemProcs.parent(me).expect("own parent");
        assert!(parent > 0);
        assert!(SystemProcs.start_time(me).is_some());
        assert_eq!(SystemProcs.parent(u32::MAX - 7), None);
    }
}
