// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! §4 Phase 2 step 5: re-read every location after an agent's steps and say
//! what is wrong, if anything. The new slug everywhere; the old one only as a
//! former slug; no signing key under the old name; and both names resolve to
//! the uid, by the same rule message delivery uses (`Store::former_name_owner`,
//! which resolves former slugs and former display names alike).

use std::path::Path;
use std::sync::Arc;

use rusqlite::OptionalExtension;
use serde_json::Value;

use super::home::{fold, open_ro, table_exists};
use super::steps::{Agent, Ctx};
use super::{steps_db, steps_files};

/// Every problem found for `a`; empty when it verifies.
pub(super) fn verify(ctx: &Ctx, a: &Agent) -> Result<Vec<String>, String> {
    let mut problems = Vec::new();
    let def_path = steps_files::def_path(ctx.home, &a.uid);
    let def: Value = serde_json::from_slice(&std::fs::read(&def_path).map_err(|e| format!("{}: {e}", def_path.display()))?)
        .map_err(|e| e.to_string())?;
    if def.pointer("/data/slug").and_then(Value::as_str) != Some(a.new.as_str()) {
        problems.push(format!("the shared definition's slug isn't {}", a.new));
    }
    let former = def.pointer("/data/former_slugs").and_then(Value::as_array).cloned().unwrap_or_default();
    if !former.iter().any(|f| f.get("slug").and_then(Value::as_str).is_some_and(|s| fold(s) == fold(&a.old))) {
        problems.push(format!("the shared definition doesn't list {} as a former slug", a.old));
    }
    let defs_store = Arc::new(crate::registry::DefinitionStore::open(ctx.home.definitions()).map_err(|e| e.to_string())?);
    for db in ctx.objects_dbs() {
        problems.extend(verify_objects_db(&db, &defs_store, a)?);
    }
    for db in ctx.wan_dbs() {
        let conn = open_ro(&db)?;
        if table_exists(&conn, "wan_agent_keys") {
            let left: Option<String> = conn
                .query_row("SELECT agent_id FROM wan_agent_keys WHERE agent_id = ?1 COLLATE NOCASE", [fold(&a.old)], |r| r.get(0))
                .optional()
                .map_err(|e| e.to_string())?;
            if left.is_some() {
                problems.push(format!("{}: a WAN key is still filed under {}", db.display(), a.old));
            }
        }
    }
    let leftover = steps_files::stale_registrations(ctx, a);
    if !leftover.is_empty() {
        problems.push(format!("{} registration(s) under {} remain", leftover.len(), a.old));
    }
    if !steps_files::mcp_files(ctx, a)?.is_empty() {
        problems.push(format!("a workspace .mcp.json still signs as {}", a.old));
    }
    let name_only = steps_db::s6_before(ctx, a)?;
    let left = ["updates", "grants"].iter().map(|k| name_only[*k].as_array().map_or(0, Vec::len)).sum::<usize>();
    if left > 0 {
        problems.push(format!("{left} cron job, queue item or trust grant row(s) still name {} without its uid", a.old));
    }
    Ok(problems)
}

fn verify_objects_db(db: &Path, defs: &Arc<crate::registry::DefinitionStore>, a: &Agent) -> Result<Vec<String>, String> {
    let mut problems = Vec::new();
    let place = db.display().to_string();
    {
        let conn = open_ro(db)?;
        let row: Option<Option<String>> = if super::home::has_agent_slugs(&conn) {
            conn.query_row("SELECT slug FROM db_agents WHERE id = ?1", [&a.uid], |r| r.get(0))
                .optional()
                .map_err(|e| e.to_string())?
        } else {
            None
        };
        if let Some(slug) = &row {
            if slug.as_deref() != Some(a.new.as_str()) {
                problems.push(format!("{place}: db_agents.slug is {:?}, not {}", slug.as_deref().unwrap_or(""), a.new));
            }
            let recorded = table_exists(&conn, "db_agent_former_slugs")
                && conn
                    .query_row("SELECT 1 FROM db_agent_former_slugs WHERE agent_uid = ?1 AND slug = ?2", [&a.uid, &fold(&a.old)], |_| Ok(()))
                    .optional()
                    .map_err(|e| e.to_string())?
                    .is_some();
            if !recorded {
                problems.push(format!("{place}: {} isn't recorded as a former slug", a.old));
            }
        }
        for table in ["db_agent_jekt_keys", "db_agent_lan_keys", "db_agent_wan_keys"].iter().filter(|t| table_exists(&conn, t)) {
            let left: Option<String> = conn
                .query_row(&format!("SELECT agent_id FROM {table} WHERE agent_id = ?1 COLLATE NOCASE"), [fold(&a.old)], |r| r.get(0))
                .optional()
                .map_err(|e| e.to_string())?;
            if left.is_some() {
                problems.push(format!("{place}: a signing key is still filed under {} in {table}", a.old));
            }
        }
        if row.is_none() {
            return Ok(problems);
        }
    }
    if !steps_db::s7_rows(db, a)?.is_empty() {
        problems.push(format!("{place}: a pane's persisted identity still names {}", a.old));
    }
    // Resolution, by srv's own rule, on this database as it is.
    let store = crate::backend::storage::store::Store::open_source_readonly(db).map_err(|e| e.to_string())?;
    store.set_def_registry(defs.clone());
    match store.former_name_owner(&a.old) {
        Ok(Some(owner)) if owner.uid == a.uid && owner.slug == a.new => {}
        Ok(other) => problems.push(format!("{place}: {} resolves to {other:?}, not to {}", a.old, a.uid)),
        Err(e) if too_old(&e) => {}
        Err(e) => problems.push(format!("{place}: resolving {} failed: {e}", a.old)),
    }
    match store.agent_ids_for_slug_folded(&a.new) {
        Ok(ids) if ids == [a.uid.clone()] => {}
        Ok(ids) => problems.push(format!("{place}: {} resolves to {ids:?}, not only to {}", a.new, a.uid)),
        Err(e) if too_old(&e) => {}
        Err(e) => problems.push(format!("{place}: resolving {} failed: {e}", a.new)),
    }
    Ok(problems)
}

/// A database from a version too old for srv's resolver to query: nothing
/// running reads it with that rule, and its rows were checked above.
fn too_old(e: &crate::backend::storage::error::StoreError) -> bool {
    e.to_string().contains("no such")
}
