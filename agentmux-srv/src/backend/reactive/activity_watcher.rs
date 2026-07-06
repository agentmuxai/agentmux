// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pushed per-agent activity summaries: periodically runs the same
//! Haiku-powered digest used by `session:activity_summary` for every
//! registered reactive agent that is actively running, and publishes the
//! result as an `agent:summary` WaveEvent — so panes (the swarm feed, in
//! particular) can show a live one-liner without polling.
//!
//! Each call goes through `app_api::session::generate_pushed_activity_summary`,
//! which routes it through the Ambient Model Call gateway (`crate::ambient`)
//! under its own purpose tag — distinct from the pull RPC's, so a periodic
//! background summary never contends with a live, user-facing pane-header
//! request for the same block.
//!
//! Cost controls:
//!   - skipped entirely for agents whose controller isn't `STATUS_RUNNING`
//!     (an idle/stopped pane costs nothing)
//!   - skipped when the block's `output` FileStore size hasn't changed since
//!     the last *successful* summary (nothing new happened; no point
//!     re-summarizing) — a failed/empty attempt does not mark the size as
//!     seen, so the next tick retries rather than being permanently
//!     suppressed until the output happens to grow again
//!   - skipped when a summarization for that block is already in flight
//!     (guards against a slow call still running when the next tick fires)
//!   - capped at `MAX_CONCURRENT_SUMMARIES` simultaneous Haiku CLI spawns
//!   - per-block bookkeeping is pruned each tick against the current
//!     registration list, so a disconnected/unregistered agent's entry
//!     doesn't linger for the rest of the process's lifetime

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::Semaphore;
use tokio::time::interval;

use crate::backend::blockcontroller::{get_block_controller_status, STATUS_RUNNING};
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;
use crate::backend::wps::{Broker, WaveEvent};

use super::get_global_handler;

/// How often to sweep registered agents for a fresh summary.
const SWEEP_INTERVAL_SECS: u64 = 20;

/// Max simultaneous Haiku CLI spawns across all agents.
const MAX_CONCURRENT_SUMMARIES: usize = 2;

/// Word budget for the pushed summary. Matches the frontend's dynamic cap
/// for `session:activity_summary` (`useAgentActivitySummary.ts`), not that
/// RPC's own bare default of 7 (`app_api/session.rs`'s `unwrap_or(7)`) — the
/// pull path's effective width varies with pane size, so 12 is the closest
/// fixed stand-in for a swarm-tree row rather than a claim of being tighter.
const WORD_TARGET: u32 = 12;

pub const EVENT_AGENT_SUMMARY: &str = "agent:summary";

/// Run the pushed-summary sweep loop. Never returns.
pub async fn run_agent_summary_loop(wstore: Arc<Store>, filestore: Arc<FileStore>, broker: Arc<Broker>) {
    let mut ticker = interval(Duration::from_secs(SWEEP_INTERVAL_SECS));
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_SUMMARIES));
    // block_id -> last output size we *successfully* summarized at, so idle
    // agents (no new output since the last summary) are skipped instead of
    // re-billed every tick. An entry is only written after a non-empty
    // summary comes back (see the spawned task below) — a transient failure
    // (CLI error, missing `cmd` in meta, missing block) leaves no entry, so
    // the next tick retries instead of being permanently suppressed until
    // the output size happens to change again.
    let last_seen_size: Arc<Mutex<HashMap<String, i64>>> = Arc::new(Mutex::new(HashMap::new()));
    // block_ids with a summarization currently in flight, so a slow call
    // (up to the 15s CLI timeout) doesn't get double-dispatched by the next
    // 20s tick before last_seen_size has a chance to reflect its result.
    // Self-cleaning: every insert below has a matching remove once that same
    // spawned task's call resolves, on every exit path.
    let in_flight: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    // Shared generation counter for the Ambient Model Call gateway — only
    // needs to strictly increase per (block_id, purpose) key over time, so
    // one counter incremented once per tick and reused across every block
    // checked in that tick is sufficient (different block_ids are different
    // gateway keys and never interact).
    let mut tick: u64 = 0;

    loop {
        ticker.tick().await;
        tick += 1;

        let agents = get_global_handler().list_agents();

        // Drop last_seen_size entries for agents that unregistered/disconnected
        // since the last sweep, so this map stays bounded by the current agent
        // count instead of growing for every block_id ever seen in the
        // process's lifetime.
        let registered: HashSet<String> = agents.iter().map(|a| a.block_id.clone()).collect();
        last_seen_size.lock().unwrap().retain(|block_id, _| registered.contains(block_id));

        for agent in agents {
            let block_id = agent.block_id.clone();

            let status = match get_block_controller_status(&block_id) {
                Some(s) => s,
                None => continue,
            };
            if status.shellprocstatus != STATUS_RUNNING || !status.is_agent_pane {
                continue;
            }

            let current_size = match filestore.stat(&block_id, "output") {
                Ok(Some(wf)) => wf.size,
                _ => continue,
            };
            if last_seen_size.lock().unwrap().get(&block_id) == Some(&current_size) {
                continue; // already summarized this exact output size — skip
            }
            if !in_flight.lock().unwrap().insert(block_id.clone()) {
                continue; // a summarization for this block is already running
            }

            let wstore = wstore.clone();
            let filestore = filestore.clone();
            let broker = broker.clone();
            let semaphore = semaphore.clone();
            let last_seen_size = last_seen_size.clone();
            let in_flight = in_flight.clone();
            let agent_id = agent.agent_id.clone();

            tokio::spawn(async move {
                let Ok(_permit) = semaphore.acquire().await else {
                    in_flight.lock().unwrap().remove(&block_id);
                    return;
                };

                let result = crate::server::app_api::session::generate_pushed_activity_summary(
                    &wstore, &filestore, &block_id, tick, WORD_TARGET,
                ).await;

                in_flight.lock().unwrap().remove(&block_id);

                let Some((summary, _tokens)) = result else {
                    // Leave last_seen_size untouched so a future tick retries
                    // this block — whether the failure was transient (CLI
                    // hiccup, stale-on-arrival via the Ambient Model Call
                    // gateway) or persistent (no CLI path in meta yet).
                    return;
                };
                last_seen_size.lock().unwrap().insert(block_id.clone(), current_size);

                let ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);

                broker.publish(WaveEvent {
                    event: EVENT_AGENT_SUMMARY.to_string(),
                    scopes: vec![format!("block:{}", block_id)],
                    sender: String::new(),
                    persist: 0,
                    data: Some(serde_json::json!({
                        "agentId": agent_id,
                        "blockId": block_id,
                        "summary": summary,
                        "ts": ts,
                    })),
                });
            });
        }
    }
}
