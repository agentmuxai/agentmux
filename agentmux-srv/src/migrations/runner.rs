// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Migration runner — invoked by `agentmux-srv migrate`.
//!
//! Progress events are emitted as newline-delimited JSON to stdout so the
//! launcher's splash screen can show "Updating your data…" while migrations
//! run.  Failures are written to `<home>/logs/migration-error.log`; the
//! process exits non-zero so the launcher can surface the error rather than
//! booting with a half-migrated data dir.

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

/// Called from `main.rs` when the `migrate` subcommand is active.
/// Returns the exit code (0 = success, 1 = failure).
pub fn run_migrate_command(data_dir: &Path, dry_run: bool, list: bool) -> i32 {
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

    // Ensure the shared/ directory exists — on a fresh install it has not been
    // created yet (the daemon does create_dir_all at startup, but migrate runs
    // before the daemon). SQLite cannot create parent directories itself.
    if let Some(parent) = shared_store_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("migration: failed to create shared dir {}: {}", parent.display(), e);
            return 1;
        }
    }

    let shared_store = match Store::open_shared(&shared_store_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("migration: failed to open shared store: {}", e);
            return 1;
        }
    };

    // Channel store (objects.db) tracks channel-scoped migrations independently
    // per channel — MigrationScope::Channel migrations record here, not in shared.
    // Always create the directory and open/create the store so that seed migrations
    // (e.g. m0008_default_bundle) run on fresh install. Data-transformation
    // migrations guard themselves with `if !ctx.channel_store_path.exists()` and
    // are no-ops when called on a newly-created empty database.
    let channel_store_path = data_dir.join("db").join("objects.db");
    if let Some(parent) = channel_store_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("migration: failed to create channel db dir {}: {}", parent.display(), e);
            return 1;
        }
    }
    let channel_store = match Store::open(&channel_store_path) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("migration: failed to open channel store: {}", e);
            return 1;
        }
    };

    let ctx = MigrationContext {
        home: home.clone(),
        data_dir: data_dir.to_path_buf(),
        shared_store_path: shared_store_path.clone(),
        channel_store_path: channel_store_path.clone(),
    };

    if list {
        return cmd_list(&shared_store, channel_store.as_ref());
    }

    // A migration is pending if its tracking store does not record it applied.
    let pending: Vec<_> = REGISTRY
        .iter()
        .filter(|m| {
            let tracking = tracking_store(m.scope(), &shared_store, channel_store.as_ref());
            tracking.map_or(false, |s| !s.migration_is_applied(m.id()))
        })
        .collect();

    if pending.is_empty() {
        emit_summary(0, REGISTRY.len());
        return 0;
    }

    if dry_run {
        for m in &pending {
            println!("pending: {} — {}", m.id(), m.description());
        }
        return 0;
    }

    // Back up before any writes.
    if let Err(e) = backup_stores(&home, &shared_store_path, data_dir) {
        eprintln!("migration: backup failed: {}", e);
        return 1;
    }

    let skipped = REGISTRY.len() - pending.len();
    let mut applied = 0;

    for m in &pending {
        emit("migration_start", m.id(), &format!("\"description\":\"{}\"", m.description()));
        let t = Instant::now();
        match m.up(&ctx) {
            Ok(()) => {
                let ms = t.elapsed().as_millis() as u64;
                let scope = m.scope().as_str();
                let tracking = tracking_store(m.scope(), &shared_store, channel_store.as_ref());
                let mark_result = tracking
                    .ok_or_else(|| format!("no tracking store for {} (channel store missing)", m.id()))
                    .and_then(|s| s.migration_mark_applied(m.id(), scope, ms).map_err(|e| e.to_string()));
                if let Err(e) = mark_result {
                    let msg = format!("migration: failed to record {} as applied: {}", m.id(), e);
                    write_error_log(&home, &msg);
                    eprintln!("{}", msg);
                    return 1;
                }
                emit("migration_done", m.id(), &format!("\"duration_ms\":{}", ms));
                applied += 1;
            }
            Err(e) => {
                let msg = format!("migration {} failed: {}", m.id(), e);
                write_error_log(&home, &msg);
                eprintln!("{}", msg);
                return 1;
            }
        }
    }

    emit_summary(applied, skipped);
    0
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

    if let Some(parent) = shared_store_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Err(format!("run_pending_migrations: create shared dir: {}", e));
        }
    }

    let home = match resolve_home() {
        Some(p) => p,
        None => return Err("run_pending_migrations: cannot resolve global shared root".to_string()),
    };

    let shared_store = match Store::open_shared(&shared_store_path) {
        Ok(s) => s,
        Err(e) => return Err(format!("run_pending_migrations: open shared store: {}", e)),
    };

    let channel_store_path = data_dir.join("db").join("objects.db");
    if let Some(parent) = channel_store_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Err(format!("run_pending_migrations: create channel db dir: {}", e));
        }
    }
    // Always open/create the channel store, mirroring run_migrate_command (runner.rs:85-98).
    // Skipping it on fresh install left Channel-scoped migrations (0002/0003/0007)
    // out of the pending list, so they were never marked applied. When wstore then
    // created objects.db, count_pending_migrations at ESTART time reported them as
    // pending and fired the "Migration failed" UI warning on every fresh first launch.
    let channel_store = match Store::open(&channel_store_path) {
        Ok(s) => Some(s),
        Err(e) => return Err(format!("run_pending_migrations: open channel store: {}", e)),
    };

    let pending: Vec<_> = REGISTRY.iter().filter(|m| {
        let tracking = tracking_store(m.scope(), &shared_store, channel_store.as_ref());
        tracking.map_or(false, |s| !s.migration_is_applied(m.id()))
    }).collect();

    if pending.is_empty() {
        return Ok(0);
    }

    let ctx = super::MigrationContext {
        home: home.clone(),
        data_dir: data_dir.to_path_buf(),
        shared_store_path: shared_store_path.clone(),
        channel_store_path: channel_store_path.clone(),
    };

    if let Err(e) = backup_stores(&home, &shared_store_path, data_dir) {
        return Err(format!("run_pending_migrations: backup failed: {}", e));
    }

    let mut applied = 0;
    for m in &pending {
        let t = std::time::Instant::now();
        tracing::info!(id = m.id(), description = m.description(), "run_pending_migrations: applying");
        match m.up(&ctx) {
            Ok(()) => {
                let ms = t.elapsed().as_millis() as u64;
                let scope = m.scope().as_str();
                let tracking = tracking_store(m.scope(), &shared_store, channel_store.as_ref());
                if let Some(s) = tracking {
                    if let Err(e) = s.migration_mark_applied(m.id(), scope, ms) {
                        return Err(format!("run_pending_migrations: mark applied {}: {}", m.id(), e));
                    }
                }
                tracing::info!(id = m.id(), duration_ms = ms, "run_pending_migrations: applied");
                applied += 1;
            }
            Err(e) => {
                return Err(format!("run_pending_migrations: migration {} failed: {}", m.id(), e));
            }
        }
    }

    Ok(applied)
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
