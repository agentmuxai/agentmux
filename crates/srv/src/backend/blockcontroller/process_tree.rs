// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Process tree traversal: BFS from a root PID to collect all descendant PIDs.
//! Used by the sysinfo loop to aggregate CPU/memory across entire process trees.

use std::collections::{HashMap, VecDeque};

use sysinfo::{Pid, System};

/// Maximum number of PIDs to track per block (safety cap against pathological trees).
pub const MAX_PIDS_PER_BLOCK: usize = 64;

/// Returns the root PID plus all descendant PIDs via BFS, capped at `max_pids`.
///
/// Requires `sys` to have been refreshed with at least a minimal `ProcessRefreshKind`
/// (so that `Process::parent()` is populated for all processes).
pub fn collect_descendants(sys: &System, root: Pid, max_pids: usize) -> Vec<Pid> {
    // Build parent → children adjacency map in O(N) over all known processes.
    let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
    for (pid, proc) in sys.processes() {
        if let Some(ppid) = proc.parent() {
            children.entry(ppid).or_default().push(*pid);
        }
    }

    // BFS from root, stopping when we hit max_pids.
    let mut result = Vec::with_capacity(max_pids.min(8));
    result.push(root);
    let mut queue = VecDeque::from([root]);

    while let Some(pid) = queue.pop_front() {
        if result.len() >= max_pids {
            break;
        }
        if let Some(kids) = children.get(&pid) {
            for &child in kids {
                if result.len() >= max_pids {
                    break;
                }
                result.push(child);
                queue.push_back(child);
            }
        }
    }

    result
}
