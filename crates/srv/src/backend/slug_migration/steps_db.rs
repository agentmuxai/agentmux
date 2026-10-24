// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The database steps: S2 (every `objects.db`'s slug and former slugs), S3
//! (name-keyed signing keys), S4 (`wan.db`), S5 (the MuxBus credential, kept),
//! S6 (name-only rows) and S7's rows (persisted `cmd:env`). Each runs in one
//! transaction per database, through a raw connection, so another version's
//! `objects.db` is changed only where the step says and is never upgraded.

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::home::{fold, has_column, in_tx, open_ro, open_rw, table_exists};
use super::steps::{parse, to_value, Agent, Ctx};

const KEY_TABLES: [&str; 3] = ["db_agent_jekt_keys", "db_agent_lan_keys", "db_agent_wan_keys"];

fn err(e: rusqlite::Error) -> String {
    e.to_string()
}

fn path_str(p: &Path) -> String {
    p.display().to_string()
}

// ── S2: db_agents.slug and db_agent_former_slugs ─────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct S2Db {
    db: String,
    slug: Option<String>,
    had_table: bool,
    had_former: bool,
}

pub(super) fn s2_before(ctx: &Ctx, a: &Agent) -> Result<Value, String> {
    let mut out = Vec::new();
    for db in ctx.objects_dbs() {
        let conn = open_ro(&db)?;
        if !super::home::has_agent_slugs(&conn) {
            continue;
        }
        let row: Option<Option<String>> =
            conn.query_row("SELECT slug FROM db_agents WHERE id = ?1", [&a.uid], |r| r.get(0)).optional().map_err(err)?;
        let Some(slug) = row else { continue };
        let had_table = table_exists(&conn, "db_agent_former_slugs");
        let had_former = had_table
            && conn
                .query_row("SELECT 1 FROM db_agent_former_slugs WHERE agent_uid = ?1 AND slug = ?2", [&a.uid, &fold(&a.old)], |_| Ok(()))
                .optional()
                .map_err(err)?
                .is_some();
        out.push(S2Db { db: path_str(&db), slug, had_table, had_former });
    }
    to_value(&out)
}

pub(super) fn s2_apply(ctx: &Ctx, a: &Agent, before: &Value) -> Result<Vec<String>, String> {
    let mut changes = Vec::new();
    for b in parse::<Vec<S2Db>>(before)? {
        let conn = open_rw(Path::new(&b.db))?;
        let (renamed, recorded) = in_tx(&conn, |c| {
            let renamed = c
                .execute("UPDATE db_agents SET slug = ?1 WHERE id = ?2 AND (slug IS NULL OR slug != ?1)", [&a.new, &a.uid])
                .map_err(err)?;
            c.execute_batch(crate::backend::storage::agents::FORMER_SLUGS_SCHEMA).map_err(err)?;
            let recorded = c
                .execute(
                    "INSERT OR IGNORE INTO db_agent_former_slugs (agent_uid, slug, retired_at)
                     SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM db_agents WHERE id = ?1)",
                    params![a.uid, fold(&a.old), ctx.retired_at],
                )
                .map_err(err)?;
            Ok((renamed, recorded))
        })?;
        if renamed + recorded > 0 {
            changes.push(format!("{}: slug {} -> {}, {} recorded as a former slug", b.db, a.old, a.new, fold(&a.old)));
        }
    }
    Ok(changes)
}

pub(super) fn s2_undo(a: &Agent, before: &Value) -> Result<Vec<String>, String> {
    let mut changes = Vec::new();
    for b in parse::<Vec<S2Db>>(before)? {
        let conn = open_rw(Path::new(&b.db))?;
        in_tx(&conn, |c| {
            c.execute("UPDATE db_agents SET slug = ?1 WHERE id = ?2", params![b.slug, a.uid]).map_err(err)?;
            if !b.had_former && table_exists(c, "db_agent_former_slugs") {
                c.execute("DELETE FROM db_agent_former_slugs WHERE agent_uid = ?1 AND slug = ?2", [&a.uid, &fold(&a.old)])
                    .map_err(err)?;
            }
            if !b.had_table && table_exists(c, "db_agent_former_slugs") {
                let left: i64 = c.query_row("SELECT COUNT(*) FROM db_agent_former_slugs", [], |r| r.get(0)).map_err(err)?;
                if left == 0 {
                    c.execute_batch("DROP TABLE db_agent_former_slugs").map_err(err)?;
                }
            }
            Ok(())
        })?;
        changes.push(format!("{}: slug back to {}", b.db, b.slug.as_deref().unwrap_or("(none)")));
    }
    Ok(changes)
}

// ── S3: name-keyed signing keys, rotated ─────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct S3Db {
    db: String,
    /// (table, name) of each key row deleted. Its contents stay in the
    /// backup copy of this database.
    rows: Vec<(String, String)>,
    tombstones: bool,
    tombstone: Option<(String, i64)>,
}

/// The names S3 and S4 clear: the old one, and the new one in case a row
/// was left under it, so the agent's next spawn mints fresh keys instead of
/// inheriting one (the planner already made sure no other agent holds it).
fn key_names(a: &Agent) -> [String; 2] {
    [fold(&a.old), fold(&a.new)]
}

pub(super) fn s3_before(ctx: &Ctx, a: &Agent) -> Result<Value, String> {
    let mut out = Vec::new();
    for db in ctx.objects_dbs() {
        let conn = open_ro(&db)?;
        let mut rows = Vec::new();
        for table in KEY_TABLES.iter().filter(|t| table_exists(&conn, t)) {
            for name in key_names(a) {
                let mut stmt = conn.prepare(&format!("SELECT agent_id FROM {table} WHERE agent_id = ?1 COLLATE NOCASE")).map_err(err)?;
                for id in stmt.query_map([&name], |r| r.get::<_, String>(0)).map_err(err)? {
                    rows.push((table.to_string(), id.map_err(err)?));
                }
            }
        }
        let tombstones = table_exists(&conn, "db_agent_key_tombstones");
        let tombstone = if tombstones {
            conn.query_row("SELECT deleted_uid, deleted_at FROM db_agent_key_tombstones WHERE name = ?1", [&fold(&a.old)], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()
            .map_err(err)?
        } else {
            None
        };
        out.push(S3Db { db: path_str(&db), rows, tombstones, tombstone });
    }
    to_value(&out)
}

pub(super) fn s3_apply(_ctx: &Ctx, a: &Agent, before: &Value) -> Result<Vec<String>, String> {
    let mut changes = Vec::new();
    for b in parse::<Vec<S3Db>>(before)? {
        let conn = open_rw(Path::new(&b.db))?;
        let (removed, tombstoned) = in_tx(&conn, |c| {
            let mut removed = 0;
            for (table, name) in &b.rows {
                removed += c.execute(&format!("DELETE FROM {table} WHERE agent_id = ?1"), [name]).map_err(err)?;
            }
            let mut tombstoned = 0;
            if b.tombstones {
                tombstoned = c.execute(
                    "INSERT INTO db_agent_key_tombstones (name, deleted_uid, deleted_at) VALUES (?1, ?2, ?3)
                     ON CONFLICT(name) DO UPDATE SET deleted_uid = excluded.deleted_uid, deleted_at = excluded.deleted_at
                     WHERE deleted_uid != excluded.deleted_uid",
                    params![fold(&a.old), a.uid, agentmux_common::time::now_secs()],
                )
                .map_err(err)?;
            }
            Ok((removed, tombstoned))
        })?;
        if removed > 0 {
            changes.push(format!("{}: {removed} signing key row(s) under {} or {} deleted", b.db, fold(&a.old), fold(&a.new)));
        }
        if tombstoned > 0 {
            changes.push(format!("{}: {} tombstoned", b.db, fold(&a.old)));
        }
    }
    Ok(changes)
}

/// Rows matching a filter: `(table, sql filter, params)`.
type Rows = [(String, String, Vec<String>)];

/// Delete `clear`, then put `rows` back from the backup copy of `db`, then
/// run `extra`, in one transaction.
fn restore_rows(ctx: &Ctx, db: &Path, clear: &Rows, rows: &Rows, extra: impl FnOnce(&Connection) -> Result<(), String>) -> Result<usize, String> {
    let conn = open_rw(db)?;
    let attached = if rows.is_empty() {
        false
    } else {
        let copy = ctx.backup_of(db)?;
        conn.execute("ATTACH DATABASE ?1 AS bk", [copy.display().to_string()]).map_err(err)?;
        true
    };
    let result = in_tx(&conn, |c| {
        for (table, filter, args) in clear.iter().filter(|(t, _, _)| table_exists(c, t)) {
            c.execute(&format!("DELETE FROM main.{table} WHERE {filter}"), rusqlite::params_from_iter(args)).map_err(err)?;
        }
        let mut restored = 0;
        for (table, filter, args) in rows {
            restored += c
                .execute(&format!("INSERT OR IGNORE INTO main.{table} SELECT * FROM bk.{table} WHERE {filter}"), rusqlite::params_from_iter(args))
                .map_err(err)?;
        }
        extra(c)?;
        Ok(restored)
    });
    if attached {
        let _ = conn.execute_batch("DETACH DATABASE bk");
    }
    result
}

pub(super) fn s3_undo(ctx: &Ctx, a: &Agent, before: &Value) -> Result<Vec<String>, String> {
    let mut changes = Vec::new();
    for b in parse::<Vec<S3Db>>(before)? {
        let rows: Vec<(String, String, Vec<String>)> =
            b.rows.iter().map(|(t, n)| (t.clone(), "agent_id = ?1".to_string(), vec![n.clone()])).collect();
        // Keys minted under the new name since (a relaunch) go too: no agent
        // holds that name once the rename is undone.
        let clear: Vec<(String, String, Vec<String>)> =
            KEY_TABLES.iter().map(|t| (t.to_string(), "agent_id = ?1 COLLATE NOCASE".to_string(), vec![fold(&a.new)])).collect();
        let restored = restore_rows(ctx, Path::new(&b.db), &clear, &rows, |c| {
            if !b.tombstones {
                return Ok(());
            }
            match &b.tombstone {
                Some((uid, at)) => c
                    .execute(
                        "INSERT OR REPLACE INTO db_agent_key_tombstones (name, deleted_uid, deleted_at) VALUES (?1, ?2, ?3)",
                        params![fold(&a.old), uid, at],
                    )
                    .map(|_| ()),
                None => c
                    .execute("DELETE FROM db_agent_key_tombstones WHERE name = ?1 AND deleted_uid = ?2", [&fold(&a.old), &a.uid])
                    .map(|_| ()),
            }
            .map_err(err)
        })?;
        changes.push(format!("{}: {restored} signing key row(s) restored from the backup", b.db));
    }
    Ok(changes)
}

// ── S4: wan.db ───────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct S4Db {
    db: String,
    keys: Vec<String>,
    publications: Vec<(String, String)>,
}

pub(super) fn s4_before(ctx: &Ctx, a: &Agent) -> Result<Value, String> {
    let mut out = Vec::new();
    for db in ctx.wan_dbs() {
        let conn = open_ro(&db)?;
        let mut keys = Vec::new();
        let mut publications = Vec::new();
        for name in key_names(a) {
            if table_exists(&conn, "wan_agent_keys") {
                let mut stmt = conn.prepare("SELECT agent_id FROM wan_agent_keys WHERE agent_id = ?1 COLLATE NOCASE").map_err(err)?;
                for id in stmt.query_map([&name], |r| r.get::<_, String>(0)).map_err(err)? {
                    keys.push(id.map_err(err)?);
                }
            }
            if table_exists(&conn, "wan_agent_key_publications") {
                let mut stmt = conn
                    .prepare("SELECT agent_id, directory FROM wan_agent_key_publications WHERE agent_id = ?1 COLLATE NOCASE")
                    .map_err(err)?;
                for row in stmt.query_map([&name], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(err)? {
                    publications.push(row.map_err(err)?);
                }
            }
        }
        if !keys.is_empty() || !publications.is_empty() {
            out.push(S4Db { db: path_str(&db), keys, publications });
        }
    }
    to_value(&out)
}

pub(super) fn s4_apply(_ctx: &Ctx, a: &Agent, before: &Value) -> Result<Vec<String>, String> {
    let mut changes = Vec::new();
    for b in parse::<Vec<S4Db>>(before)? {
        let conn = open_rw(Path::new(&b.db))?;
        let removed = in_tx(&conn, |c| {
            let mut n = 0;
            for id in &b.keys {
                n += c.execute("DELETE FROM wan_agent_keys WHERE agent_id = ?1", [id]).map_err(err)?;
            }
            for (id, dir) in &b.publications {
                n += c.execute("DELETE FROM wan_agent_key_publications WHERE agent_id = ?1 AND directory = ?2", [id, dir]).map_err(err)?;
            }
            Ok(n)
        })?;
        if removed > 0 {
            changes.push(format!(
                "{}: the WAN key and {} publication record(s) under {} retired",
                b.db,
                b.publications.len(),
                fold(&a.old)
            ));
        }
    }
    Ok(changes)
}

pub(super) fn s4_undo(ctx: &Ctx, a: &Agent, before: &Value) -> Result<Vec<String>, String> {
    let mut changes = Vec::new();
    for b in parse::<Vec<S4Db>>(before)? {
        let clear: Vec<(String, String, Vec<String>)> = ["wan_agent_keys", "wan_agent_key_publications"]
            .iter()
            .map(|t| (t.to_string(), "agent_id = ?1 COLLATE NOCASE".to_string(), vec![fold(&a.new)]))
            .collect();
        let mut rows: Vec<(String, String, Vec<String>)> =
            b.keys.iter().map(|id| ("wan_agent_keys".to_string(), "agent_id = ?1".to_string(), vec![id.clone()])).collect();
        rows.extend(b.publications.iter().map(|(id, dir)| {
            ("wan_agent_key_publications".to_string(), "agent_id = ?1 AND directory = ?2".to_string(), vec![id.clone(), dir.clone()])
        }));
        let restored = restore_rows(ctx, Path::new(&b.db), &clear, &rows, |_| Ok(()))?;
        changes.push(format!("{}: {restored} WAN row(s) restored from the backup", b.db));
    }
    Ok(changes)
}

// ── S5: the MuxBus credential, kept ──────────────────────────────────

pub(super) fn s5_before(ctx: &Ctx, a: &Agent) -> Result<Value, String> {
    let mut found = Vec::new();
    for db in ctx.host_stores().into_iter().chain(ctx.objects_dbs()) {
        let conn = open_ro(&db)?;
        if !table_exists(&conn, "db_agent_credentials") {
            continue;
        }
        let mut stmt = conn
            .prepare(
                "SELECT agent_id FROM db_agent_credentials WHERE agent_id = ?1 COLLATE NOCASE
                 OR (instr(agent_id, '/') > 0 AND substr(agent_id, instr(agent_id, '/') + 1) = ?1 COLLATE NOCASE)",
            )
            .map_err(err)?;
        for id in stmt.query_map([&fold(&a.old)], |r| r.get::<_, String>(0)).map_err(err)? {
            found.push(format!("{}: {}", db.display(), id.map_err(err)?));
        }
    }
    to_value(&found)
}

pub(super) fn s5_apply(before: &Value) -> Result<Vec<String>, String> {
    let found: Vec<String> = parse(before)?;
    Ok(found.into_iter().map(|f| format!("{f} kept for the 30-day receive-only grace subscription")).collect())
}

// ── S6: rows that name the agent and carry no uid ────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct S6Update {
    db: String,
    table: String,
    id: String,
    /// Each column it sets, with the value it had.
    set: Vec<(String, String)>,
}

#[derive(Debug, Serialize, Deserialize)]
struct S6Grant {
    db: String,
    agent_id: String,
    peer: String,
    tier: String,
    granted_at: i64,
    new_existed: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct S6Before {
    updates: Vec<S6Update>,
    grants: Vec<S6Grant>,
}

/// The name-only rules: (table, the name column, its uid column, extra filter).
const NAME_ONLY: [(&str, &str, &str, &str); 3] = [
    ("db_cron_jobs", "target", "target_uid", ""),
    ("db_work_queue", "target_agent", "target_agent_uid", "AND state IN ('open', 'claimed')"),
    ("db_work_queue", "claimed_by", "claimed_by_uid", "AND state = 'claimed'"),
];

fn renamed(name: &str, a: &Agent) -> String {
    if fold(name) == fold(&a.old) {
        fold(&a.new)
    } else {
        name.to_string()
    }
}

pub(super) fn s6_before(ctx: &Ctx, a: &Agent) -> Result<Value, String> {
    let mut b = S6Before::default();
    for db in ctx.host_stores().into_iter().chain(ctx.objects_dbs()) {
        let conn = open_ro(&db)?;
        for (table, col, uid_col, filter) in NAME_ONLY {
            if !table_exists(&conn, table) || !has_column(&conn, table, uid_col) {
                continue;
            }
            let mut stmt = conn
                .prepare(&format!("SELECT id, {col} FROM {table} WHERE {uid_col} = '' AND {col} = ?1 COLLATE NOCASE {filter}"))
                .map_err(err)?;
            for row in stmt.query_map([&a.old], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(err)? {
                let (id, name) = row.map_err(err)?;
                b.updates.push(S6Update {
                    db: path_str(&db),
                    table: table.to_string(),
                    id,
                    set: vec![(col.to_string(), name), (uid_col.to_string(), String::new())],
                });
            }
        }
        if table_exists(&conn, "db_conversation_trust_grants") {
            let mut stmt = conn
                .prepare(
                    "SELECT agent_id, granted_peer_agent_id, tier, granted_at FROM db_conversation_trust_grants
                     WHERE agent_id = ?1 OR granted_peer_agent_id = ?1",
                )
                .map_err(err)?;
            let rows: Vec<(String, String, String, i64)> = stmt
                .query_map([&fold(&a.old)], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .map_err(err)?
                .collect::<Result<_, _>>()
                .map_err(err)?;
            for (agent_id, peer, tier, granted_at) in rows {
                let new_existed = conn
                    .query_row(
                        "SELECT 1 FROM db_conversation_trust_grants WHERE agent_id = ?1 AND granted_peer_agent_id = ?2 AND tier = ?3",
                        [&renamed(&agent_id, a), &renamed(&peer, a), &tier],
                        |_| Ok(()),
                    )
                    .optional()
                    .map_err(err)?
                    .is_some();
                b.grants.push(S6Grant { db: path_str(&db), agent_id, peer, tier, granted_at, new_existed });
            }
        }
    }
    to_value(&b)
}

/// The value S6 gives a column: the uid for a uid column, the new name for
/// a name column.
fn s6_value(col: &str, a: &Agent) -> String {
    if col.ends_with("_uid") {
        a.uid.clone()
    } else {
        fold(&a.new)
    }
}

fn by_db<T>(items: &[T], db: impl Fn(&T) -> &str) -> BTreeMap<String, Vec<&T>> {
    let mut out: BTreeMap<String, Vec<&T>> = BTreeMap::new();
    for item in items {
        out.entry(db(item).to_string()).or_default().push(item);
    }
    out
}

pub(super) fn s6_apply(_ctx: &Ctx, a: &Agent, before: &Value) -> Result<Vec<String>, String> {
    let b: S6Before = parse(before)?;
    let mut changes = Vec::new();
    let mut dbs: Vec<String> = b.updates.iter().map(|u| u.db.clone()).chain(b.grants.iter().map(|g| g.db.clone())).collect();
    dbs.sort();
    dbs.dedup();
    let updates = by_db(&b.updates, |u| &u.db);
    let grants = by_db(&b.grants, |g| &g.db);
    for db in dbs {
        let conn = open_rw(Path::new(&db))?;
        let n = in_tx(&conn, |c| {
            let mut n = 0;
            for u in updates.get(&db).into_iter().flatten() {
                let (name_col, old_name) = &u.set[0];
                let assigns = u.set.iter().map(|(col, _)| format!("{col} = ?")).collect::<Vec<_>>().join(", ");
                let mut args: Vec<String> = u.set.iter().map(|(col, _)| s6_value(col, a)).collect();
                args.push(u.id.clone());
                args.push(old_name.clone());
                n += c
                    .execute(&format!("UPDATE {} SET {assigns} WHERE id = ? AND {name_col} = ?", u.table), rusqlite::params_from_iter(&args))
                    .map_err(err)?;
            }
            for g in grants.get(&db).into_iter().flatten() {
                n += c
                    .execute(
                        "DELETE FROM db_conversation_trust_grants WHERE agent_id = ?1 AND granted_peer_agent_id = ?2 AND tier = ?3",
                        [&g.agent_id, &g.peer, &g.tier],
                    )
                    .map_err(err)?;
                c.execute(
                    "INSERT OR IGNORE INTO db_conversation_trust_grants (agent_id, granted_peer_agent_id, tier, granted_at) VALUES (?1, ?2, ?3, ?4)",
                    params![renamed(&g.agent_id, a), renamed(&g.peer, a), g.tier, g.granted_at],
                )
                .map_err(err)?;
            }
            Ok(n)
        })?;
        if n > 0 {
            changes.push(format!("{db}: {n} row(s) naming {} now name {} and carry its uid", fold(&a.old), fold(&a.new)));
        }
    }
    Ok(changes)
}

pub(super) fn s6_undo(a: &Agent, before: &Value) -> Result<Vec<String>, String> {
    let b: S6Before = parse(before)?;
    let mut changes = Vec::new();
    let mut dbs: Vec<String> = b.updates.iter().map(|u| u.db.clone()).chain(b.grants.iter().map(|g| g.db.clone())).collect();
    dbs.sort();
    dbs.dedup();
    let updates = by_db(&b.updates, |u| &u.db);
    let grants = by_db(&b.grants, |g| &g.db);
    for db in dbs {
        let conn = open_rw(Path::new(&db))?;
        in_tx(&conn, |c| {
            for u in updates.get(&db).into_iter().flatten() {
                let (name_col, _) = &u.set[0];
                let assigns = u.set.iter().map(|(col, _)| format!("{col} = ?")).collect::<Vec<_>>().join(", ");
                let mut args: Vec<String> = u.set.iter().map(|(_, old)| old.clone()).collect();
                args.push(u.id.clone());
                args.push(s6_value(name_col, a));
                c.execute(&format!("UPDATE {} SET {assigns} WHERE id = ? AND {name_col} = ?", u.table), rusqlite::params_from_iter(&args))
                    .map_err(err)?;
            }
            for g in grants.get(&db).into_iter().flatten() {
                c.execute(
                    "INSERT OR IGNORE INTO db_conversation_trust_grants (agent_id, granted_peer_agent_id, tier, granted_at) VALUES (?1, ?2, ?3, ?4)",
                    params![g.agent_id, g.peer, g.tier, g.granted_at],
                )
                .map_err(err)?;
                if !g.new_existed {
                    c.execute(
                        "DELETE FROM db_conversation_trust_grants WHERE agent_id = ?1 AND granted_peer_agent_id = ?2 AND tier = ?3",
                        [&renamed(&g.agent_id, a), &renamed(&g.peer, a), &g.tier],
                    )
                    .map_err(err)?;
                }
            }
            Ok(())
        })?;
        changes.push(format!("{db}: name-only rows back to {}", fold(&a.old)));
    }
    Ok(changes)
}

// ── S7: persisted identity in cmd:env ────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct S7Change {
    /// JSON pointer to the `cmd:env` object, within the row's data.
    pointer: String,
    key: String,
    old: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct S7Row {
    db: String,
    table: String,
    oid: String,
    /// The row's version before the step, which its undo puts back when
    /// nothing else has written the row since.
    version: i64,
    changes: Vec<S7Change>,
}

/// The value an identity variable takes after the rename, if it named the
/// old id: the id and slug variables and the git names equal to it, and a
/// git email whose local part is it (the domain kept).
fn rewrite(key: &str, value: &str, a: &Agent) -> Option<String> {
    match key {
        "AGENTMUX_AGENT_ID" | "MUXBUS_AGENT_ID" | "AGENTMUX_AGENT_SLUG" | "GIT_AUTHOR_NAME" | "GIT_COMMITTER_NAME" => {
            (fold(value) == fold(&a.old)).then(|| a.new.clone())
        }
        "GIT_AUTHOR_EMAIL" | "GIT_COMMITTER_EMAIL" => {
            let (local, domain) = value.split_once('@')?;
            (fold(local) == fold(&a.old)).then(|| format!("{}@{domain}", fold(&a.new)))
        }
        _ => None,
    }
}

fn escape(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

/// Every identity variable in `value` (a block's meta, or a layout snapshot)
/// that names the old id, in an object that is this agent's: its `agentId`
/// is the uid, or it has none and its own id or slug variable names the old
/// id. Another agent's block is never touched.
fn collect(value: &Value, pointer: &str, a: &Agent, out: &mut Vec<S7Change>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::Object(env)) = map.get("cmd:env") {
                let owner = ["agentId", "agent:id"].iter().find_map(|k| map.get(*k).and_then(Value::as_str)).filter(|s| !s.is_empty());
                let names_old = ["AGENTMUX_AGENT_ID", "AGENTMUX_AGENT_SLUG"]
                    .iter()
                    .any(|k| env.get(*k).and_then(Value::as_str).is_some_and(|v| fold(v) == fold(&a.old)));
                let ours = match owner {
                    Some(uid) => uid == a.uid,
                    None => names_old,
                };
                if ours {
                    for (key, v) in env {
                        if let Some(old) = v.as_str().filter(|v| rewrite(key, v, a).is_some()) {
                            out.push(S7Change { pointer: format!("{pointer}/cmd:env"), key: key.clone(), old: old.to_string() });
                        }
                    }
                }
            }
            for (k, child) in map.iter().filter(|(k, _)| k.as_str() != "cmd:env") {
                collect(child, &format!("{pointer}/{}", escape(k)), a, out);
            }
        }
        Value::Array(items) => {
            for (i, child) in items.iter().enumerate() {
                collect(child, &format!("{pointer}/{i}"), a, out);
            }
        }
        _ => {}
    }
}

/// Where each table keeps what S7 rewrites.
const S7_ROOTS: [(&str, &str); 2] = [("db_block", "/meta"), ("db_client", "/meta/session:last_topology")];

/// A row S7 would change: its table, oid, version and the variables.
pub(super) type S7Found = (String, String, i64, Vec<S7Change>);

pub(super) fn s7_rows(db: &Path, a: &Agent) -> Result<Vec<S7Found>, String> {
    let conn = open_ro(db)?;
    let mut out = Vec::new();
    for (table, root) in S7_ROOTS.iter().filter(|(t, _)| table_exists(&conn, t)) {
        let mut stmt = conn.prepare(&format!("SELECT oid, version, data FROM {table}")).map_err(err)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, Vec<u8>>(2)?)))
            .map_err(err)?;
        for row in rows {
            let (oid, version, data) = row.map_err(err)?;
            let Ok(value) = serde_json::from_slice::<Value>(&data) else { continue };
            let Some(start) = value.pointer(root) else { continue };
            let mut changes = Vec::new();
            collect(start, root, a, &mut changes);
            if !changes.is_empty() {
                out.push((table.to_string(), oid, version, changes));
            }
        }
    }
    Ok(out)
}

pub(super) fn s7_before(ctx: &Ctx, a: &Agent) -> Result<Value, String> {
    let mut rows = Vec::new();
    for db in ctx.objects_dbs() {
        for (table, oid, version, changes) in s7_rows(&db, a)? {
            rows.push(S7Row { db: path_str(&db), table, oid, version, changes });
        }
    }
    Ok(serde_json::json!({ "rows": to_value(&rows)? }))
}

/// Rewrite each recorded variable whose value is still `from` to `to`, in
/// every row of `before`; `forward` picks the direction.
fn s7_write(a: &Agent, before: &Value, forward: bool) -> Result<Vec<String>, String> {
    let rows: Vec<S7Row> = parse(&before["rows"])?;
    let mut changes = Vec::new();
    for (db, group) in by_db(&rows, |r| &r.db) {
        let conn = open_rw(Path::new(&db))?;
        let n = in_tx(&conn, |c| {
            let mut n = 0;
            for row in group {
                let data: Option<Vec<u8>> =
                    c.query_row(&format!("SELECT data FROM {} WHERE oid = ?1", row.table), [&row.oid], |r| r.get(0)).optional().map_err(err)?;
                let Some(mut value) = data.and_then(|d| serde_json::from_slice::<Value>(&d).ok()) else { continue };
                let mut changed = false;
                for ch in &row.changes {
                    let Some(new) = rewrite(&ch.key, &ch.old, a) else { continue };
                    let (from, to) = if forward { (ch.old.as_str(), new.as_str()) } else { (new.as_str(), ch.old.as_str()) };
                    let Some(env) = value.pointer_mut(&ch.pointer).and_then(Value::as_object_mut) else { continue };
                    if env.get(&ch.key).and_then(Value::as_str) == Some(from) {
                        env.insert(ch.key.clone(), Value::String(to.to_string()));
                        changed = true;
                    }
                }
                if changed {
                    let bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
                    if forward {
                        c.execute(&format!("UPDATE {} SET data = ?1, version = version + 1 WHERE oid = ?2", row.table), params![bytes, row.oid])
                    } else {
                        c.execute(
                            &format!("UPDATE {} SET data = ?1, version = CASE WHEN version = ?3 + 1 THEN ?3 ELSE version + 1 END WHERE oid = ?2", row.table),
                            params![bytes, row.oid, row.version],
                        )
                    }
                    .map_err(err)?;
                    n += 1;
                }
            }
            Ok(n)
        })?;
        if n > 0 {
            let what = if forward { format!("now name {}", a.new) } else { format!("name {} again", a.old) };
            changes.push(format!("{db}: {n} pane(s) or layout snapshot(s) whose persisted identity {what}"));
        }
    }
    Ok(changes)
}

pub(super) fn s7_apply(_ctx: &Ctx, a: &Agent, before: &Value) -> Result<Vec<String>, String> {
    s7_write(a, before, true)
}

pub(super) fn s7_undo(a: &Agent, before: &Value) -> Result<Vec<String>, String> {
    s7_write(a, before, false)
}

/// The working folders of the agent's blocks (`cmd:cwd`), from every
/// `objects.db`, for S7's `.mcp.json`.
pub(super) fn block_cwds(ctx: &Ctx, a: &Agent) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for db in ctx.objects_dbs() {
        let conn = open_ro(&db)?;
        if !table_exists(&conn, "db_block") {
            continue;
        }
        let mut stmt = conn.prepare("SELECT data FROM db_block").map_err(err)?;
        for data in stmt.query_map([], |r| r.get::<_, Vec<u8>>(0)).map_err(err)? {
            let Ok(block) = serde_json::from_slice::<Value>(&data.map_err(err)?) else { continue };
            let Some(meta) = block.get("meta") else { continue };
            let ours = ["agentId", "agent:id"].iter().any(|k| meta.get(*k).and_then(Value::as_str) == Some(a.uid.as_str()));
            if let Some(cwd) = meta.get("cmd:cwd").and_then(Value::as_str).filter(|c| ours && !c.trim().is_empty()) {
                out.push(cwd.to_string());
            }
        }
    }
    Ok(out)
}
