// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The turn follower: a delivered run follows its pane's `agentturn` events
//! (`blockcontroller::health::TurnLedger`) to the end of the turn it started
//! or joined, and takes that turn's cost, output tokens and steps (spec §3.2).
//!
//! One task applies arms and turn events in the order they arrive. An arm
//! also reads the pane's latest ledger from the broker's history, so a turn
//! that opened while the fire was being delivered, before the arm, is not
//! missed. Applying the same ledger twice changes nothing.

use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::Duration;

use agentmux_common::time::now_ms;

use super::{caps, RunLedger};
use crate::backend::mps::{Broker, MuxEvent, EVENT_AGENT_TURN};
use crate::backend::storage::job_runs::{infra, RunCost};

/// A delivered run whose pane shows no turn for this long is closed with
/// its cost unknown: a pane with no turn ledger (a terminal agent) never
/// reports one.
pub const NO_TURN_AFTER_MS: i64 = 15 * 60 * 1000;
const SWEEP_EVERY: Duration = Duration::from_secs(60);

/// The fields of one `agentturn` payload the follower reads.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnSeen {
    pub block_id: String,
    pub turn_id: i64,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    /// `completed` or `exited` once the turn is over.
    pub end: Option<String>,
    pub cost_usd: f64,
    pub output_tokens: i64,
    pub steps: i64,
}

impl TurnSeen {
    pub fn parse(data: &serde_json::Value) -> Option<Self> {
        let i64_at = |k: &str| data.get(k).and_then(|v| v.as_i64());
        Some(TurnSeen {
            block_id: data.get("block_id")?.as_str()?.to_string(),
            turn_id: i64_at("turn_id")?,
            started_at_ms: i64_at("started_at_ms").unwrap_or(0),
            ended_at_ms: i64_at("ended_at_ms"),
            end: data.get("end").and_then(|v| v.as_str()).map(str::to_string),
            cost_usd: data.get("cost_usd").and_then(|v| v.as_f64()).unwrap_or(0.0),
            output_tokens: i64_at("output_tokens").unwrap_or(0),
            steps: i64_at("steps").unwrap_or(0),
        })
    }

    /// The turn's figures. Only a provider that reports a cost gives one
    /// above zero (Claude's `result` frame today), so zero is "not known".
    pub fn cost(&self) -> RunCost {
        RunCost {
            cost_usd: self.cost_usd,
            cost_known: self.cost_usd > 0.0,
            output_tokens: self.output_tokens,
            steps: self.steps,
        }
    }
}

/// A run waiting on its pane's turn.
#[derive(Debug, Clone, PartialEq)]
pub struct Armed {
    pub run_id: String,
    /// When the fire was delivered: a turn already over by then isn't its.
    pub armed_at_ms: i64,
    pub turn_id: Option<i64>,
    pub last: Option<TurnSeen>,
    /// The spend at which the run is stopped (its per-run cap, §5.3); none
    /// for a provider that reports no cost.
    pub cap_usd: Option<f64>,
}

/// What a turn event means for one armed run.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Bind { turn_id: i64, partial: bool },
    Finish { infra: &'static str, cost: Option<RunCost>, summary: &'static str },
    /// The turn is still going and has spent `cap` dollars: stop it.
    OverBudget { cost: RunCost, cap: f64 },
}

/// Apply `t` to `a`. Pure, so the rules are testable without a broker.
pub fn on_turn(a: &mut Armed, t: &TurnSeen) -> Vec<Effect> {
    let mut out = Vec::new();
    match a.turn_id {
        None => {
            if t.ended_at_ms.is_some_and(|e| e < a.armed_at_ms) {
                return out;
            }
            a.turn_id = Some(t.turn_id);
            out.push(Effect::Bind { turn_id: t.turn_id, partial: t.started_at_ms < a.armed_at_ms });
        }
        // An older turn's late publish.
        Some(bound) if t.turn_id < bound => return out,
        // A newer turn opened: the bound one ended without its closing
        // ledger reaching here. Close with what was last seen.
        Some(bound) if t.turn_id > bound => {
            out.push(Effect::Finish {
                infra: infra::OK,
                cost: a.last.as_ref().map(TurnSeen::cost),
                summary: "",
            });
            return out;
        }
        Some(_) => {}
    }
    a.last = Some(t.clone());
    if let Some(cap) = a.cap_usd.filter(|cap| t.end.is_none() && caps::reached(t.cost_usd, *cap)) {
        out.push(Effect::OverBudget { cost: t.cost(), cap });
        return out;
    }
    if let Some(end) = t.end.as_deref() {
        let status = if end == "exited" { infra::INTERRUPTED } else { infra::OK };
        out.push(Effect::Finish { infra: status, cost: Some(t.cost()), summary: "" });
    }
    out
}

enum Msg {
    Arm { block_id: String, run_id: String, armed_at_ms: i64, cap_usd: Option<f64> },
    /// A run another channel's pane took: checked at each sweep
    /// (`RunLedger::check_remote`, #1988).
    Watch { run_id: String, armed_at_ms: i64 },
    Turn(TurnSeen),
    Sweep,
}

/// The follower's handle: arms go to its task.
pub struct Follower {
    tx: tokio::sync::mpsc::UnboundedSender<Msg>,
}

impl Follower {
    /// Start the task and watch `broker` for turn events. Without a tokio
    /// runtime nothing is followed (and runs stay `running` until restart).
    pub(super) fn start(ledger: Weak<RunLedger>, broker: Arc<Broker>) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Msg>();
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return Follower { tx };
        };
        let observer_tx = tx.clone();
        broker.add_observer(Arc::new(move |ev: &MuxEvent| {
            if ev.event != EVENT_AGENT_TURN {
                return;
            }
            if let Some(t) = ev.data.as_ref().and_then(TurnSeen::parse) {
                let _ = observer_tx.send(Msg::Turn(t));
            }
        }));
        let sweep_tx = tx.clone();
        handle.spawn(async move {
            loop {
                tokio::time::sleep(SWEEP_EVERY).await;
                if sweep_tx.send(Msg::Sweep).is_err() {
                    break;
                }
            }
        });
        handle.spawn(run(ledger, broker, rx));
        Follower { tx }
    }

    pub(super) fn arm(&self, block_id: &str, run_id: &str, armed_at_ms: i64, cap_usd: Option<f64>) {
        let _ = self.tx.send(Msg::Arm { block_id: block_id.to_string(), run_id: run_id.to_string(), armed_at_ms, cap_usd });
    }

    pub(super) fn watch(&self, run_id: &str, armed_at_ms: i64) {
        let _ = self.tx.send(Msg::Watch { run_id: run_id.to_string(), armed_at_ms });
    }
}

async fn run(ledger: Weak<RunLedger>, broker: Arc<Broker>, mut rx: tokio::sync::mpsc::UnboundedReceiver<Msg>) {
    let mut armed: HashMap<String, Vec<Armed>> = HashMap::new();
    let mut watched: HashMap<String, i64> = HashMap::new();
    while let Some(msg) = rx.recv().await {
        let Some(ledger) = ledger.upgrade() else { break };
        match msg {
            Msg::Watch { run_id, armed_at_ms } => {
                watched.insert(run_id, armed_at_ms);
            }
            Msg::Arm { block_id, run_id, armed_at_ms, cap_usd } => {
                armed.entry(block_id.clone()).or_default().push(Armed { run_id, armed_at_ms, turn_id: None, last: None, cap_usd });
                let latest = broker
                    .read_event_history(EVENT_AGENT_TURN, &format!("block:{block_id}"), 1)
                    .pop()
                    .and_then(|e| e.data)
                    .and_then(|d| TurnSeen::parse(&d));
                if let Some(t) = latest {
                    apply(&ledger, &mut armed, &t).await;
                }
            }
            Msg::Turn(t) => apply(&ledger, &mut armed, &t).await,
            Msg::Sweep => {
                let now = now_ms();
                let mut stale = Vec::new();
                for runs in armed.values_mut() {
                    runs.retain(|a| {
                        let lapsed = a.turn_id.is_none() && now - a.armed_at_ms > NO_TURN_AFTER_MS;
                        if lapsed {
                            stale.push(a.run_id.clone());
                        }
                        !lapsed
                    });
                }
                armed.retain(|_, runs| !runs.is_empty());
                for run_id in stale {
                    let ledger = ledger.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        ledger.turn_finished(&run_id, infra::OK, None, "AgentMux saw no turn for this run")
                    })
                    .await;
                }
                if !watched.is_empty() {
                    let (l, list) = (ledger.clone(), watched.clone());
                    let done = tokio::task::spawn_blocking(move || {
                        list.into_iter().filter(|(id, at)| l.check_remote(id, *at, now)).map(|(id, _)| id).collect::<Vec<_>>()
                    })
                    .await
                    .unwrap_or_default();
                    for id in done {
                        watched.remove(&id);
                    }
                }
            }
        }
    }
}

/// Apply one turn event to every run armed on its pane, writing the effects.
async fn apply(ledger: &Arc<RunLedger>, armed: &mut HashMap<String, Vec<Armed>>, t: &TurnSeen) {
    let Some(runs) = armed.get_mut(&t.block_id) else { return };
    let mut writes = Vec::new();
    runs.retain_mut(|a| {
        let effects = on_turn(a, t);
        let done = effects.iter().any(|e| matches!(e, Effect::Finish { .. } | Effect::OverBudget { .. }));
        writes.push((a.run_id.clone(), effects));
        !done
    });
    if runs.is_empty() {
        armed.remove(&t.block_id);
    }
    let ledger = ledger.clone();
    let _ = tokio::task::spawn_blocking(move || {
        for (run_id, effects) in writes {
            for e in effects {
                match e {
                    Effect::Bind { turn_id, partial } => ledger.turn_bound(&run_id, turn_id, partial),
                    Effect::Finish { infra, cost, summary } => ledger.turn_finished(&run_id, infra, cost, summary),
                    Effect::OverBudget { cost, cap } => ledger.over_budget(&run_id, cost, cap),
                }
            }
        }
    })
    .await;
}
