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
use std::sync::{Arc, LazyLock};

use crate::backend::agent_state::AgentState;
use crate::backend::obj::MetaMapType;
use crate::backend::work_facts::live_phrase;
use crate::backend::storage::agents::AgentDefinition;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;
use crate::backend::storage::work_queue::work_state;
use crate::backend::wconfig::ConfigState;
use super::urgent_asks::{self, Ask};
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
/// An agent whose turn ended with a command of its own still running in the background is waiting on that command, not
/// stuck: its quiet time is this many times the usual one, and it is not nudged (a nudge would only cost it a turn).
const BACKGROUND_FACTOR: u64 = 4;
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
    /// Between turns, with a command or job the agent started still running in the background (`background_set`).
    pub background: bool,
}

/// The quiet time of an agent between turns: longer while a command of its own runs in the background.
fn idle_limit(background: bool, lim: &Limits) -> u64 {
    if background {
        lim.quiet_ms.saturating_mul(BACKGROUND_FACTOR)
    } else {
        lim.quiet_ms
    }
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
        // Told once for a quiet: the same agent is not told again until it moves (a new episode).
        let may_tell = |ep: &Episode| ep.told_at.is_none();
        if reading.turn_active {
            if idle >= lim.running_ms.saturating_add(lim.escalate_ms) && may_tell(ep) {
                return Some(Action::Tell { running: true, idle_ms: idle });
            }
            return None;
        }
        if idle < idle_limit(reading.background, lim) {
            return None;
        }
        if reading.background {
            // It is waiting on its own command: no nudge, its watchers are told (naming the command).
            ep.nudge_skipped = true;
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

    /// When the quiet that `step` is looking at began (unix ms), if the agent `key` is quiet at `now_ms`: it is between
    /// turns and idle for the quiet time, or in a turn that has written nothing for the longer running time. Whether
    /// it was nudged or its watchers told does not matter: this is what the Swarm and the Kanban show.
    pub fn quiet_since(&self, key: &str, turn_active: bool, background: bool, lim: &Limits, now_ms: u64) -> Option<u64> {
        let at = self.episodes.get(key)?.activity_at;
        let limit = if turn_active { lim.running_ms } else { idle_limit(background, lim) };
        (now_ms.saturating_sub(at) >= limit).then_some(at)
    }

    /// The watchers of `key` were told at `now_ms`: it is not told again until it moves. Called only once a note went
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

/// An agent that is quiet now, as the Swarm and the Kanban show it (the `quiet` group of the live facts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuietView {
    /// When the quiet began (unix ms): the agent's last activity.
    pub since_ms: u64,
    /// The tasks it holds (numbers); empty for a spawned worker that owes its leader a report.
    pub tasks: Vec<i64>,
    /// The agents told when it has been quiet long enough (their names).
    pub watchers: Vec<String>,
}

/// The agents quiet at the last sweep, by definition id. A sweep replaces it whole.
static QUIET: std::sync::LazyLock<parking_lot::Mutex<HashMap<String, QuietView>>> = std::sync::LazyLock::new(Default::default);

/// `def_id` as the last sweep found it, when it was quiet.
pub fn quiet_view(def_id: &str) -> Option<QuietView> {
    QUIET.lock().get(def_id).cloned()
}

/// The most minutes a leader may mute one agent for.
pub const MAX_MUTE_MINUTES: u64 = 240;
/// The longest reason kept, in characters.
const MUTE_REASON_CHARS: usize = 120;

/// A leader's mute of one agent: its quiet clock is stopped until `until_ms`, then runs again from there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mute {
    pub until_ms: u64,
    /// Why, in the leader's words (cut, one line).
    pub reason: String,
    /// The agent that muted it (its slug): recorded as that agent, never as the operator.
    pub by: String,
}

/// The mutes in force, by the muted agent's definition id. Kept in memory: a mute is for at most four hours, and a
/// restart of srv gives the agent a whole quiet time anyway.
static MUTES: LazyLock<parking_lot::Mutex<HashMap<String, Mute>>> = LazyLock::new(Default::default);

/// Mute `def_id` for `minutes` (1 to [`MAX_MUTE_MINUTES`]) from `now_ms`, replacing an earlier mute of it.
pub fn mute(def_id: &str, minutes: u64, reason: &str, by: &str, now_ms: u64) -> Result<Mute, String> {
    if minutes == 0 || minutes > MAX_MUTE_MINUTES {
        return Err(format!("minutes must be 1 to {MAX_MUTE_MINUTES} (0 lifts a mute); got {minutes}"));
    }
    let reason: String = reason.split_whitespace().collect::<Vec<_>>().join(" ").chars().filter(|c| !c.is_control()).take(MUTE_REASON_CHARS).collect();
    let muted = Mute { until_ms: now_ms.saturating_add(minutes.saturating_mul(60_000)), reason, by: by.to_string() };
    MUTES.lock().insert(def_id.to_string(), muted.clone());
    Ok(muted)
}

/// Lift the mute of `def_id`; whether there was one in force.
pub fn unmute(def_id: &str, now_ms: u64) -> bool {
    MUTES.lock().remove(def_id).is_some_and(|m| m.until_ms > now_ms)
}

/// The mute of `def_id` in force at `now_ms`, if any.
pub fn mute_of(def_id: &str, now_ms: u64) -> Option<Mute> {
    MUTES.lock().get(def_id).filter(|m| m.until_ms > now_ms).cloned()
}

/// How long an ended mute is kept for the quiet clock: the clock runs again from the mute's end, which a sweep that comes
/// late (the machine slept) must still know.
const ENDED_MUTE_KEPT_MS: u64 = 24 * 60 * 60_000;

/// The mute of `def_id` as the quiet clock reads it: one in force, or one that ended at most a day ago (so a sweep that
/// runs late still starts the clock at the mute's end, not at the last sweep). Older ones are dropped.
fn mute_for_clock(def_id: &str, now_ms: u64) -> Option<Mute> {
    let mut mutes = MUTES.lock();
    mutes.retain(|_, m| now_ms.saturating_sub(m.until_ms) < ENDED_MUTE_KEPT_MS);
    mutes.get(def_id).cloned()
}

/// Whether `caller` leads `target` (both agent uids): it is the lead of the target's seat, which for a spawned worker is
/// the agent that spawned it. Only a lead may mute.
pub fn leads(mstore: &Store, caller: &str, target: &str) -> bool {
    let seat_lead = crate::backend::agent_seat::effective_seat_by_id(mstore, target).and_then(|seat| seat.lead);
    seat_lead.as_deref() == Some(caller) || crate::backend::agent_seat::worker_of(mstore, target).as_deref() == Some(caller)
}

/// When the newest of the tasks the agent holds moved (unix ms; 0 for none). An urgent message sent to it is not
/// the agent's own activity, so the tasks it only asked about do not count.
fn held_moved_ms(tasks: &[OwedTask]) -> u64 {
    tasks.iter().filter(|t| !t.asked).map(|t| t.updated_ms).max().unwrap_or(0)
}

/// Whether the agent holds on purpose while quota mode is tight: it set its own `waiting` status line (the leader's
/// note would only cost the leader a turn to read "idle by design"). The clock is stopped for as long as that line
/// is its current one and the mode stays tight.
pub fn holding_for_quota(status: &MetaMapType, quota_tight: bool) -> bool {
    quota_tight && live_phrase::declared_of(status).is_some_and(|line| line.state.as_deref() == Some("waiting") && line.src == live_phrase::SRC_AGENT)
}

/// What the agent's status line and a mute add to its clock (unix ms; 0 for nothing). A new status line is something
/// the agent did, so it counts at the time it was set. A `waiting` line with its time, and a mute, stop the clock:
/// while one is in force the agent counts as active now, and when it ends the clock runs again from its end.
pub fn paused_activity(status: &MetaMapType, mute: Option<&Mute>, now_ms: u64) -> u64 {
    let mut at = live_phrase::declared_of(status).map_or(0, |line| line.at_ms);
    if let Some(until) = live_phrase::waiting_until(status) {
        at = at.max(until.min(now_ms));
    }
    if let Some(m) = mute {
        at = at.max(m.until_ms.min(now_ms));
    }
    at
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
    /// Not held: an urgent message from a proven sender named it and the agent has not answered (`urgent_asks`).
    pub asked: bool,
}

/// The claimed tasks of every holder, keyed by the holder's uid, or `name:<lowercased claimed_by>` when the
/// holder's uid is not known: the key the Swarm's facts use. A task on hold, waiting on a question, in review,
/// done or open is not claimed and is not here.
pub fn claimed_by_holder(items: &[crate::backend::storage::work_board::BoardTask]) -> HashMap<String, Vec<OwedTask>> {
    let mut out: HashMap<String, Vec<OwedTask>> = HashMap::new();
    for t in items.iter().filter(|t| t.item.state == work_state::CLAIMED) {
        let key = if t.item.claimed_by_uid.is_empty() { format!("name:{}", t.item.claimed_by.to_lowercase()) } else { t.item.claimed_by_uid.clone() };
        let moved = t.item.updated_at.max(t.card.updated_at).max(0) as u64;
        out.entry(key).or_default().push(OwedTask { num: t.card.num, filer_uid: t.item.created_by_uid.clone(), updated_ms: moved, asked: false });
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

/// Whether an urgent ask is answered: the recipient messaged the sender since (`replied_ms`), updated or completed a
/// task it named (a held task that moved since), or none of the tasks named is waiting on it any more (done, handed
/// elsewhere or cancelled). Pure.
pub fn ask_answered(ask: &Ask, replied_ms: Option<u64>, held: &[OwedTask], still_waiting: bool) -> bool {
    replied_ms.is_some_and(|at| at >= ask.at_ms) || held.iter().any(|t| ask.tasks.contains(&t.num) && t.updated_ms > ask.at_ms) || !still_waiting
}

/// The tasks each agent was asked about by an urgent message and has not answered, by the agent's definition id, as
/// `asked` entries whose filer is the sender (so the sender is told, as a filer is). The answered asks are forgotten.
fn open_asks_by_def(mstore: &Store, work: &Store, defs: &[AgentDefinition], claimed: &HashMap<String, Vec<OwedTask>>, now_ms: u64) -> HashMap<String, Vec<OwedTask>> {
    use crate::backend::obj::{self, Block};
    let mut open: HashMap<String, Vec<OwedTask>> = HashMap::new();
    let mut done: Vec<Ask> = Vec::new();
    for ask in urgent_asks::open_asks(now_ms) {
        let def_id = mstore.get::<Block>(&ask.block_id).ok().flatten().map(|b| obj::meta_get_string(&b.meta, "agentId", "")).unwrap_or_default();
        let Some(def) = defs.iter().find(|d| d.id == def_id) else {
            done.push(ask);
            continue;
        };
        let Some(sender) = defs.iter().find(|d| d.slug.eq_ignore_ascii_case(&ask.sender) || d.name.eq_ignore_ascii_case(&ask.sender) || d.id == ask.sender) else { continue };
        let held = owed_by(claimed, def);
        let replied = notes::last_report(&def.slug, &notes::live_panes_of(mstore, &sender.id));
        let waiting = work.work_numbers_waiting_on(&ask.tasks, &def.id, &def.slug).unwrap_or(true);
        if ask_answered(&ask, replied, &held, waiting) {
            done.push(ask);
            continue;
        }
        let entry = open.entry(def.id.clone()).or_default();
        for num in &ask.tasks {
            if !entry.iter().any(|t| t.num == *num && t.filer_uid == sender.id) {
                entry.push(OwedTask { num: *num, filer_uid: sender.id.clone(), updated_ms: ask.at_ms, asked: true });
            }
        }
    }
    if !done.is_empty() {
        urgent_asks::settle(|a| done.contains(a));
    }
    open
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
    let (held, asked) = split_asked(tasks);
    let mut owes: Vec<String> = Vec::new();
    if !held.is_empty() {
        owes.push(format!("You hold {}", task_list(&held)));
    }
    if !asked.is_empty() {
        let lead = if held.is_empty() { "You have" } else { "have" };
        owes.push(format!("{lead} an unanswered urgent message about {}", task_list(&asked)));
    }
    let owes = if owes.is_empty() { "You have not reported to your leader".to_string() } else { owes.join(", ") };
    format!("{owes} and nothing has run for {quiet_minutes} minutes. Carry on, or say what you are waiting on.")
}

/// The tasks an agent holds, and the ones an urgent message asked it about.
fn split_asked(tasks: &[OwedTask]) -> (Vec<OwedTask>, Vec<OwedTask>) {
    tasks.iter().cloned().partition(|t| !t.asked)
}

fn task_list(tasks: &[OwedTask]) -> String {
    let mut nums: Vec<i64> = tasks.iter().map(|t| t.num).collect();
    nums.sort_unstable();
    nums.dedup();
    nums.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(", ")
}

/// The note to a watcher about the quiet agent `name`.
pub fn quiet_body(name: &str, is_worker: bool, tasks: &[OwedTask], idle_minutes: u64, running: bool, last_words: &str, background: &str) -> String {
    let who = if is_worker { "worker" } else { "agent" };
    let (held, asked) = split_asked(tasks);
    let on = match (held.is_empty(), asked.is_empty()) {
        (true, true) => " without reporting to you".to_string(),
        (false, true) => format!(" on {}", task_list(&held)),
        (true, false) => format!(" without answering your urgent message about {}", task_list(&asked)),
        (false, false) => format!(" on {}, and has not answered your urgent message about {}", task_list(&held), task_list(&asked)),
    };
    let state = if running {
        "Its turn is still running but has written nothing.".to_string()
    } else if !background.is_empty() {
        format!("Its turn ended, and a background command is still running: {background}")
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

/// The tasks an agent owes: the ones it holds, and the ones an urgent message asked it about, each ask kept once. A
/// task that is both held and asked stays in both lists, so the sender of the message is a watcher (its filer) and
/// the words name the message as well as the task.
fn with_asked(mut held: Vec<OwedTask>, asked: Vec<OwedTask>) -> Vec<OwedTask> {
    for more in asked {
        if !held.iter().any(|t| t.asked && t.num == more.num && t.filer_uid == more.filer_uid) {
            held.push(more);
        }
    }
    held.sort_by_key(|t| (t.num, t.asked));
    held
}

/// One look at every agent that owes work.
pub fn sweep(watch: &mut Watch, mstore: &Store, work: &Store, filestore: &FileStore, config: &ConfigState, now_ms: u64) {
    let Some(lim) = limits_from(&config.get_settings().extra) else {
        watch.retain(&HashSet::new());
        QUIET.lock().clear();
        return;
    };
    let defs = mstore.agent_def_list().unwrap_or_default();
    let panes = first_pane_by_def(mstore);
    notes::absorb(&crate::backend::reactive::get_global_handler().get_audit_log(usize::MAX));
    let claimed = claimed_by_holder(&read_claimed(work, now_ms as i64));
    let mut asked = open_asks_by_def(mstore, work, &defs, &claimed, now_ms);
    let quota_tight = crate::backend::quota_mode::is_tight(&config.get_settings().extra);
    let mut keep = HashSet::new();
    let mut quiet_now = HashMap::new();
    for def in &defs {
        let Some(block_id) = panes.get(&def.id) else { continue };
        let tasks = with_asked(owed_by(&claimed, def), asked.remove(&def.id).unwrap_or_default());
        if tasks.is_empty() && !owes_report(mstore, &defs, def, block_id) {
            continue;
        }
        let state = crate::backend::agent_state::current_state(mstore, block_id);
        let Some(reading) = read(mstore, block_id, def, &tasks, state, now_ms, quota_tight) else { continue };
        keep.insert(def.id.clone());
        let action = watch.step(&def.id, reading, &lim, now_ms);
        if let Some(since_ms) = watch.quiet_since(&def.id, reading.turn_active, reading.background, &lim, now_ms) {
            let filers: Vec<String> = tasks.iter().map(|t| t.filer_uid.clone()).collect();
            let watchers = watcher_panes(mstore, &def.id, &filers).iter().filter_map(|p| agent_name_of_pane(mstore, &defs, p)).collect();
            let mut nums: Vec<i64> = tasks.iter().map(|t| t.num).collect();
            nums.dedup();
            quiet_now.insert(def.id.clone(), QuietView { since_ms, tasks: nums, watchers });
        }
        match action {
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
    *QUIET.lock() = quiet_now;
}

/// The name of the agent whose pane is `block_id`.
fn agent_name_of_pane(mstore: &Store, defs: &[AgentDefinition], block_id: &str) -> Option<String> {
    use crate::backend::obj::{self, Block};
    let block = mstore.get::<Block>(block_id).ok().flatten()?;
    let def_id = obj::meta_get_string(&block.meta, "agentId", "");
    defs.iter().find(|d| d.id == def_id).map(|d| d.name.clone())
}

/// What the controller and the stores say about `def`'s pane, or `None` when it cannot be called quiet: it cannot
/// run, waits for the user, or its controller cannot say when it last wrote (a controller with no turn tracker).
fn read(mstore: &Store, block_id: &str, def: &AgentDefinition, tasks: &[OwedTask], state: Option<AgentState>, now_ms: u64, quota_tight: bool) -> Option<Reading> {
    if !callable(state) {
        return None;
    }
    let ctrl = crate::backend::blockcontroller::get_controller(block_id)?;
    let tracker = ctrl.turn_tracker()?;
    let output = ctrl.last_output_ms().and_then(|v| u64::try_from(v).ok()).unwrap_or(0);
    let sent = notes::last_message_by(&def.slug).unwrap_or(0);
    let moved = held_moved_ms(tasks);
    let status = mstore.get::<crate::backend::obj::Block>(block_id).ok().flatten().map(|b| b.meta).unwrap_or_default();
    let paused = if holding_for_quota(&status, quota_tight) { now_ms } else { paused_activity(&status, mute_for_clock(&def.id, now_ms).as_ref(), now_ms) };
    let turn_active = tracker.pass_started_ms().is_some();
    let background = !turn_active && !super::background_set::for_block(block_id, now_ms).is_empty();
    Some(Reading { activity_ms: output.max(sent).max(moved).max(paused), turn_active, background })
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
/// What the agent in `block_id` has running in the background, in a few words (one line, cut): the first command, and how
/// many more there are.
fn background_label(block_id: &str, now_ms: u64) -> String {
    let items = super::background_set::for_block(block_id, now_ms);
    let Some(first) = items.first() else { return String::new() };
    let one_line: String = first.label.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(90).collect();
    match items.len() {
        1 => one_line,
        n => format!("{one_line} (and {} more)", n - 1),
    }
}

fn tell(mstore: &Store, filestore: &FileStore, config: &ConfigState, def: &AgentDefinition, block_id: &str, tasks: &[OwedTask], running: bool, idle_minutes: u64, now_ms: u64) -> bool {
    let filers: Vec<String> = tasks.iter().map(|t| t.filer_uid.clone()).collect();
    let watchers = watcher_panes(mstore, &def.id, &filers);
    if watchers.is_empty() {
        return false;
    }
    let words = if running { String::new() } else { notes::last_words_of(mstore, filestore, block_id) };
    let is_worker = crate::backend::agent_seat::worker_of(mstore, &def.id).is_some();
    let background = if running { String::new() } else { background_label(block_id, now_ms) };
    let body = quiet_body(&def.name, is_worker, tasks, idle_minutes, running, &words, &background);
    let Some(body) = notes::admit(&def.id, body, now_ms) else { return false };
    // Information that asks nothing: a cold leader hears it at its next turn, and with the other notes about its agents
    // when they fall in the same window (`note_outbox`).
    let line = super::note_outbox::Line {
        agent: def.id.clone(),
        text: crate::backend::reactive::sanitize::quote_in_system_note(&body),
        held_key: Some(format!("quiet:{}", def.id)),
    };
    super::note_outbox::queue(mstore, config, &watchers, &line, now_ms);
    true
}

#[cfg(test)]
#[path = "quiet_watch_tests.rs"]
mod tests;
