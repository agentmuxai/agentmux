// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The Kanban's side of the sync layer (SPEC_SHARED_KANBAN_ACROSS_MACHINES §3.3, build step K1): what a board's
//! tasks look like as records in the change log, and the merge rule for each kind of record.
//!
//! [`capture`] reads the Kanban's tables and writes whatever changed since the last capture into the log of the
//! task's board, one record per field (so two machines editing different fields of a card both keep their edit),
//! one for the task's lifecycle (state, holder, attempts, result: these move together), one per dependency and
//! one per history line. It sends nothing and changes no Kanban table: taking a record from another machine into
//! the tables comes with the numbering of the next step. A lease's expiry is not recorded (every heartbeat would
//! write a row); the number a card has on a machine is not either (numbers are per board, see the spec §3.2).
//!
//! Records of a board are in the collection [`collection_id`] names; a record id is `<task>.<field>`,
//! `<task>.life`, `<task>.after.<other task>` or `<task>.event.<machine>.<line>`.

use std::collections::BTreeMap;

use rusqlite::types::ValueRef;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use super::adapter::{Adapter, Merge, NewestWins};
use super::log::{self, Signer, SyncError};
use super::record::Header;

/// The kind of a Kanban board's collection.
pub const KIND: &str = "kanban";

/// The collection a board's records are in.
pub fn collection_id(board_id: &str) -> String {
    format!("kanban.{board_id}")
}

/// The lifecycle fields: written as one record, because they only make sense together.
const LIFE_FIELDS: [&str; 5] = ["state", "claimed_by", "claimed_by_uid", "attempts", "result"];

/// What a capture reads: the task's own columns and its card's. The first two columns name the task and its board;
/// every other one is a field (or a lifecycle field).
const TASK_SQL: &str = "SELECT q.id AS id, c.board_id AS board_id,
    q.title, q.payload, q.kind, q.target_agent, q.target_agent_uid, q.target_group, q.priority, q.max_attempts,
    q.created_by, q.created_by_uid, q.not_before,
    q.state, q.claimed_by, q.claimed_by_uid, q.attempts, q.result,
    c.repo, c.branch, c.pr_url, c.rank, c.review_required, c.checklist, c.question, c.note,
    c.requested_by, c.requested_by_kind, c.request_text, c.filed_by
  FROM db_work_queue q LEFT JOIN db_work_cards c ON c.work_id = q.id";

/// What a capture remembers: per task its board, a digest of what was recorded (so an unchanged task costs one
/// lookup) and the record ids written; and how far the history lines have been read.
const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_sync_kanban_seen (
        work_id TEXT PRIMARY KEY,
        board   TEXT NOT NULL,
        hash    TEXT NOT NULL,
        records TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS db_sync_kanban_mark (
        name  TEXT PRIMARY KEY,
        value INTEGER NOT NULL
    );";

/// Creates the log's tables and this module's.
pub fn init(conn: &Connection) -> rusqlite::Result<()> {
    log::init(conn)?;
    conn.execute_batch(SCHEMA)
}

/// What a capture did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Captured {
    /// Records written (a new value, or a task's first).
    pub written: usize,
    /// Records ended with a tombstone (a task gone, moved to another board, or a dependency taken off).
    pub tombstoned: usize,
}

/// A task as it is now: its board and every record it should have.
struct Task {
    board: String,
    records: BTreeMap<String, Value>,
}

fn json_of(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Integer(i) => json!(i),
        ValueRef::Real(f) => json!(f),
        ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Null | ValueRef::Blob(_) => Value::Null,
    }
}

fn read_tasks(conn: &Connection, default_board: &str) -> rusqlite::Result<BTreeMap<String, Task>> {
    let mut stmt = conn.prepare(TASK_SQL)?;
    let names: Vec<String> = stmt.column_names().into_iter().map(|s| s.to_string()).collect();
    let mut rows = stmt.query([])?;
    let mut tasks = BTreeMap::new();
    while let Some(r) = rows.next()? {
        let work_id: String = r.get(0)?;
        let board = r.get::<_, Option<String>>(1)?.filter(|b| !b.is_empty()).unwrap_or_else(|| default_board.to_string());
        let mut records = BTreeMap::new();
        let mut life = Map::new();
        for (i, name) in names.iter().enumerate().skip(2) {
            let v = json_of(r.get_ref(i)?);
            if LIFE_FIELDS.contains(&name.as_str()) {
                life.insert(name.clone(), v);
            } else {
                records.insert(format!("{work_id}.{name}"), v);
            }
        }
        records.insert(format!("{work_id}.life"), Value::Object(life));
        tasks.insert(work_id, Task { board, records });
    }
    let mut deps = conn.prepare("SELECT work_id, after_id FROM db_work_deps")?;
    let pairs = deps.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for pair in pairs {
        let (work_id, after) = pair?;
        if let Some(t) = tasks.get_mut(&work_id) {
            t.records.insert(format!("{work_id}.after.{after}"), Value::Bool(true));
        }
    }
    Ok(tasks)
}

fn digest(records: &BTreeMap<String, Value>) -> String {
    let bytes = serde_json::to_vec(records).unwrap_or_default();
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// What was last recorded of a task: its board, the digest and the record ids.
struct Seen {
    board: String,
    hash: String,
    records: Vec<String>,
}

fn seen_of(conn: &Connection, work_id: &str) -> rusqlite::Result<Option<Seen>> {
    conn.query_row("SELECT board, hash, records FROM db_sync_kanban_seen WHERE work_id = ?1", params![work_id], |r| {
        let records: String = r.get(2)?;
        Ok(Seen { board: r.get(0)?, hash: r.get(1)?, records: serde_json::from_str(&records).unwrap_or_default() })
    })
    .optional()
}

fn save_seen(conn: &Connection, work_id: &str, task: &Task, hash: &str) -> rusqlite::Result<()> {
    let ids: Vec<&String> = task.records.keys().collect();
    conn.execute(
        "INSERT OR REPLACE INTO db_sync_kanban_seen (work_id, board, hash, records) VALUES (?1, ?2, ?3, ?4)",
        params![work_id, task.board, hash, serde_json::to_string(&ids).unwrap_or_else(|_| "[]".into())],
    )?;
    Ok(())
}

/// Writes a record unless the log's current state already holds exactly this value.
fn write_if_changed(
    conn: &Connection,
    signer: &mut Signer,
    collection: &str,
    record: &str,
    body: Vec<u8>,
    now_ms: i64,
) -> Result<bool, SyncError> {
    if let Some(cur) = log::current(conn, collection, record)? {
        if !cur.header.tombstone && cur.body == body {
            return Ok(false);
        }
    }
    log::append_local(conn, signer, collection, record, body, false, now_ms)?;
    Ok(true)
}

/// Ends a record, if it is live.
fn tombstone(conn: &Connection, signer: &mut Signer, collection: &str, record: &str, now_ms: i64) -> Result<bool, SyncError> {
    match log::current(conn, collection, record)? {
        Some(cur) if !cur.header.tombstone => {
            log::append_local(conn, signer, collection, record, Vec::new(), true, now_ms)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn capture_task(conn: &Connection, signer: &mut Signer, work_id: &str, task: &Task, now_ms: i64, out: &mut Captured) -> Result<(), SyncError> {
    let hash = digest(&task.records);
    let seen = seen_of(conn, work_id)?;
    if seen.as_ref().is_some_and(|s| s.board == task.board && s.hash == hash) {
        return Ok(());
    }
    let collection = collection_id(&task.board);
    log::ensure_collection(conn, &collection, KIND, now_ms)?;
    for (record, value) in &task.records {
        let body = serde_json::to_vec(value).unwrap_or_default();
        if write_if_changed(conn, signer, &collection, record, body, now_ms)? {
            out.written += 1;
        }
    }
    // What the task had before and no longer has: a dependency taken off, or everything when it changed board.
    if let Some(old) = seen {
        let old_collection = collection_id(&old.board);
        for record in old.records.iter().filter(|r| old.board != task.board || !task.records.contains_key(*r)) {
            if tombstone(conn, signer, &old_collection, record, now_ms)? {
                out.tombstoned += 1;
            }
        }
    }
    save_seen(conn, work_id, task, &hash)?;
    Ok(())
}

/// Ends the records of tasks that are no longer in the Kanban.
fn capture_gone(conn: &Connection, signer: &mut Signer, tasks: &BTreeMap<String, Task>, now_ms: i64, out: &mut Captured) -> Result<(), SyncError> {
    let mut stmt = conn.prepare("SELECT work_id, board, records FROM db_sync_kanban_seen")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
    let gone: Vec<(String, String, String)> = rows.collect::<Result<_, _>>()?;
    for (work_id, board, records) in gone.into_iter().filter(|(w, _, _)| !tasks.contains_key(w)) {
        let ids: Vec<String> = serde_json::from_str(&records).unwrap_or_default();
        let collection = collection_id(&board);
        for record in ids {
            if tombstone(conn, signer, &collection, &record, now_ms)? {
                out.tombstoned += 1;
            }
        }
        conn.execute("DELETE FROM db_sync_kanban_seen WHERE work_id = ?1", params![work_id])?;
    }
    Ok(())
}

const EVENT_BATCH: i64 = 500;

fn event_mark(conn: &Connection) -> rusqlite::Result<i64> {
    let v: Option<i64> = conn.query_row("SELECT value FROM db_sync_kanban_mark WHERE name = 'events'", [], |r| r.get(0)).optional()?;
    Ok(v.unwrap_or(0))
}

/// History lines are written once each, after the id of the last one read. A line of a task that is not in the
/// Kanban any more is skipped.
fn capture_events(conn: &Connection, signer: &mut Signer, tasks: &BTreeMap<String, Task>, now_ms: i64, out: &mut Captured) -> Result<(), SyncError> {
    loop {
        let after = event_mark(conn)?;
        let mut stmt = conn.prepare(
            "SELECT id, work_id, at, actor, actor_kind, kind, from_state, to_state, body FROM db_work_events
             WHERE id > ?1 ORDER BY id LIMIT ?2",
        )?;
        let lines = stmt
            .query_map(params![after, EVENT_BATCH], |r| {
                let body = json!({
                    "at": r.get::<_, i64>(2)?, "actor": r.get::<_, String>(3)?, "actor_kind": r.get::<_, String>(4)?,
                    "kind": r.get::<_, String>(5)?, "from_state": r.get::<_, String>(6)?, "to_state": r.get::<_, String>(7)?,
                    "body": r.get::<_, String>(8)?,
                });
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, body))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let Some(last) = lines.last().map(|l| l.0) else { return Ok(()) };
        for (id, work_id, body) in &lines {
            let Some(task) = tasks.get(work_id) else { continue };
            let collection = collection_id(&task.board);
            log::ensure_collection(conn, &collection, KIND, now_ms)?;
            let record = format!("{work_id}.event.{}.{id}", signer.machine);
            if write_if_changed(conn, signer, &collection, &record, serde_json::to_vec(body).unwrap_or_default(), now_ms)? {
                out.written += 1;
            }
        }
        conn.execute("INSERT OR REPLACE INTO db_sync_kanban_mark (name, value) VALUES ('events', ?1)", params![last])?;
        if (lines.len() as i64) < EVENT_BATCH {
            return Ok(());
        }
    }
}

/// Writes into the log whatever changed in the Kanban since the last capture. `default_board` is the board a
/// task with no board of its own is on. Safe to run again at any time: a record is written only when its value
/// differs from what the log holds, so a run that stopped half way is finished by the next.
pub fn capture(conn: &Connection, signer: &mut Signer, default_board: &str, now_ms: i64) -> Result<Captured, SyncError> {
    let tasks = read_tasks(conn, default_board)?;
    let mut out = Captured::default();
    for (work_id, task) in &tasks {
        capture_task(conn, signer, work_id, task, now_ms, &mut out)?;
    }
    capture_gone(conn, signer, &tasks, now_ms, &mut out)?;
    capture_events(conn, signer, &tasks, now_ms, &mut out)?;
    Ok(out)
}

/// The Kanban's merge rule: a field, the lifecycle, a dependency: the newest version is current (a deletion is
/// the newest version of its record, so it wins over an older edit and loses to a newer one); a history line is
/// written once, and the first one held stays. Which machine may write a task's lifecycle (its home) comes with
/// the home field of the numbering step.
pub struct KanbanAdapter;

impl Adapter for KanbanAdapter {
    fn merge(&self, local: Option<&Header>, incoming: &Header) -> Merge {
        match local {
            Some(_) if incoming.record.contains(".event.") => Merge::KeptLocal,
            _ => NewestWins.merge(local, incoming),
        }
    }
}
