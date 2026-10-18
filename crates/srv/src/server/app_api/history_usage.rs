// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `history.usage`: the history total in the status bar, and its panel.
//! The window's, not an agent API: a connection registered as an agent is
//! refused. Measuring walks tens of thousands of files, so an answer is reused
//! for [`FRESH_FOR`] unless the request asks to `refresh`.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::tower_pane::not_an_agent_api;
use super::*;
use crate::backend::history_usage;

/// How long a measurement answers later requests.
const FRESH_FOR: Duration = Duration::from_secs(300);

static LAST: Mutex<Option<(Instant, HistoryUsage)>> = Mutex::new(None);

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    engine.register_typed(COMMAND_HISTORY_USAGE, move |req: HistoryUsageReq, ctx| {
        let mstore = mstore.clone();
        async move {
            not_an_agent_api(&ctx)?;
            if !req.refresh {
                if let Some((at, usage)) = LAST.lock().unwrap().as_ref() {
                    if at.elapsed() < FRESH_FOR {
                        return Ok(usage.clone());
                    }
                }
            }
            let Some(roots) = history_usage::Roots::on_this_machine() else {
                return Err("history.usage: the data folders can't be resolved".to_string());
            };
            // The walk and the store reads: off the async workers.
            let usage = tokio::task::spawn_blocking(move || {
                let names: std::collections::HashMap<String, String> = mstore
                    .agent_def_list()
                    .map(|defs| defs.into_iter().map(|d| (d.id, d.name)).collect())
                    .unwrap_or_default();
                let transcripts = crate::backend::agent_session::global_transcript_store();
                history_usage::compute(&roots, transcripts.map(|t| &**t), |id| names.get(id).cloned(), agentmux_common::time::now_ms_u64())
            })
            .await
            .map_err(|e| format!("history.usage: {e}"))?;
            *LAST.lock().unwrap() = Some((Instant::now(), usage.clone()));
            Ok(usage)
        }
    });
}
