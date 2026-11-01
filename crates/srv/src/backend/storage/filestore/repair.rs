// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Rebuilding a shared transcript store that SQLite reports as corrupt.
//!
//! A store whose pages are cross-linked (two srvs wrote it at once, a lost write, a bad disk) fails
//! every query that walks a damaged page, while a query that does not still works. Nothing could
//! repair it from inside AgentMux: the offline `scripts/recover-shared-transcripts` needs a
//! developer. Now the first corrupt error on the store ([`crate::backend::transcript::damage`])
//! asks for a repair ([`request`]), and the next start that finds no other srv running does it,
//! the way a reclaim does (`reclaim.rs`: the same gate, the same swap, a crash at any point
//! leaves files [`recover`] puts right):
//!
//! 1. `PRAGMA quick_check` on the store. If it passes, the request was a false alarm and is dropped.
//! 2. Every table is copied row by row into a new file. A table that fails part way is copied again
//!    row by row through its primary key, and each row that cannot be read is listed in the result
//!    ([`Lost`]) with its zone.
//! 3. The new file must pass `quick_check` and gets the original's `user_version`.
//! 4. The original is moved aside as `filestore.db.damaged-<ms>` and **never deleted**: it holds the
//!    rows the copy could not read, for the offline script, and the app does not decide that they
//!    are worth nothing. The new file takes its place.
//!
//! 5. The conversation of each agent whose zone lost rows is put back from the per-channel copies of
//!    it (`transcript_backfill::restore_damaged`), still under the gate and before the store is
//!    opened for use, so no live append can land between a size check and the replacement. The zones
//!    to put back are written to `filestore.db.repair-last` before the swap, and a start that finds
//!    them still pending does the restore (a crash between the swap and the restore loses nothing).

use std::path::{Path, PathBuf};
use std::time::Instant;

use rusqlite::types::Value;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

use super::FileStore;
use super::reclaim::{free_disk_bytes, open_gate_file, opens, read_json, remove_with_sidecars, rename_with_sidecars, sidecar, write_json};

/// Failed attempts before a request is dropped and its error shown.
const MAX_FAILURES: u32 = 2;
/// A failed repair is tried again at most this often.
const RETRY_EVERY_MS: u64 = 60 * 60 * 1000;
/// Free disk the rebuild needs beyond the live data: the new file is about the original's size.
const DISK_SLACK_BYTES: u64 = 1 << 30;
/// Rows written per transaction while copying.
const BATCH: usize = 256;
/// Unreadable rows kept in the result; the count is exact beyond it.
const LOST_LISTED: usize = 10_000;

/// The files beside `filestore.db` that a repair uses.
#[derive(Debug, Clone)]
pub struct Paths {
    pub db: PathBuf,
    /// The request, and why it waits or failed.
    pub marker: PathBuf,
    /// The rebuilt store, before the swap.
    pub new: PathBuf,
    /// The original, between the two renames of the swap.
    pub pre: PathBuf,
    /// The last repair's result.
    pub last: PathBuf,
    /// Shared with the reclaim: held exclusively while rebuilding, shared while an srv opens the store.
    pub gate: PathBuf,
}

impl Paths {
    pub fn for_db(db: &Path) -> Self {
        Self {
            db: db.to_path_buf(),
            marker: sidecar(db, ".repair"),
            new: sidecar(db, ".repair-new"),
            pre: sidecar(db, ".pre-repair"),
            last: sidecar(db, ".repair-last"),
            gate: sidecar(db, ".gate"),
        }
    }

    /// Where the damaged original is kept.
    pub fn damaged(&self, at_ms: u64) -> PathBuf {
        sidecar(&self.db, &format!(".damaged-{at_ms}"))
    }
}

/// A request to repair at the next start (`filestore.db.repair`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    pub requested_at_ms: u64,
    /// What found the damage (`history.usage`, an append, a read).
    pub found_by: String,
    /// Why the last start didn't repair (another srv ran, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_for: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default)]
    pub failures: u32,
    #[serde(default)]
    pub last_try_ms: u64,
}

/// A row the copy could not read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lost {
    pub table: String,
    /// The row's primary key, its columns joined by `|`; empty when the table has none.
    pub key: String,
    /// Its zone, for the tables that have one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
}

/// What the last repair did (`filestore.db.repair-last`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Last {
    pub at_ms: u64,
    /// Where the damaged original is kept.
    #[serde(default)]
    pub kept_as: String,
    #[serde(default)]
    pub before_bytes: u64,
    #[serde(default)]
    pub after_bytes: u64,
    #[serde(default)]
    pub took_ms: u64,
    #[serde(default)]
    pub rows_copied: u64,
    /// How many rows could not be read (the first [`LOST_LISTED`] are in `lost`).
    #[serde(default)]
    pub rows_lost: u64,
    #[serde(default)]
    pub lost: Vec<Lost>,
    /// The zones that lost rows, for putting their history back from other copies.
    #[serde(default)]
    pub damaged_zones: Vec<String>,
    /// Whether the history of `damaged_zones` still has to be put back from other copies.
    #[serde(default)]
    pub restore_pending: bool,
    /// Set when the request was dropped after failing [`MAX_FAILURES`] times.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Ask for a repair at the next start. Asking again keeps the first request.
pub fn request(db: &Path, found_by: &str, now_ms: u64) -> std::io::Result<()> {
    let paths = Paths::for_db(db);
    if paths.marker.exists() {
        return Ok(());
    }
    write_json(&paths.marker, &Marker { requested_at_ms: now_ms, found_by: found_by.to_string(), ..Default::default() })
}

/// Whether a repair was asked for.
pub fn requested(db: &Path) -> bool {
    Paths::for_db(db).marker.exists()
}

/// Withdraw a request that hasn't run.
pub fn cancel(db: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(Paths::for_db(db).marker) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// The pending request and the last result.
pub fn status(db: &Path) -> (Option<Marker>, Option<Last>) {
    let paths = Paths::for_db(db);
    (read_json(&paths.marker), read_json(&paths.last))
}

/// Whether [`run_at_start`] has work: a repair was asked for, or an earlier one still has
/// conversations to put back.
pub fn needs_start(db: &Path) -> bool {
    let paths = Paths::for_db(db);
    paths.marker.exists() || read_json::<Last>(&paths.last).is_some_and(|l| l.restore_pending)
}

/// Put back the conversations an earlier repair left pending, if any, in the store at `paths.db`
/// (opened for it alone: the caller holds the gate). `restore` is given the store and the zones.
fn finish_restore(paths: &Paths, restore: &impl Fn(&FileStore, &[String])) -> Option<Last> {
    let mut last = read_json::<Last>(&paths.last).filter(|l| l.restore_pending && !l.damaged_zones.is_empty() && l.error.is_none())?;
    match FileStore::open_global_transcripts(&paths.db) {
        Ok(fs) => {
            restore(&fs, &last.damaged_zones);
            drop(fs);
            last.restore_pending = false;
            write_json(&paths.last, &last).ok();
            Some(last)
        }
        Err(e) => {
            tracing::warn!(error = %e, "history repair: the rebuilt store didn't open to put the conversations back");
            None
        }
    }
}

/// What [`run_at_start`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    NotRequested,
    /// Not now: another srv runs, the gate is held, or a failure retries later.
    Waiting(String),
    /// The store passed its check; the request was dropped.
    NotNeeded,
    Failed(String),
    Done(Box<Last>),
}

impl crate::boot_timing::StepOutcome for Outcome {
    fn succeeded(&self) -> bool {
        !matches!(self, Outcome::Failed(_))
    }
}

/// Put right whatever an interrupted repair left. Runs before a repair, whether or not one was
/// asked for, under the gate.
pub fn recover(paths: &Paths) {
    if !paths.db.exists() && paths.pre.exists() {
        // Interrupted between the two renames of the swap: the original goes back.
        remove_with_sidecars(&paths.new);
        if rename_with_sidecars(&paths.pre, &paths.db).is_ok() {
            tracing::warn!(db = %paths.db.display(), "history repair: an interrupted swap was rolled back");
        }
        return;
    }
    if paths.db.exists() && paths.pre.exists() {
        // The new store is in place. If it opens, the original is kept where a repair keeps it.
        if opens(&paths.db) {
            let kept = paths.damaged(read_json::<Marker>(&paths.marker).map_or(0, |m| m.last_try_ms));
            if rename_with_sidecars(&paths.pre, &kept).is_ok() {
                tracing::info!(kept = %kept.display(), "history repair: finished an interrupted swap");
            }
        } else {
            remove_with_sidecars(&paths.db);
            if rename_with_sidecars(&paths.pre, &paths.db).is_ok() {
                tracing::warn!("history repair: the rebuilt store didn't open; the original is back");
            }
        }
        return;
    }
    if paths.new.exists() {
        remove_with_sidecars(&paths.new);
        tracing::info!("history repair: deleted an interrupted rebuild");
    }
}

/// Repair if it was asked for and this srv is alone, then put back the conversations that lost rows.
/// Call before the store opens. `others` names another running srv (`framer::other_srv`); it is asked
/// at the start and again right before the swap. `restore` puts the conversations of the given zones
/// back in the store it is given (`transcript_backfill::restore_damaged`).
pub fn run_at_start(db: &Path, others: impl Fn() -> Option<String>, restore: impl Fn(&FileStore, &[String]), now_ms: u64) -> Outcome {
    let paths = Paths::for_db(db);
    // The reclaim's gate: nothing below may run while another srv reclaims or opens the store.
    let Ok(gate) = open_gate_file(&paths.gate) else {
        return Outcome::Waiting("the store's gate file can't be opened".to_string());
    };
    if gate.try_lock().is_err() {
        return Outcome::Waiting("another AgentMux is opening the store".to_string());
    }
    recover(&paths);
    let Some(mut marker) = read_json::<Marker>(&paths.marker) else {
        // No new request: an earlier repair may still have conversations to put back.
        if paths.db.exists() && others().is_none() {
            if let Some(last) = finish_restore(&paths, &restore) {
                return Outcome::Done(Box::new(last));
            }
        }
        return Outcome::NotRequested;
    };
    if !paths.db.exists() {
        cancel(db).ok();
        return Outcome::NotRequested;
    }
    if marker.failures > 0 && now_ms.saturating_sub(marker.last_try_ms) < RETRY_EVERY_MS {
        return Outcome::Waiting("the last try failed; it's tried again later".to_string());
    }
    let wait = |marker: &mut Marker, why: String| {
        marker.waiting_for = Some(why.clone());
        write_json(&paths.marker, marker).ok();
        tracing::info!(why = %why, "history repair: waiting");
        Outcome::Waiting(why)
    };
    if let Some(who) = others() {
        return wait(&mut marker, who);
    }
    marker.last_try_ms = now_ms;
    write_json(&paths.marker, &marker).ok();
    let started = Instant::now();
    match rebuild_and_swap(&paths, &others, now_ms) {
        Ok(Some(mut last)) => {
            last.took_ms = started.elapsed().as_millis() as u64;
            write_json(&paths.last, &last).ok();
            cancel(db).ok();
            if let Some(done) = finish_restore(&paths, &restore) {
                last = done;
            }
            tracing::warn!(
                rows_copied = last.rows_copied,
                rows_lost = last.rows_lost,
                zones = last.damaged_zones.len(),
                kept_as = %last.kept_as,
                "history repair: the shared store was rebuilt; the damaged original is kept"
            );
            Outcome::Done(Box::new(last))
        }
        Ok(None) => {
            cancel(db).ok();
            tracing::info!("history repair: the store passed its check; nothing to repair");
            // An earlier repair that was cut off after its swap still has its conversations to put back.
            if let Some(last) = finish_restore(&paths, &restore) {
                return Outcome::Done(Box::new(last));
            }
            Outcome::NotNeeded
        }
        Err(Step::Wait(why)) => wait(&mut marker, why),
        Err(Step::Fail(e)) => {
            remove_with_sidecars(&paths.new);
            marker.failures += 1;
            marker.last_error = Some(e.clone());
            marker.waiting_for = None;
            tracing::warn!(error = %e, failures = marker.failures, "history repair: failed; the original store is unchanged");
            if marker.failures >= MAX_FAILURES {
                write_json(&paths.last, &Last { at_ms: now_ms, error: Some(e.clone()), ..Default::default() }).ok();
                cancel(db).ok();
            } else {
                write_json(&paths.marker, &marker).ok();
            }
            Outcome::Failed(e)
        }
    }
}

enum Step {
    Wait(String),
    Fail(String),
}

fn fail<E: std::fmt::Display>(what: &str) -> impl FnOnce(E) -> Step + '_ {
    move |e| Step::Fail(format!("{what}: {e}"))
}

/// `Ok(None)`: the store passes its check. `Ok(Some(last))`: rebuilt, swapped in and opened.
fn rebuild_and_swap(paths: &Paths, others: &impl Fn() -> Option<String>, now_ms: u64) -> Result<Option<Last>, Step> {
    let before_bytes = std::fs::metadata(&paths.db).map(|m| m.len()).unwrap_or(0);
    {
        let src = open_read_only(&paths.db).map_err(fail("opening the store"))?;
        if check(&src).is_ok() {
            return Ok(None);
        }
    }
    if let Some(avail) = free_disk_bytes(&paths.db) {
        let need = before_bytes + before_bytes / 5 + DISK_SLACK_BYTES;
        if avail < need {
            return Err(Step::Wait(format!("{} GB of free disk is needed, {} GB is free", need.div_ceil(1 << 30), avail / (1 << 30))));
        }
    }
    remove_with_sidecars(&paths.new);
    let copied = salvage(&paths.db, &paths.new).map_err(|e| Step::Fail(format!("rebuilding the store: {e}")))?;
    {
        let new = Connection::open(&paths.new).map_err(fail("opening the rebuilt store"))?;
        check(&new).map_err(|e| Step::Fail(format!("the rebuilt store failed its check: {e}")))?;
    }
    if let Some(who) = others() {
        remove_with_sidecars(&paths.new);
        return Err(Step::Wait(who));
    }
    let kept = paths.damaged(now_ms);
    let mut zones: Vec<String> = copied.zones.iter().cloned().collect();
    zones.sort();
    let mut last = Last {
        at_ms: now_ms,
        kept_as: kept.to_string_lossy().into_owned(),
        before_bytes,
        after_bytes: 0,
        took_ms: 0,
        rows_copied: copied.rows,
        rows_lost: copied.lost_count,
        lost: copied.lost,
        restore_pending: !zones.is_empty(),
        damaged_zones: zones,
        error: None,
    };
    // Before the swap, so that a start after a crash right past it still knows what to put back.
    write_json(&paths.last, &last).map_err(fail("recording what to put back"))?;
    rename_with_sidecars(&paths.db, &paths.pre).map_err(fail("moving the original aside"))?;
    if let Err(e) = std::fs::rename(&paths.new, &paths.db) {
        let _ = rename_with_sidecars(&paths.pre, &paths.db);
        return Err(Step::Fail(format!("moving the rebuilt store in: {e}")));
    }
    if !opens(&paths.db) {
        remove_with_sidecars(&paths.db);
        let _ = rename_with_sidecars(&paths.pre, &paths.db);
        return Err(Step::Fail("the rebuilt store didn't open; the original is back".to_string()));
    }
    // Past the swap: if this rename fails the original stays as `.pre-repair`, which `recover` keeps.
    if rename_with_sidecars(&paths.pre, &kept).is_err() {
        last.kept_as = paths.pre.to_string_lossy().into_owned();
    }
    last.after_bytes = std::fs::metadata(&paths.db).map(|m| m.len()).unwrap_or(0);
    Ok(Some(last))
}

fn open_read_only(path: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
}

/// `PRAGMA quick_check`: `Ok` when it says so, else what it said (or the error that stopped it).
pub fn check(conn: &Connection) -> Result<(), String> {
    let mut stmt = conn.prepare("PRAGMA quick_check(5)").map_err(|e| e.to_string())?;
    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
    let mut said = Vec::new();
    loop {
        match rows.next() {
            Ok(Some(r)) => said.push(r.get::<_, String>(0).map_err(|e| e.to_string())?),
            Ok(None) => break,
            Err(e) => return Err(e.to_string()),
        }
    }
    if said == ["ok"] {
        Ok(())
    } else {
        Err(said.join("; "))
    }
}

/// What the copy made.
#[derive(Debug, Default)]
struct Copied {
    rows: u64,
    lost: Vec<Lost>,
    lost_count: u64,
    /// Every zone that lost a row, however many there were: the list above is capped, this is not.
    zones: std::collections::BTreeSet<String>,
}

impl Copied {
    fn lose(&mut self, table: &str, key: String, zone: Option<String>) {
        self.lost_count += 1;
        if let Some(zone) = &zone {
            self.zones.insert(zone.clone());
        }
        if self.lost.len() < LOST_LISTED {
            self.lost.push(Lost { table: table.to_string(), key, zone });
        }
    }
}

fn is_corrupt(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(f, _) if matches!(f.code, rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase))
}

/// The tables, then nothing else: indexes the tables declare come with them, and the triggers are
/// made again when the store opens (`run_filestore_migrations`), after the rows are in.
fn tables_of(src: &Connection) -> rusqlite::Result<Vec<(String, String)>> {
    let mut stmt = src.prepare("SELECT name, sql FROM sqlite_master WHERE type = 'table' AND sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY name")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    rows.collect()
}

/// A table's columns, and which of them make its primary key in order.
fn columns_of(src: &Connection, table: &str) -> rusqlite::Result<(Vec<String>, Vec<String>)> {
    let mut stmt = src.prepare(&format!("PRAGMA table_info(\"{table}\")"))?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(5)?)))?;
    let all: Vec<(String, i64)> = rows.collect::<Result<_, _>>()?;
    let mut pk: Vec<(String, i64)> = all.iter().filter(|(_, p)| *p > 0).cloned().collect();
    pk.sort_by_key(|(_, p)| *p);
    Ok((all.into_iter().map(|(n, _)| n).collect(), pk.into_iter().map(|(n, _)| n).collect()))
}

/// Copy `old` into a new store at `new`: the schema's tables, every row that can be read, and the
/// `user_version`.
fn salvage(old: &Path, new: &Path) -> Result<Copied, String> {
    let src = open_read_only(old).map_err(|e| format!("opening it: {e}"))?;
    let tables = tables_of(&src).map_err(|e| format!("reading its schema: {e}"))?;
    if tables.is_empty() {
        return Err("it has no tables I can read".to_string());
    }
    let user_version: i64 = src.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(|e| format!("reading its version: {e}"))?;
    let dst = Connection::open(new).map_err(|e| format!("creating the new store: {e}"))?;
    dst.execute_batch("PRAGMA page_size=4096; PRAGMA auto_vacuum=INCREMENTAL; PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF;").map_err(|e| e.to_string())?;
    for (name, sql) in &tables {
        dst.execute_batch(sql).map_err(|e| format!("creating {name}: {e}"))?;
    }
    let mut copied = Copied::default();
    for (name, _) in &tables {
        copy_table(&src, &dst, name, &mut copied).map_err(|e| format!("copying {name}: {e}"))?;
    }
    dst.execute_batch(&format!("PRAGMA user_version={user_version}; PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")).map_err(|e| e.to_string())?;
    Ok(copied)
}

/// One row's values.
fn row_values(r: &rusqlite::Row<'_>, n: usize) -> rusqlite::Result<Vec<Value>> {
    (0..n).map(|i| r.get::<_, Value>(i)).collect()
}

fn copy_table(src: &Connection, dst: &Connection, table: &str, copied: &mut Copied) -> Result<(), String> {
    let (columns, pk) = columns_of(src, table).map_err(|e| e.to_string())?;
    let marks = vec!["?"; columns.len()].join(",");
    let insert = format!("INSERT OR IGNORE INTO \"{table}\" VALUES ({marks})");
    // Pass 1: the table in order. Stops at the first page that cannot be read.
    match scan(src, dst, table, &insert, columns.len(), copied) {
        Ok(()) => return Ok(()),
        Err(e) if is_corrupt(&e) => {}
        Err(e) => return Err(e.to_string()),
    }
    // Pass 2: the keys from the primary key's own index (it doesn't read the table's pages), then
    // each row by its key; a row on a bad page is lost, the rest are copied.
    let key_list = keys(src, table, &pk);
    let select = format!("SELECT * FROM \"{table}\" WHERE {}", pk.iter().map(|c| format!("\"{c}\" = ?")).collect::<Vec<_>>().join(" AND "));
    let mut stmt = src.prepare(&select).map_err(|e| e.to_string())?;
    dst.execute_batch("BEGIN").map_err(|e| e.to_string())?;
    let mut in_batch = 0;
    for key in key_list.keys {
        match stmt.query_row(rusqlite::params_from_iter(key.iter()), |r| row_values(r, columns.len())) {
            Ok(values) => {
                dst.execute(&insert, rusqlite::params_from_iter(values.iter())).map_err(|e| e.to_string())?;
                copied.rows += 1;
                in_batch += 1;
                if in_batch >= BATCH {
                    dst.execute_batch("COMMIT; BEGIN").map_err(|e| e.to_string())?;
                    in_batch = 0;
                }
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {}
            Err(e) if is_corrupt(&e) => copied.lose(table, render_key(&key), zone_of(&pk, &key)),
            Err(e) => {
                dst.execute_batch("COMMIT").ok();
                return Err(e.to_string());
            }
        }
    }
    dst.execute_batch("COMMIT").map_err(|e| e.to_string())?;
    if key_list.cut {
        // The index itself ended in damage: rows after the last key are unknown.
        copied.lose(table, "(the rows after the last readable key)".to_string(), None);
    }
    Ok(())
}

/// Copy the table in order. Rows already in the new store (pass 2 after a partial pass 1) are skipped.
fn scan(src: &Connection, dst: &Connection, table: &str, insert: &str, n: usize, copied: &mut Copied) -> rusqlite::Result<()> {
    let mut stmt = src.prepare(&format!("SELECT * FROM \"{table}\""))?;
    let mut rows = stmt.query([])?;
    dst.execute_batch("BEGIN")?;
    let mut in_batch = 0;
    let result = loop {
        match rows.next() {
            Ok(Some(r)) => {
                let values = match row_values(r, n) {
                    Ok(v) => v,
                    Err(e) => break Err(e),
                };
                if let Err(e) = dst.execute(insert, rusqlite::params_from_iter(values.iter())) {
                    break Err(e);
                }
                copied.rows += 1;
                in_batch += 1;
                if in_batch >= BATCH {
                    dst.execute_batch("COMMIT; BEGIN")?;
                    in_batch = 0;
                }
            }
            Ok(None) => break Ok(()),
            Err(e) => break Err(e),
        }
    };
    // What was read before the damage stays.
    dst.execute_batch("COMMIT")?;
    result
}

struct Keys {
    keys: Vec<Vec<Value>>,
    /// The scan of the keys stopped on damage.
    cut: bool,
}

/// The primary keys of `table`, in order, as far as they can be read.
fn keys(src: &Connection, table: &str, pk: &[String]) -> Keys {
    let mut out = Keys { keys: Vec::new(), cut: false };
    if pk.is_empty() {
        out.cut = true;
        return out;
    }
    let cols = pk.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(",");
    let Ok(mut stmt) = src.prepare(&format!("SELECT {cols} FROM \"{table}\" ORDER BY {cols}")) else {
        out.cut = true;
        return out;
    };
    let Ok(mut rows) = stmt.query([]) else {
        out.cut = true;
        return out;
    };
    loop {
        match rows.next() {
            Ok(Some(r)) => match row_values(r, pk.len()) {
                Ok(k) => out.keys.push(k),
                Err(_) => {
                    out.cut = true;
                    break;
                }
            },
            Ok(None) => break,
            Err(_) => {
                out.cut = true;
                break;
            }
        }
    }
    out
}

fn render_key(key: &[Value]) -> String {
    key.iter()
        .map(|v| match v {
            Value::Text(t) => t.clone(),
            Value::Integer(i) => i.to_string(),
            Value::Real(f) => f.to_string(),
            Value::Null => String::new(),
            Value::Blob(b) => format!("<{} bytes>", b.len()),
        })
        .collect::<Vec<_>>()
        .join("|")
}

/// The zone of a row, when its key starts with one.
fn zone_of(pk: &[String], key: &[Value]) -> Option<String> {
    match (pk.first().map(String::as_str), key.first()) {
        (Some("zoneid"), Some(Value::Text(zone))) => Some(zone.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Copied, LOST_LISTED};

    #[test]
    fn every_zone_that_lost_a_row_is_kept_however_many_rows_were_lost() {
        let mut copied = Copied::default();
        for i in 0..LOST_LISTED + 50 {
            copied.lose("db_file_frame", i.to_string(), Some(format!("agent:z{i}:current")));
        }
        assert_eq!(copied.lost.len(), LOST_LISTED, "the list of rows is capped");
        assert_eq!(copied.zones.len(), LOST_LISTED + 50, "the zones are not");
        assert_eq!(copied.lost_count, (LOST_LISTED + 50) as u64);
    }
}
