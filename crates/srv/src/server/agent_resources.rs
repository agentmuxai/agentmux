// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What an agent owns — the one inventory
// (docs/specs/SPEC_AGENT_TEARDOWN_SINGLE_PATH_2026_10_01.md §6.1).
//
// Everything that answers "what does this agent own?" reads it: the teardown
// (`sagas::agent_teardown`), and — as later phases wire them — the close
// dialog, the `/quit` summary and Swarm. It covers the sources srv tracks;
// `Shell()` and `!cmd` processes appear under `processes` since they join the
// agent's tracker (§6.5). [`survivors`] re-checks a snapshot after a teardown
// (§6.2 step 8).

use serde::Serialize;

use crate::backend::storage::cron::CronJob;
use crate::backend::storage::work_queue::{work_state, WorkItem};
use crate::server::AppState;

/// One process the agent started (the CLI or a descendant), from its
/// block's process tracker.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct ProcessEntry {
    pub pid: u32,
    pub command: String,
    /// Unix ms; 0 if the platform doesn't expose it. With `pid`, identifies
    /// the process across a later re-check (PID reuse).
    pub started_at_ms: u64,
}

/// A `run_in_background` Bash command the agent declared (bashwrap).
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct BackgroundTaskEntry {
    pub id: String,
    pub label: String,
    pub pid: Option<i64>,
    /// Unix ms, when bashwrap first saw it.
    pub started_at_ms: i64,
}

/// A srv-spawned `Shell()` session the agent started.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct ShellEntry {
    pub shell_id: String,
    pub cmd: String,
}

/// Everything one agent owns, at one moment.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct AgentResources {
    pub block_id: String,
    /// The CLI and every process it started that the tracker sees (R1, R2).
    pub processes: Vec<ProcessEntry>,
    /// Still-running declared background tasks (the R2 subset, e.g. `task dev`).
    pub background_tasks: Vec<BackgroundTaskEntry>,
    /// Running `Shell()` sessions (R3).
    pub shell_sessions: Vec<ShellEntry>,
    /// Sub-blocks, e.g. PtyShell drawers (R4).
    pub sub_blocks: Vec<String>,
    /// A container agent's container (R6), by name.
    pub container: Option<String>,
}

/// Who an agent is, for the work-queue and cron sources, which are keyed by
/// identity rather than block.
#[derive(Clone, Debug, Default)]
pub struct AgentIdentity {
    pub agent_id: String,
    pub uid: Option<String>,
}

impl AgentIdentity {
    /// Resolve while the block is still registered: a teardown unregisters it.
    pub fn of(state: &AppState, block_id: &str) -> Self {
        let reg = state.reactive_handler.get_agent_by_block(block_id);
        Self {
            agent_id: reg.as_ref().map(|a| a.agent_id.clone()).unwrap_or_default(),
            uid: reg.and_then(|a| a.uid),
        }
    }

    /// The name to show: the agent id, else the block id.
    pub fn label(&self, block_id: &str) -> String {
        if self.agent_id.is_empty() { block_id.to_string() } else { self.agent_id.clone() }
    }
}

/// The process-side inventory of `block_id`: in-memory reads plus two
/// local-store (SQLite) reads, so call it off the async workers
/// (`spawn_blocking`), as the teardown does.
pub fn snapshot(state: &AppState, block_id: &str) -> AgentResources {
    let mut processes: Vec<ProcessEntry> = state
        .process_tracker
        .list_block(block_id)
        .into_iter()
        .map(|p| ProcessEntry { pid: p.pid, command: p.command, started_at_ms: p.started_at_ms })
        .collect();
    // The Windows tracker doesn't record start times; take them from the OS,
    // so a later re-check can tell a reused PID from the same process.
    let missing: Vec<u32> = processes.iter().filter(|p| p.started_at_ms == 0).map(|p| p.pid).collect();
    if !missing.is_empty() {
        let times = start_times(&missing);
        for p in processes.iter_mut().filter(|p| p.started_at_ms == 0) {
            p.started_at_ms = times.get(&p.pid).copied().unwrap_or(0);
        }
    }
    let background_tasks = state
        .mstore
        .background_task_list_for_block(block_id)
        .unwrap_or_default()
        .into_iter()
        .filter(|t| t.ended_at_ms.is_none())
        .map(|t| BackgroundTaskEntry { id: t.id, label: t.label, pid: t.pid, started_at_ms: t.started_at_ms })
        .collect();
    let shell_sessions = state
        .shell_sessions
        .list_active()
        .into_iter()
        .filter(|s| s.block_id == block_id)
        .map(|s| ShellEntry { shell_id: s.shell_id, cmd: s.cmd })
        .collect();
    let block = state.mstore.get::<crate::backend::obj::Block>(block_id).ok().flatten();
    let container = block.as_ref().and_then(container_of);
    let sub_blocks = block.and_then(|b| b.subblockids).unwrap_or_default();
    AgentResources { block_id: block_id.to_string(), processes, background_tasks, shell_sessions, sub_blocks, container }
}

/// The container a container agent's block runs in (`agentMode` =
/// `container`), named from its `agentId` as the launch path names it.
pub fn container_of(block: &crate::backend::obj::Block) -> Option<String> {
    use crate::backend::obj::meta_get_string;
    if meta_get_string(&block.meta, "agentMode", "") != "container" {
        return None;
    }
    let agent_id = meta_get_string(&block.meta, "agentId", "");
    (!agent_id.is_empty()).then(|| crate::backend::container::container_name_for_slug(&agent_id))
}

/// Is `container` used by a live controller other than `block_id`'s (the same
/// agent open in a second pane)? Store reads: call off the async workers.
pub fn container_shared(state: &AppState, block_id: &str, container: &str) -> bool {
    crate::backend::blockcontroller::get_all_controllers()
        .into_keys()
        .filter(|id| id != block_id)
        .filter_map(|id| state.mstore.get::<crate::backend::obj::Block>(&id).ok().flatten())
        .any(|b| container_of(&b).as_deref() == Some(container))
}

/// Start time (Unix ms, at the OS's one-second resolution) of each of `pids`
/// that is running now; a PID that isn't running is absent. One targeted OS
/// query.
pub fn start_times(pids: &[u32]) -> std::collections::HashMap<u32, u64> {
    if pids.is_empty() {
        return Default::default();
    }
    let list: Vec<sysinfo::Pid> = pids.iter().map(|p| sysinfo::Pid::from_u32(*p)).collect();
    let mut sys = sysinfo::System::new();
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&list),
        true,
        sysinfo::ProcessRefreshKind::nothing(),
    );
    sys.processes().iter().map(|(pid, p)| (pid.as_u32(), p.start_time() * 1000)).collect()
}

/// The processes of a snapshot that are still running: the same PID with the
/// same start time. A PID the OS has since reused for another process is not
/// a survivor (spec §8).
pub fn survivors(before: &[ProcessEntry]) -> Vec<ProcessEntry> {
    let pids: Vec<u32> = before.iter().map(|p| p.pid).collect();
    let live = start_times(&pids);
    before
        .iter()
        .filter(|p| live.get(&p.pid).is_some_and(|now| same_start(p.started_at_ms, *now)))
        .cloned()
        .collect()
}

/// The PIDs of `tasks` that are still that task: running, and started no
/// later than the task was first seen (a PID the OS reused since started
/// after it). Spec §8, PID reuse.
pub fn live_background_pids(tasks: &[BackgroundTaskEntry]) -> Vec<u32> {
    let pids: Vec<u32> = tasks.iter().filter_map(|t| t.pid.and_then(|p| u32::try_from(p).ok())).filter(|p| *p > 1).collect();
    let live = start_times(&pids);
    tasks
        .iter()
        .filter_map(|t| {
            let pid = u32::try_from(t.pid?).ok()?;
            let started = *live.get(&pid)?;
            // bashwrap reports a task a moment after it starts; allow for that
            // and the OS's one-second resolution.
            (started <= (t.started_at_ms.max(0) as u64) + 5_000).then_some(pid)
        })
        .collect()
}

/// `recorded` 0 means the start time wasn't known; the OS reports seconds.
fn same_start(recorded_ms: u64, now_ms: u64) -> bool {
    recorded_ms == 0 || recorded_ms.abs_diff(now_ms) <= 1000
}

/// Does a claim held by (`claimed_by`, `claimed_by_uid`) belong to the agent?
/// Same holder rule as the store's release (`HOLDER_BY_UID`): by UID when
/// both sides have one, else by name.
pub fn holds_claim(claimed_by: &str, claimed_by_uid: &str, agent_id: &str, uid: Option<&str>) -> bool {
    match uid.filter(|u| !u.is_empty()) {
        Some(u) if !claimed_by_uid.is_empty() => claimed_by_uid == u,
        _ => !agent_id.is_empty() && claimed_by == agent_id,
    }
}

/// Does a cron job aimed at (`target`, `target_uid`) deliver to the agent?
/// By UID when the job has one, else by name (case-insensitive, like agent
/// addressing).
pub fn cron_targets(target: &str, target_uid: &str, agent_id: &str, uid: Option<&str>) -> bool {
    match uid.filter(|u| !u.is_empty()) {
        Some(u) if !target_uid.is_empty() => target_uid == u,
        _ => !agent_id.is_empty() && target.eq_ignore_ascii_case(agent_id),
    }
}

/// The agent's claimed work items (R7).
pub(crate) fn claims_held_by<'a>(items: &'a [WorkItem], agent_id: &str, uid: Option<&str>) -> Vec<&'a WorkItem> {
    items
        .iter()
        .filter(|w| w.state == work_state::CLAIMED && holds_claim(&w.claimed_by, &w.claimed_by_uid, agent_id, uid))
        .collect()
}

/// Names of the cron jobs targeting the agent (R8).
pub(crate) fn crons_targeting(jobs: &[CronJob], agent_id: &str, uid: Option<&str>) -> Vec<String> {
    jobs.iter()
        .filter(|j| cron_targets(&j.target, &j.target_uid, agent_id, uid))
        .map(|j| j.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_live_process_survives_and_a_reused_pid_does_not() {
        let me = std::process::id();
        let started = *start_times(&[me]).get(&me).expect("this process is running");
        assert!(started > 0);
        let entry = |started_at_ms| ProcessEntry { pid: me, command: "test".into(), started_at_ms };
        assert_eq!(survivors(&[entry(started)]).len(), 1, "same pid, same start: still running");
        assert_eq!(survivors(&[entry(0)]).len(), 1, "unknown start time: judged by pid");
        assert!(survivors(&[entry(started - 60_000)]).is_empty(), "same pid, other start: a reused pid");
    }

    #[test]
    fn a_background_task_pid_counts_only_while_it_is_that_task() {
        let me = std::process::id();
        let started = *start_times(&[me]).get(&me).unwrap() as i64;
        let task = |started_at_ms| BackgroundTaskEntry {
            id: "t".into(),
            label: "task dev".into(),
            pid: Some(me as i64),
            started_at_ms,
        };
        assert_eq!(live_background_pids(&[task(started + 500)]), vec![me], "seen just after it started");
        assert!(live_background_pids(&[task(started - 60_000)]).is_empty(), "the PID was reused after the task was seen");
        let no_pid = BackgroundTaskEntry { pid: None, ..task(started) };
        assert!(live_background_pids(&[no_pid]).is_empty());
    }

    #[test]
    fn only_a_container_agent_has_a_container() {
        let block = |meta: &[(&str, &str)]| crate::backend::obj::Block {
            meta: meta.iter().map(|(k, v)| (k.to_string(), serde_json::json!(v))).collect(),
            ..Default::default()
        };
        assert_eq!(
            container_of(&block(&[("agentMode", "container"), ("agentId", "lark")])).as_deref(),
            Some(crate::backend::container::container_name_for_slug("lark").as_str())
        );
        assert_eq!(container_of(&block(&[("agentMode", "host"), ("agentId", "lark")])), None);
        assert_eq!(container_of(&block(&[("agentMode", "container")])), None, "no agentId, no name");
    }

    #[test]
    fn an_exited_process_is_not_a_survivor() {
        #[cfg(windows)]
        let mut child = std::process::Command::new("cmd").args(["/C", "exit 0"]).spawn().unwrap();
        #[cfg(unix)]
        let mut child = std::process::Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        drop(child);
        assert!(survivors(&[ProcessEntry { pid, command: "gone".into(), started_at_ms: 0 }]).is_empty());
    }
}
