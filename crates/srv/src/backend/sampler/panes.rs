// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Each pane's CPU and memory for its stats badge, taken from Tower's sample
//! (`backend::tower_sampler`), so the badge and Tower count the same processes
//! the same way: a pane's Job Object or cgroup, which nothing it starts can
//! leave, else its shell's process tree. A Tower pane open at the same time
//! shares the process snapshot (Tower reuses one taken within the last second).
//! The badge's tooltip (`blockstats.detail`) reads the same task from
//! [`tower_snapshot`], so it lists the processes the badge counts.
//! docs/reports/REPORT_SYSINFO_REWRITE_AND_MODERN_MONITOR_2026_10_13.md section 5.5.

use std::sync::Arc;

use crate::backend::blockcontroller::pidregistry;
use crate::backend::blockstats_detail::Shell;
use crate::backend::rpc_types::{TowerProcessRole, TowerSnapshot, TowerTaskKind};
use crate::backend::tower_sampler::{BlockLabel, Inputs, Tower};

/// What one pane's badge shows.
#[derive(Debug, Clone, PartialEq)]
pub struct PaneStat {
    pub block_id: String,
    /// Percent of one core; `None` on the first sample, before there is a rate.
    pub cpu_pct: Option<f64>,
    /// Resident memory of its processes, bytes: what the badge calls Memory ("in
    /// RAM now, including pages shared"). Its tooltip shows private memory
    /// separately, from the detail.
    pub mem: u64,
    pub pids: u32,
    /// Its main process, so a restarted shell starts a new peak.
    pub shell: Shell,
}

/// Measures every pane's stats. Blocking: reads the process table.
pub type PaneSampler = Arc<dyn Fn() -> std::io::Result<Vec<PaneStat>> + Send + Sync>;

/// The panes' stats in a Tower snapshot: one per pane task, AgentMux's own
/// task left out.
pub fn from_snapshot(snap: &TowerSnapshot) -> Vec<PaneStat> {
    snap.tasks
        .iter()
        .filter(|t| t.kind != TowerTaskKind::Agentmux)
        .map(|t| {
            let main = t.processes.iter().find(|p| p.role == Some(TowerProcessRole::Main));
            PaneStat {
                block_id: t.id.clone(),
                cpu_pct: t.cpu.map(|c| c * 100.0),
                mem: t.processes.iter().filter_map(|p| p.mem_resident).sum(),
                pids: t.processes.len() as u32,
                shell: main.map_or((0, 0), |p| (p.pid, p.started_at_ms.unwrap_or(0))),
            }
        })
        .collect()
}

/// The live sampler: Tower's sample of this machine, with each pane named by
/// `labels` (a pane `labels` doesn't know, such as one that was closed, is left out).
pub fn tower_sampler(labels: impl Fn(&str) -> Option<BlockLabel> + Send + Sync + 'static) -> PaneSampler {
    Arc::new(move || Ok(from_snapshot(&tower_snapshot(&labels)?)))
}

/// Tower's sample of this machine's panes, each named by `labels`. Blocking:
/// reads the process table, unless Tower read it within the last 900 ms.
pub fn tower_snapshot(labels: impl Fn(&str) -> Option<BlockLabel>) -> std::io::Result<TowerSnapshot> {
    let inputs = || Inputs {
        blocks: crate::backend::process_tracker::registry::global()
            .map(|t| t.members_by_block())
            .unwrap_or_default(),
        roots: pidregistry::get_all(),
        own_pid: std::process::id(),
    };
    Tower::global().sample(false, "", inputs, labels, |_| None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::rpc_types::{TowerProcess, TowerTask};

    fn process(pid: u32, role: TowerProcessRole, started: u64) -> TowerProcess {
        TowerProcess {
            pid,
            role: Some(role),
            started_at_ms: Some(started),
            mem: Some(1),
            mem_resident: Some(1000 + u64::from(pid)),
            ..Default::default()
        }
    }

    fn snapshot(tasks: Vec<TowerTask>) -> TowerSnapshot {
        TowerSnapshot {
            ts_ms: 0,
            hostname: String::new(),
            os: String::new(),
            cpu_count: 1,
            memory_metric: String::new(),
            interval_ms: 2000,
            remote: false,
            tasks,
            machine: None,
            host: None,
        }
    }

    fn task(id: &str, kind: TowerTaskKind, cpu: Option<f64>, mem: u64, processes: Vec<TowerProcess>) -> TowerTask {
        TowerTask {
            id: id.to_string(),
            kind,
            label: id.to_string(),
            tracking: "tree".to_string(),
            cpu,
            cpu_account: false,
            mem,
            processes,
            exited: None,
        }
    }

    #[test]
    fn each_pane_task_becomes_a_badge_and_agentmux_itself_does_not() {
        let snap = snapshot(vec![
                task(
                    "term-1",
                    TowerTaskKind::Terminal,
                    Some(0.25),
                    4096,
                    vec![process(10, TowerProcessRole::Main, 111), process(11, TowerProcessRole::Started, 112)],
                ),
                task("agent-1", TowerTaskKind::Agent, Some(1.5), 8192, vec![process(20, TowerProcessRole::Main, 222)]),
                task(crate::backend::tower_sampler::AGENTMUX_TASK_ID, TowerTaskKind::Agentmux, Some(0.1), 1, vec![]),
        ]);
        let stats = from_snapshot(&snap);
        assert_eq!(
            stats,
            vec![
                // Resident memory summed over the processes, not the task's private total.
                PaneStat { block_id: "term-1".into(), cpu_pct: Some(25.0), mem: 1010 + 1011, pids: 2, shell: (10, 111) },
                PaneStat { block_id: "agent-1".into(), cpu_pct: Some(150.0), mem: 1020, pids: 1, shell: (20, 222) },
            ]
        );
    }

    #[test]
    fn a_first_sample_has_no_rate_yet_and_a_task_without_a_main_process_has_no_shell() {
        let snap = snapshot(vec![task("agent-2", TowerTaskKind::Agent, None, 10, vec![process(30, TowerProcessRole::Support, 5)])]);
        let s = &from_snapshot(&snap)[0];
        assert_eq!((s.cpu_pct, s.shell), (None, (0, 0)));
    }
}
