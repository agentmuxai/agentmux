// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Giving disk space back (`docs/specs/SPEC_HISTORY_STORE_RECLAIM_SPACE_2026_10_23.md`).
//!
//! The framer deletes part rows as it writes frames, and SQLite keeps the
//! freed pages inside the file. Once, asked for from Warden's Storage view,
//! the shared transcript store is rewritten into a new file (`VACUUM INTO`) at
//! an srv's start, before it opens the store and only when no other srv runs,
//! and swapped in. The new file is `auto_vacuum=INCREMENTAL`, so from then on
//! [`FileStore::reclaim_free_pages`] gives freed pages back in small steps.
//!
//! The original is never written: a crash at any point leaves files that
//! [`recover`] puts right at the next start (spec §4.3).

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::core::FileStore;
use crate::backend::storage::error::StoreError;

/// Failed attempts before a request is dropped and its error shown.
const MAX_FAILURES: u32 = 2;
/// A failed reclaim is tried again at most this often.
const RETRY_EVERY_MS: u64 = 24 * 60 * 60 * 1000;
/// How long an srv opening the store waits for a reclaim in progress.
const GATE_WAIT: Duration = Duration::from_secs(120);
/// Free disk the rewrite needs beyond the live data: ×1.2 + 1 GB (spec §4.2).
const DISK_SLACK_BYTES: u64 = 1 << 30;
/// Below these the incremental step does nothing (spec §5).
pub const INCREMENTAL_MIN_FREE_PAGES: i64 = 2048;
pub const INCREMENTAL_STEP_PAGES: i64 = 1024;
pub const INCREMENTAL_PASS_PAGES: i64 = 16384;

/// The files beside `filestore.db` that a reclaim uses.
#[derive(Debug, Clone)]
pub struct Paths {
    pub db: PathBuf,
    /// The request, and why it waits or failed.
    pub marker: PathBuf,
    /// The rewritten store, before the swap.
    pub new: PathBuf,
    /// The original, between the swap and the first good open.
    pub pre: PathBuf,
    /// Held exclusively while reclaiming, shared while an srv opens the store.
    pub gate: PathBuf,
    /// The last reclaim's result.
    pub last: PathBuf,
}

impl Paths {
    pub fn for_db(db: &Path) -> Self {
        let with = |suffix: &str| {
            let mut s = db.as_os_str().to_os_string();
            s.push(suffix);
            PathBuf::from(s)
        };
        Self {
            db: db.to_path_buf(),
            marker: with(".reclaim"),
            new: with(".reclaim-new"),
            pre: with(".pre-reclaim"),
            gate: with(".gate"),
            last: with(".reclaim-last"),
        }
    }
}

fn sidecar(path: &Path, ext: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(ext);
    PathBuf::from(s)
}

/// A request to reclaim at the next start (`filestore.db.reclaim`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    pub requested_at_ms: u64,
    pub requested_by: String,
    /// Why the last start didn't reclaim (another srv ran, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_for: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default)]
    pub failures: u32,
    #[serde(default)]
    pub last_try_ms: u64,
}

/// What the last reclaim did (`filestore.db.reclaim-last`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Last {
    pub at_ms: u64,
    #[serde(default)]
    pub before_bytes: u64,
    #[serde(default)]
    pub after_bytes: u64,
    #[serde(default)]
    pub took_ms: u64,
    /// Set when the request was dropped after failing [`MAX_FAILURES`] times.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let tmp = sidecar(path, ".tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value).unwrap_or_default())?;
    std::fs::rename(&tmp, path)
}

/// Ask for a reclaim at the next start. Asking again keeps the first request.
pub fn request(db: &Path, by: &str, now_ms: u64) -> std::io::Result<()> {
    let paths = Paths::for_db(db);
    if paths.marker.exists() {
        return Ok(());
    }
    write_json(&paths.marker, &Marker { requested_at_ms: now_ms, requested_by: by.to_string(), ..Default::default() })
}

/// Whether a reclaim was asked for (a cheap check, before saying so on the splash).
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

/// The pending request and the last result, for the Storage view.
pub fn status(db: &Path) -> (Option<Marker>, Option<Last>) {
    let paths = Paths::for_db(db);
    (read_json(&paths.marker), read_json(&paths.last))
}

/// What [`recover`] found and did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recovered {
    Nothing,
    /// A rewrite was interrupted before the swap: its file was deleted.
    DroppedNew,
    /// The swap was interrupted between its renames: the original is back.
    RolledBack,
    /// The swap had finished; the original was deleted after the new store opened.
    Finished,
}

fn remove_quiet(path: &Path) {
    let _ = std::fs::remove_file(path);
}

fn remove_with_sidecars(path: &Path) {
    remove_quiet(path);
    remove_quiet(&sidecar(path, "-wal"));
    remove_quiet(&sidecar(path, "-shm"));
}

fn rename_with_sidecars(from: &Path, to: &Path) -> std::io::Result<()> {
    for ext in ["-wal", "-shm"] {
        let f = sidecar(from, ext);
        if f.exists() {
            std::fs::rename(&f, sidecar(to, ext))?;
        }
    }
    std::fs::rename(from, to)
}

/// Whether `path` opens as an SQLite store and its file table reads.
fn opens(path: &Path) -> bool {
    Connection::open(path)
        .and_then(|c| c.query_row("SELECT count(*) FROM db_wave_file", [], |r| r.get::<_, i64>(0)))
        .is_ok()
}

/// Put right whatever an interrupted reclaim left (spec §4.3). Runs at every
/// start, before the store opens, whether or not a reclaim was asked for.
pub fn recover(paths: &Paths) -> Recovered {
    let has_db = paths.db.exists();
    let has_pre = paths.pre.exists();
    if !has_db && has_pre {
        remove_with_sidecars(&paths.new);
        return match rename_with_sidecars(&paths.pre, &paths.db) {
            Ok(()) => {
                tracing::warn!(db = %paths.db.display(), "history reclaim: an interrupted swap was rolled back");
                Recovered::RolledBack
            }
            Err(e) => {
                tracing::error!(error = %e, "history reclaim: couldn't roll back an interrupted swap");
                Recovered::Nothing
            }
        };
    }
    if has_db && has_pre {
        if opens(&paths.db) {
            remove_with_sidecars(&paths.pre);
            tracing::info!("history reclaim: finished an interrupted swap");
            return Recovered::Finished;
        }
        remove_with_sidecars(&paths.db);
        if rename_with_sidecars(&paths.pre, &paths.db).is_ok() {
            tracing::warn!("history reclaim: the swapped-in store didn't open; the original is back");
            return Recovered::RolledBack;
        }
        return Recovered::Nothing;
    }
    if paths.new.exists() {
        remove_with_sidecars(&paths.new);
        tracing::info!("history reclaim: deleted an interrupted rewrite");
        return Recovered::DroppedNew;
    }
    Recovered::Nothing
}

/// Hold the gate shared while opening the store, so an srv that starts during
/// another's reclaim waits instead of opening the file mid-swap. `None` when
/// the gate can't be taken in [`GATE_WAIT`] or the file can't be made: the
/// open goes ahead (a running reclaim then sees this srv and gives up).
pub fn gate_shared(db: &Path) -> Option<File> {
    let file = OpenOptions::new().create(true).truncate(false).write(true).open(Paths::for_db(db).gate).ok()?;
    let deadline = Instant::now() + GATE_WAIT;
    loop {
        match file.try_lock_shared() {
            Ok(()) => return Some(file),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(250)),
            Err(_) => {
                tracing::warn!("history reclaim: the store's gate stayed locked; opening anyway");
                return None;
            }
        }
    }
}

/// Free bytes on the disk that holds `path`, as sysinfo sees it.
fn free_disk_bytes(path: &Path) -> Option<u64> {
    let full = std::fs::canonicalize(path).ok()?;
    let full = full.to_string_lossy().trim_start_matches(r"\\?\").to_ascii_lowercase();
    let disks = sysinfo::Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        .filter(|d| full.starts_with(&d.mount_point().to_string_lossy().to_ascii_lowercase()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space())
}

/// A store's page numbers: `(page_size, page_count, freelist_count, auto_vacuum)`.
pub(crate) fn page_stats(conn: &Connection) -> rusqlite::Result<(i64, i64, i64, i64)> {
    let one = |q: &str| conn.query_row(q, [], |r| r.get::<_, i64>(0));
    Ok((one("PRAGMA page_size")?, one("PRAGMA page_count")?, one("PRAGMA freelist_count")?, one("PRAGMA auto_vacuum")?))
}

/// The contents a rewrite must keep: the schema, the version stamp, and each
/// table's row count and byte total.
#[derive(Debug, PartialEq, Eq)]
struct Contents {
    schema: Vec<String>,
    user_version: i64,
    tables: Vec<(String, i64, i64)>,
}

fn contents(conn: &Connection) -> rusqlite::Result<Contents> {
    let mut stmt = conn.prepare("SELECT type || ' ' || name || ' ' || coalesce(sql, '') FROM sqlite_master ORDER BY type, name")?;
    let schema = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    let user_version = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let exists = |t: &str| conn.query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1", [t], |r| r.get::<_, i64>(0)).map(|n| n > 0);
    let mut tables = Vec::new();
    for (table, bytes) in [("db_wave_file", "size"), ("db_file_data", "length(data)"), ("db_file_frame", "length(data)")] {
        if exists(table)? {
            let (n, b) = conn.query_row(&format!("SELECT count(*), coalesce(sum({bytes}), 0) FROM {table}"), [], |r| Ok((r.get(0)?, r.get(1)?)))?;
            tables.push((table.to_string(), n, b));
        }
    }
    Ok(Contents { schema, user_version, tables })
}

/// What [`run_at_start`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    NotRequested,
    /// Not now: another srv runs, the gate is held, or a failure retries tomorrow.
    Waiting(String),
    /// The store already gives space back by itself; the request was dropped.
    NotNeeded,
    Failed(String),
    Done(Last),
}

/// Reclaim if it was asked for and this srv is alone. Call before the store
/// opens. `others` names another running srv (`framer::other_srv`); it is
/// asked once at the start and again right before the swap.
pub fn run_at_start(db: &Path, others: impl Fn() -> Option<String>, now_ms: u64) -> Outcome {
    let paths = Paths::for_db(db);
    recover(&paths);
    let Some(mut marker) = read_json::<Marker>(&paths.marker) else {
        return Outcome::NotRequested;
    };
    if !paths.db.exists() {
        cancel(db).ok();
        return Outcome::NotRequested;
    }
    if marker.failures > 0 && now_ms.saturating_sub(marker.last_try_ms) < RETRY_EVERY_MS {
        return Outcome::Waiting("the last try failed; it's tried again a day later".to_string());
    }
    let wait = |marker: &mut Marker, why: String| {
        marker.waiting_for = Some(why.clone());
        write_json(&paths.marker, marker).ok();
        tracing::info!(why = %why, "history reclaim: waiting");
        Outcome::Waiting(why)
    };
    let Ok(gate) = OpenOptions::new().create(true).truncate(false).write(true).open(&paths.gate) else {
        return wait(&mut marker, "the store's gate file can't be opened".to_string());
    };
    if gate.try_lock().is_err() {
        return wait(&mut marker, "another AgentMux is opening the store".to_string());
    }
    if let Some(who) = others() {
        return wait(&mut marker, who);
    }
    marker.last_try_ms = now_ms;
    let started = Instant::now();
    match rewrite_and_swap(&paths, &others) {
        Ok(Some((before_bytes, after_bytes))) => {
            let last = Last { at_ms: now_ms, before_bytes, after_bytes, took_ms: started.elapsed().as_millis() as u64, error: None };
            write_json(&paths.last, &last).ok();
            cancel(db).ok();
            tracing::info!(before_bytes, after_bytes, took_ms = last.took_ms, "history store: reclaimed {} bytes", before_bytes.saturating_sub(after_bytes));
            Outcome::Done(last)
        }
        Ok(None) => {
            cancel(db).ok();
            Outcome::NotNeeded
        }
        Err(Step::Wait(why)) => wait(&mut marker, why),
        Err(Step::Fail(e)) => {
            remove_with_sidecars(&paths.new);
            marker.failures += 1;
            marker.last_error = Some(e.clone());
            marker.waiting_for = None;
            tracing::warn!(error = %e, failures = marker.failures, "history reclaim: failed; the original store is unchanged");
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

impl crate::boot_timing::StepOutcome for Outcome {
    fn succeeded(&self) -> bool {
        !matches!(self, Outcome::Failed(_))
    }
}

enum Step {
    Wait(String),
    Fail(String),
}

fn fail<E: std::fmt::Display>(what: &str) -> impl FnOnce(E) -> Step + '_ {
    move |e| Step::Fail(format!("{what}: {e}"))
}

/// The rewrite (spec §4.2 steps 1–6). `Ok(None)`: nothing to do, the store is
/// already incremental. `Ok(Some((before, after)))`: swapped in and opened.
fn rewrite_and_swap(paths: &Paths, others: &impl Fn() -> Option<String>) -> Result<Option<(u64, u64)>, Step> {
    let before_bytes = std::fs::metadata(&paths.db).map(|m| m.len()).unwrap_or(0);
    let src = Connection::open(&paths.db).map_err(fail("opening the store"))?;
    src.execute_batch("PRAGMA busy_timeout=5000;").map_err(fail("busy_timeout"))?;
    let (page_size, page_count, free, auto_vacuum) = page_stats(&src).map_err(fail("reading its pages"))?;
    if auto_vacuum == 2 {
        return Ok(None);
    }
    let live = ((page_count - free).max(0) * page_size) as u64;
    if let Some(avail) = free_disk_bytes(&paths.db) {
        let need = live + live / 5 + DISK_SLACK_BYTES;
        if avail < need {
            return Err(Step::Wait(format!("{} GB of free disk is needed, {} GB is free", need.div_ceil(1 << 30), avail / (1 << 30))));
        }
    }
    let busy: i64 = src.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0)).map_err(fail("checkpoint"))?;
    if busy != 0 {
        return Err(Step::Wait("the store is busy (another connection holds it)".to_string()));
    }
    let want = contents(&src).map_err(fail("reading the store's contents"))?;
    remove_with_sidecars(&paths.new);
    src.execute_batch("PRAGMA auto_vacuum=INCREMENTAL;").map_err(fail("auto_vacuum"))?;
    src.execute("VACUUM INTO ?1", [paths.new.to_string_lossy().as_ref()]).map_err(fail("VACUUM INTO"))?;
    {
        let new = Connection::open(&paths.new).map_err(fail("opening the rewritten store"))?;
        let check: String = new.query_row("PRAGMA quick_check", [], |r| r.get(0)).map_err(fail("quick_check"))?;
        if check != "ok" {
            return Err(Step::Fail(format!("the rewritten store failed quick_check: {check}")));
        }
        let (_, _, _, av) = page_stats(&new).map_err(fail("reading the rewritten store"))?;
        if av != 2 {
            return Err(Step::Fail(format!("the rewritten store has auto_vacuum={av}, not 2")));
        }
        let got = contents(&new).map_err(fail("reading the rewritten store's contents"))?;
        if got != want {
            return Err(Step::Fail("the rewritten store's contents differ from the original".to_string()));
        }
    }
    if let Some(who) = others() {
        remove_with_sidecars(&paths.new);
        return Err(Step::Wait(who));
    }
    drop(src);
    rename_with_sidecars(&paths.db, &paths.pre).map_err(fail("moving the original aside"))?;
    if let Err(e) = std::fs::rename(&paths.new, &paths.db) {
        let _ = rename_with_sidecars(&paths.pre, &paths.db);
        return Err(Step::Fail(format!("moving the rewritten store in: {e}")));
    }
    if !opens(&paths.db) {
        remove_with_sidecars(&paths.db);
        let _ = rename_with_sidecars(&paths.pre, &paths.db);
        return Err(Step::Fail("the rewritten store didn't open; the original is back".to_string()));
    }
    remove_with_sidecars(&paths.pre);
    let after_bytes = std::fs::metadata(&paths.db).map(|m| m.len()).unwrap_or(0);
    Ok(Some((before_bytes, after_bytes)))
}

impl FileStore {
    /// The store's page numbers: `(page_size, page_count, freelist_count, auto_vacuum)`.
    pub fn page_stats(&self) -> Result<(i64, i64, i64, i64), StoreError> {
        Ok(page_stats(&self.conn.lock().unwrap())?)
    }

    /// Give freed pages back to the disk in short steps (spec §5): only in an
    /// `auto_vacuum=INCREMENTAL` store with at least
    /// [`INCREMENTAL_MIN_FREE_PAGES`] free, [`INCREMENTAL_STEP_PAGES`] a step,
    /// each its own write, up to [`INCREMENTAL_PASS_PAGES`]. The file shrinks
    /// at the next checkpoint. Returns the pages given back.
    pub fn reclaim_free_pages(&self) -> Result<i64, StoreError> {
        let (_, _, free, auto_vacuum) = self.page_stats()?;
        if auto_vacuum != 2 || free < INCREMENTAL_MIN_FREE_PAGES {
            return Ok(0);
        }
        let mut left = free;
        let mut steps = 0;
        while left > 0 && steps * INCREMENTAL_STEP_PAGES < INCREMENTAL_PASS_PAGES {
            if steps > 0 {
                std::thread::sleep(Duration::from_millis(20));
            }
            // One step under the lock, then let this srv's own writes in.
            let conn = self.conn.lock().unwrap();
            // The pragma frees one page per step of its statement: step it to the end.
            let mut stmt = conn.prepare(&format!("PRAGMA incremental_vacuum({INCREMENTAL_STEP_PAGES})"))?;
            let mut rows = stmt.query([])?;
            while rows.next()?.is_some() {}
            drop(rows);
            drop(stmt);
            left = conn.query_row("PRAGMA freelist_count", [], |r| r.get::<_, i64>(0))?;
            steps += 1;
        }
        Ok(free - left)
    }
}
