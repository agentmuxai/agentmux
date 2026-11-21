// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A leader is told when an agent that owes it work has gone quiet
//! (docs/specs/SPEC_LEADER_TOLD_WHEN_AN_AGENT_GOES_QUIET_2026_11_21.md, phase 1).
//!
//! An agent is **watched** while it owes work: it holds a claimed Kanban task (not on hold, not waiting on an
//! answer), or it is a spawned worker that has not reported to its leader since the leader last messaged it. It
//! is **quiet** when it has done nothing for the quiet time: no output, no message sent, no update of a task it
//! holds. A sweep every 30 seconds reads that from memory (no model call, no poll of the agent): an agent between
//! turns gets one nudge (a system turn) and, five minutes after it, its watchers are told; a turn that is still
//! running and has written nothing for the longer running time gets its watchers told, with no nudge.
//!
//! The decision (`Watch::step`) is pure, so the timeline is tested with a fake clock; `sweep` reads the stores
//! and the controllers and delivers.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::backend::agent_state::AgentState;
use crate::backend::storage::agents::AgentDefinition;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;
use crate::backend::storage::work_queue::work_state;
use crate::backend::wconfig::ConfigState;
use super::worker_notes as notes;

/// The quiet time of an agent between turns, in minutes; 0 turns the whole feature off.
pub const SETTING_QUIET_MINUTES: &str = "agents:quietminutes";
/// The quiet time of a turn that is running, in minutes.
pub const SETTING_QUIET_RUNNING_MINUTES: &str = "agents:quietrunningminutes";
/// How long after the nudge (or, for a running turn, after its quiet time) the watchers are told, in minutes.
pub const SETTING_QUIET_ESCALATE_MINUTES: &str = "agents:quietescalateminutes";

const DEFAULT_QUIET_MINUTES: i64 = 15;
const DEFAULT_QUIET_RUNNING_MINUTES: i64 = 30;
const DEFAULT_ESCALATE_MINUTES: i64 = 5;
/// An agent is nudged at most this often.
const NUDGE_SPACING_MS: u64 = 60 * 60_000;
/// After its watchers were told, they are not told again about the same quiet for this long.
const RETELL_MS: u64 = 60 * 60_000;
/// How often the sweep looks.
const SWEEP_EVERY: std::time::Duration = std::time::Duration::from_secs(30);

/// The three times, from settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub quiet_ms: u64,
    pub running_ms: u64,
    pub escalate_ms: u64,
}

/// The limits in `extra`, or `None` when the feature is off (`agents:quietminutes` of 0 or below).
pub fn limits_from(extra: &HashMap<String, serde_json::Value>) -> Option<Limits> {
    let minutes = |key: &str, default: i64| extra.get(key).and_then(|v| v.as_i64()).filter(|m| *m >= 0).unwrap_or(default);
    let quiet = minutes(SETTING_QUIET_MINUTES, DEFAULT_QUIET_MINUTES);
    if quiet == 0 {
        return None;
    }
    let ms = |m: i64| (m as u64).saturating_mul(60_000);
    Some(Limits {
        quiet_ms: ms(quiet),
        running_ms: ms(minutes(SETTING_QUIET_RUNNING_MINUTES, DEFAULT_QUIET_RUNNING_MINUTES)),
        escalate_ms: ms(minutes(SETTING_QUIET_ESCALATE_MINUTES, DEFAULT_ESCALATE_MINUTES)),
    })
}

/// What the sweep read about one agent that owes work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading {
    /// The newest of its own activity (unix ms, 0 when none is known): output, a message it sent, an update of a
    /// task it holds. A message *to* it is not activity.
    pub activity_ms: u64,
    /// A turn is running.
    pub turn_active: bool,
}

/// What a quiet agent is owed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Between turns, the quiet time is up: nudge the agent once.
    Nudge,
    /// Tell its watchers; `running` when a turn is still running and has written nothing; `idle_ms` is how long
    /// it has been quiet.
    Tell { running: bool, idle_ms: u64 },
}

#[derive(Debug, Default)]
struct Episode {
    /// The clock: the newest of its activity and when it was first seen owing work (so an agent just given work,
    /// or one found after a restart of srv, gets a whole quiet time).
    activity_at: u64,
    nudged_at: Option<u64>,
    /// The nudge was skipped (one was sent within the hour): the watchers are told at the quiet time.
    nudge_skipped: bool,
    told_at: Option<u64>,
}

/// The quiet episodes of the agents that owe work, by agent.
#[derive(Debug, Default)]
pub struct Watch {
    episodes: HashMap<String, Episode>,
    last_nudge: HashMap<String, u64>,
}

impl Watch {
    /// The next step for the agent `key` at `now_ms`, or `None`. Call it once a sweep for each agent that owes
    /// work and can be called quiet (see [`callable`]).
    pub fn step(&mut self, key: &str, reading: Reading, lim: &Limits, now_ms: u64) -> Option<Action> {
        let ep = self.episodes.entry(key.to_string()).or_insert_with(|| Episode { activity_at: now_ms.max(reading.activity_ms), ..Episode::default() });
        if reading.activity_ms > ep.activity_at {
            // It moved: a new episode, and a nudge may be sent again once the hour since the last one is over.
            *ep = Episode { activity_at: reading.activity_ms, ..Episode::default() };
        }
        let idle = now_ms.saturating_sub(ep.activity_at);
        let may_tell = |ep: &Episode| ep.told_at.map_or(true, |t| now_ms.saturating_sub(t) >= RETELL_MS);
        if reading.turn_active {
            if idle >= lim.running_ms.saturating_add(lim.escalate_ms) && may_tell(ep) {
                return Some(Action::Tell { running: true, idle_ms: idle });
            }
            return None;
        }
        if idle < lim.quiet_ms {
            return None;
        }
        if ep.nudged_at.is_none() && !ep.nudge_skipped {
            let nudged_lately = self.last_nudge.get(key).is_some_and(|at| now_ms.saturating_sub(*at) < NUDGE_SPACING_MS);
            if nudged_lately {
                ep.nudge_skipped = true;
            } else {
                ep.nudged_at = Some(now_ms);
                self.last_nudge.insert(key.to_string(), now_ms);
                return Some(Action::Nudge);
            }
        }
        let due = ep.nudge_skipped || ep.nudged_at.is_some_and(|at| now_ms.saturating_sub(at) >= lim.escalate_ms);
        if due && may_tell(ep) {
            return Some(Action::Tell { running: false, idle_ms: idle });
        }
        None
    }

    /// The watchers of `key` were told at `now_ms`: it is not told again for an hour. Called only once a note went
    /// out, so a note nobody could receive (no watcher open) or that the one-a-minute gap held is tried again at the
    /// next sweep.
    pub fn mark_told(&mut self, key: &str, now_ms: u64) {
        if let Some(ep) = self.episodes.get_mut(key) {
            ep.told_at = Some(now_ms);
        }
    }

    /// Forget every agent not in `keep` (it owes nothing now, or cannot be called quiet): its next episode starts
    /// with a whole quiet time.
    pub fn retain(&mut self, keep: &HashSet<String>) {
        self.episodes.retain(|k, _| keep.contains(k));
        if self.last_nudge.len() > 512 {
            let cutoff = agentmux_common::time::now_ms_u64().saturating_sub(NUDGE_SPACING_MS);
            self.last_nudge.retain(|_, at| *at >= cutoff);
        }
    }
}

/// Whether an agent in this state can be called quiet: it is working or idle. A question waiting for the user,
/// an agent that cannot run (stopped, failed, limited, signed out) and one with no state are told another way.
pub fn callable(state: Option<AgentState>) -> bool {
    matches!(state, Some(AgentState::Working | AgentState::Idle))
}

/// A task an agent holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwedTask {
    pub num: i64,
    /// Who filed it (an agent's uid), empty when a person did or it is not known.
    pub filer_uid: String,
    /// When the task last moved (unix ms).
    pub updated_ms: u64,
}

/// The claimed tasks of every holder, keyed by the holder's uid, or `name:<lowercased claimed_by>` when the
/// holder's uid is not known: the key the Swarm's facts use. A task on hold, waiting on a question, in review,
/// done or open is not claimed and is not here.
pub fn claimed_by_holder(items: &[crate::backend::storage::work_board::BoardTask]) -> HashMap<String, Vec<OwedTask>> {
    let mut out: HashMap<String, Vec<OwedTask>> = HashMap::new();
    for t in items.iter().filter(|t| t.item.state == work_state::CLAIMED) {
        let key = if t.item.claimed_by_uid.is_empty() { format!("name:{}", t.item.claimed_by.to_lowercase()) } else { t.item.claimed_by_uid.clone() };
        let moved = t.item.updated_at.max(t.card.updated_at).max(0) as u64;
        out.entry(key).or_default().push(OwedTask { num: t.card.num, filer_uid: t.item.created_by_uid.clone(), updated_ms: moved });
    }
    out.values_mut().for_each(|v| v.sort_by_key(|t| t.num));
    out
}

/// The tasks the agent `def` holds.
pub fn owed_by(claimed: &HashMap<String, Vec<OwedTask>>, def: &AgentDefinition) -> Vec<OwedTask> {
    let mut tasks: Vec<OwedTask> = claimed.get(&def.id).cloned().unwrap_or_default();
    for name in [def.slug.to_lowercase(), def.name.to_lowercase()] {
        if let Some(more) = claimed.get(&format!("name:{name}")) {
            tasks.extend(more.iter().filter(|t| !tasks.iter().any(|have| have.num == t.num)).cloned().collect::<Vec<_>>());
        }
    }
    tasks.sort_by_key(|t| t.num);
    tasks
}

/// Every claimed task, on every board and with no row limit, read once for a sweep or a note: a limit on the
/// board's rows would cut off the oldest, which are the tasks of the agents that have been quiet longest. Empty
/// when the read fails.
pub fn read_claimed(work: &Store, now_ms: i64) -> Vec<crate::backend::storage::work_board::BoardTask> {
    work.work_claimed_list(now_ms).unwrap_or_default()
}

/// The first open pane of each agent definition.
fn first_pane_by_def(mstore: &Store) -> HashMap<String, String> {
    use crate::backend::obj::{self, Block, Tab};
    let mut out = HashMap::new();
    for tab in mstore.get_all::<Tab>().unwrap_or_default() {
        for id in &tab.blockids {
            let Some(block) = mstore.get::<Block>(id).ok().flatten() else { continue };
            let def_id = obj::meta_get_string(&block.meta, "agentId", "");
            if !def_id.is_empty() {
                out.entry(def_id).or_insert_with(|| id.clone());
            }
        }
    }
    out
}

/// The panes that watch the agent `def_id`: its seat's lead and its spawned-worker leader (when open on this
/// machine), and the open pane of each agent that filed a task in `filers`. Never its own pane, each once.
pub fn watcher_panes(mstore: &Store, def_id: &str, filers: &[String]) -> Vec<String> {
    let own = notes::live_panes_of(mstore, def_id);
    let mut panes: Vec<String> = Vec::new();
    let mut add = |pane: Option<String>| {
        if let Some(p) = pane.filter(|p| !own.contains(p) && !panes.contains(p)) {
            panes.push(p);
        }
    };
    add(crate::backend::agent_seat::worker_of(mstore, def_id).and_then(|leader| notes::live_panes_of(mstore, &leader).into_iter().next()));
    add(notes::leader_pane_of(mstore, def_id));
    for filer in filers.iter().filter(|f| !f.is_empty() && f.as_str() != def_id) {
        add(notes::live_panes_of(mstore, filer).into_iter().next());
    }
    panes
}

/// Whether the spawned worker `def` owes its leader a report: its leader has messaged it since it last messaged
/// the leader (or it never has).
fn owes_report(mstore: &Store, defs: &[AgentDefinition], def: &AgentDefinition, worker_pane: &str) -> bool {
    let Some(leader_id) = crate::backend::agent_seat::worker_of(mstore, &def.id) else { return false };
    let Some(leader) = defs.iter().find(|d| d.id == leader_id) else { return false };
    let given = notes::last_report(&leader.slug, &[worker_pane.to_string()]);
    let reported = notes::last_report(&def.slug, &notes::live_panes_of(mstore, &leader_id));
    match (given, reported) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(g), Some(r)) => r < g,
    }
}

/// The nudge's text.
fn nudge_text(tasks: &[OwedTask], quiet_minutes: u64) -> String {
    let owes = if tasks.is_empty() { "You have not reported to your leader".to_string() } else { format!("You hold {}", task_list(tasks)) };
    format!("{owes} and nothing has run for {quiet_minutes} minutes. Carry on, or say what you are waiting on.")
}

fn task_list(tasks: &[OwedTask]) -> String {
    tasks.iter().map(|t| format!("#{}", t.num)).collect::<Vec<_>>().join(", ")
}

/// The note to a watcher about the quiet agent `name`.
pub fn quiet_body(name: &str, is_worker: bool, tasks: &[OwedTask], idle_minutes: u64, running: bool, last_words: &str) -> String {
    let who = if is_worker { "worker" } else { "agent" };
    let on = if tasks.is_empty() { " without reporting to you".to_string() } else { format!(" on {}", task_list(tasks)) };
    let state = if running {
        "Its turn is still running but has written nothing.".to_string()
    } else if last_words.is_empty() {
        "It is idle.".to_string()
    } else {
        format!("It is idle after a turn that ended: \"{last_words}\"")
    };
    format!("Your {who} {name} has been quiet for {idle_minutes} min{on}. {state}")
}

/// Start the sweep. Called once from `main`, beside `worker_notes::install`.
pub fn install(mstore: Arc<Store>, work: Arc<Store>, filestore: Arc<FileStore>, config: Arc<ConfigState>) {
    tokio::spawn(async move {
        let watch = Arc::new(parking_lot::Mutex::new(Watch::default()));
        let mut tick = tokio::time::interval(SWEEP_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let (mstore, work, filestore, config, watch) = (mstore.clone(), work.clone(), filestore.clone(), config.clone(), watch.clone());
            // The store reads and the delivery are blocking.
            let _ = tokio::task::spawn_blocking(move || {
                sweep(&mut watch.lock(), &mstore, &work, &filestore, &config, agentmux_common::time::now_ms_u64());
            })
            .await;
        }
    });
}

/// One look at every agent that owes work.
pub fn sweep(watch: &mut Watch, mstore: &Store, work: &Store, filestore: &FileStore, config: &ConfigState, now_ms: u64) {
    let Some(lim) = limits_from(&config.get_settings().extra) else {
        watch.retain(&HashSet::new());
        return;
    };
    let defs = mstore.agent_def_list().unwrap_or_default();
    let panes = first_pane_by_def(mstore);
    notes::absorb(&crate::backend::reactive::get_global_handler().get_audit_log(usize::MAX));
    let claimed = claimed_by_holder(&read_claimed(work, now_ms as i64));
    let mut keep = HashSet::new();
    for def in &defs {
        let Some(block_id) = panes.get(&def.id) else { continue };
        let tasks = owed_by(&claimed, def);
        if tasks.is_empty() && !owes_report(mstore, &defs, def, block_id) {
            continue;
        }
        let state = crate::backend::agent_state::current_state(mstore, block_id);
        let Some(reading) = read(block_id, def, &tasks, state) else { continue };
        keep.insert(def.id.clone());
        match watch.step(&def.id, reading, &lim, now_ms) {
            None => {}
            Some(Action::Nudge) => nudge(&def.id, block_id, &tasks, lim.quiet_ms / 60_000),
            Some(Action::Tell { running, idle_ms }) => {
                if tell(mstore, filestore, config, def, block_id, &tasks, running, idle_ms / 60_000, now_ms) {
                    watch.mark_told(&def.id, now_ms);
                }
            }
        }
    }
    watch.retain(&keep);
}

/// What the controller and the stores say about `def`'s pane, or `None` when it cannot be called quiet: it cannot
/// run, waits for the user, or its controller cannot say when it last wrote (a controller with no turn tracker).
fn read(block_id: &str, def: &AgentDefinition, tasks: &[OwedTask], state: Option<AgentState>) -> Option<Reading> {
    if !callable(state) {
        return None;
    }
    let ctrl = crate::backend::blockcontroller::get_controller(block_id)?;
    let tracker = ctrl.turn_tracker()?;
    let output = ctrl.last_output_ms().and_then(|v| u64::try_from(v).ok()).unwrap_or(0);
    let sent = notes::last_message_by(&def.slug).unwrap_or(0);
    let moved = tasks.iter().map(|t| t.updated_ms).max().unwrap_or(0);
    Some(Reading { activity_ms: output.max(sent).max(moved), turn_active: tracker.pass_started_ms().is_some() })
}

fn nudge(def_id: &str, block_id: &str, tasks: &[OwedTask], quiet_minutes: u64) {
    let Some(target) = notes::note_target(block_id) else { return };
    let text = format!("[AgentMux] {}", crate::backend::reactive::sanitize::quote_in_system_note(&nudge_text(tasks, quiet_minutes)));
    match crate::backend::reactive::get_global_handler().deliver_system_note(&target, &text) {
        Ok(()) => tracing::info!(def_id, "quiet agent nudged"),
        Err(e) => tracing::warn!(def_id, error = %e, "quiet agent not nudged"),
    }
}

#[allow(clippy::too_many_arguments)]
fn tell(mstore: &Store, filestore: &FileStore, config: &ConfigState, def: &AgentDefinition, block_id: &str, tasks: &[OwedTask], running: bool, idle_minutes: u64, now_ms: u64) -> bool {
    let filers: Vec<String> = tasks.iter().map(|t| t.filer_uid.clone()).collect();
    let watchers = watcher_panes(mstore, &def.id, &filers);
    if watchers.is_empty() {
        return false;
    }
    let words = if running { String::new() } else { notes::last_words_of(mstore, filestore, block_id) };
    let is_worker = crate::backend::agent_seat::worker_of(mstore, &def.id).is_some();
    let body = quiet_body(&def.name, is_worker, tasks, idle_minutes, running, &words);
    let Some(body) = notes::admit(&def.id, body, now_ms) else { return false };
    let text = format!("[AgentMux] {}", crate::backend::reactive::sanitize::quote_in_system_note(&body));
    let key = format!("quiet:{}", def.id);
    // Information that asks nothing: a cold leader hears it at its next turn. Told when any watcher took it.
    watchers.iter().fold(false, |told, leader| notes::deliver(mstore, config, leader, &text, Some(&key)) || told)
}

#[cfg(test)]
#[path = "quiet_watch_tests.rs"]
mod tests;
