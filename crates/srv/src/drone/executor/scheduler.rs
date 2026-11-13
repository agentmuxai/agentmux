// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The step scheduler: runs a graph one block at a time, in the order its edges
//! allow, and takes a loop round again when its guard says so
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.1).
//!
//! A block is **ready** when every edge into it, back-edges apart, has been
//! decided (its source ran and the edge carries the run on, or its source was
//! skipped, or a Condition took the other branch). It **runs** when it has no
//! such edge or at least one of them carries the run on; otherwise it is
//! **skipped**, and so are the blocks that only it led to.
//!
//! A back-edge that carries the run on starts the loop's next iteration: the
//! blocks of the cycle are cleared and its head is ready again. Where a block
//! leaves a loop by an edge that is not taken, the decision is held until
//! nothing else can run, because a later iteration may still take it.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use serde_json::Value;
use tokio::sync::mpsc;

use super::blocks;
use super::engine::{mark_state, RunEvent};
use super::schedule::{self, Guard, OnCap, Plan};
use crate::drone::data_flow::ExecutionScope;
use crate::drone::types::{BlockKind, BlockState, DroneGraph, FlowEdge, FlowNode};
use agentmux_common::time::now_ms;

/// How many earlier outputs of a block a later iteration can read as
/// `{{block.history}}`.
const HISTORY_LEN: usize = 50;

type States = Arc<tokio::sync::Mutex<HashMap<String, BlockState>>>;

/// Why the run must not start another block: what its Agent blocks have spent
/// has reached its cap (`caps::reached`: a run that spent nothing has not
/// reached any cap).
pub(super) fn over_run_cap(spent: f64, cap: Option<f64>) -> Option<String> {
    let cap = cap?;
    crate::backend::jobs::caps::reached(spent, cap).then(|| format!("stopped: the run spent ${spent:.2}, its cap of ${cap:.2}"))
}

fn kind_of(node: &FlowNode) -> Result<BlockKind, String> {
    let kind = node.data.get("kind").and_then(|v| v.as_str()).ok_or_else(|| format!("node {} has no `kind` field", node.id))?;
    BlockKind::parse(kind).ok_or_else(|| format!("unknown block kind: {kind}"))
}

/// Whether `edge` carries the run on once its source has run: a Condition's
/// edge only when its handle matches the result.
fn carries_on(edge: &FlowEdge, source: &FlowNode, output: &Value) -> bool {
    if source.data.get("kind").and_then(|v| v.as_str()) != Some("condition") {
        return true;
    }
    match (edge.source_handle.as_deref(), output.get("result").and_then(|v| v.as_bool())) {
        (Some("true"), Some(r)) => r,
        (Some("false"), Some(r)) => !r,
        // An edge off a Condition with no handle (a graph saved before the
        // canvas set one) is not pruned.
        _ => true,
    }
}

struct Scheduler<'a> {
    run_id: &'a str,
    plan: Plan,
    node_of: HashMap<&'a str, &'a FlowNode>,
    /// The edges into a block, back-edges left out.
    incoming: HashMap<&'a str, Vec<&'a FlowEdge>>,
    outgoing: HashMap<&'a str, Vec<&'a FlowEdge>>,
    /// Edges decided this pass of their loop: `true` when they carry the run on.
    decided: HashMap<&'a str, bool>,
    /// Blocks that ran or were skipped this pass of their loop.
    finished: HashSet<&'a str>,
    ready: VecDeque<(&'a str, bool)>,
    queued: HashSet<&'a str>,
    /// Edges out of a loop that were not taken, decided when nothing can run.
    held: Vec<&'a FlowEdge>,
    /// How many times each guard has sent the run round its loop.
    rounds: HashMap<&'a str, u32>,
    loop_started: HashMap<String, Instant>,
    spent: f64,
    spent_by: HashMap<&'a str, f64>,
    history: HashMap<&'a str, VecDeque<Value>>,
    scope: ExecutionScope,
    response: Option<Value>,
    run_cap_usd: Option<f64>,
    tx: &'a mpsc::UnboundedSender<RunEvent>,
    states: &'a States,
}

pub(super) async fn execute(
    run_id: &str,
    (graph, trigger, run_cap_usd): (&DroneGraph, Value, Option<f64>),
    tx: &mpsc::UnboundedSender<RunEvent>,
    states: &States,
) -> Result<Value, String> {
    if graph.nodes.is_empty() {
        return Err("drone has no blocks".to_string());
    }
    let plan = schedule::analyze(graph).map_err(|issues| {
        issues.iter().map(|i| i.message.as_str()).collect::<Vec<_>>().join("; ")
    })?;
    let mut s = Scheduler::new(run_id, graph, plan, ExecutionScope::with_trigger(trigger), run_cap_usd, tx, states);
    s.run().await?;
    Ok(s.response.unwrap_or(Value::Null))
}

impl<'a> Scheduler<'a> {
    fn new(
        run_id: &'a str,
        graph: &'a DroneGraph,
        plan: Plan,
        scope: ExecutionScope,
        run_cap_usd: Option<f64>,
        tx: &'a mpsc::UnboundedSender<RunEvent>,
        states: &'a States,
    ) -> Self {
        let mut incoming: HashMap<&str, Vec<&FlowEdge>> = HashMap::new();
        let mut outgoing: HashMap<&str, Vec<&FlowEdge>> = HashMap::new();
        for e in &graph.edges {
            outgoing.entry(e.source.as_str()).or_default().push(e);
            if !plan.back_edges.contains(&e.id) {
                incoming.entry(e.target.as_str()).or_default().push(e);
            }
        }
        Self {
            run_id,
            plan,
            node_of: graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect(),
            incoming,
            outgoing,
            decided: HashMap::new(),
            finished: HashSet::new(),
            ready: VecDeque::new(),
            queued: HashSet::new(),
            held: Vec::new(),
            rounds: HashMap::new(),
            loop_started: HashMap::new(),
            spent: 0.0,
            spent_by: HashMap::new(),
            history: HashMap::new(),
            scope,
            response: None,
            run_cap_usd,
            tx,
            states,
        }
    }

    async fn run(&mut self) -> Result<(), String> {
        let mut roots: Vec<&str> = self.node_of.keys().copied().filter(|id| self.incoming.get(id).is_none_or(|v| v.is_empty())).collect();
        roots.sort_unstable();
        for id in roots {
            self.enqueue(id, false);
        }
        loop {
            while let Some((id, forced)) = self.ready.pop_front() {
                self.queued.remove(id);
                self.step(id, forced).await?;
            }
            if self.held.is_empty() {
                return Ok(());
            }
            // Nothing else can run, so the edges a loop did not take are final.
            for edge in std::mem::take(&mut self.held) {
                self.decide(edge, false);
            }
        }
    }

    fn enqueue(&mut self, id: &'a str, forced: bool) {
        if self.finished.contains(id) || !self.queued.insert(id) {
            return;
        }
        self.ready.push_back((id, forced));
    }

    /// Record an edge's outcome, and ready its target if that was the last one.
    fn decide(&mut self, edge: &'a FlowEdge, carries: bool) {
        self.decided.insert(edge.id.as_str(), carries);
        if carries {
            self.held.retain(|h| h.id != edge.id);
        }
        let target = edge.target.as_str();
        if self.incoming.get(target).is_some_and(|ins| ins.iter().all(|e| self.decided.contains_key(e.id.as_str()))) {
            self.enqueue(target, false);
        }
    }

    /// An edge whose source has finished. A edge out of a loop that is not taken
    /// waits, because another iteration may still take it.
    fn leave(&mut self, edge: &'a FlowEdge, carries: bool) {
        let leaves_loop = self.plan.cycle_of.get(edge.source.as_str()).is_some_and(|c| !c.contains(&edge.target));
        if !carries && leaves_loop {
            self.held.push(edge);
        } else {
            self.decide(edge, carries);
        }
    }

    async fn step(&mut self, id: &'a str, forced: bool) -> Result<(), String> {
        let node = self.node_of[id];
        let ins = self.incoming.get(id).cloned().unwrap_or_default();
        let runs = ins.is_empty() || forced || ins.iter().any(|e| self.decided.get(e.id.as_str()) == Some(&true));
        if !runs {
            self.finished.insert(id);
            mark_state(self.states, id, BlockState { status: "skipped".to_string(), output: None, error: None, started_at: None, completed_at: Some(now_ms()) }).await;
            for edge in self.outgoing.get(id).cloned().unwrap_or_default() {
                if !self.plan.back_edges.contains(&edge.id) {
                    self.leave(edge, false);
                }
            }
            return Ok(());
        }
        if let Some(stopped) = over_run_cap(self.spent, self.run_cap_usd) {
            return Err(stopped);
        }
        // A loop's time runs from the first block of its cycle, not from the
        // first time its guard sends the run round.
        let guards: Vec<String> = self.plan.cycle_of.iter().filter(|(g, group)| self.plan.guards.contains_key(*g) && group.contains(id)).map(|(g, _)| g.clone()).collect();
        for guard in guards {
            self.loop_started.entry(guard).or_insert_with(Instant::now);
        }
        let output = self.run_block(node).await?;
        self.finished.insert(id);
        self.route(node, &output)
    }

    /// Run one block with its events and its recorded state.
    async fn run_block(&mut self, node: &'a FlowNode) -> Result<Value, String> {
        let id = node.id.as_str();
        let kind = kind_of(node)?;
        let _ = self.tx.send(RunEvent::BlockStarted { run_id: self.run_id.to_string(), block_id: id.to_string() });
        // Captured once so the running, done and error states carry the same start.
        let started_at = now_ms();
        let state = |status: &str, output: Option<Value>, error: Option<String>, done: bool| BlockState {
            status: status.to_string(),
            output,
            error,
            started_at: Some(started_at),
            completed_at: done.then(now_ms),
        };
        mark_state(self.states, id, state("running", None, None, false)).await;
        let scope = &mut self.scope;
        let result = match kind {
            BlockKind::Variables => blocks::variables::run(node, scope).await,
            BlockKind::Api => blocks::api::run(node, scope).await,
            BlockKind::Condition => blocks::condition::run(node, scope).await,
            BlockKind::Response => blocks::response::run(node, scope).await,
            BlockKind::Agent => blocks::agent::run(node, scope).await,
            BlockKind::Message => blocks::message::run(node, scope).await,
            BlockKind::Task => blocks::task::run(node, scope).await,
            BlockKind::Notify => blocks::notify::run(node, scope).await,
        };
        match result {
            Ok(mut output) => {
                self.keep_history(id, &mut output);
                let cost = output.get("cost_usd").and_then(Value::as_f64).unwrap_or(0.0);
                self.spent += cost;
                *self.spent_by.entry(id).or_default() += cost;
                self.scope.outputs.insert(id.to_string(), output.clone());
                self.mark_bits(node, kind);
                let _ = self.tx.send(RunEvent::BlockDone { run_id: self.run_id.to_string(), block_id: id.to_string(), output: output.clone() });
                mark_state(self.states, id, state("done", Some(output.clone()), None, true)).await;
                if matches!(kind, BlockKind::Response) {
                    // Response emits `{ "value": <resolved-template> }`; the run's
                    // output is the bare value, not the wrapper.
                    self.response = Some(output.get("value").cloned().unwrap_or_else(|| output.clone()));
                }
                Ok(output)
            }
            Err(e) => {
                let _ = self.tx.send(RunEvent::BlockError { run_id: self.run_id.to_string(), block_id: id.to_string(), error: e.clone() });
                mark_state(self.states, id, state("error", None, Some(e.clone()), true)).await;
                Err(format!("block {id} failed: {e}"))
            }
        }
    }

    /// What a block's output is made of, for the taint bits: an Agent's answer, an
    /// API response and a message reply are outside text; a Condition's result is
    /// a boolean made from what it read (derived when that was tainted); a
    /// Response is its template filled in. A Variables block sets its own.
    fn mark_bits(&mut self, node: &FlowNode, kind: BlockKind) {
        let read = |key: &str| node.data.get(key).and_then(Value::as_str).map(|t| self.scope.bits_of(t)).unwrap_or((false, false));
        let bits = match kind {
            BlockKind::Agent | BlockKind::Api | BlockKind::Message => (true, false),
            BlockKind::Condition => {
                let (tainted, derived) = read("expr");
                (false, tainted || derived)
            }
            BlockKind::Response => read("template"),
            BlockKind::Variables => return,
            BlockKind::Task | BlockKind::Notify => (false, false),
        };
        self.scope.bits.insert(node.id.clone(), bits);
    }

    /// A block that runs again carries its earlier outputs, newest last, as
    /// `{{block.history}}`.
    fn keep_history(&mut self, id: &'a str, output: &mut Value) {
        let past = self.history.entry(id).or_default();
        if !past.is_empty() {
            if let Some(map) = output.as_object_mut() {
                map.insert("history".to_string(), Value::Array(past.iter().cloned().collect()));
            }
        }
        let mut plain = output.clone();
        if let Some(map) = plain.as_object_mut() {
            map.remove("history");
        }
        past.push_back(plain);
        if past.len() > HISTORY_LEN {
            past.pop_front();
        }
    }

    /// Send the run along the edges out of a block that ran, and count a loop's
    /// iteration when a guard takes it round.
    fn route(&mut self, node: &'a FlowNode, output: &Value) -> Result<(), String> {
        let id = node.id.as_str();
        let edges = self.outgoing.get(id).cloned().unwrap_or_default();
        let mut carry: Vec<bool> = edges.iter().map(|e| carries_on(e, node, output)).collect();
        if let Some(guard) = self.plan.guards.get(id).cloned() {
            let goes_round = edges.iter().zip(&carry).any(|(e, c)| *c && guard.loop_edges.contains(&e.id));
            if goes_round {
                if let Some(why) = self.loop_cap(id, &guard) {
                    match guard.on_cap {
                        OnCap::EndRun => return Err(why),
                        OnCap::ExitLoop => {
                            // Leaving the loop takes every edge that is not one of its own.
                            for (e, c) in edges.iter().zip(carry.iter_mut()) {
                                *c = !guard.loop_edges.contains(&e.id);
                            }
                            if let Some(map) = self.scope.outputs.get_mut(id).and_then(Value::as_object_mut) {
                                map.insert("capped".to_string(), Value::Bool(true));
                                map.insert("capped_reason".to_string(), Value::String(why));
                            }
                        }
                    }
                } else {
                    *self.rounds.entry(id).or_default() += 1;
                }
            }
        }
        // Back-edges first: going round clears the decisions inside the loop, so
        // an edge decided before it would be erased.
        let (back, forward): (Vec<_>, Vec<_>) = edges.into_iter().zip(carry).partition(|(e, _)| self.plan.back_edges.contains(&e.id));
        for (edge, carries) in back {
            if carries {
                self.go_round(edge);
            }
        }
        for (edge, carries) in forward {
            self.leave(edge, carries);
        }
        Ok(())
    }

    /// Why a loop may not go round again, in plain words, or `None`.
    fn loop_cap(&self, id: &str, guard: &Guard) -> Option<String> {
        let passes = self.rounds.get(id).copied().unwrap_or(0) + 1;
        if passes >= guard.max_iterations {
            return Some(format!("stopped after {passes} of at most {} iterations: the loop at {id} did not finish", guard.max_iterations));
        }
        let group = self.plan.cycle_of.get(id)?;
        if let Some(max) = guard.max_cost_usd {
            let spent: f64 = group.iter().map(|n| self.spent_by.get(n.as_str()).copied().unwrap_or(0.0)).sum();
            if crate::backend::jobs::caps::reached(spent, max) {
                return Some(format!("stopped: the loop at {id} spent ${spent:.2}, its cap of ${max:.2}"));
            }
        }
        if let (Some(max), Some(since)) = (guard.max_minutes, self.loop_started.get(id)) {
            let minutes = since.elapsed().as_secs_f64() / 60.0;
            if minutes >= max {
                return Some(format!("stopped: the loop at {id} ran {minutes:.1} minutes, its cap of {max} minutes"));
            }
        }
        None
    }

    /// A back-edge carried the run on: the loop's blocks run again, from its head.
    fn go_round(&mut self, back: &'a FlowEdge) {
        let head = back.target.as_str();
        let group = self.plan.cycle_of.get(head).cloned().unwrap_or_default();
        for id in &group {
            self.finished.remove(id.as_str());
        }
        let inner: Vec<&'a str> = self
            .incoming
            .values()
            .flatten()
            .filter(|e| group.contains(&e.source) && group.contains(&e.target))
            .map(|e| e.id.as_str())
            .collect();
        for edge in inner {
            self.decided.remove(edge);
        }
        // What the last iteration did not take is decided again by this one.
        self.held.retain(|e| !group.contains(&e.source));
        self.enqueue(head, true);
    }
}

#[cfg(test)]
#[path = "scheduler_tests.rs"]
mod tests;
