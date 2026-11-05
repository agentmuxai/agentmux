// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent has no bundle of its own: a bundle holds only what several agents share
//! (`docs/reports/REPORT_AGENT_BUNDLES_REFINE_OR_DROP_2026_11_03.md`).
//!
//! Until now an agent's definition could be bound to one "own" bundle (`db_agents.default_memory_id`)
//! that launch put in front of the agent's picks. For each local agent still bound to one:
//!
//! - a bundle that carries something (text, files, skills, or a name that isn't the empty "— ABF"
//!   copy) is put first among the agent's picks, so launch still reads it in the same place in the
//!   order. A blank, system or global bundle, a deleted one, and an empty "— ABF" copy are not
//!   picked: the first three are not something to pick, and the copy holds nothing;
//! - the binding is then cleared. The exception is an agent with no provider of its own (a
//!   definition `m0035` couldn't reach): it keeps the binding, because the bundle's provider is the
//!   only record of what it runs (`Store::resolve_effective_provider_id`).
//!
//! Then the empty "— ABF" bundles nothing points at any more are removed, by the rule `m0046` uses
//! (every channel's store is read, and a store that can't be read stops the removal). A bundle
//! another channel's store still binds stays until that channel runs this too. What is removed is
//! listed in `removed-empty-agent-bundles-<time>.json` in the channel's data folder.
//!
//! Idempotent: a second run finds no binding.

use std::sync::Arc;

use rusqlite::params;

use crate::backend::storage::store::{Bundle, Store};

use super::m0046_remove_empty_agent_bundles::{holds_nothing, is_abf_name, remove_empty_unreferenced, table_exists};
use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0047AgentsDropOwnBundle;

/// A local agent still bound to a bundle: its id, its provider, the bundle id.
struct Bound {
    id: String,
    provider: String,
    bundle_id: String,
}

fn bound_agents(store: &Store) -> Result<Vec<Bound>, String> {
    let conn = store.conn().lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT id, provider, default_memory_id FROM db_agents WHERE default_memory_id != '' ORDER BY id")
        .map_err(|e| format!("list bound agents: {e}"))?;
    let rows = stmt
        .query_map([], |r| Ok(Bound { id: r.get(0)?, provider: r.get(1)?, bundle_id: r.get(2)? }))
        .map_err(|e| format!("list bound agents: {e}"))?;
    rows.collect::<Result<_, _>>().map_err(|e| format!("list bound agents: {e}"))
}

/// Whether skills are bound to the bundle in this channel's store.
fn has_skill_refs(store: &Store, bundle_id: &str) -> bool {
    let conn = store.conn().lock().unwrap();
    table_exists(&conn, "db_bundle_skills_ref")
        && conn
            .query_row("SELECT COUNT(*) FROM db_bundle_skills_ref WHERE bundle_id = ?1", params![bundle_id], |r| r.get::<_, i64>(0))
            .map(|n| n > 0)
            .unwrap_or(true)
}

/// Whether the bundle belongs among the agent's picks: it is a bundle to pick at all, and it is
/// not the empty copy that was minted for the agent.
fn worth_picking(store: &Store, b: &Bundle) -> bool {
    let pickable = !b.is_blank && !b.is_system && !b.is_global && b.id != "blank";
    pickable && !(is_abf_name(&b.name) && holds_nothing(b) && !has_skill_refs(store, &b.id))
}

impl Migration for M0047AgentsDropOwnBundle {
    fn id(&self) -> &'static str { "0047_agents_drop_own_bundle" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str {
        "Move each agent's own bundle into its picks and remove the empty ones nothing points at"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        let err = |e: String| MigrationError(format!("agents_drop_own_bundle: {e}"));
        if !ctx.channel_store_path.exists() || !ctx.shared_store_path.exists() {
            return Ok(());
        }
        let mstore = Arc::new(Store::open(&ctx.channel_store_path).map_err(|e| err(format!("open channel store: {e}")))?);
        let bundle_store = Store::open_shared(&ctx.shared_store_path).map_err(|e| err(format!("open shared store: {e}")))?;

        let (mut picked, mut cleared) = (0usize, 0usize);
        for agent in bound_agents(&mstore).map_err(err)? {
            let bundle = bundle_store.bundle_get(&agent.bundle_id).map_err(|e| err(format!("read bundle {}: {e}", agent.bundle_id)))?;
            if let Some(b) = bundle.as_ref().filter(|b| worth_picking(&mstore, b)) {
                let mut picks = vec![b.id.clone()];
                picks.extend(mstore.agent_bundle_ids(&agent.id).map_err(|e| err(format!("read picks of {}: {e}", agent.id)))?);
                mstore.agent_bundles_set(&agent.id, &picks).map_err(|e| err(format!("pick for {}: {e}", agent.id)))?;
                picked += 1;
            }
            // Without a provider of its own the agent still reads one from the bundle.
            if agent.provider.is_empty() && bundle.is_some() {
                continue;
            }
            mstore.agent_def_clear_memory_id(&agent.id).map_err(|e| err(format!("clear binding of {}: {e}", agent.id)))?;
            cleared += 1;
        }
        if picked + cleared > 0 {
            tracing::info!(picked, cleared, "agents_drop_own_bundle: moved agents' own bundles into their picks");
        }
        remove_empty_unreferenced(ctx, false)
    }
}

#[cfg(test)]
#[path = "m0047_agents_drop_own_bundle_tests.rs"]
mod tests;
