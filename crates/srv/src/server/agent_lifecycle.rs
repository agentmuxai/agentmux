// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What keeps agents that outlive their pane from piling up
//! (`docs/specs/SPEC_ALWAYS_AVAILABLE_AGENTS_2026_10_27.md` §7, D5 and D6; P4a). With
//! `agents:always_available` on, closing a pane no longer stops its agent, so two limits stand in for
//! the pane:
//!
//! - **Idle stop (D5).** An agent with no view (its block is parked) that has been idle for the window
//!   (`agents:idle_stop_minutes`, default 60, 0 = never; `agent:idle_stop_minutes` on its block
//!   overrides it) is stopped by a sweep once a minute. It stays addressable: a message starts it and
//!   resumes its session (`jekt_start`). Never stopped: an agent in a turn, one waiting on the user, one
//!   that holds a claimed Kanban task or has a held message, or one that holds the build slot.
//! - **Cap (D6).** A start (`agent_attach::start_agent`: `agent.start`, a message, a Job) is admitted
//!   while fewer than `agents:max_running` (default 12, 0 = no cap) agent processes run and, on Windows,
//!   commit use is under `agents:commit_limit_pct` (default 85, 0 = off). Over either, the agent idle
//!   longest with no view is stopped to make room; when none can be, the start is refused with
//!   `capacity:` and a message stays held.
//!
//! An agent that is shown in a pane is never stopped by either: it has a view. Off, none of this runs.

use std::collections::HashMap;
use std::time::Duration;

use crate::backend::agent_state::{agent_status_of_block, AgentState};
use crate::backend::{blockcontroller, obj};

use super::AppState;

pub const SETTING_IDLE_STOP_MINUTES: &str = "agents:idle_stop_minutes";
pub const SETTING_MAX_RUNNING: &str = "agents:max_running";
pub const SETTING_COMMIT_LIMIT_PCT: &str = "agents:commit_limit_pct";
/// Block meta: this agent's own idle window in minutes (0 = never stop it).
pub const META_IDLE_STOP_MINUTES: &str = "agent:idle_stop_minutes";

const DEFAULT_IDLE_STOP_MINUTES: f64 = 60.0;
const DEFAULT_MAX_RUNNING: f64 = 12.0;
const DEFAULT_COMMIT_LIMIT_PCT: f64 = 85.0;
/// How often the idle sweep runs.
const SWEEP_EVERY: Duration = Duration::from_secs(60);

/// The limits, from settings. `None`: that limit is off.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Policy {
    pub idle_stop_ms: Option<u64>,
    pub max_running: Option<usize>,
    pub commit_limit_pct: Option<f64>,
}

impl Policy {
    pub(crate) fn from_extra(extra: &HashMap<String, serde_json::Value>) -> Self {
        let num = |key: &str, default: f64| extra.get(key).and_then(|v| v.as_f64()).unwrap_or(default);
        let on = |n: f64| (n > 0.0).then_some(n);
        Self {
            idle_stop_ms: on(num(SETTING_IDLE_STOP_MINUTES, DEFAULT_IDLE_STOP_MINUTES)).map(|m| (m * 60_000.0) as u64),
            max_running: on(num(SETTING_MAX_RUNNING, DEFAULT_MAX_RUNNING)).map(|n| n as usize),
            commit_limit_pct: on(num(SETTING_COMMIT_LIMIT_PCT, DEFAULT_COMMIT_LIMIT_PCT)),
        }
    }
}

fn policy(state: &AppState) -> Policy {
    Policy::from_extra(&state.config_watcher.get_full_config().settings.extra)
}

/// A running agent process and what the policy needs to know about it.
#[derive(Debug, Clone)]
pub(crate) struct Live {
    pub block_id: String,
    /// The agent's slug (what a Kanban claim and the build slot name), else its id.
    pub agent_id: String,
    pub uid: String,
    /// In no tab: nobody is looking at it.
    pub parked: bool,
    pub state: Option<AgentState>,
    pub since_ms: u64,
    /// The block's own window: `Some(None)` never stop it, `Some(Some(ms))` its own window.
    pub idle_override: Option<Option<u64>>,
}

impl Live {
    /// Idle with no view, so it may be stopped to make room. An agent set to never stop is not.
    pub(crate) fn evictable(&self) -> bool {
        self.parked && self.state == Some(AgentState::Idle) && self.idle_override != Some(None)
    }

    /// Idle with no view for longer than its window (`default_ms` unless the block sets its own).
    pub(crate) fn idle_expired(&self, now_ms: u64, default_ms: Option<u64>) -> bool {
        let window = self.idle_override.unwrap_or(default_ms);
        self.evictable() && window.is_some_and(|w| now_ms.saturating_sub(self.since_ms) >= w)
    }
}

/// The agent processes running now, one per agent block that has a live controller.
pub(crate) fn live_agents(state: &AppState) -> Vec<Live> {
    let mstore = &state.mstore;
    let now = agentmux_common::time::now_ms_u64();
    let Ok(blocks) = super::app_api::agent_block::blocks_in_tabs_or_parked(mstore) else {
        return Vec::new();
    };
    blocks
        .into_iter()
        .filter_map(|(block, tab)| {
            let controller = blockcontroller::get_controller(&block.oid)?;
            if controller.get_runtime_status().shellprocstatus != blockcontroller::STATUS_RUNNING {
                return None;
            }
            let agent = crate::sagas::park_agents::agent_of_block(state, &block.oid, &block)?;
            let slug = mstore.agent_def_get(&agent.id).ok().flatten().map(|d| d.slug).filter(|s| !s.is_empty());
            let status = agent_status_of_block(mstore, &block.oid);
            let idle_override = block.meta.get(META_IDLE_STOP_MINUTES).and_then(|v| v.as_f64()).map(|m| (m > 0.0).then(|| (m * 60_000.0) as u64));
            Some(Live {
                block_id: block.oid.clone(),
                agent_id: slug.unwrap_or_else(|| agent.id.clone()),
                uid: agent.id,
                parked: tab.is_none(),
                state: status.map(|s| s.state),
                since_ms: status.map_or(now, |s| s.since_ms),
                idle_override,
            })
        })
        .collect()
}

/// Why this agent is not stopped even when idle: it holds a claimed Kanban task, has a message held
/// for it, or holds the build slot. `None`: nothing keeps it.
pub(crate) fn exempt(state: &AppState, live: &Live) -> Option<&'static str> {
    if state.work_store.work_queue_claimed_by(&live.agent_id, &live.uid).is_ok_and(|claimed| !claimed.is_empty()) {
        return Some("holds a claimed task");
    }
    if state.mstore.jekt_held_targets().is_ok_and(|held| held.iter().any(|t| *t == live.uid)) {
        return Some("has a held message");
    }
    if build_slot_holder().is_some_and(|holder| holder.eq_ignore_ascii_case(&live.agent_id)) {
        return Some("holds the build slot");
    }
    None
}

/// The agent that holds the host's build slot (`~/.agentmux/shared/locks/build-slot.sh`), if any.
fn build_slot_holder() -> Option<String> {
    let owner = dirs::home_dir()?.join(".agentmux").join("shared").join("locks").join("build").join("slot").join("owner");
    holder_in(&std::fs::read_to_string(owner).ok()?)
}

/// The `agent=` line of a build-slot owner file.
fn holder_in(owner_file: &str) -> Option<String> {
    owner_file.lines().find_map(|l| l.strip_prefix("agent=")).map(|a| a.trim().to_string()).filter(|a| !a.is_empty())
}

/// Stop `live` the way an explicit stop does (its process; the block, the transcript and the session stay).
async fn stop(state: &AppState, live: &Live, why: &str) -> bool {
    match crate::sagas::agent_teardown::stop(state, &live.block_id, true, None).await {
        Ok(_) => {
            tracing::info!(agent = %live.agent_id, block_id = %live.block_id, "agent lifecycle: stopped it ({why})");
            true
        }
        Err(e) => {
            tracing::warn!(agent = %live.agent_id, block_id = %live.block_id, "agent lifecycle: could not stop it ({why}): {e}");
            false
        }
    }
}

/// One idle sweep: stop each agent with no view that has been idle past its window and that nothing
/// keeps running. Returns how many were stopped.
pub(crate) async fn sweep_once(state: &AppState) -> usize {
    if !crate::sagas::park_agents::enabled(state) {
        return 0;
    }
    let default_ms = policy(state).idle_stop_ms;
    let now = agentmux_common::time::now_ms_u64();
    let mut stopped = 0;
    for live in live_agents(state) {
        if live.idle_expired(now, default_ms) && exempt(state, &live).is_none() && stop(state, &live, "idle with no view").await {
            stopped += 1;
        }
    }
    stopped
}

/// Run the idle sweep once a minute.
pub(crate) fn install(state: &AppState) {
    let state = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(SWEEP_EVERY).await;
            sweep_once(&state).await;
        }
    });
}

/// Commit use as a percentage of the commit limit, where the host reports it (Windows:
/// `GlobalMemoryStatusEx`). `None` elsewhere: Linux overcommits and macOS has no commit limit, so no
/// figure means no refusal.
pub(crate) fn commit_used_pct() -> Option<f64> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut mem: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        mem.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if unsafe { GlobalMemoryStatusEx(&mut mem) } != 0 && mem.ullTotalPageFile > 0 {
            return Some((mem.ullTotalPageFile - mem.ullAvailPageFile) as f64 / mem.ullTotalPageFile as f64 * 100.0);
        }
    }
    None
}

/// Whether the host is at its cap or over its commit limit, given the agents running.
fn over_limit(policy: &Policy, running: usize, commit_pct: Option<f64>) -> Option<String> {
    if let Some(max) = policy.max_running.filter(|max| running >= *max) {
        return Some(format!("{running} agents are running (the limit is {max})"));
    }
    match (policy.commit_limit_pct, commit_pct) {
        (Some(limit), Some(used)) if used > limit => Some(format!("memory commit is at {used:.0}% (the limit is {limit:.0}%)")),
        _ => None,
    }
}

/// May `agent_id` be started? Yes when it already runs, when nothing is over a limit, or when an idle
/// agent with no view can be stopped to make room; otherwise `Err("capacity: ...")`.
pub(crate) async fn admit_start(state: &AppState, agent_id: &str) -> Result<(), String> {
    admit_start_with(state, agent_id, commit_used_pct).await
}

pub(crate) async fn admit_start_with(state: &AppState, agent_id: &str, commit_pct: impl Fn() -> Option<f64>) -> Result<(), String> {
    if !crate::sagas::park_agents::enabled(state) {
        return Ok(());
    }
    let policy = policy(state);
    if policy.max_running.is_none() && policy.commit_limit_pct.is_none() {
        return Ok(());
    }
    let mut live = live_agents(state);
    if live.iter().any(|l| l.uid == agent_id || l.agent_id == agent_id) {
        return Ok(());
    }
    let mut could_not_stop = std::collections::HashSet::new();
    loop {
        let Some(over) = over_limit(&policy, live.len(), commit_pct()) else {
            return Ok(());
        };
        let victim = live
            .iter()
            .filter(|l| l.evictable() && !could_not_stop.contains(&l.block_id) && exempt(state, l).is_none())
            .min_by_key(|l| l.since_ms)
            .cloned();
        let Some(victim) = victim else {
            return Err(format!("capacity: {over}, and no agent with no view is idle enough to stop for it"));
        };
        if stop(state, &victim, &format!("to make room: {over}")).await {
            live.retain(|l| l.block_id != victim.block_id);
        } else {
            could_not_stop.insert(victim.block_id);
        }
    }
}

#[cfg(test)]
mod tests;
