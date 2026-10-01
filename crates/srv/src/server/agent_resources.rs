// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What an agent owns — the one inventory
// (docs/specs/SPEC_AGENT_TEARDOWN_SINGLE_PATH_2026_10_01.md §6.1).
//
// Everything that answers "what does this agent own?" reads it: the teardown
// (`sagas::agent_teardown`), and — as later phases wire them — the close
// dialog, the `/quit` summary and Swarm. Phase 1 covers the sources srv
// already tracks; containers (§6.7) and srv-spawned `!cmd` processes (§6.5)
// join in Phase 2.

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

/// The process-side inventory of `block_id`: cheap, in-memory and local-store
/// reads only.
pub fn snapshot(state: &AppState, block_id: &str) -> AgentResources {
    let processes = state
        .process_tracker
        .list_block(block_id)
        .into_iter()
        .map(|p| ProcessEntry { pid: p.pid, command: p.command, started_at_ms: p.started_at_ms })
        .collect();
    let background_tasks = state
        .mstore
        .background_task_list_for_block(block_id)
        .unwrap_or_default()
        .into_iter()
        .filter(|t| t.ended_at_ms.is_none())
        .map(|t| BackgroundTaskEntry { id: t.id, label: t.label, pid: t.pid })
        .collect();
    let shell_sessions = state
        .shell_sessions
        .list_active()
        .into_iter()
        .filter(|s| s.block_id == block_id)
        .map(|s| ShellEntry { shell_id: s.shell_id, cmd: s.cmd })
        .collect();
    let sub_blocks = state
        .mstore
        .get::<crate::backend::obj::Block>(block_id)
        .ok()
        .flatten()
        .and_then(|b| b.subblockids)
        .unwrap_or_default();
    AgentResources { block_id: block_id.to_string(), processes, background_tasks, shell_sessions, sub_blocks }
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
