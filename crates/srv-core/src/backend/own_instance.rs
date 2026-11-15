// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which processes belong to THIS AgentMux instance.
//!
//! The commit attribution log (`sysinfo::log_memory_attribution`) used to add up
//! every process named `agentmux*` on the machine, so a stable install running
//! beside a `task dev` build, or two channels, were mixed into one number.
//! An instance is its launcher and everything under it: the launcher starts srv
//! and the CEF host, and the host starts the GPU, utility and renderer
//! processes. This module finds that tree from srv's own pid, as a pure
//! function over a process table so it can be tested without a real one.
//! docs/specs/SPEC_INSTANCE_MEMORY_ATTRIBUTION_2026_10_18.md

use std::collections::{HashMap, HashSet};

/// One row of the process table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcNode {
    pub pid: u32,
    pub parent: Option<u32>,
    /// Whether the process is named `agentmux*`.
    pub is_agentmux: bool,
    /// When it started, unix ms; `None` when unknown. A parent link can be
    /// stale (on Windows it is never updated and can name a newer process that
    /// reused the parent's pid), and a parent that started after its child is
    /// exactly that.
    pub started_at_ms: Option<u64>,
}

impl ProcNode {
    /// Whether `parent` is really this process's parent: the link names its pid
    /// and, where both start times are known, the parent did not start later
    /// (the rule `agentmux_procstats::ProcInfo::is_child_of` applies).
    fn is_child_of(&self, parent: &ProcNode) -> bool {
        if self.parent != Some(parent.pid) || parent.pid == self.pid {
            return false;
        }
        match (parent.started_at_ms, self.started_at_ms) {
            (Some(p), Some(c)) => p <= c,
            _ => true,
        }
    }
}

/// The pids of the instance `self_pid` (srv) belongs to: the topmost `agentmux*`
/// ancestor reached by walking up from srv (the launcher; srv itself when its
/// parent isn't AgentMux, as when it is run by hand), and every process below
/// it. The walk stops at the first parent that isn't `agentmux*`, so the
/// terminal, `task` or AppImageLauncher that started the instance is not part
/// of it. Empty when `self_pid` isn't in the table.
///
/// Another AgentMux started from inside this one, say an agent running
/// `task dev` in a pane's shell, is below this instance's root too, but it is
/// its own instance: an `agentmux*` process whose parent is not `agentmux*`
/// is a launcher, so it and everything under it are left out.
///
/// The table's parent links can be stale (a parent that exited and whose pid
/// was reused), so the upward walk is bounded by the table's size and a seen
/// set rather than trusted to end.
pub fn own_instance_pids(nodes: &[ProcNode], self_pid: u32) -> HashSet<u32> {
    let by_pid: HashMap<u32, &ProcNode> = nodes.iter().map(|n| (n.pid, n)).collect();
    let Some(&start) = by_pid.get(&self_pid) else {
        return HashSet::new();
    };

    let mut root = start;
    let mut seen = HashSet::from([start.pid]);
    while let Some(parent) = root.parent.and_then(|p| by_pid.get(&p).copied()) {
        if !root.is_child_of(parent) || !parent.is_agentmux || !seen.insert(parent.pid) {
            break;
        }
        root = parent;
    }

    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for n in nodes {
        // Only a link to a parent that could really be the parent.
        if let Some(parent) = n.parent.and_then(|p| by_pid.get(&p)) {
            if n.is_child_of(parent) {
                children.entry(parent.pid).or_default().push(n.pid);
            }
        }
    }
    let mut own = HashSet::from([root.pid]);
    let mut queue = vec![root.pid];
    while let Some(pid) = queue.pop() {
        let parent_is_agentmux = by_pid.get(&pid).is_some_and(|p| p.is_agentmux);
        for &child in children.get(&pid).into_iter().flatten() {
            // A nested launcher: AgentMux started by something that isn't AgentMux.
            let nested_root = !parent_is_agentmux && by_pid.get(&child).is_some_and(|c| c.is_agentmux);
            if !nested_root && own.insert(child) {
                queue.push(child);
            }
        }
    }
    own
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(pid: u32, parent: Option<u32>, is_agentmux: bool) -> ProcNode {
        ProcNode { pid, parent, is_agentmux, started_at_ms: None }
    }

    fn started(mut node: ProcNode, at_ms: u64) -> ProcNode {
        node.started_at_ms = Some(at_ms);
        node
    }

    /// A stable install (launcher 100) beside a dev build (launcher 200), each
    /// with srv, a CEF host and a renderer under the host: the case that mixed.
    fn two_instances() -> Vec<ProcNode> {
        vec![
            n(1, None, false),
            n(100, Some(1), true),
            n(101, Some(100), true), // stable srv
            n(102, Some(100), true), // stable cef host
            n(103, Some(102), true), // stable renderer
            n(200, Some(1), true),
            n(201, Some(200), true), // dev srv
            n(202, Some(200), true), // dev cef host
            n(203, Some(202), true), // dev renderer
            n(300, Some(201), false), // a dev agent's shell
        ]
    }

    #[test]
    fn two_instances_side_by_side_are_told_apart_from_srv() {
        let t = two_instances();
        assert_eq!(own_instance_pids(&t, 201), HashSet::from([200, 201, 202, 203, 300]));
        assert_eq!(own_instance_pids(&t, 101), HashSet::from([100, 101, 102, 103]));
    }

    #[test]
    fn an_agentmux_started_from_a_pane_of_this_one_is_its_own_instance() {
        // An agent's shell (300, under srv 201) runs `task dev` (301), which
        // starts a launcher (400) with its own srv (401) and host (402).
        let mut t = two_instances();
        t.extend([
            n(301, Some(300), false),
            n(400, Some(301), true),
            n(401, Some(400), true),
            n(402, Some(400), true),
        ]);
        let outer = own_instance_pids(&t, 201);
        assert_eq!(outer, HashSet::from([200, 201, 202, 203, 300, 301]));
        assert!(![400, 401, 402].iter().any(|p| outer.contains(p)), "the nested instance is not the outer one's");
        // And from the nested srv, only the nested tree.
        assert_eq!(own_instance_pids(&t, 401), HashSet::from([400, 401, 402]));
    }

    #[test]
    fn a_stale_parent_link_to_a_reused_pid_does_not_cross_into_another_instance() {
        // srv 201's launcher (200) started at t=1000. A process that started
        // later at t=5000 now has the pid of srv 301's long-gone parent, 400,
        // and is itself an agentmux* launcher with its own srv: 301's recorded
        // parent (400) is a reused pid, not its parent.
        let t = vec![
            started(n(400, None, true), 5_000), // a NEWER process reusing pid 400
            started(n(401, Some(400), true), 5_100), // its real child
            started(n(301, Some(400), true), 1_000), // the stale link: started before 400
            started(n(302, Some(301), true), 1_100),
        ];
        // From 301: the walk up refuses 400 (it started after 301), so 301 is
        // its own root, with its child 302 and without 400's real tree.
        assert_eq!(own_instance_pids(&t, 301), HashSet::from([301, 302]));
        // From 401: its tree is 400 and 401 only; 301 is not 400's child.
        assert_eq!(own_instance_pids(&t, 401), HashSet::from([400, 401]));
    }

    #[test]
    fn a_parent_that_started_first_or_at_the_same_time_is_trusted() {
        let t = vec![
            started(n(10, None, true), 1_000),
            started(n(11, Some(10), true), 1_000),
            started(n(12, Some(11), true), 2_000),
        ];
        assert_eq!(own_instance_pids(&t, 12), HashSet::from([10, 11, 12]));
    }

    #[test]
    fn the_same_tree_is_found_from_any_member() {
        let t = two_instances();
        assert_eq!(own_instance_pids(&t, 203), own_instance_pids(&t, 201));
        assert_eq!(own_instance_pids(&t, 200), own_instance_pids(&t, 201));
    }

    #[test]
    fn the_walk_up_stops_at_the_first_parent_that_isnt_agentmux() {
        // An AppImage: AppImageLauncher (not agentmux*) starts the launcher.
        let t = vec![n(1, None, false), n(50, Some(1), false), n(60, Some(50), true), n(61, Some(60), true)];
        let own = own_instance_pids(&t, 61);
        assert_eq!(own, HashSet::from([60, 61]));
        assert!(!own.contains(&50) && !own.contains(&1));
    }

    #[test]
    fn srv_run_by_hand_is_its_own_instance() {
        let t = vec![n(1, None, false), n(70, Some(1), true), n(71, Some(70), false)];
        assert_eq!(own_instance_pids(&t, 70), HashSet::from([70, 71]));
    }

    #[test]
    fn an_unknown_pid_or_a_missing_parent_is_handled() {
        let t = two_instances();
        assert!(own_instance_pids(&t, 9999).is_empty());
        // The parent exited: srv's link points at a pid that isn't in the table.
        let orphan = vec![n(80, Some(7777), true), n(81, Some(80), true)];
        assert_eq!(own_instance_pids(&orphan, 80), HashSet::from([80, 81]));
    }

    #[test]
    fn stale_links_that_form_a_cycle_end() {
        // A reused pid can make a parent link loop back down.
        let t = vec![n(10, Some(11), true), n(11, Some(10), true), n(12, Some(11), true)];
        let own = own_instance_pids(&t, 12);
        assert!(own.contains(&12) && own.len() <= 3);
    }
}
