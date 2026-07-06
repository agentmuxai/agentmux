// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pushed per-agent activity summaries: periodically runs the same
//! Haiku-powered digest used by `session:activity_summary` for every
//! registered reactive agent that is actively running, and publishes the
//! result as an `agent:summary` WaveEvent — so panes (the swarm feed, in
//! particular) can show a live one-liner without polling.
//!
//! Cost controls:
//!   - skipped entirely for agents whose controller isn't `STATUS_RUNNING`
//!     (an idle/stopped pane costs nothing)
//!   - skipped when the block's `output` FileStore size hasn't changed since
//!     the last summary (nothing new happened; no point re-summarizing)
//!   - capped at `MAX_CONCURRENT_SUMMARIES` simultaneous Haiku CLI spawns

use std::collections::HashMap;
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

/// Word budget for the pushed summary (a bit tighter than the pull RPC's
/// default, since this renders in a narrow swarm-tree row rather than a
/// pane title).
const WORD_TARGET: u32 = 12;

pub const EVENT_AGENT_SUMMARY: &str = "agent:summary";

/// Run the pushed-summary sweep loop. Never returns.
pub async fn run_agent_summary_loop(wstore: Arc<Store>, filestore: Arc<FileStore>, broker: Arc<Broker>) {
    let mut ticker = interval(Duration::from_secs(SWEEP_INTERVAL_SECS));
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_SUMMARIES));
    // block_id -> last output size we summarized at, so idle agents (no new
    // output since last sweep) are skipped instead of re-billed every tick.
    let last_seen_size: Arc<Mutex<HashMap<String, i64>>> = Arc::new(Mutex::new(HashMap::new()));

    // Shared generation counter for the Ambient Model Call gateway — only
    // needs to strictly increase per (block_id, purpose) key over time, so
    // one counter incremented once per tick and reused across every block
    // checked in that tick is sufficient (different block_ids are different
    // gateway keys and never interact).
    let mut tick: u64 = 0;

    loop {
        ticker.tick().await;
        tick += 1;

        for agent in get_global_handler().list_agents() {
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
            {
                let mut seen = last_seen_size.lock().unwrap();
                if seen.get(&block_id) == Some(&current_size) {
                    continue; // no new output since the last summary — skip
                }
                seen.insert(block_id.clone(), current_size);
            }

            let wstore = wstore.clone();
            let filestore = filestore.clone();
            let broker = broker.clone();
            let semaphore = semaphore.clone();
            let agent_id = agent.agent_id.clone();

            tokio::spawn(async move {
                let Ok(_permit) = semaphore.acquire().await else { return };

                let Some((summary, _tokens)) = crate::server::app_api::session::generate_pushed_activity_summary(
                    &wstore, &filestore, &block_id, tick, WORD_TARGET,
                ).await else {
                    return;
                };

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
