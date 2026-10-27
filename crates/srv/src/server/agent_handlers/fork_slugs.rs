// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agents.migrateForkSlugs`: the Settings › Agents card's way into the fork
//! slug migration (`backend::slug_migration`). `plan`, `apply`, `rollback`
//! and `restore` run on the disk, off the async workers; `stop` asks each
//! running affected pane to stop, through the same 15 s user override an
//! agent's stop request waits for (`fleet_stop_keep_pane_with_override`).

use std::sync::Arc;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    CommandMigrateForkSlugsData, ForkSlugChoice, ForkSlugMigrationMode, ForkSlugStopPending, MigrateForkSlugsResult, COMMAND_AGENTS_MIGRATE_FORK_SLUGS,
};
use crate::backend::slug_migration::{self, Home};
use crate::server::AppState;

/// Who a stop's banner names.
const STOPPED_BY: &str = "Settings › Agents (agent id update)";

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_typed(COMMAND_AGENTS_MIGRATE_FORK_SLUGS, move |cmd: CommandMigrateForkSlugsData, _ctx| {
        let state = state.clone();
        async move {
            let home = Home::from_env().map_err(|e| format!("agents.migrateForkSlugs: {e}"))?;
            if cmd.mode == ForkSlugMigrationMode::Stop {
                return stop(&state, home, cmd.choices.clone().unwrap_or_default()).await;
            }
            let quick = cmd.quick.unwrap_or(false);
            let journal = cmd.journal.clone();
            let choices = cmd.choices.clone().unwrap_or_default();
            let result = tokio::task::spawn_blocking(move || slug_migration::run_choosing(&home, cmd.mode, journal.as_deref(), quick, &choices))
                .await
                .map_err(|e| format!("agents.migrateForkSlugs: task: {e}"))?
                .map_err(|e| format!("agents.migrateForkSlugs: {e}"))?;
            if cmd.mode != ForkSlugMigrationMode::Plan {
                state.broker.publish_event::<crate::backend::mps_events::AgentsChanged>(&(), vec![], 0);
                tracing::info!(mode = ?cmd.mode, summary = %result.summary, "agents.migrateForkSlugs: done");
            }
            Ok(result)
        }
    });
}

/// Stop every pane the plan finds running an agent it can migrate.
async fn stop(state: &AppState, home: Home, choices: Vec<ForkSlugChoice>) -> Result<MigrateForkSlugsResult, String> {
    let plan = tokio::task::spawn_blocking(move || slug_migration::plan_choosing(&home, false, &choices))
        .await
        .map_err(|e| format!("agents.migrateForkSlugs: task: {e}"))??;
    let mut blocks: Vec<String> = plan
        .candidates
        .iter()
        .filter(|c| c.refused.is_none())
        .flat_map(|c| c.running.iter().map(|r| r.block_id.clone()))
        .filter(|b| !b.is_empty())
        .collect();
    blocks.sort();
    blocks.dedup();
    let asked = blocks.len();
    let (now, pending) = crate::server::app_api::fleet::fleet_stop_keep_pane_with_override(state, STOPPED_BY, blocks, None, None).await;
    let pending: Vec<ForkSlugStopPending> =
        pending.into_iter().map(|p| ForkSlugStopPending { block_id: p.block_id, request_id: p.request_id }).collect();
    let wait_ms = if pending.is_empty() { 0 } else { crate::sagas::pending_shutdown::OVERRIDE_WINDOW.as_millis() as u64 };
    let mut summary = format!("Asked {asked} pane(s) to stop: {} waiting for their override, {} stopped.", pending.len(), now.succeeded.len());
    for f in &now.failed {
        summary.push_str(&format!(" {} didn't stop: {}.", f.id, f.error));
    }
    Ok(MigrateForkSlugsResult { mode: ForkSlugMigrationMode::Stop, plan: Some(plan), report: None, pending, wait_ms, summary })
}
