// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Carries the Supervisor's decisions out: listens for completed turns on the
//! broker, judges supervised panes' turns, sends continuations, writes the
//! state Swarm shows and sends the notifications (spec §3.2 to §3.7).

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Deserialize;

use crate::ambient::digest;
use crate::backend::blockcontroller::core::broadcast_block_update;
use crate::backend::blockcontroller::health::{TriggerKind, TurnEnd, TurnInput, TurnOrigin, TurnTrigger};
use crate::backend::eventbus::EventBus;
use crate::backend::mps::{Broker, MuxEvent, EVENT_AGENT_TURN};
use crate::backend::notify::policy::NotifyKind;
use crate::backend::obj::Block;
use crate::backend::storage::background_tasks::BackgroundTaskStatus;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;

use super::judge::{self, Judgement};
use super::policy::{self, Action, PreJudge, TurnFacts};
use super::{
    message, supports, SupervisorAction, SupervisorDecision, SupervisorMode, SupervisorState, SupervisorStatus,
    META_MODE, META_STATE,
};

/// The fields of a published `agentturn` the Supervisor reads.
#[derive(Deserialize)]
struct PublishedTurn {
    block_id: String,
    turn_id: u64,
    #[serde(default)]
    ended_at_ms: Option<u64>,
    #[serde(default)]
    end: Option<TurnEnd>,
    #[serde(default)]
    trigger: Option<TurnTrigger>,
    #[serde(default)]
    steps: u32,
}

/// A completed turn, as the Supervisor takes it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TurnEnded {
    block_id: String,
    turn_id: u64,
    ended_at_ms: u64,
    trigger: Option<TriggerKind>,
    steps: u32,
}

/// A completed turn from an `agentturn` payload; `None` for a turn still
/// running or settling, or one that ended by its process exiting.
fn parse_turn_ended(data: &serde_json::Value) -> Option<TurnEnded> {
    let t = PublishedTurn::deserialize(data).ok()?;
    if t.end != Some(TurnEnd::Completed) {
        return None;
    }
    Some(TurnEnded {
        block_id: t.block_id,
        turn_id: t.turn_id,
        ended_at_ms: t.ended_at_ms.unwrap_or(0),
        trigger: t.trigger.map(|t| t.kind),
        steps: t.steps,
    })
}

/// Bound on the turn ends remembered to handle each once.
const SEEN_MAX: usize = 1024;

pub struct Supervisor {
    mstore: Arc<Store>,
    filestore: Arc<FileStore>,
    event_bus: Arc<EventBus>,
    broker: Arc<Broker>,
    /// Each supervised pane's run state while srv runs; mirrored to meta.
    runs: Mutex<HashMap<String, SupervisorState>>,
    /// Each pane's latest judgement request; a newer one supersedes an older.
    generations: Mutex<HashMap<String, u64>>,
    /// One evaluation per pane at a time.
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Turn ends already handled: one end can be published twice.
    seen: Mutex<HashSet<(String, u64, u64)>>,
}

static INSTANCE: OnceLock<Arc<Supervisor>> = OnceLock::new();

/// The running Supervisor, once `start` has run.
pub fn instance() -> Option<Arc<Supervisor>> {
    INSTANCE.get().cloned()
}

/// Start the Supervisor: watch the broker for completed turns. Idempotent.
pub fn start(mstore: Arc<Store>, filestore: Arc<FileStore>, event_bus: Arc<EventBus>, broker: Arc<Broker>) {
    let sup = Arc::new(Supervisor {
        mstore,
        filestore,
        event_bus,
        broker: broker.clone(),
        runs: Mutex::default(),
        generations: Mutex::default(),
        locks: Mutex::default(),
        seen: Mutex::default(),
    });
    if INSTANCE.set(sup.clone()).is_err() {
        return;
    }
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<TurnEnded>();
    // On the broker's publish path: parse and enqueue only.
    broker.add_observer(Arc::new(move |ev: &MuxEvent| {
        if ev.event != EVENT_AGENT_TURN {
            return;
        }
        if let Some(end) = ev.data.as_ref().and_then(parse_turn_ended) {
            let _ = tx.send(end);
        }
    }));
    tokio::spawn(async move {
        while let Some(end) = rx.recv().await {
            if !sup.first_sighting(&end) {
                continue;
            }
            let sup = sup.clone();
            tokio::spawn(async move {
                sup.evaluate(&end.block_id, end.trigger, end.steps, true).await;
            });
        }
    });
}

impl Supervisor {
    fn first_sighting(&self, end: &TurnEnded) -> bool {
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        if seen.len() > SEEN_MAX {
            seen.clear();
        }
        seen.insert((end.block_id.clone(), end.turn_id, end.ended_at_ms))
    }

    fn next_generation(&self, block_id: &str) -> u64 {
        let mut g = self.generations.lock().unwrap_or_else(|e| e.into_inner());
        let n = g.entry(block_id.to_string()).or_insert(0);
        *n += 1;
        *n
    }

    fn is_current(&self, block_id: &str, generation: u64) -> bool {
        self.generations.lock().unwrap_or_else(|e| e.into_inner()).get(block_id) == Some(&generation)
    }

    fn lock_for(&self, block_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(|e| e.into_inner());
        locks.entry(block_id.to_string()).or_default().clone()
    }

    fn block(&self, block_id: &str) -> Option<Block> {
        self.mstore.get::<Block>(block_id).ok().flatten()
    }

    fn state_for(&self, block_id: &str, block: &Block, mode: SupervisorMode, now_ms: u64) -> SupervisorState {
        let runs = self.runs.lock().unwrap_or_else(|e| e.into_inner());
        runs.get(block_id)
            .cloned()
            .or_else(|| SupervisorState::from_meta(&block.meta))
            .unwrap_or_else(|| SupervisorState::new(mode, now_ms))
    }

    /// Keep `state` and write it to the pane's meta, for Swarm, unless a newer
    /// request (a later turn, a mode change) has taken the pane over since
    /// `generation`.
    fn save(&self, block_id: &str, generation: u64, state: &SupervisorState) {
        if !self.is_current(block_id, generation) {
            return;
        }
        self.runs.lock().unwrap_or_else(|e| e.into_inner()).insert(block_id.to_string(), state.clone());
        let Ok(value) = serde_json::to_value(state) else { return };
        let written = self.mstore.with_tx(|tx| {
            let mut block = tx.must_get::<Block>(block_id)?;
            block.meta.insert(META_STATE.to_string(), value);
            tx.update(&mut block)?;
            Ok(())
        });
        match written {
            Ok(()) => broadcast_block_update(&self.mstore, &self.event_bus, block_id),
            Err(e) => tracing::warn!(block_id = %block_id, error = %e, "supervisor: could not store its state"),
        }
    }

    fn notify(&self, block_id: &str, kind: NotifyKind, text: Option<String>) {
        let Some(router) = crate::backend::notify::router::get(&self.broker) else { return };
        let block_id = block_id.to_string();
        // The router's emit reads the store: off the async workers.
        tokio::task::spawn_blocking(move || router.emit(kind, &block_id, text.as_deref(), 0));
    }

    /// Judge `block_id`'s last turn and act on it. `notify` is false for the
    /// judgement a mode change asks for: the user is looking at the pane.
    pub async fn evaluate(&self, block_id: &str, trigger: Option<TriggerKind>, steps: u32, notify: bool) {
        let generation = self.next_generation(block_id);
        let lock = self.lock_for(block_id);
        let _held = lock.lock().await;
        if !self.is_current(block_id, generation) {
            return;
        }
        let Some(block) = self.block(block_id) else { return };
        let mode = SupervisorMode::from_meta(&block.meta);
        if mode == SupervisorMode::Off || !supports(&block.meta) {
            return;
        }
        // No readable conversation (none yet, or a hidden memory reinjection).
        let Some(activity) = digest::read_recent_activity(&self.filestore, block_id) else { return };
        let now_ms = agentmux_common::time::now_ms_u64();
        let facts = TurnFacts {
            trigger,
            steps,
            ending: activity.ending,
            background_running: self.background_running(block_id),
            reply: fingerprint(&activity.text),
            now_ms,
        };
        let mut state = self.state_for(block_id, &block, mode, now_ms);
        match policy::on_turn_end(&mut state, mode, &facts) {
            PreJudge::Ignore => {
                self.save(block_id, generation, &state);
                return;
            }
            PreJudge::Wait => {
                policy::apply_wait(&mut state, &facts);
                self.save(block_id, generation, &state);
                return;
            }
            PreJudge::Pause(why) => {
                policy::apply_pause(&mut state, &why, &facts);
                self.save(block_id, generation, &state);
                if notify {
                    self.notify(block_id, NotifyKind::InputWaiting, Some(format!("Supervisor paused: {why}")));
                }
                return;
            }
            PreJudge::Judge => {}
        }

        state.status = SupervisorStatus::Judging;
        state.reason.clear();
        self.save(block_id, generation, &state);

        let outcome = judge::judge(&self.mstore, block_id, generation, mode, &activity.text, state.continuations).await;
        let judged: Option<Judgement> = match outcome {
            judge::Outcome::Superseded => return,
            judge::Outcome::NoVerdict => None,
            judge::Outcome::Judged(j) => Some(j),
        };
        // While the judge ran: a newer turn ended, the mode changed, or the
        // user (or anything else) started the agent again. Any of these takes over.
        if !self.is_current(block_id, generation) {
            return;
        }
        if self.block(block_id).map(|b| SupervisorMode::from_meta(&b.meta)) != Some(mode) {
            return;
        }
        if turn_active(block_id) {
            state.status = SupervisorStatus::Watching;
            self.save(block_id, generation, &state);
            return;
        }

        let verdict = judged.as_ref().map(|j| (j.verdict, j.reason.as_str()));
        let mut action = policy::decide(&state, mode, verdict, &facts);
        if let Action::Continue { n, reason } = &action {
            let next = judged.as_ref().map(|j| j.next.as_str()).unwrap_or("");
            let text = message::continuation(mode, *n, mode.limits().continuations, reason, next);
            if let Err(e) = deliver(block_id, &text) {
                action = Action::Pause(format!("Couldn't continue it: {e}"));
            }
        }
        policy::apply(&mut state, &action, judged.as_ref().map(|j| j.verdict), &facts);
        self.save(block_id, generation, &state);
        if !notify {
            return;
        }
        match action {
            Action::Continue { .. } => {}
            Action::Pause(why) => {
                self.notify(block_id, NotifyKind::InputWaiting, Some(format!("Supervisor paused: {why}")))
            }
            Action::Done(_) | Action::NoVerdict => self.notify(block_id, NotifyKind::TurnCompleted, None),
        }
    }

    fn background_running(&self, block_id: &str) -> bool {
        self.mstore
            .background_task_list_for_block(block_id)
            .map(|tasks| tasks.iter().any(|t| t.status == BackgroundTaskStatus::Running))
            .unwrap_or(false)
    }

    /// Set a pane's mode: a new run, recorded in its history. An idle agent's
    /// last turn is judged at once (spec §3.1).
    pub async fn set_mode(self: &Arc<Self>, block_id: &str, mode: SupervisorMode) -> Result<(), String> {
        let block = self.block(block_id).ok_or_else(|| "no such pane".to_string())?;
        if mode != SupervisorMode::Off && !supports(&block.meta) {
            return Err("the Supervisor works with Claude agents only for now".to_string());
        }
        let now_ms = agentmux_common::time::now_ms_u64();
        // A newer request: an evaluation in flight for the pane gives way
        // without saving, so this never waits behind a judgement.
        self.next_generation(block_id);

        let history = self.state_for(block_id, &block, mode, now_ms).history;
        let mut state = SupervisorState { history, ..SupervisorState::new(mode, now_ms) };
        let label = match mode {
            SupervisorMode::Off => "Turned off.",
            SupervisorMode::Steady => "Set to Steady.",
            SupervisorMode::Drive => "Set to Drive.",
        };
        state.record(SupervisorDecision {
            at_ms: now_ms,
            action: SupervisorAction::ModeSet,
            verdict: None,
            reason: label.to_string(),
        });
        let value = serde_json::to_value(&state).map_err(|e| e.to_string())?;
        self.mstore
            .with_tx(|tx| {
                let mut block = tx.must_get::<Block>(block_id)?;
                if mode == SupervisorMode::Off {
                    block.meta.remove(META_MODE);
                } else {
                    block.meta.insert(META_MODE.to_string(), serde_json::Value::String(mode.as_str().to_string()));
                }
                block.meta.insert(META_STATE.to_string(), value);
                tx.update(&mut block)?;
                Ok(())
            })
            .map_err(|e| e.to_string())?;
        self.runs.lock().unwrap_or_else(|e| e.into_inner()).insert(block_id.to_string(), state);
        broadcast_block_update(&self.mstore, &self.event_bus, block_id);

        if mode != SupervisorMode::Off && !turn_active(block_id) {
            let sup = self.clone();
            let block_id = block_id.to_string();
            tokio::spawn(async move { sup.evaluate(&block_id, None, 0, false).await });
        }
        Ok(())
    }
}

fn turn_active(block_id: &str) -> bool {
    crate::backend::blockcontroller::get_block_controller_status(block_id).is_some_and(|s| s.turn_active)
}

/// Send the continuation the way a broadcast reaches an agent, as automated
/// input. An agent whose process isn't running is not started (spec §3.5).
fn deliver(block_id: &str, text: &str) -> Result<(), String> {
    use crate::backend::reactive::SenderDelivery;
    use crate::bootstrap::AgentRoute;
    let input = TurnInput { origin: TurnOrigin::Automated, text: text.to_string(), joins_turn: None };
    match crate::bootstrap::route_agent_message_from(block_id, text, Some(input), &[]) {
        Ok(AgentRoute::Sent(SenderDelivery::Delivered | SenderDelivery::Deferred)) => Ok(()),
        Ok(AgentRoute::Sent(SenderDelivery::Pty)) => Err("the pane is not an agent session".to_string()),
        Ok(AgentRoute::StartTurn { .. }) => Err("the agent isn't running".to_string()),
        Err(e) => Err(e),
    }
}

/// A fingerprint of the agent's last message in a digest, to see it repeat.
pub(super) fn fingerprint(digest_text: &str) -> String {
    let Some(at) = digest_text.rfind("[assistant] ") else { return String::new() };
    let mut h = std::collections::hash_map::DefaultHasher::new();
    digest_text[at..].trim().hash(&mut h);
    format!("{:016x}", h.finish())
}

#[cfg(test)]
pub(super) fn parse_turn_ended_for_test(data: &serde_json::Value) -> Option<(String, Option<TriggerKind>, u32)> {
    parse_turn_ended(data).map(|t| (t.block_id, t.trigger, t.steps))
}
