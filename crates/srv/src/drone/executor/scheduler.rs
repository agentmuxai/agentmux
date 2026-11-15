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

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::blocks;
use super::engine::{mark_state, Cancel, RunEnd, RunEvent};
use super::retry;
use super::schedule::{self, Guard, OnCap, Plan};
use crate::drone::checkpoint::{Checkpoint, GraphRef};
use crate::drone::data_flow::ExecutionScope;
use crate::drone::host::DroneLookup;
use crate::drone::recorder::{Persist, Recorder};
use crate::drone::types::{BlockKind, BlockState, DroneGraph, FlowEdge, FlowNode};
use agentmux_common::time::now_ms;

/// How many earlier outputs of a block a later iteration can read as
/// `{{block.history}}`.
const HISTORY_LEN: usize = 50;

type States = Arc<tokio::sync::Mutex<HashMap<String, BlockState>>>;

/// What a run is given besides its graph: its dollar cap, the way to cancel it,
/// and where to keep its checkpoints (none for a run that is not kept).
#[derive(Default)]
pub(super) struct Limits {
    pub(super) run_cap_usd: Option<f64>,
    pub(super) cancel: Cancel,
    pub(super) persist: Option<Persist>,
    /// The drones above this one when it was called by a Drone block, outermost first.
    pub(super) chain: Vec<String>,
    /// Where a Drone block finds the drone it calls.
    pub(super) lookup: Option<Arc<dyn DroneLookup>>,
}

/// How long a block may run when its graph does not say (`timeout_seconds`): an
/// Agent block 15 minutes, a block that reaches outside the run 5. An API block
/// has its own request timeout, and the rest do no waiting.
fn default_timeout(kind: BlockKind) -> Option<Duration> {
    match kind {
        BlockKind::Agent => Some(Duration::from_secs(15 * 60)),
        BlockKind::Message | BlockKind::Task | BlockKind::Notify | BlockKind::AppQuery | BlockKind::Spawn => Some(Duration::from_secs(5 * 60)),
        // A wait is for as long as it says, and its own `timeout_seconds` may cut it short.
        BlockKind::Api | BlockKind::Condition | BlockKind::Response | BlockKind::Variables | BlockKind::Transform | BlockKind::ForEach | BlockKind::Wait | BlockKind::Join => None,
        // The callee has the limits of its own blocks, and the caller may set one for the whole call.
        BlockKind::Drone => None,
    }
}

fn plain_duration(d: Duration) -> String {
    let secs = d.as_secs();
    match secs {
        s if s >= 3600 && s % 3600 == 0 => format!("{} hours", s / 3600),
        s if s >= 60 && s % 60 == 0 => format!("{} minutes", s / 60),
        s => format!("{s} seconds"),
    }
}

/// `work`, or its failure when it takes longer than `limit`.
async fn within<T>(limit: Option<Duration>, work: impl Future<Output = Result<T, String>>) -> Result<T, String> {
    let Some(limit) = limit else { return work.await };
    tokio::time::timeout(limit, work).await.unwrap_or_else(|_| Err(format!("timed out after {}", plain_duration(limit))))
}

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
fn carries_on(edge: &FlowEdge, source: &FlowNode, output: &Value, failed: bool) -> bool {
    // An error edge is for a block that failed, and only it: a block that
    // failed takes no other.
    if edge.source_handle.as_deref() == Some("error") {
        return failed;
    }
    if failed {
        return false;
    }
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
    cancel: Cancel,
    /// Set when the run ended because `cancel` did, so that is not taken for a failure.
    cancelled: bool,
    /// The block that is running while it has effects, and whether it was forced:
    /// the one a restart must not run again on its own.
    in_flight: Option<(&'a str, bool)>,
    /// The Wait block the run is stopped at, and whether it was forced, and what
    /// it waits for: a time, as milliseconds since 1970, or the key of an event.
    waiting: Option<(&'a str, bool)>,
    deadlines: BTreeMap<String, i64>,
    resume_on: BTreeMap<String, String>,
    /// When the Wait block runs out of its own `timeout_seconds`.
    wait_timeouts: BTreeMap<String, i64>,
    /// The drones above this one (a Drone block called it), and where to find a drone to call.
    chain: Vec<String>,
    lookup: Option<Arc<dyn DroneLookup>>,
    /// What callees spent that nobody was waiting for any more, by child run: counted at the next block.
    late_spend: Arc<call::LateSpend>,
    /// Writes the run's checkpoints, with the graph version they say the run is on.
    recorder: Option<(Recorder, GraphRef)>,
    started: Instant,
    /// How long the run had already run before this process took it up, when it is resumed.
    elapsed_before_ms: u64,
    tx: &'a mpsc::UnboundedSender<RunEvent>,
    states: &'a States,
}

pub(super) async fn execute(
    run_id: &str,
    args: (&DroneGraph, Value, Limits, &str),
    tx: &mpsc::UnboundedSender<RunEvent>,
    states: &States,
) -> Result<Value, RunEnd> {
    execute_costed(run_id, args, tx, states).await.0
}

/// [`execute`], and with it what the run spent, which a caller counts against its own budget.
pub(super) async fn execute_costed(
    run_id: &str,
    (graph, trigger, limits, drone_id): (&DroneGraph, Value, Limits, &str),
    tx: &mpsc::UnboundedSender<RunEvent>,
    states: &States,
) -> (Result<Value, RunEnd>, f64) {
    if graph.nodes.is_empty() {
        return (Err(RunEnd::Failed("drone has no blocks".to_string())), 0.0);
    }
    let plan = match schedule::analyze(graph) {
        Ok(plan) => plan,
        Err(issues) => return (Err(RunEnd::Failed(issues.iter().map(|i| i.message.as_str()).collect::<Vec<_>>().join("; "))), 0.0),
    };
    let mut s = Scheduler::new(run_id, graph, plan, ExecutionScope::for_drone(drone_id, trigger), limits, tx, states);
    s.seed_roots();
    let started = s.checkpoint(None);
    let ended = finish(&mut s, started).await;
    (ended, s.spent)
}

/// Take up a run from its newest checkpoint, `c`, and run it to its end: the
/// blocks that were finished stay finished and what they said is what the rest
/// read. `graph` must be the graph the checkpoint was written on.
pub(super) async fn resume(
    (graph, c, limits): (&DroneGraph, &Checkpoint, Limits),
    tx: &mpsc::UnboundedSender<RunEvent>,
    states: &States,
) -> Result<Value, RunEnd> {
    let plan = schedule::analyze(graph).map_err(|issues| {
        RunEnd::Failed(issues.iter().map(|i| i.message.as_str()).collect::<Vec<_>>().join("; "))
    })?;
    let mut s = Scheduler::new(&c.run_id, graph, plan, ExecutionScope::for_drone(&c.drone_id, c.trigger.clone()), limits, tx, states);
    s.restore(c).map_err(RunEnd::Failed)?;
    s.show_restored().await;
    if let Some((id, forced)) = s.in_flight {
        // It may have done its work before the machine stopped, and a second time
        // would repeat it: it is not run again on its own, the run waits to be told.
        if let Err(why) = s.settle_in_flight(id, forced).await {
            return Err(if s.cancelled { RunEnd::Cancelled(s.cancel.reason()) } else { RunEnd::Failed(why) });
        }
    }
    finish(&mut s, Ok(())).await
}

/// Run what is left of a run that is set up, and say how it ended. `started` is
/// how setting it up went: a run that could not even keep its first checkpoint
/// ends before its first block.
async fn finish(s: &mut Scheduler<'_>, started: Result<(), String>) -> Result<Value, RunEnd> {
    let ran = match started {
        Ok(()) => s.drive().await,
        Err(e) => Err(e),
    };
    // However it ended (a stop included), what callees it stopped were spending is its own.
    s.settle_late_spend().await;
    match ran {
        Ok(()) => Ok(s.response.take().unwrap_or(Value::Null)),
        Err(_) if s.cancelled => Err(RunEnd::Cancelled(s.cancel.reason())),
        Err(e) => Err(RunEnd::Failed(e)),
    }
}

impl<'a> Scheduler<'a> {
    fn new(
        run_id: &'a str,
        graph: &'a DroneGraph,
        plan: Plan,
        scope: ExecutionScope,
        limits: Limits,
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
            run_cap_usd: limits.run_cap_usd,
            cancel: limits.cancel,
            cancelled: false,
            in_flight: None,
            waiting: None,
            deadlines: BTreeMap::new(),
            resume_on: BTreeMap::new(),
            wait_timeouts: BTreeMap::new(),
            chain: limits.chain,
            lookup: limits.lookup,
            late_spend: Arc::default(),
            recorder: limits.persist.map(|p| (Recorder::new(p.sink, run_id), p.graph)),
            started: Instant::now(),
            elapsed_before_ms: 0,
            tx,
            states,
        }
    }

    /// The blocks nothing leads to are ready first.
    fn seed_roots(&mut self) {
        let mut roots: Vec<&str> = self.node_of.keys().copied().filter(|id| !self.plan.owned.contains(*id) && self.incoming.get(id).is_none_or(|v| v.is_empty())).collect();
        roots.sort_unstable();
        for id in roots {
            self.enqueue(id, false);
        }
    }

    /// Run blocks until none is ready and no edge is left to decide.
    async fn drive(&mut self) -> Result<(), String> {
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
        if self.cancel.is_cancelled() {
            self.cancelled = true;
            return Err(format!("stopped: {}", self.cancel.reason()));
        }
        let node = self.node_of[id];
        let ins = self.incoming.get(id).cloned().unwrap_or_default();
        let runs = ins.is_empty() || forced || ins.iter().any(|e| self.decided.get(e.id.as_str()) == Some(&true)) || self.join_has_failed_branch(id);
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
        self.settle_late_spend().await;
        if let Some(stopped) = over_run_cap(self.spent, self.run_cap_usd) {
            self.cap_reached(id, &stopped);
            return Err(stopped);
        }
        // A loop's time runs from the first block of its cycle, not from the
        // first time its guard sends the run round.
        let guards: Vec<String> = self.plan.cycle_of.iter().filter(|(g, group)| self.plan.guards.contains_key(*g) && group.contains(id)).map(|(g, _)| g.clone()).collect();
        for guard in guards {
            self.loop_started.entry(guard).or_insert_with(Instant::now);
        }
        self.mark_in_flight(node, forced)?;
        let waits = kind_of(node).is_ok_and(|k| k == BlockKind::Wait);
        if waits {
            self.waiting = Some((id, forced));
        }
        let ran = self.run_block(node).await;
        if waits {
            let late = ran.as_ref().is_ok_and(|o| o.get("late").and_then(Value::as_bool).unwrap_or(false));
            self.end_wait(id, late);
        }
        let (output, failed) = match ran {
            Ok(output) => (output, false),
            Err(e) if !self.cancelled && self.handles_failure(id) => {
                // The graph has a handler for this block: its output says what
                // went wrong, as text that came from outside, and the run goes on.
                let output = json!({ "error": e, "failed": true });
                self.scope.outputs.insert(id.to_string(), output.clone());
                self.scope.bits.insert(id.to_string(), (true, false));
                (output, true)
            }
            Err(e) => return Err(e),
        };
        self.finished.insert(id);
        self.in_flight = None;
        self.route(node, &output, failed)?;
        self.checkpoint(Some(id))
    }

    fn has_error_edge(&self, id: &str) -> bool {
        self.outgoing.get(id).is_some_and(|edges| edges.iter().any(|e| e.source_handle.as_deref() == Some("error")))
    }

    fn cap_reached(&self, block_id: &str, reason: &str) {
        let _ = self.tx.send(RunEvent::CapReached { run_id: self.run_id.to_string(), block_id: block_id.to_string(), reason: reason.to_string() });
    }

    /// A block's code, ended when the run is cancelled (an Agent block's process
    /// goes with it) or the block passes its timeout.
    async fn guarded(&mut self, node: &'a FlowNode, kind: BlockKind) -> Result<Value, String> {
        let retry = retry::retry_of(node)?;
        let mut ran = 0;
        loop {
            let result = self.attempt(node, kind).await;
            ran += 1;
            if self.cancelled || !retry::again(retry, kind, &result, ran) {
                return match result {
                    Err(e) if ran > 1 => Err(format!("{e} (after {ran} tries)")),
                    Ok(Value::Object(mut out)) if ran > 1 => {
                        out.insert("tries".to_string(), json!(ran));
                        Ok(Value::Object(out))
                    }
                    other => other,
                };
            }
            let wait = retry.map_or(Duration::ZERO, |r| retry::delay(&r, ran));
            let cancel = self.cancel.clone();
            tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    self.cancelled = true;
                    return Err(format!("stopped: {}", cancel.reason()));
                }
                () = tokio::time::sleep(wait) => {}
            }
        }
    }

    /// One run of a block's code, within its time limit.
    async fn attempt(&mut self, node: &'a FlowNode, kind: BlockKind) -> Result<Value, String> {
        // A wait keeps its own time out in the checkpoint, so a restart does not start it over.
        let limit = match schedule::timeout_seconds(node)? {
            _ if kind == BlockKind::Wait => None,
            Some(secs) => Some(Duration::from_secs(secs)),
            None => default_timeout(kind),
        };
        let cancel = self.cancel.clone();
        tokio::select! {
            biased;
            () = cancel.cancelled() => {
                self.cancelled = true;
                Err(format!("stopped: {}", cancel.reason()))
            }
            result = within(limit, self.dispatch(node, kind)) => result,
        }
    }

    /// Hand a block to the code for its kind. A for-each runs its body block
    /// through [`run_block`](Self::run_block), so it is handled here, not in
    /// `blocks`.
    async fn dispatch(&mut self, node: &'a FlowNode, kind: BlockKind) -> Result<Value, String> {
        if kind == BlockKind::ForEach {
            return self.run_foreach(node).await;
        }
        if kind == BlockKind::Wait {
            return self.run_wait(node).await;
        }
        if kind == BlockKind::Drone {
            return self.run_call(node).await;
        }
        if kind == BlockKind::Join {
            return self.run_join(node);
        }
        let scope = &mut self.scope;
        match kind {
            BlockKind::Variables => blocks::variables::run(node, scope).await,
            BlockKind::Api => blocks::api::run(node, scope).await,
            BlockKind::Condition => blocks::condition::run(node, scope).await,
            BlockKind::Response => blocks::response::run(node, scope).await,
            BlockKind::Agent => blocks::agent::run(node, scope).await,
            BlockKind::Message => blocks::message::run(node, scope).await,
            BlockKind::Task => blocks::task::run(node, scope).await,
            BlockKind::Notify => blocks::notify::run(node, scope).await,
            BlockKind::AppQuery => blocks::app_query::run(node, scope).await,
            BlockKind::Spawn => blocks::spawn::run(node, scope).await,
            BlockKind::Transform => blocks::transform::run(node, scope).await,
            BlockKind::ForEach | BlockKind::Wait | BlockKind::Drone | BlockKind::Join => unreachable!("handled above"),
        }
    }

    /// Run one block with its events and its recorded state.
    async fn run_block(&mut self, node: &'a FlowNode) -> Result<Value, String> {
        let id = node.id.as_str();
        let kind = kind_of(node)?;
        self.settle_late_spend().await;
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
        let spent_before = self.spent;
        let result = self.guarded(node, kind).await;
        // What a block spent and then failed (a callee that ran, then failed) was still spent: it
        // is on the block's state, which is what the ledger sums.
        // Only a Drone block: a for-each's body has put its own spend on its own state.
        let failed_cost = (matches!(kind, BlockKind::Drone) && self.spent > spent_before).then(|| json!({ "cost_usd": self.spent - spent_before }));
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
            Err(e) if self.cancelled => {
                mark_state(self.states, id, state("cancelled", failed_cost, Some(e.clone()), true)).await;
                Err(e)
            }
            Err(e) => {
                let _ = self.tx.send(RunEvent::BlockError { run_id: self.run_id.to_string(), block_id: id.to_string(), error: e.clone() });
                mark_state(self.states, id, state("error", failed_cost, Some(e.clone()), true)).await;
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
            BlockKind::Agent | BlockKind::Api | BlockKind::Message | BlockKind::AppQuery => (true, false),
            BlockKind::Condition => {
                let (tainted, derived) = read("expr");
                (false, tainted || derived)
            }
            BlockKind::Response => read("template"),
            BlockKind::Variables | BlockKind::Transform | BlockKind::ForEach => return,
            // What an event brought is outside text; a wait for a time brings nothing.
            BlockKind::Wait if node.data.get("on").is_some() => (true, false),
            BlockKind::Wait => (false, false),
            // What a callee said is outside text: nothing yet promises its outputs are clean.
            BlockKind::Drone => (true, false),
            BlockKind::Join => self.join_bits(&node.id),
            BlockKind::Task | BlockKind::Notify | BlockKind::Spawn => (false, false),
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
    fn route(&mut self, node: &'a FlowNode, output: &Value, failed: bool) -> Result<(), String> {
        let id = node.id.as_str();
        let edges = self.outgoing.get(id).cloned().unwrap_or_default();
        let mut carry: Vec<bool> = edges.iter().map(|e| carries_on(e, node, output, failed)).collect();
        if let Some(guard) = self.plan.guards.get(id).cloned() {
            let goes_round = edges.iter().zip(&carry).any(|(e, c)| *c && guard.loop_edges.contains(&e.id));
            if goes_round {
                if let Some(why) = self.loop_cap(id, &guard) {
                    self.cap_reached(id, &why);
                    match guard.on_cap {
                        OnCap::EndRun => return Err(why),
                        OnCap::ExitLoop => {
                            // Leaving the loop takes every edge that is not one of its own,
                            // except that an error edge is for a block that failed, and
                            // only it (`carries_on`).
                            for (e, c) in edges.iter().zip(carry.iter_mut()) {
                                let error_edge = e.source_handle.as_deref() == Some("error");
                                *c = !guard.loop_edges.contains(&e.id) && error_edge == failed;
                            }
                            if let Some(map) = self.scope.outputs.get_mut(id).and_then(Value::as_object_mut) {
                                map.insert("capped".to_string(), Value::Bool(true));
                                map.insert("capped_reason".to_string(), Value::String(why));
                            }
                        }
                    }
                } else {
                    let rounds = self.rounds.entry(id).or_default();
                    *rounds += 1;
                    let _ = self.tx.send(RunEvent::LoopIteration { run_id: self.run_id.to_string(), block_id: id.to_string(), iteration: *rounds + 1 });
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

mod attention;
mod call;
mod checkpoint;
mod foreach;
mod join;
mod wait;

#[cfg(test)]
#[path = "scheduler_tests.rs"]
mod tests;
