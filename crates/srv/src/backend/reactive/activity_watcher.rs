// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Empty-title recovery: a running agent with conversation and no usable
//! session title gets one, written straight to its block's
//! `term:ambient_summary`, which the pane header and the Swarm row read.
//!
//! The title is otherwise computed only when a human submits a message
//! (`useAgentActivitySummary.ts`). An agent driven by jekts or tool work, one
//! reattached mid-turn, or one whose title call failed or abstained could sit on
//! an empty title for as long as it ran. This sweep is the second route.
//!
//! It replaces what this module used to do: a "what is being worked on" summary
//! every 20 s for every running agent, published as an `agent:summary` event that
//! nothing subscribed to, so every call was spent and its output went nowhere.
//! Now an agent that already has a title costs nothing here.
//! docs/specs/SPEC_AMBIENT_SWARM_SUMMARY_HARDENING_2026_10_02.md sections 2.1
//! (defects 2 and 5), 5.6 and 5.7.
//!
//! Each call goes through `crate::ambient::tasks::generate_recovered_title`, so
//! the Ambient Model Call gateway admits, cancels and accounts for it under its
//! own purpose tag, distinct from the pane's own title request.
//!
//! Cost controls:
//!   - only agents whose controller is running (`STATUS_RUNNING`) and that have no usable title;
//!   - only agents whose `output` is in a shape the digest can read
//!     (`digest::reads_output_format`); another provider's file holds no
//!     conversation it can find;
//!   - only when the block's `output` has grown since the last completed attempt;
//!   - at most [`MAX_ATTEMPTS`] completed attempts per block while the title stays
//!     empty, counted again from zero once a title exists, so an agent whose
//!     activity never shows what the work is (the model abstains) stops costing;
//!   - no conversation in the digest means no call at all (`digest` returns
//!     `None` for activity with no user or assistant text);
//!   - one call in flight per block, waiting in the background class's queue
//!     (`ambient::limits`) across all blocks;
//!   - per-block bookkeeping is pruned each tick against the registration list.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::time::interval;

use crate::ambient::digest;
use crate::ambient::title::{store_title, Replace, META_TITLE};
use crate::ambient::validate::is_usable_title;
use crate::backend::blockcontroller::core::broadcast_block_update;
use crate::backend::blockcontroller::{get_block_controller_status, STATUS_RUNNING};
use crate::backend::eventbus::EventBus;
use crate::backend::obj::{self, Block};
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;

use super::get_global_handler;

/// How often to sweep registered agents.
const SWEEP_INTERVAL_SECS: u64 = 20;

/// Completed attempts per block while its title stays empty. Bounded, as the
/// `lys` design bounds its retries (spec section 3, research point 2).
pub(crate) const MAX_ATTEMPTS: u32 = 3;

/// Word budget for a recovered title: within the range the pane's own request
/// uses (5 to 12, by pane width), so a recovered title looks like any other.
const WORD_TARGET: u32 = 8;

/// Per-block state the sweep keeps between ticks.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Attempts {
    /// Completed (billed) attempts since the title was last seen non-empty.
    pub count: u32,
    /// The `output` size at the last completed attempt.
    pub last_size: Option<i64>,
}

/// Why a block is not attempted this tick, or `None` to attempt it. Pure, so the
/// cost rules are testable without a model, a store or a clock.
pub(crate) fn skip_reason(has_title: bool, attempts: Attempts, output_size: i64) -> Option<&'static str> {
    if has_title {
        return Some("has a title");
    }
    if attempts.count >= MAX_ATTEMPTS {
        return Some("attempts exhausted");
    }
    if attempts.last_size == Some(output_size) {
        return Some("no new output");
    }
    None
}

/// Run the recovery sweep. Never returns.
pub async fn run_agent_summary_loop(mstore: Arc<Store>, filestore: Arc<FileStore>, event_bus: Arc<EventBus>) {
    let mut ticker = interval(Duration::from_secs(SWEEP_INTERVAL_SECS));
    let attempts: Arc<Mutex<HashMap<String, Attempts>>> = Arc::new(Mutex::new(HashMap::new()));
    // One call per block at a time: a slow call (up to the CLI timeout) must not
    // be dispatched again by the next tick. Every insert has a matching remove.
    let in_flight: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    // The gateway's generation only has to increase per (block, purpose).
    let mut tick: u64 = 0;

    loop {
        ticker.tick().await;
        tick += 1;

        let agents = get_global_handler().list_agents();
        let registered: HashSet<String> = agents.iter().map(|a| a.block_id.clone()).collect();
        attempts.lock().unwrap().retain(|block_id, _| registered.contains(block_id));

        for agent in agents {
            let block_id = agent.block_id.clone();

            let Some(status) = get_block_controller_status(&block_id) else {
                continue;
            };
            if status.shellprocstatus != STATUS_RUNNING || !status.is_agent_pane {
                continue;
            }
            let Ok(Some(block)) = mstore.get::<Block>(&block_id) else {
                continue;
            };
            let has_title = is_usable_title(&obj::meta_get_string(&block.meta, META_TITLE, ""));
            if has_title {
                // A title exists: the budget starts over if it is ever lost.
                attempts.lock().unwrap().remove(&block_id);
                continue;
            }
            if !digest::reads_output_format(&obj::meta_get_string(&block.meta, "agentOutputFormat", "")) {
                continue;
            }
            let Ok(Some(output)) = filestore.stat(&block_id, "output") else {
                continue;
            };
            let state = attempts.lock().unwrap().get(&block_id).copied().unwrap_or_default();
            if skip_reason(false, state, output.size).is_some() {
                continue;
            }
            if !in_flight.lock().unwrap().insert(block_id.clone()) {
                continue;
            }

            let mstore = mstore.clone();
            let filestore = filestore.clone();
            let event_bus = event_bus.clone();
            let attempts = attempts.clone();
            let in_flight = in_flight.clone();
            let output_size = output.size;

            tokio::spawn(async move {
                let result =
                    crate::ambient::tasks::generate_recovered_title(&mstore, &filestore, &block_id, tick, WORD_TARGET)
                        .await;
                in_flight.lock().unwrap().remove(&block_id);

                // `None`: nothing ran (no conversation in the digest, no CLI path,
                // superseded). Not an attempt; a later tick may try again.
                let Some(generated) = result else {
                    return;
                };
                let attempt = {
                    let mut map = attempts.lock().unwrap();
                    let entry = map.entry(block_id.clone()).or_default();
                    entry.count += 1;
                    entry.last_size = Some(output_size);
                    entry.count
                };

                let Some(title) = generated.text else {
                    tracing::info!(
                        block_id = %block_id,
                        attempt,
                        max = MAX_ATTEMPTS,
                        "ambient: title recovery produced nothing usable (abstained or rejected)"
                    );
                    return;
                };
                // The pane's own request can win the race; it is the better source.
                match store_title(&mstore, &block_id, &title, Replace::IfEmpty) {
                    Ok(true) => {
                        tracing::info!(block_id = %block_id, attempt, title = %title, "ambient: recovered a missing session title");
                        broadcast_block_update(&mstore, &event_bus, &block_id);
                    }
                    Ok(false) => {
                        tracing::debug!(block_id = %block_id, "ambient: a title appeared while recovery ran; kept it");
                    }
                    Err(e) => {
                        tracing::warn!(block_id = %block_id, error = %e, "ambient: could not store a recovered title");
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tried(count: u32, last_size: i64) -> Attempts {
        Attempts { count, last_size: Some(last_size) }
    }

    #[test]
    fn an_agent_with_a_title_costs_nothing() {
        assert_eq!(skip_reason(true, Attempts::default(), 100), Some("has a title"));
    }

    #[test]
    fn an_untitled_agent_is_tried_once_per_growth_of_its_output() {
        assert_eq!(skip_reason(false, Attempts::default(), 100), None, "first try");
        assert_eq!(skip_reason(false, tried(1, 100), 100), Some("no new output"));
        assert_eq!(skip_reason(false, tried(1, 100), 250), None, "new output, try again");
    }

    #[test]
    fn attempts_are_bounded_while_the_title_stays_empty() {
        assert_eq!(skip_reason(false, tried(MAX_ATTEMPTS - 1, 1), 2), None);
        assert_eq!(skip_reason(false, tried(MAX_ATTEMPTS, 1), 2), Some("attempts exhausted"));
    }
}
