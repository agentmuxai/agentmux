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
use crate::registry::resolve_shared_store_path;

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

    let home = match shared_store_path.parent().and_then(|p| p.parent()) {
        Some(p) => p.to_path_buf(),
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
    let channel_store_path = data_dir.join("db").join("objects.db");
    let channel_store = if channel_store_path.exists() {
        match Store::open(&channel_store_path) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("migration: failed to open channel store: {}", e);
                return 1;
            }
        }
    } else {
        None
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
            !tracking.map(|s| s.migration_is_applied(m.id())).unwrap_or(false)
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
/// Returns None only when a Channel migration is requested but no channel store
/// is open (fresh install with no objects.db yet — migration is a no-op).
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

// ── Error log ─────────────────────────────────────────────────────────────────

fn write_error_log(home: &Path, msg: &str) {
    let log_dir = home.join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let path = log_dir.join("migration-error.log");
    let content = format!("{}\n", msg);
    let _ = std::fs::write(path, content);
}
