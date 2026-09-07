// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Migration runner — invoked by `agentmux-srv migrate`.
//!
//! Progress events are emitted as newline-delimited JSON to stdout so the
//! launcher's splash screen can show "Updating your data…" while migrations
//! run.  Failures are written to `<home>/logs/migration-error.log`; the
//! process exits non-zero so the launcher can surface the error rather than
//! booting with a half-migrated data dir.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::backend::storage::store::Store;
use crate::registry::{resolve_global_shared_root, resolve_shared_store_path};

use super::{MigrationContext, MigrationScope, REGISTRY};

// ── Progress events (newline-delimited JSON → launcher reads these) ───────────

fn emit(event: &str, id: &str, extra: &str) {
    if extra.is_empty() {
        println!("{{\"event\":\"{}\",\"id\":\"{}\"}}", event, id);
    } else {
        println!("{{\"event\":\"{}\",\"id\":\"{}\",{}}}", event, id, extra);
    }
}

fn emit_summary(applied: usize, skipped: usize) {
    println!(
        "{{\"event\":\"complete\",\"applied\":{},\"skipped\":{}}}",
        applied, skipped
    );
}

/// The true, unconditional `~/.agentmux` root — `MigrationContext.home`.
///
/// Deliberately NOT derived from `resolve_shared_store_path()` (which
/// varies under isolated-auth mode, resolving to a channel-scoped path
/// instead of the global one) — 17 of 19 registered migrations build
/// paths directly off `ctx.home` (registry/definitions/transcripts dirs),
/// and `backup_stores`/`write_error_log` below use it too. All of those
/// must stay anchored to the real global root regardless of isolation;
/// only `ctx.shared_store_path` itself is meant to vary. A single shared
/// helper (not one inline derivation per call site) so the two can't
/// drift apart. See `registry::resolve_global_shared_root`'s doc comment
/// and `docs/specs/SPEC_ISOLATED_AUTH_DEV_TESTING_2026_07_27.md`.
fn resolve_home() -> Option<PathBuf> {
    resolve_global_shared_root().and_then(|p| p.parent().map(Path::to_path_buf))
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Exit code of `migrate --verify` when at least one applied migration's
/// post-condition is a mismatch or its check errored. Distinct from 1 (a
/// migration RUN failed) so a caller can tell "your data is inconsistent"
/// from "the migrate command itself broke", and from 2, which clap uses for
/// a CLI usage error (a typoed flag must not read as a data finding —
/// reagent P2 on #3058). See docs/exe-return-codes.md.
pub const VERIFY_FAILED_EXIT_CODE: i32 = 3;

/// Called from `main.rs` when the `migrate` subcommand is active.
/// Returns the exit code (0 = success, 1 = failure, 3 = `--verify` found a
/// mismatch — see `VERIFY_FAILED_EXIT_CODE`).
pub fn run_migrate_command(data_dir: &Path, dry_run: bool, list: bool, verify: bool) -> i32 {
    // Unresolvable shared store path means we're in CI or an unusual env that
    // has no AGENTMUX_SHARED_DIR. Mirror the daemon's behaviour: treat as a
    // no-op and exit 0 so the launcher proceeds normally.
    let shared_store_path = match resolve_shared_store_path() {
        Some(p) => p,
        None => {
            emit_summary(0, 0);
            return 0;
        }
    };

    let home = match resolve_home() {
        Some(p) => p,
        None => {
            emit_summary(0, 0);
            return 0;
        }
    };

    // `--verify` is read-only by contract, so it dispatches BEFORE the store
    // opens below: `Store::open*` run schema setup, WAL configuration and
    // legacy-table adoption, and create the database file when it is missing
    // (codex P2 on #3058). A doctor must not do any of that — least of all
    // turn a typoed data dir into a fresh empty store that then "verifies".
    if verify {
        return cmd_verify(data_dir, &home, &shared_store_path);
    }

    // Ensure the shared/ directory exists — on a fresh install it has not been
    // created yet (the daemon does create_dir_all at startup, but migrate runs
    // before the daemon). SQLite cannot create parent directories itself.
    if let Some(parent) = shared_store_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("migration: failed to create shared dir {}: {}", parent.display(), e);
            return 1;
        }
    }

    // `--list` and `--dry-run` are read-only previews and deliberately do NOT
    // take the migration lock: an operator uses them precisely while another
    // process may be mid-migration, and a 30-minute wait would defeat that
    // (reagent P1 on #3062). They open the stores the ordinary way (schema
    // setup and all — the same thing the daemon does on every boot).
    if list || dry_run {
        let (shared_store, channel_store) = match open_stores_for_preview(&shared_store_path, data_dir) {
            Ok(pair) => pair,
            Err(msg) => {
                eprintln!("migration: {}", msg);
                return 1;
            }
        };
        if list {
            return cmd_list(&shared_store, channel_store.as_ref());
        }
        let pending = pending_of(REGISTRY, &shared_store, channel_store.as_ref());
        if pending.is_empty() {
            emit_summary(0, REGISTRY.len());
        }
        for m in &pending {
            println!("pending: {} — {}", m.id(), m.description());
        }
        return 0;
    }

    // Apply: the SAME locked batch the daemon runs (`apply_pending`), so the
    // CLI can never race a booting srv — and the lock is taken before the
    // stores are opened, so a daemon holding a write transaction longer than
    // `Store::open*`'s own 5 s busy timeout waits on the lock instead of
    // failing with a spurious "database is locked" (reagent P1 on #3062).
    // The only CLI-specific parts are the NDJSON progress lines the
    // launcher's `run_migrate` reader expects and the error log file.
    let progress = ApplyProgress {
        on_start: &|id, description| {
            emit("migration_start", id, &format!("\"description\":\"{}\"", description));
        },
        on_done: &|id, ms| emit("migration_done", id, &format!("\"duration_ms\":{}", ms)),
    };
    match apply_pending(REGISTRY, &home, &shared_store_path, data_dir, Some(&progress)) {
        Ok(outcome) => {
            emit_summary(outcome.applied, outcome.skipped);
            0
        }
        Err(msg) => {
            write_error_log(&home, &msg);
            eprintln!("{}", msg);
            1
        }
    }
}

/// Return the store that tracks applied state for a migration of the given scope.
/// Global migrations → shared store; Channel migrations → channel store.
fn tracking_store<'a>(
    scope: super::MigrationScope,
    shared: &'a Store,
    channel: Option<&'a Store>,
) -> Option<&'a Store> {
    match scope {
        super::MigrationScope::Global => Some(shared),
        super::MigrationScope::Channel => channel,
    }
}

fn cmd_list(shared_store: &Store, channel_store: Option<&Store>) -> i32 {
    let shared_applied = shared_store.migrations_list_applied().unwrap_or_default();
    let channel_applied = channel_store
        .and_then(|s| s.migrations_list_applied().ok())
        .unwrap_or_default();
    for m in REGISTRY.iter() {
        let applied_ids = match m.scope() {
            super::MigrationScope::Global => &shared_applied,
            super::MigrationScope::Channel => &channel_applied,
        };
        let status = if applied_ids.contains(&m.id().to_string()) { "applied" } else { "pending" };
        println!("{} [{}] [{}] — {}", m.id(), m.scope().as_str(), status, m.description());
    }
    0
}

// ── Backup ────────────────────────────────────────────────────────────────────

fn backup_stores(home: &Path, shared_store_path: &Path, data_dir: &Path) -> std::io::Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup_dir = home
        .join("shared")
        .join("backups")
        .join(format!("pre-migration-{}-{}", version, ts));
    std::fs::create_dir_all(&backup_dir)?;

    // shared store.db
    if shared_store_path.exists() {
        std::fs::copy(shared_store_path, backup_dir.join("store.db"))?;
    }

    // channel objects.db
    let objects_db = data_dir.join("db").join("objects.db");
    if objects_db.exists() {
        std::fs::copy(&objects_db, backup_dir.join("objects.db"))?;
    }

    prune_old_backups(home);
    Ok(())
}

fn prune_old_backups(home: &Path) {
    let backups_dir = home.join("shared").join("backups");
    let cutoff = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .saturating_sub(30 * 24 * 60 * 60); // 30 days

    let Ok(entries) = std::fs::read_dir(&backups_dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue; }
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        // parse timestamp from "pre-migration-<version>-<ts>"
        if let Some(ts_str) = name.rsplit('-').next() {
            if let Ok(ts) = ts_str.parse::<u64>() {
                if ts < cutoff {
                    let _ = std::fs::remove_dir_all(&path);
                }
            }
        }
    }
}

// ── In-process migration runner (used by srv startup) ────────────────────────

/// Apply all pending migrations in-process before srv opens its stores.
///
/// Called unconditionally at srv startup so the shared store is fully backfilled
/// before `id_store` binds to it. This prevents apparent data loss (empty shared
/// store) on first boot after an upgrade. Fast-path: returns `Ok(0)` immediately
/// when all migrations are already applied.
///
/// Unlike `run_migrate_command` this writes progress via `tracing` rather than
/// stdout JSON and is meant to be called from within the daemon process rather
/// than a subprocess. Returns the number of migrations applied. On error, returns
/// `Err` — callers should fall back to the per-channel store rather than binding
/// `id_store` to an un-backfilled shared store.
///
/// `data_dir` must be the wave data dir (parent of `db/`), not the db dir.
pub fn run_pending_migrations(data_dir: &Path) -> Result<usize, String> {
    let shared_store_path = match resolve_shared_store_path() {
        Some(p) => p,
        None => {
            tracing::info!("run_pending_migrations: shared store path unresolvable — nothing to do");
            return Ok(0);
        }
    };

    let home = match resolve_home() {
        Some(p) => p,
        None => return Err("run_pending_migrations: cannot resolve global shared root".to_string()),
    };

    apply_pending(REGISTRY, &home, &shared_store_path, data_dir, None).map(|o| o.applied)
}

// ── Cross-process migration lock (Phase 3, F6) ────────────────────────────────
//
// Two `agentmux-srv` processes against the same data dir (a crash-restart
// race, or a misconfigured channel) could both see a migration as pending
// and both run `up()`: SQLite's own locking serializes individual statements,
// not the runner's check → run → mark-applied sequence (classic TOCTOU).
//
// The lock is a dedicated one-table SQLite file next to the shared store
// with `BEGIN IMMEDIATE` held for the whole batch. That IS a cross-process
// advisory lock — SQLite takes an OS-level file lock — with three properties
// a hand-rolled lockfile does not get for free: it blocks (busy_timeout)
// instead of failing, it is released by the OS when the holder dies (no
// stale-lockfile cleanup), and it is portable to Windows. It is NOT taken on
// the real stores because migrations open their own connections to those
// (m0007 opens the channel store itself); a transaction held there would
// deadlock them.

/// File name of the lock database, created beside the shared store.
pub(super) const MIGRATION_LOCK_FILE: &str = "migrations.lock.db";

/// How long a second runner waits for the first to finish before giving up.
/// Matches the launcher's extended ESTART deadline for a long migration
/// batch (`srv_spawner.rs` / `sidecar.rs`: 30 minutes).
const MIGRATION_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// Held for the duration of a migration batch. Dropping it releases the lock.
pub(super) struct MigrationLock {
    _conn: rusqlite::Connection,
}

impl MigrationLock {
    /// Acquire the batch lock at `path`, waiting up to `wait` for another
    /// holder. `Err` only if the lock database cannot be opened or the wait
    /// expires — both fatal for a migration run, by design.
    pub(super) fn acquire(path: &Path, wait: std::time::Duration) -> Result<Self, String> {
        let conn = rusqlite::Connection::open(path)
            .map_err(|e| format!("open migration lock {}: {}", path.display(), e))?;
        conn.busy_timeout(wait)
            .map_err(|e| format!("migration lock busy_timeout: {}", e))?;
        // The table exists only so the file is a well-formed database on
        // first use; the lock is the RESERVED lock BEGIN IMMEDIATE takes.
        conn.execute_batch("CREATE TABLE IF NOT EXISTS lock (id INTEGER PRIMARY KEY); BEGIN IMMEDIATE;")
            .map_err(|e| format!("acquire migration lock {} (another runner may still hold it): {}", path.display(), e))?;
        Ok(Self { _conn: conn })
    }
}

/// Open the shared + channel stores the way every writer does (creating the
/// channel dir/db on a fresh install — see the comment inside). Shared by the
/// locked batch and the lock-free `--list`/`--dry-run` previews.
fn open_stores_for_preview(shared_store_path: &Path, data_dir: &Path) -> Result<(Store, Option<Store>), String> {
    let shared_store = Store::open_shared(shared_store_path)
        .map_err(|e| format!("open shared store: {}", e))?;
    let channel_store_path = data_dir.join("db").join("objects.db");
    if let Some(parent) = channel_store_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create channel db dir: {}", e))?;
    }
    // Channel store (objects.db) tracks channel-scoped migrations independently
    // per channel — MigrationScope::Channel migrations record here, not in shared.
    // Always open/create it so that seed migrations (e.g. m0008_default_bundle)
    // run on fresh install. Skipping it on a fresh install once left the
    // Channel-scoped migrations (0002/0003/0007) out of the pending list, so
    // they were never marked applied; when wstore then created objects.db,
    // count_pending_migrations at ESTART reported them pending and fired the
    // "Migration failed" UI warning on every fresh first launch.
    // Data-transformation migrations guard themselves with
    // `if !ctx.channel_store_path.exists()` and are no-ops on an empty db.
    let channel_store = Store::open(&channel_store_path)
        .map(Some)
        .map_err(|e| format!("open channel store: {}", e))?;
    Ok((shared_store, channel_store))
}

/// The migrations in `regs` whose tracking store does not record them applied.
fn pending_of<'r>(
    regs: &'r [&'r (dyn super::Migration + Sync)],
    shared: &Store,
    channel: Option<&Store>,
) -> Vec<&'r (dyn super::Migration + Sync)> {
    regs.iter()
        .copied()
        .filter(|m| {
            tracking_store(m.scope(), shared, channel).map_or(false, |s| !s.migration_is_applied(m.id()))
        })
        .collect()
}

/// Per-migration progress hooks for the CLI's NDJSON lines. The daemon passes
/// `None` and relies on the `tracing` lines `apply_pending` always emits.
pub(super) struct ApplyProgress<'a> {
    pub on_start: &'a dyn Fn(&str, &str),
    pub on_done: &'a dyn Fn(&str, u64),
}

/// What a batch did. `skipped` = already applied at the time of the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ApplyOutcome {
    pub applied: usize,
    pub skipped: usize,
}

/// The locked migration batch — the ONE apply path, used by the daemon
/// (`run_pending_migrations`) and by `agentmux-srv migrate`. Generic over the
/// registry so the concurrency contract can be tested with a fake migration.
///
/// Order matters and is the point of Phase 3: the cross-process lock is
/// taken FIRST, before the stores are even opened, so that check → run →
/// mark is one critical section and a second runner blocks on the lock
/// (30 min) rather than tripping `Store::open*`'s 5 s busy timeout against
/// a migrating peer.
pub(super) fn apply_pending(
    regs: &[&(dyn super::Migration + Sync)],
    home: &Path,
    shared_store_path: &Path,
    data_dir: &Path,
    progress: Option<&ApplyProgress<'_>>,
) -> Result<ApplyOutcome, String> {
    if let Some(parent) = shared_store_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Err(format!("run_pending_migrations: create shared dir: {}", e));
        }
    }

    // Two locks, one beside each store this batch can write, always in the
    // same order (shared, then channel) so two runners can never deadlock
    // on each other. One lock beside the shared store is not enough: two
    // processes can share a `data_dir` (hence the same `objects.db`) while
    // resolving DIFFERENT shared stores — isolated-auth instances with
    // different `AGENTMUX_INSTANCE_DIR`s do exactly that — and would then
    // hold different shared-side locks while racing on the same
    // channel-scoped migrations (codex P1 on #3062).
    let shared_lock_path = shared_store_path
        .parent()
        .map(|p| p.join(MIGRATION_LOCK_FILE))
        .unwrap_or_else(|| PathBuf::from(MIGRATION_LOCK_FILE));
    let _shared_lock = MigrationLock::acquire(&shared_lock_path, MIGRATION_LOCK_WAIT)
        .map_err(|e| format!("run_pending_migrations: {}", e))?;
    let channel_db_dir = data_dir.join("db");
    if let Err(e) = std::fs::create_dir_all(&channel_db_dir) {
        return Err(format!("run_pending_migrations: create channel db dir: {}", e));
    }
    let _channel_lock = MigrationLock::acquire(&channel_db_dir.join(MIGRATION_LOCK_FILE), MIGRATION_LOCK_WAIT)
        .map_err(|e| format!("run_pending_migrations: {}", e))?;

    let (shared_store, channel_store) = open_stores_for_preview(shared_store_path, data_dir)
        .map_err(|e| format!("run_pending_migrations: {}", e))?;
    let channel_store_path = channel_db_dir.join("objects.db");

    let pending = pending_of(regs, &shared_store, channel_store.as_ref());
    if pending.is_empty() {
        return Ok(ApplyOutcome { applied: 0, skipped: regs.len() });
    }
    let skipped = regs.len() - pending.len();

    let ctx = super::MigrationContext {
        home: home.to_path_buf(),
        data_dir: data_dir.to_path_buf(),
        shared_store_path: shared_store_path.to_path_buf(),
        channel_store_path: channel_store_path.clone(),
    };

    if let Err(e) = backup_stores(home, shared_store_path, data_dir) {
        return Err(format!("run_pending_migrations: backup failed: {}", e));
    }

    let mut applied = 0;
    for m in &pending {
        let t = Instant::now();
        tracing::info!(id = m.id(), description = m.description(), "run_pending_migrations: applying");
        if let Some(p) = progress {
            (p.on_start)(m.id(), m.description());
        }
        match m.up(&ctx) {
            Ok(()) => {
                let ms = t.elapsed().as_millis() as u64;
                let scope = m.scope().as_str();
                let tracking = tracking_store(m.scope(), &shared_store, channel_store.as_ref())
                    .ok_or_else(|| format!("run_pending_migrations: no tracking store for {} (channel store missing)", m.id()))?;
                if let Err(e) = tracking.migration_mark_applied(m.id(), scope, ms) {
                    return Err(format!("run_pending_migrations: failed to record {} as applied: {}", m.id(), e));
                }
                tracing::info!(id = m.id(), duration_ms = ms, "run_pending_migrations: applied");
                if let Some(p) = progress {
                    (p.on_done)(m.id(), ms);
                }
                applied += 1;
            }
            Err(e) => {
                return Err(format!("run_pending_migrations: migration {} failed: {}", m.id(), e));
            }
        }
    }

    Ok(ApplyOutcome { applied, skipped })
}

// ── Pending count (used by srv startup before migration and for ESTART) ──────

/// Return the number of REGISTRY migrations that have not yet been applied.
/// Opens stores read-write (SQLite does not have a read-only open for WAL mode);
/// this may create `objects.db` if it does not exist. Returns 0 on any error so
/// startup is never blocked. `data_dir` must be the wave data dir (parent of
/// `db/`) not the db dir itself.
pub fn count_pending_migrations(data_dir: &Path) -> usize {
    let shared_store_path = match resolve_shared_store_path() {
        Some(p) => p,
        None => {
            tracing::warn!("count_pending_migrations: could not resolve shared store path — reporting 0");
            return 0;
        }
    };
    let shared_store = match Store::open_shared(&shared_store_path) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("count_pending_migrations: failed to open shared store at {}: {} — reporting 0", shared_store_path.display(), e);
            return 0;
        }
    };
    let channel_store_path = data_dir.join("db").join("objects.db");
    let channel_store = match Store::open(&channel_store_path) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!("count_pending_migrations: failed to open channel store at {}: {} — channel-scoped migrations will not be counted", channel_store_path.display(), e);
            None
        }
    };
    REGISTRY
        .iter()
        .filter(|m| {
            let tracking = tracking_store(m.scope(), &shared_store, channel_store.as_ref());
            tracking.map_or(false, |s| !s.migration_is_applied(m.id()))
        })
        .count()
}

// ── Error log ─────────────────────────────────────────────────────────────────

fn write_error_log(home: &Path, msg: &str) {
    let log_dir = home.join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let path = log_dir.join("migration-error.log");
    let content = format!("{}\n", msg);
    let _ = std::fs::write(path, content);
}

// ── Verify (doctor pass) ──────────────────────────────────────────────────────
//
// SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03 Phase 1, second bullet: "applied"
// in `db_migrations` means `up()` returned Ok — nothing re-checks that it
// wrote anything. `migrate --verify` asks each APPLIED migration for its own
// post-condition (`Migration::verify`) and reports per migration. Unapplied
// migrations are skipped: there is nothing to verify yet, and `--list`
// already shows them.
//
// Read-only by contract (codex P2 on #3058): nothing on this path calls
// `Store::open*`. Those run schema setup, WAL configuration and legacy-table
// adoption, and create the file when missing. This path opens plain SQLite
// connections with SQLITE_OPEN_READ_ONLY, so a typoed data dir yields "no
// store here", never a freshly created empty database that then "verifies".

/// One line of the verify report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyRecord {
    pub id: &'static str,
    pub scope: &'static str,
    pub outcome: super::VerifyOutcome,
}

/// Which migration ids each tracking store records as applied, read through
/// a FALLIBLE path. `Err` means the store exists but its tracking state
/// could not be read (not a database, malformed table, locked): every
/// migration of that scope then reports `VerifyOutcome::Error` instead of
/// being silently skipped as "unapplied" and letting the pass exit 0 —
/// codex P1 on #3058, which `Store::migration_is_applied`'s
/// `unwrap_or(false)` would have caused. A store file that does not exist,
/// or one predating the `db_migrations` table, has nothing recorded and is
/// `Ok(empty)`.
pub struct AppliedIds {
    pub global: Result<HashSet<String>, String>,
    pub channel: Result<HashSet<String>, String>,
}

impl AppliedIds {
    fn for_scope(&self, scope: MigrationScope) -> &Result<HashSet<String>, String> {
        match scope {
            MigrationScope::Global => &self.global,
            MigrationScope::Channel => &self.channel,
        }
    }
}

/// Open an existing SQLite file read-only. `Ok(None)` when the file is
/// absent — never creates one (the whole point of this path).
pub(super) fn open_readonly(path: &Path) -> Result<Option<rusqlite::Connection>, String> {
    if !path.exists() {
        return Ok(None);
    }
    rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map(Some)
        .map_err(|e| format!("open {} read-only: {}", path.display(), e))
}

/// `SELECT id FROM db_migrations` through a read-only connection. A missing
/// file or a missing table is `Ok(empty)` — nothing was ever recorded there.
/// Any other failure is `Err`, which `verify_applied` turns into an
/// `Error` outcome for every migration of that scope.
pub(super) fn read_applied_ids(path: &Path) -> Result<HashSet<String>, String> {
    let Some(conn) = open_readonly(path)? else {
        return Ok(HashSet::new());
    };
    let mut stmt = match conn.prepare("SELECT id FROM db_migrations") {
        Ok(s) => s,
        Err(rusqlite::Error::SqliteFailure(_, Some(msg))) if msg.contains("no such table") => {
            return Ok(HashSet::new());
        }
        Err(e) => return Err(format!("read db_migrations in {}: {}", path.display(), e)),
    };
    stmt.query_map([], |row| row.get::<_, String>(0))
        .and_then(|rows| rows.collect::<Result<HashSet<String>, _>>())
        .map_err(|e| format!("read db_migrations in {}: {}", path.display(), e))
}

/// Run `verify()` for every migration in `regs` whose tracking store records
/// it as applied. Generic over the registry slice so the harness can be
/// tested with fakes; production passes `REGISTRY`.
pub fn verify_applied(
    regs: &[&(dyn super::Migration + Sync)],
    ctx: &MigrationContext,
    applied: &AppliedIds,
) -> Vec<VerifyRecord> {
    let mut out = Vec::new();
    for m in regs {
        let outcome = match applied.for_scope(m.scope()) {
            Err(e) => super::VerifyOutcome::Error(format!("cannot read applied state: {}", e)),
            Ok(ids) if ids.contains(m.id()) => m.verify(ctx),
            Ok(_) => continue,
        };
        out.push(VerifyRecord { id: m.id(), scope: m.scope().as_str(), outcome });
    }
    out
}

/// Exit code for a verify report: 0 unless any record is a mismatch or an
/// error. `NotVerifiable` is not a failure — it is the honest default.
pub fn verify_exit_code(records: &[VerifyRecord]) -> i32 {
    if records.iter().any(|r| r.outcome.is_failure()) {
        VERIFY_FAILED_EXIT_CODE
    } else {
        0
    }
}

/// One `migration_verify` NDJSON line. `detail` goes through serde_json so
/// control characters (a multi-line SQLite error, say) cannot break the
/// line-delimited stream a consumer tails (reagent P2 on #3058).
fn verify_event_line(r: &VerifyRecord) -> String {
    let detail = serde_json::to_string(r.outcome.detail()).unwrap_or_else(|_| "\"\"".to_string());
    format!(
        "{{\"event\":\"migration_verify\",\"id\":\"{}\",\"scope\":\"{}\",\"status\":\"{}\",\"detail\":{}}}",
        r.id,
        r.scope,
        r.outcome.label(),
        detail
    )
}

fn cmd_verify(data_dir: &Path, home: &Path, shared_store_path: &Path) -> i32 {
    let channel_store_path = data_dir.join("db").join("objects.db");
    let ctx = MigrationContext {
        home: home.to_path_buf(),
        data_dir: data_dir.to_path_buf(),
        shared_store_path: shared_store_path.to_path_buf(),
        channel_store_path: channel_store_path.clone(),
    };
    let applied = AppliedIds {
        global: read_applied_ids(shared_store_path),
        channel: read_applied_ids(&channel_store_path),
    };
    let records = verify_applied(REGISTRY, &ctx, &applied);
    let mut counts = [0usize; 4]; // ok, mismatch, error, n/a
    for r in &records {
        let detail = r.outcome.detail();
        if detail.is_empty() {
            println!("verify: {} [{}] [{}]", r.id, r.scope, r.outcome.label());
        } else {
            println!("verify: {} [{}] [{}] — {}", r.id, r.scope, r.outcome.label(), detail);
        }
        println!("{}", verify_event_line(r));
        match r.outcome {
            super::VerifyOutcome::Ok(_) => counts[0] += 1,
            super::VerifyOutcome::Mismatch(_) => counts[1] += 1,
            super::VerifyOutcome::Error(_) => counts[2] += 1,
            super::VerifyOutcome::NotVerifiable => counts[3] += 1,
        }
    }
    println!(
        "verify: {} applied checked — {} ok, {} mismatch, {} error, {} not verifiable (data dir {})",
        records.len(),
        counts[0],
        counts[1],
        counts[2],
        counts[3],
        data_dir.display()
    );
    println!(
        "{{\"event\":\"verify_complete\",\"checked\":{},\"ok\":{},\"mismatch\":{},\"error\":{},\"not_verifiable\":{}}}",
        records.len(),
        counts[0],
        counts[1],
        counts[2],
        counts[3]
    );
    verify_exit_code(&records)
}

/// `SELECT COUNT(*) FROM <table>` on a (read-only) connection, treating a
/// missing table as 0 rows — a store from before the table existed has
/// nothing in it, which is exactly the answer a post-condition wants. Any
/// other SQLite error is surfaced. `table` must be a literal identifier from
/// the calling migration, never user input (it is interpolated, not bound).
pub(super) fn table_count(conn: &rusqlite::Connection, table: &str) -> Result<i64, String> {
    match conn.query_row(&format!("SELECT COUNT(*) FROM {}", table), [], |row| row.get::<_, i64>(0)) {
        Ok(n) => Ok(n),
        Err(rusqlite::Error::SqliteFailure(_, Some(msg))) if msg.contains("no such table") => Ok(0),
        Err(e) => Err(format!("count {}: {}", table, e)),
    }
}

#[cfg(test)]
mod lock_tests {
    use super::*;
    use crate::migrations::{Migration, MigrationError};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn lock_is_exclusive_across_connections_and_released_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(MIGRATION_LOCK_FILE);
        let first = MigrationLock::acquire(&path, std::time::Duration::from_millis(100)).expect("first holder");
        // A second acquirer waits out its budget and fails while the first holds.
        let err = MigrationLock::acquire(&path, std::time::Duration::from_millis(200))
            .err()
            .expect("second acquire must fail while the first is held");
        assert!(err.contains("another runner may still hold it"), "{}", err);
        drop(first);
        MigrationLock::acquire(&path, std::time::Duration::from_millis(100)).expect("free after drop");
    }

    /// A Global migration that records how many times `up()` ran and is slow
    /// enough that two runners started together both pass the "is it
    /// pending?" check before either can mark it applied — unless the
    /// critical section holds.
    static UP_RUNS: AtomicUsize = AtomicUsize::new(0);
    struct SlowOnce;
    impl Migration for SlowOnce {
        fn id(&self) -> &'static str { "9900_slow_once" }
        fn scope(&self) -> MigrationScope { MigrationScope::Global }
        fn description(&self) -> &'static str { "counts its own runs" }
        fn up(&self, _ctx: &MigrationContext) -> Result<(), MigrationError> {
            UP_RUNS.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(300));
            Ok(())
        }
    }
    static SLOW_ONCE: SlowOnce = SlowOnce;

    /// Channel-scoped twin of `SlowOnce`, for the different-shared-store case.
    static CHANNEL_UP_RUNS: AtomicUsize = AtomicUsize::new(0);
    struct SlowOnceChannel;
    impl Migration for SlowOnceChannel {
        fn id(&self) -> &'static str { "9901_slow_once_channel" }
        fn scope(&self) -> MigrationScope { MigrationScope::Channel }
        fn description(&self) -> &'static str { "counts its own runs (channel scope)" }
        fn up(&self, _ctx: &MigrationContext) -> Result<(), MigrationError> {
            CHANNEL_UP_RUNS.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(300));
            Ok(())
        }
    }
    static SLOW_ONCE_CHANNEL: SlowOnceChannel = SlowOnceChannel;

    #[test]
    fn runners_with_different_shared_stores_but_one_data_dir_still_serialize() {
        // codex P1 on #3062: isolated-auth instances resolve different shared
        // stores yet can share a data dir, i.e. the same objects.db and the
        // same channel-scoped migrations. The shared-side lock alone would let
        // both through; the channel-side lock must catch it.
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_path_buf();
        let data_dir = home.join("data");
        let shared_a = home.join("iso-a").join("shared").join("store.db");
        let shared_b = home.join("iso-b").join("shared").join("store.db");
        CHANNEL_UP_RUNS.store(0, Ordering::SeqCst);

        let results: Vec<Result<ApplyOutcome, String>> = std::thread::scope(|s| {
            let handles: Vec<_> = [shared_a.clone(), shared_b.clone()]
                .into_iter()
                .map(|shared| {
                    let (home, data_dir) = (home.clone(), data_dir.clone());
                    s.spawn(move || {
                        let regs: Vec<&(dyn Migration + Sync)> = vec![&SLOW_ONCE_CHANNEL];
                        apply_pending(&regs, &home, &shared, &data_dir, None)
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().expect("runner thread")).collect()
        });

        for r in &results {
            assert!(r.is_ok(), "both runners must succeed: {:?}", results);
        }
        let applied: usize = results.iter().map(|r| r.as_ref().unwrap().applied).sum();
        assert_eq!(applied, 1, "exactly one runner applied it: {:?}", results);
        assert_eq!(CHANNEL_UP_RUNS.load(Ordering::SeqCst), 1, "channel-scoped up() ran once despite two shared stores");
    }

    #[test]
    fn previews_do_not_wait_on_the_migration_lock() {
        // reagent P1 on #3062: `--list` / `--dry-run` must stay usable while
        // another process is mid-migration. Hold the lock, then take the same
        // read path the previews use and require it to come back at once.
        let dir = tempfile::tempdir().unwrap();
        let shared = dir.path().join("shared").join("store.db");
        std::fs::create_dir_all(shared.parent().unwrap()).unwrap();
        let data_dir = dir.path().join("data");
        let _held = MigrationLock::acquire(&shared.parent().unwrap().join(MIGRATION_LOCK_FILE), std::time::Duration::from_millis(100))
            .expect("hold the lock");

        let before = UP_RUNS.load(Ordering::SeqCst);
        let started = std::time::Instant::now();
        let (shared_store, channel_store) = open_stores_for_preview(&shared, &data_dir).expect("preview opens stores");
        let regs: Vec<&(dyn Migration + Sync)> = vec![&SLOW_ONCE];
        let pending = pending_of(&regs, &shared_store, channel_store.as_ref());
        assert_eq!(pending.len(), 1, "nothing applied yet, so the fake is pending");
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "preview must not block on the lock");
        assert_eq!(UP_RUNS.load(Ordering::SeqCst), before, "preview never runs up()");
    }

    #[test]
    fn two_concurrent_runners_on_one_data_dir_apply_a_migration_exactly_once() {
        // Phase 3 acceptance (SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03):
        // "two concurrent run_pending_migrations calls against the same
        // fresh data dir; assert the migration's up() body executes exactly
        // once." Both runners must also succeed: the loser sees the winner's
        // db_migrations row once it gets the lock and applies nothing.
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_path_buf();
        let shared = home.join("shared").join("store.db");
        let data_dir = home.join("data");
        UP_RUNS.store(0, Ordering::SeqCst);

        let results: Vec<Result<usize, String>> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..2)
                .map(|_| {
                    let (home, shared, data_dir) = (home.clone(), shared.clone(), data_dir.clone());
                    s.spawn(move || {
                        let regs: Vec<&(dyn Migration + Sync)> = vec![&SLOW_ONCE];
                        apply_pending(&regs, &home, &shared, &data_dir, None).map(|o| o.applied)
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().expect("runner thread")).collect()
        });

        for r in &results {
            assert!(r.is_ok(), "both runners must succeed: {:?}", results);
        }
        let applied: usize = results.iter().map(|r| *r.as_ref().unwrap()).sum();
        assert_eq!(applied, 1, "exactly one runner applied it: {:?}", results);
        assert_eq!(UP_RUNS.load(Ordering::SeqCst), 1, "up() body must execute exactly once");

        // And the record is durable: a third run has nothing to do.
        let regs: Vec<&(dyn Migration + Sync)> = vec![&SLOW_ONCE];
        assert_eq!(apply_pending(&regs, &home, &shared, &data_dir, None).map(|o| o.applied).unwrap(), 0);
        assert_eq!(UP_RUNS.load(Ordering::SeqCst), 1);
    }
}

#[cfg(test)]
mod verify_tests {
    use super::*;
    use crate::migrations::{Migration, MigrationError, VerifyOutcome};

    /// A migration whose `verify()` returns a canned outcome. `up()` is never
    /// called by the verify harness.
    struct Fake {
        id: &'static str,
        scope: MigrationScope,
        outcome: VerifyOutcome,
    }
    impl Migration for Fake {
        fn id(&self) -> &'static str { self.id }
        fn scope(&self) -> MigrationScope { self.scope }
        fn description(&self) -> &'static str { "fake" }
        fn up(&self, _ctx: &MigrationContext) -> Result<(), MigrationError> { Ok(()) }
        fn verify(&self, _ctx: &MigrationContext) -> VerifyOutcome { self.outcome.clone() }
    }

    /// Default `verify()` — a migration that never opted in.
    struct Silent;
    impl Migration for Silent {
        fn id(&self) -> &'static str { "9998_silent" }
        fn scope(&self) -> MigrationScope { MigrationScope::Global }
        fn description(&self) -> &'static str { "no verify impl" }
        fn up(&self, _ctx: &MigrationContext) -> Result<(), MigrationError> { Ok(()) }
    }

    fn ctx_in(dir: &tempfile::TempDir) -> MigrationContext {
        MigrationContext {
            home: dir.path().to_path_buf(),
            data_dir: dir.path().join("data"),
            shared_store_path: dir.path().join("shared").join("store.db"),
            channel_store_path: dir.path().join("data").join("db").join("objects.db"),
        }
    }

    fn ids(list: &[&str]) -> Result<HashSet<String>, String> {
        Ok(list.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn only_applied_migrations_are_verified_and_unapplied_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let applied = Fake { id: "9001_applied", scope: MigrationScope::Global, outcome: VerifyOutcome::Ok("fine".into()) };
        let pending = Fake { id: "9002_pending", scope: MigrationScope::Global, outcome: VerifyOutcome::Mismatch("would fail".into()) };
        let state = AppliedIds { global: ids(&["9001_applied"]), channel: ids(&[]) };

        let regs: Vec<&(dyn Migration + Sync)> = vec![&applied, &pending];
        let records = verify_applied(&regs, &ctx_in(&dir), &state);
        assert_eq!(records.len(), 1, "the pending one has nothing to verify yet");
        assert_eq!(records[0].id, "9001_applied");
        assert_eq!(records[0].outcome, VerifyOutcome::Ok("fine".into()));
        assert_eq!(verify_exit_code(&records), 0);
    }

    #[test]
    fn channel_scoped_migrations_consult_the_channel_state_not_the_global_one() {
        let dir = tempfile::tempdir().unwrap();
        let m = Fake { id: "9003_channel", scope: MigrationScope::Channel, outcome: VerifyOutcome::Ok("ok".into()) };
        let regs: Vec<&(dyn Migration + Sync)> = vec![&m];
        // Recorded in the GLOBAL set only: wrong tracking store for a Channel
        // migration, so it must read as unapplied.
        let wrong = AppliedIds { global: ids(&["9003_channel"]), channel: ids(&[]) };
        assert!(verify_applied(&regs, &ctx_in(&dir), &wrong).is_empty());
        let right = AppliedIds { global: ids(&[]), channel: ids(&["9003_channel"]) };
        assert_eq!(verify_applied(&regs, &ctx_in(&dir), &right).len(), 1);
    }

    #[test]
    fn unreadable_tracking_state_is_an_error_for_every_migration_of_that_scope() {
        // codex P1 on #3058: a broken db_migrations must not read as "nothing
        // applied" and let the pass exit 0.
        let dir = tempfile::tempdir().unwrap();
        let g = Fake { id: "9004_global", scope: MigrationScope::Global, outcome: VerifyOutcome::Ok("never reached".into()) };
        let c = Fake { id: "9005_channel", scope: MigrationScope::Channel, outcome: VerifyOutcome::Ok("fine".into()) };
        let regs: Vec<&(dyn Migration + Sync)> = vec![&g, &c];
        let state = AppliedIds { global: Err("file is not a database".into()), channel: ids(&["9005_channel"]) };
        let records = verify_applied(&regs, &ctx_in(&dir), &state);
        assert_eq!(records.len(), 2);
        assert!(matches!(&records[0].outcome, VerifyOutcome::Error(e) if e.contains("not a database")));
        assert_eq!(records[1].outcome, VerifyOutcome::Ok("fine".into()));
        assert_eq!(verify_exit_code(&records), VERIFY_FAILED_EXIT_CODE);
    }

    #[test]
    fn a_migration_without_a_verify_impl_reports_not_verifiable_and_passes() {
        let dir = tempfile::tempdir().unwrap();
        let regs: Vec<&(dyn Migration + Sync)> = vec![&Silent];
        let state = AppliedIds { global: ids(&["9998_silent"]), channel: ids(&[]) };
        let records = verify_applied(&regs, &ctx_in(&dir), &state);
        assert_eq!(records[0].outcome, VerifyOutcome::NotVerifiable);
        assert_eq!(verify_exit_code(&records), 0, "not verifiable is honest, not a failure");
    }

    #[test]
    fn mismatch_or_error_fails_the_pass_with_a_code_distinct_from_run_failure_and_clap() {
        let mk = |o: VerifyOutcome| VerifyRecord { id: "x", scope: "global", outcome: o };
        assert_eq!(verify_exit_code(&[mk(VerifyOutcome::Ok("a".into())), mk(VerifyOutcome::NotVerifiable)]), 0);
        assert_eq!(verify_exit_code(&[mk(VerifyOutcome::Ok("a".into())), mk(VerifyOutcome::Mismatch("gone".into()))]), VERIFY_FAILED_EXIT_CODE);
        assert_eq!(verify_exit_code(&[mk(VerifyOutcome::Error("locked".into()))]), VERIFY_FAILED_EXIT_CODE);
        assert_ne!(VERIFY_FAILED_EXIT_CODE, 1, "1 = a migration run failed");
        assert_ne!(VERIFY_FAILED_EXIT_CODE, 2, "2 = clap usage error (bad flag)");
    }

    #[test]
    fn read_applied_ids_reads_real_tracking_state_and_never_creates_a_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shared").join("store.db");
        // Absent file: nothing recorded, and still absent afterwards.
        assert_eq!(read_applied_ids(&path).unwrap(), HashSet::new());
        assert!(!path.exists(), "verify must never create a database");

        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let store = Store::open_shared(&path).expect("open shared");
        store.migration_mark_applied("9010_a", "global", 1).unwrap();
        store.migration_mark_applied("9011_b", "global", 1).unwrap();
        drop(store);
        let got = read_applied_ids(&path).unwrap();
        assert!(got.contains("9010_a") && got.contains("9011_b"));
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn read_applied_ids_errors_on_a_file_that_is_not_a_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("objects.db");
        std::fs::write(&path, b"definitely not sqlite\n").unwrap();
        let err = read_applied_ids(&path).unwrap_err();
        assert!(err.contains("objects.db"), "names the store: {}", err);
    }

    #[test]
    fn table_count_treats_a_missing_table_as_zero_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("objects.db");
        drop(Store::open(&path).expect("create channel store"));
        let conn = open_readonly(&path).unwrap().expect("exists");
        assert_eq!(table_count(&conn, "db_table_that_never_existed").unwrap(), 0);
        assert!(table_count(&conn, "db_migrations").is_ok());
    }

    #[test]
    fn verify_event_line_is_valid_json_even_with_control_characters_in_detail() {
        let r = VerifyRecord {
            id: "9020_x",
            scope: "channel",
            outcome: VerifyOutcome::Error("line one\nline \"two\"\t\\end".into()),
        };
        let line = verify_event_line(&r);
        let parsed: serde_json::Value = serde_json::from_str(&line).expect("one JSON object per line");
        assert_eq!(parsed["event"], "migration_verify");
        assert_eq!(parsed["status"], "error");
        assert_eq!(parsed["detail"], "line one\nline \"two\"\t\\end");
        assert!(!line.contains('\n'), "must stay on one line");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Process-global env access — shared with registry::paths and
    // migrations::m0011_shared_store_backfill's tests, which mutate the SAME
    // AGENTMUX_ISOLATED_AUTH/AGENTMUX_INSTANCE_DIR vars. A module-local lock
    // only serializes tests within this file; Cargo runs a crate's tests in
    // one multi-threaded process, so a local-only lock still let this
    // module's tests race against those two (reagent/codex on PR #2318).
    use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_LOCK;

    fn clear() {
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        std::env::remove_var("AGENTMUX_SHARED_DIR");
        std::env::remove_var("AGENTMUX_ISOLATED_AUTH");
        std::env::remove_var("AGENTMUX_INSTANCE_DIR");
        std::env::remove_var("AGENTMUX_CHANNEL");
    }

    /// The regression test that would have caught the `ctx.home` /
    /// `shared_store_path` coupling bug before it shipped: `resolve_home()`
    /// must return the SAME value whether or not isolated-auth is active,
    /// even though `resolve_shared_store_path()` itself deliberately
    /// returns a DIFFERENT (channel-scoped) path under isolation. Every
    /// other Global migration, plus backups and the error log, anchor to
    /// `home` and must never silently move just because one channel opted
    /// into isolated auth.
    #[test]
    fn home_is_invariant_to_isolated_auth() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", "/tmp/test-home");

        let home_default = resolve_home().unwrap();
        assert_eq!(home_default, PathBuf::from("/tmp/test-home"));

        let shared_store_path_default = resolve_shared_store_path().unwrap();

        std::env::set_var("AGENTMUX_ISOLATED_AUTH", "1");
        std::env::set_var("AGENTMUX_INSTANCE_DIR", "/tmp/test-home/dev/some-branch");
        let home_isolated = resolve_home().unwrap();
        let shared_store_path_isolated = resolve_shared_store_path().unwrap();

        assert_eq!(
            home_default, home_isolated,
            "resolve_home() must be invariant to AGENTMUX_ISOLATED_AUTH/AGENTMUX_INSTANCE_DIR"
        );
        // Meanwhile resolve_shared_store_path() DOES vary — confirms the two
        // functions have genuinely diverged as designed, not that isolation
        // silently does nothing.
        assert_ne!(
            shared_store_path_default, shared_store_path_isolated,
            "resolve_shared_store_path() must actually change under isolation"
        );

        clear();
    }

    /// Same invariant as `home_is_invariant_to_isolated_auth`, but for
    /// isolation reached via the channel-based default
    /// (SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06.md) rather than
    /// an explicit `AGENTMUX_ISOLATED_AUTH=1`. `resolve_home()` must
    /// still anchor to the true global root even when a non-"stable"
    /// `AGENTMUX_CHANNEL` alone is what triggers isolation.
    #[test]
    fn home_is_invariant_to_channel_default_isolation() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", "/tmp/test-home");

        let home_default = resolve_home().unwrap();
        let shared_store_path_default = resolve_shared_store_path().unwrap();

        // No AGENTMUX_ISOLATED_AUTH set at all — only a non-"stable"
        // channel, which is now sufficient on its own to isolate.
        std::env::set_var("AGENTMUX_CHANNEL", "dev-some-branch");
        std::env::set_var("AGENTMUX_INSTANCE_DIR", "/tmp/test-home/dev/some-branch");
        let home_isolated = resolve_home().unwrap();
        let shared_store_path_isolated = resolve_shared_store_path().unwrap();

        assert_eq!(
            home_default, home_isolated,
            "resolve_home() must be invariant to channel-default isolation too"
        );
        assert_ne!(
            shared_store_path_default, shared_store_path_isolated,
            "resolve_shared_store_path() must actually isolate on channel default alone"
        );

        clear();
    }
}
