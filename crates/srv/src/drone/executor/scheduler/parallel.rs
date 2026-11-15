// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Branches that run at the same time
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.7).
//!
//! When a block has several successors they are ready together, and the ones that only
//! read the run and give an output (an Agent, an API call, a Message, a Task, a Notify, an
//! App query, a Spawn, a Transform, a Condition) run together as a *batch*: up to
//! [`PARALLELISM`] blocks, of which at most [`MAX_AGENTS`] start an agent. The fleet's limit of
//! three unattended agent runs counts a drone run as one run, so a run starts one agent at a
//! time, as before, until a run can reserve shared agent capacity; the other blocks of a batch
//! only read, call or send. Each block of a batch reads the run as it
//! was when the batch began; their outputs are taken up one by one in the order they were
//! readied, so what the run does next does not depend on which finished first. A block that
//! writes the run (Variables, Response), waits, calls or loops is not batched: it runs by
//! itself, as before.
//!
//! The run keeps a checkpoint that lists every block of the batch that has effects as
//! running before any of them starts, so a restart asks about each of them in turn.

use std::time::Duration;

use serde_json::{json, Value};

use super::{dispatch_scoped, over_run_cap, within, Scheduler};
use crate::drone::executor::engine::Cancel;
use crate::drone::executor::{retry, schedule};
use crate::drone::data_flow::ExecutionScope;
use crate::drone::types::{BlockKind, FlowNode};

/// The most blocks that run at once.
pub(super) const PARALLELISM: usize = 4;

/// The most blocks that start an agent at once in one run. The fleet's limit of unattended agent
/// runs counts each drone run as one, so a run that started more would exceed it with the others.
pub(super) const MAX_AGENTS: usize = 1;

/// The kinds that only read the run and give an output.
fn is_pure(kind: BlockKind) -> bool {
    matches!(
        kind,
        BlockKind::Agent | BlockKind::Api | BlockKind::Message | BlockKind::Task | BlockKind::Notify | BlockKind::AppQuery | BlockKind::Spawn | BlockKind::Transform | BlockKind::Condition
    )
}

impl<'a> Scheduler<'a> {
    /// The blocks at the front of the ready list that can run together, in order: none when
    /// the first cannot, one when only it can.
    pub(super) fn next_batch(&self) -> Vec<(&'a str, bool)> {
        let mut batch = Vec::new();
        let mut agents = 0;
        for &(id, forced) in &self.ready {
            let kind = super::kind_of(self.node_of[id]).ok().filter(|k| is_pure(*k));
            let in_loop = self.plan.cycle_of.values().any(|group| group.contains(id));
            let Some(kind) = kind.filter(|_| !in_loop && self.will_run(id, forced)) else { break };
            let starts_agent = matches!(kind, BlockKind::Agent | BlockKind::Spawn);
            if batch.len() >= PARALLELISM || (starts_agent && agents >= MAX_AGENTS) {
                break;
            }
            agents += usize::from(starts_agent);
            batch.push((id, forced));
        }
        batch
    }

    /// Run `batch`, which [`next_batch`](Self::next_batch) gave, together. Every block's
    /// outcome is taken up even when one fails; the first failure the graph does not handle
    /// is the run's.
    pub(super) async fn step_batch(&mut self, batch: Vec<(&'a str, bool)>) -> Result<(), String> {
        if self.cancel.is_cancelled() {
            self.cancelled = true;
            return Err(format!("stopped: {}", self.cancel.reason()));
        }
        for _ in &batch {
            if let Some((id, _)) = self.ready.pop_front() {
                self.queued.remove(id);
            }
        }
        self.settle_late_spend().await;
        if let Some(stopped) = over_run_cap(self.spent, self.run_cap_usd) {
            self.cap_reached(batch[0].0, &stopped);
            return Err(stopped);
        }
        self.mark_batch_in_flight(&batch)?;
        let mut began = Vec::new();
        for (id, _) in &batch {
            let node = self.node_of[*id];
            let kind = super::kind_of(node)?;
            began.push((node, kind, self.begin_block(node).await));
        }
        let spent_before = self.spent;
        let runs: Vec<_> = began.iter().map(|(node, kind, _)| run_pure(node, *kind, self.scope.clone(), self.cancel.clone())).collect();
        let results = futures_util::future::join_all(runs).await;
        let mut first_failure = None;
        for ((node, kind, started_at), pure) in began.into_iter().zip(results) {
            self.cancelled |= pure.cancelled;
            if let Some(bits) = pure.bits {
                self.scope.bits.insert(node.id.clone(), bits);
            }
            let ran = self.end_block(node, kind, pure.result, (started_at, spent_before)).await;
            if let Err(e) = self.finish_step(node, ran) {
                first_failure.get_or_insert(e);
            }
        }
        first_failure.map_or(Ok(()), Err)
    }
}

/// What a block of a batch came to.
pub(super) struct Pure {
    pub(super) result: Result<Value, String>,
    /// The run was stopped while it ran.
    pub(super) cancelled: bool,
    /// The bits the block set on its own output, as a Transform does.
    pub(super) bits: Option<(bool, bool)>,
}

/// A block's code on its own copy of the run's scope: the same limit of time, the same
/// retries and the same stop as [`Scheduler::guarded`] gives a block that runs by itself.
pub(super) async fn run_pure(node: &FlowNode, kind: BlockKind, mut scope: ExecutionScope, cancel: Cancel) -> Pure {
    let done = |result: Result<Value, String>, cancelled: bool, scope: &ExecutionScope| Pure { result, cancelled, bits: scope.bits.get(&node.id).copied() };
    let retry = match retry::retry_of(node) {
        Ok(retry) => retry,
        Err(e) => return done(Err(e), false, &scope),
    };
    let limit = match schedule::timeout_seconds(node) {
        Ok(Some(secs)) => Some(Duration::from_secs(secs)),
        Ok(None) => super::default_timeout(kind),
        Err(e) => return done(Err(e), false, &scope),
    };
    let mut ran = 0;
    loop {
        let stopped = || Err(format!("stopped: {}", cancel.reason()));
        let result = tokio::select! {
            biased;
            () = cancel.cancelled() => return done(stopped(), true, &scope),
            result = within(limit, dispatch_scoped(node, kind, &mut scope)) => result,
        };
        ran += 1;
        if !retry::again(retry, kind, &result, ran) {
            let result = match result {
                Err(e) if ran > 1 => Err(format!("{e} (after {ran} tries)")),
                Ok(Value::Object(mut out)) if ran > 1 => {
                    out.insert("tries".to_string(), json!(ran));
                    Ok(Value::Object(out))
                }
                other => other,
            };
            return done(result, false, &scope);
        }
        let wait = retry.map_or(Duration::ZERO, |r| retry::delay(&r, ran));
        tokio::select! {
            biased;
            () = cancel.cancelled() => return done(stopped(), true, &scope),
            () = tokio::time::sleep(wait) => {}
        }
    }
}

#[cfg(test)]
#[path = "parallel_tests.rs"]
mod tests;
