// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The for-each block: runs its body block once per item of a list
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.1).
//!
//! The body is a block of the graph with no edges of its own (`schedule.rs`
//! checks that), so the scheduler leaves it alone and this runs it with
//! `{{item}}` and `{{index}}` set. The output is
//! `{ "results": [..], "errors": [..], "count": n, "capped": bool }`:
//! `results` holds each finished body's output in order, `errors` the items
//! that failed when `on_item_error` is `continue`.
//!
//! Items run one at a time. Every limit is hard and ends the run with a reason
//! in plain words (`on_cap: exit_loop` goes on instead, with `capped` set):
//! `max_items` (100, at most 1,000), the run's dollar cap, and the loop's own
//! `max_cost_usd` and `max_minutes`.

use std::time::Instant;

use serde_json::{json, Value};

use super::{over_run_cap, Scheduler};
use crate::drone::executor::schedule::{foreach_config, ForEachConfig, OnCap, OnItemError};
use crate::drone::types::FlowNode;

fn kind_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "nothing",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "text",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

impl<'a> Scheduler<'a> {
    pub(super) async fn run_foreach(&mut self, node: &'a FlowNode) -> Result<Value, String> {
        let id = node.id.as_str();
        let cfg = foreach_config(node).map_err(|e| format!("for-each {id}: {e}"))?;
        let body = self.node_of.get(cfg.body.as_str()).copied().ok_or_else(|| format!("for-each {id}: the body {:?} is not a block", cfg.body))?;
        let (tainted, derived) = self.scope.bits_of(&cfg.items);
        let items = match self.scope.value_of(&cfg.items) {
            Value::Array(items) => items,
            other => return Err(format!("for-each {id}: items must be a list, but {} is {}", cfg.items, kind_name(&other))),
        };
        let total = items.len();
        let (started, spent_before) = (Instant::now(), self.spent);
        let mut results = Vec::new();
        let mut errors = Vec::new();
        let mut stopped: Option<String> = None;
        for (index, item) in items.into_iter().enumerate() {
            if let Some(why) = self.item_limit(&cfg, id, (index, total), (started, spent_before)) {
                stopped = Some(why);
                break;
            }
            self.scope.locals.insert("item".to_string(), item);
            self.scope.locals.insert("index".to_string(), json!(index));
            self.scope.local_bits = (tainted, derived);
            let outcome = Box::pin(self.run_block(body)).await;
            self.scope.locals.clear();
            self.scope.local_bits = (false, false);
            match outcome {
                Ok(output) => results.push(output),
                Err(e) => match cfg.on_item_error {
                    OnItemError::Stop => return Err(format!("for-each {id}: item {index}: {e}")),
                    OnItemError::Continue => errors.push(json!({ "index": index, "error": e })),
                },
            }
        }
        if let Some(why) = &stopped {
            if cfg.on_cap == OnCap::EndRun {
                return Err(why.clone());
            }
        }
        // The body's spend counts as the for-each's own, so a loop that holds the
        // for-each sees it against its cap (`loop_cap` sums the cycle's blocks).
        *self.spent_by.entry(id).or_default() += self.spent - spent_before;
        let (body_tainted, body_derived) = self.scope.bits.get(body.id.as_str()).copied().unwrap_or((false, false));
        let any_tainted = tainted || body_tainted;
        self.scope.bits.insert(node.id.clone(), (any_tainted, !any_tainted && (derived || body_derived)));
        Ok(json!({ "results": results, "errors": errors, "count": results.len() + errors.len(), "capped": stopped.is_some(), "capped_reason": stopped }))
    }

    /// Why the next item must not run, in plain words, or `None`.
    fn item_limit(&self, cfg: &ForEachConfig, id: &str, (index, total): (usize, usize), (started, spent_before): (Instant, f64)) -> Option<String> {
        if index >= cfg.max_items {
            return Some(format!("stopped after {} of {total} items: the for-each {id} allows at most {}", cfg.max_items, cfg.max_items));
        }
        if let Some(stopped) = over_run_cap(self.spent, self.run_cap_usd) {
            return Some(stopped);
        }
        let spent = self.spent - spent_before;
        if let Some(max) = cfg.max_cost_usd.filter(|max| crate::backend::jobs::caps::reached(spent, *max)) {
            return Some(format!("stopped: the for-each {id} spent ${spent:.2} after {index} of {total} items, its cap of ${max:.2}"));
        }
        let minutes = started.elapsed().as_secs_f64() / 60.0;
        cfg.max_minutes.filter(|max| minutes >= *max).map(|max| format!("stopped: the for-each {id} ran {minutes:.1} minutes ({index} of {total} items), its cap of {max} minutes"))
    }
}
