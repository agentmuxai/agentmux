// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Remove the empty per-agent bundles nothing points at
//! (`docs/reports/REPORT_AGENT_BUNDLES_REFINE_OR_DROP_2026_11_03.md`).
//!
//! Every agent used to get a dedicated, empty "<Name> — ABF" bundle, and `m0021` minted a new one
//! for the same agent in every channel's first boot, so the shared store filled with copies no
//! agent used (881 of 926 bundles on one machine). A bundle is removed only when all of this holds:
//!
//! - it is named "<something> — ABF" or "<something> — ABF (n)", and is not blank, global, system
//!   or seeded;
//! - it holds nothing: no description, instructions, per-provider instructions, context files,
//!   skills or MCP servers, and no version of it ever held instructions;
//! - nothing in this channel points at it (an agent's `memory_id` or `default_memory_id`, an
//!   agent's picked bundles, a skill binding) and no agent's recorded bundle
//!   (`agent_bundle_sidecar`) is it.
//!
//! The recorded bundle of every agent is kept, so what another channel holds is kept too. A
//! bundle that only another channel's agent row still named is empty, and an agent whose bundle is
//! missing is read as having none, so removing it loses nothing.
//!
//! The removed bundles, with their version and scope rows, are listed in
//! `removed-empty-agent-bundles-<time>.json` in the channel's data folder, and in the log.
//! Idempotent: a second run finds none.

use std::collections::BTreeSet;
use std::sync::Arc;

use rusqlite::{params, Connection};

use crate::backend::storage::store::{Bundle, Store};
use crate::registry::{resolve_shared_definitions_dir, DefinitionStore};

use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0046RemoveEmptyAgentBundles;

/// "<x> — ABF" or "<x> — ABF (n)": the name the per-agent bundle was given.
fn is_abf_name(name: &str) -> bool {
    let base = match name.rsplit_once(" (") {
        Some((base, tail)) if tail.len() > 1 && tail.ends_with(')') && tail[..tail.len() - 1].bytes().all(|b| b.is_ascii_digit()) => base,
        _ => name,
    };
    base.ends_with(" — ABF")
}

/// Whether `b` is a per-agent bundle that holds nothing (the references and history are checked
/// separately).
fn is_empty_agent_bundle(b: &Bundle) -> bool {
    !b.is_blank
        && !b.is_global
        && !b.is_system
        && b.id != "blank"
        && !b.id.starts_with("seed-")
        && is_abf_name(&b.name)
        && b.description.trim().is_empty()
        && b.instructions.trim().is_empty()
        && [&b.instructions_by_provider, &b.context_files, &b.mcp_servers, &b.skills]
            .iter()
            .all(|s| matches!(s.trim(), "" | "{}" | "[]"))
}

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1", params![table], |_| Ok(())).is_ok()
}

fn column(conn: &Connection, sql: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn.prepare(sql).map_err(|e| format!("{sql}: {e}"))?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(|e| format!("{sql}: {e}"))?;
    rows.collect::<Result<_, _>>().map_err(|e| format!("{sql}: {e}"))
}

/// The bundle ids this channel's agent rows, picks and skill bindings name.
fn referenced_in_channel(mstore: &Store) -> Result<BTreeSet<String>, String> {
    let conn = mstore.conn().lock().unwrap();
    let mut ids: Vec<String> = Vec::new();
    ids.extend(column(&conn, "SELECT memory_id FROM db_agents WHERE memory_id != ''")?);
    ids.extend(column(&conn, "SELECT default_memory_id FROM db_agents WHERE default_memory_id != ''")?);
    ids.extend(column(&conn, "SELECT bundle_id FROM db_agent_bundles")?);
    if table_exists(&conn, "db_bundle_skills_ref") {
        ids.extend(column(&conn, "SELECT bundle_id FROM db_bundle_skills_ref")?);
    }
    Ok(ids.into_iter().collect())
}

/// What one run removed, for the record file.
#[derive(Debug, serde::Serialize)]
struct Removed {
    id: String,
    name: String,
    created_at: i64,
}

impl Migration for M0046RemoveEmptyAgentBundles {
    fn id(&self) -> &'static str { "0046_remove_empty_agent_bundles" }
    fn scope(&self) -> MigrationScope { MigrationScope::Channel }
    fn description(&self) -> &'static str {
        "Remove the empty per-agent bundles that no agent points at"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if !ctx.channel_store_path.exists() || !ctx.shared_store_path.exists() {
            return Ok(());
        }
        let mstore = Arc::new(
            Store::open(&ctx.channel_store_path)
                .map_err(|e| MigrationError(format!("remove_empty_agent_bundles: open channel store: {e}")))?,
        );
        let bundle_store = Store::open_shared(&ctx.shared_store_path)
            .map_err(|e| MigrationError(format!("remove_empty_agent_bundles: open shared store: {e}")))?;
        // The global registry lists agents that exist only there, so their recorded bundles count.
        if let Some(def_dir) = resolve_shared_definitions_dir() {
            match DefinitionStore::open(def_dir) {
                Ok(def_store) => mstore.set_def_registry(Arc::new(def_store)),
                Err(e) => tracing::warn!(error = %e, "remove_empty_agent_bundles: global def registry unavailable"),
            }
        }
        let mut keep = referenced_in_channel(&mstore).map_err(|e| MigrationError(format!("remove_empty_agent_bundles: {e}")))?;
        // Without the recorded-bundle store there is no telling which bundle an agent in another
        // channel still uses, so nothing is removed. Not an error: a start without the shared
        // folder must still go on.
        let Some(sidecar) = crate::backend::agent_bundle_sidecar::open_global() else {
            tracing::warn!("remove_empty_agent_bundles: the recorded-bundle store is unavailable; nothing removed");
            return Ok(());
        };
        let defs = mstore
            .agent_def_list()
            .map_err(|e| MigrationError(format!("remove_empty_agent_bundles: list definitions: {e}")))?;
        for def in &defs {
            if let Ok(Some(id)) = crate::backend::agent_bundle_sidecar::get(&sidecar, &def.id) {
                keep.insert(id);
            }
        }
        let held_content = {
            let conn = bundle_store.conn().lock().unwrap();
            if table_exists(&conn, "db_bundle_versions") {
                column(&conn, "SELECT DISTINCT bundle_id FROM db_bundle_versions WHERE trim(instructions) != ''")
                    .map_err(|e| MigrationError(format!("remove_empty_agent_bundles: {e}")))?
            } else {
                Vec::new()
            }
        };
        keep.extend(held_content);

        let doomed: Vec<Bundle> = bundle_store
            .bundle_list()
            .map_err(|e| MigrationError(format!("remove_empty_agent_bundles: list bundles: {e}")))?
            .into_iter()
            .filter(|b| is_empty_agent_bundle(b) && !keep.contains(&b.id))
            .collect();
        if doomed.is_empty() {
            return Ok(());
        }
        let removed: Vec<Removed> = doomed
            .iter()
            .map(|b| Removed { id: b.id.clone(), name: b.name.clone(), created_at: b.created_at })
            .collect();
        record(ctx, &removed);
        delete(&bundle_store, &removed).map_err(|e| MigrationError(format!("remove_empty_agent_bundles: {e}")))?;
        tracing::info!(removed = removed.len(), kept_referenced = keep.len(), "remove_empty_agent_bundles: removed empty per-agent bundles nothing points at");
        Ok(())
    }
}

/// Write the list of what is about to go, so it can be seen afterwards. Not fatal: every one is empty.
fn record(ctx: &MigrationContext, removed: &[Removed]) {
    let now = agentmux_common::time::now_ms();
    let path = ctx.data_dir.join(format!("removed-empty-agent-bundles-{now}.json"));
    match serde_json::to_vec_pretty(removed) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(&path, bytes) {
                tracing::warn!(error = %e, path = %path.display(), "remove_empty_agent_bundles: could not write the list of removed bundles");
            }
        }
        Err(e) => tracing::warn!(error = %e, "remove_empty_agent_bundles: could not encode the list of removed bundles"),
    }
}

/// Delete the bundles with their version and scope rows, in one transaction.
fn delete(store: &Store, removed: &[Removed]) -> Result<(), String> {
    let mut conn = store.conn().lock().unwrap();
    let (has_scope, has_versions) = (table_exists(&conn, "db_bundle_scope"), table_exists(&conn, "db_bundle_versions"));
    let tx = conn.transaction().map_err(|e| format!("begin: {e}"))?;
    for r in removed {
        if has_versions {
            tx.execute("DELETE FROM db_bundle_versions WHERE bundle_id = ?1", params![r.id]).map_err(|e| format!("delete versions of {}: {e}", r.id))?;
        }
        if has_scope {
            tx.execute("DELETE FROM db_bundle_scope WHERE bundle_id = ?1", params![r.id]).map_err(|e| format!("delete scope of {}: {e}", r.id))?;
        }
        tx.execute("DELETE FROM db_bundles WHERE id = ?1", params![r.id]).map_err(|e| format!("delete {}: {e}", r.id))?;
    }
    tx.commit().map_err(|e| format!("commit: {e}"))
}

#[cfg(test)]
#[path = "m0046_remove_empty_agent_bundles_tests.rs"]
mod tests;
