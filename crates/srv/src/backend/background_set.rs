// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What an agent has running in the background while no turn is in flight
//! (`docs/specs/SPEC_AGENT_STATE_BACKGROUND_WORK_2026_11_01.md` §5, §9 step 1).
//!
//! **Shadow only.** This decides nothing yet: no state, no surface reads it. Every
//! [`SAMPLE_EVERY_MS`] the fleet look hands over the agents that read Idle, and for each this works out
//! the set it would call Background and logs when an agent enters or leaves it (target
//! `background_shadow`), with a count for App health to read later ([`shadow_counts`]). A week of that
//! log sets the denylist and the grace period (D7) before the state is switched on.
//!
//! Two sources in this step: the declared background shells (`db_background_tasks` rows still
//! `running`) and the process trees an agent left behind (the pane's tracker). Background subagents and
//! Monitor come with the state itself (the Monitor's task type is not verified, §5).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::backend::background_work;
use crate::backend::process_tracker::TrackedProcess;
use crate::backend::storage::background_tasks::{BackgroundTask, BackgroundTaskStatus};
use crate::backend::storage::store::Store;

/// A process tree counts once it has run this long: a command that is about to finish is not background work.
pub const GRACE_MS: u64 = 30_000;
/// A declared background shell with no pid to check counts for this long, then no more (rows have stuck
/// `running` before, #2518).
const NO_PID_FOR_MS: i64 = 30 * 60 * 1000;
/// How often the shadow looks. The tracker read is not free and a minute of latency costs nothing here.
const SAMPLE_EVERY_MS: u64 = 10_000;
/// Daemons that outlive the command that started them and are not work the agent waits on (sccache, a git
/// fsmonitor, build servers). A command line containing one of these is left out. The shadow log is what
/// completes this list (D7).
const DENY: [&str; 7] = ["sccache", "fsmonitor--daemon", "vbcscompiler", "mspdbsrv", "gradle daemon", "buildkitd", "rust-analyzer"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A shell command the agent put in the background (`run_in_background`).
    Shell,
    /// A process tree the agent's call left behind.
    Process,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub kind: Kind,
    pub label: String,
    pub started_at_ms: u64,
}

fn denied(label: &str) -> bool {
    let l = label.to_ascii_lowercase();
    DENY.iter().any(|d| l.contains(d))
}

/// The trees among `members` that count: past the grace period, and not a daemon on the denylist.
pub fn from_tracker(now_ms: u64, members: &[TrackedProcess]) -> Vec<Item> {
    background_work::trees("", "", members)
        .into_iter()
        .filter(|w| !denied(&w.label) && (w.started_at_ms == 0 || now_ms.saturating_sub(w.started_at_ms) >= GRACE_MS))
        .map(|w| Item { kind: Kind::Process, label: w.label, started_at_ms: w.started_at_ms })
        .collect()
}

/// The declared background shells that still run: a row counts while its pid is in the tracker, or, with
/// no pid, for [`NO_PID_FOR_MS`].
pub fn from_tasks(now_ms: i64, tasks: &[BackgroundTask], members: &[TrackedProcess]) -> Vec<Item> {
    tasks
        .iter()
        .filter(|t| t.status == BackgroundTaskStatus::Running)
        .filter(|t| match t.pid {
            Some(pid) => members.iter().any(|m| i64::from(m.pid) == pid),
            None => now_ms.saturating_sub(t.started_at_ms) < NO_PID_FOR_MS,
        })
        .map(|t| Item { kind: Kind::Shell, label: t.label.clone(), started_at_ms: t.started_at_ms.max(0) as u64 })
        .collect()
}

/// Everything one agent has in the background now: the declared shells, then the trees left behind.
pub fn set_of(now_ms: u64, tasks: &[BackgroundTask], members: &[TrackedProcess]) -> Vec<Item> {
    let mut items = from_tasks(now_ms as i64, tasks, members);
    items.extend(from_tracker(now_ms, members));
    items
}

/// Who entered or left the would-be Background set since the last look, and how long each who left was in it.
/// `before` is the set (block id → when it entered) the last look left; `now` is this look's.
fn changes(before: &mut HashMap<String, u64>, now: &HashMap<String, u64>, at_ms: u64) -> (Vec<String>, Vec<(String, u64)>) {
    let entered: Vec<String> = now.keys().filter(|b| !before.contains_key(*b)).cloned().collect();
    let left: Vec<(String, u64)> =
        before.iter().filter(|(b, _)| !now.contains_key(*b)).map(|(b, since)| (b.clone(), at_ms.saturating_sub(*since))).collect();
    let next: HashMap<String, u64> = now.keys().map(|b| (b.clone(), before.get(b).copied().unwrap_or(at_ms))).collect();
    *before = next;
    (entered, left)
}

static STORE: OnceLock<Arc<Store>> = OnceLock::new();
static LAST_SAMPLE_MS: AtomicU64 = AtomicU64::new(0);
static IN_SET: AtomicU32 = AtomicU32::new(0);
static ENTERED: AtomicU64 = AtomicU64::new(0);
static SET: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

/// Where the declared background shells are read from. Called once, with the fleet source.
pub fn watch(store: Arc<Store>) {
    let _ = STORE.set(store);
}

/// How many agents are in the would-be Background set now, and how many times an agent has entered it
/// since srv started.
pub fn shadow_counts() -> (u32, u64) {
    (IN_SET.load(Ordering::Relaxed), ENTERED.load(Ordering::Relaxed))
}

/// One look, from the fleet's once-a-second observation: `idle` is the agents that read Idle (agent, block).
/// Only every [`SAMPLE_EVERY_MS`]. Blocking: reads the tracker and the store.
pub fn shadow_sample(idle: &[(String, String)], now_ms: u64) {
    let last = LAST_SAMPLE_MS.load(Ordering::Relaxed);
    if now_ms.saturating_sub(last) < SAMPLE_EVERY_MS || LAST_SAMPLE_MS.compare_exchange(last, now_ms, Ordering::Relaxed, Ordering::Relaxed).is_err() {
        return;
    }
    let Some(registry) = crate::backend::process_tracker::registry::global() else { return };
    let mut now: HashMap<String, u64> = HashMap::new();
    let mut named: HashMap<String, (String, Item, usize)> = HashMap::new();
    for (agent, block_id) in idle {
        let members = registry.list_block(block_id);
        let tasks = STORE.get().and_then(|s| s.background_task_list_for_block(block_id).ok()).unwrap_or_default();
        let items = set_of(now_ms, &tasks, &members);
        if let Some(first) = items.first().cloned() {
            now.insert(block_id.clone(), now_ms);
            named.insert(block_id.clone(), (agent.clone(), first, items.len()));
        }
    }
    let (entered, left) = {
        let mut guard = SET.lock().unwrap_or_else(|e| e.into_inner());
        changes(guard.get_or_insert_with(HashMap::new), &now, now_ms)
    };
    for block_id in &entered {
        if let Some((agent, first, n)) = named.get(block_id) {
            ENTERED.fetch_add(1, Ordering::Relaxed);
            tracing::info!(target: "background_shadow", agent = %agent, kind = ?first.kind, items = n, first = %first.label, "would be Background");
        }
    }
    for (block_id, for_ms) in &left {
        tracing::info!(target: "background_shadow", block = %block_id, for_s = for_ms / 1000, "no longer would be Background");
    }
    IN_SET.store(now.len() as u32, Ordering::Relaxed);
}

#[cfg(test)]
#[path = "background_set_tests.rs"]
mod tests;
