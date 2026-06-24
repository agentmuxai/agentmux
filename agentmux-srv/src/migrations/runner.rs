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
    let shared_store_path = match resolve_shared_store_path() {
        Some(p) => p,
        None => {
            eprintln!("migration: cannot resolve shared store path");
            return 1;
        }
    };

    let home = match shared_store_path.parent().and_then(|p| p.parent()) {
        Some(p) => p.to_path_buf(),
        None => {
            eprintln!("migration: cannot resolve home from shared store path");
            return 1;
        }
    };

    let shared_store = match Store::open_shared(&shared_store_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("migration: failed to open shared store: {}", e);
            return 1;
        }
    };

    let ctx = MigrationContext {
        home: home.clone(),
        data_dir: data_dir.to_path_buf(),
        shared_store_path: shared_store_path.clone(),
    };

    if list {
        return cmd_list(&shared_store);
    }

    let pending: Vec<_> = REGISTRY
        .iter()
        .filter(|m| !shared_store.migration_is_applied(m.id()))
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
    if let Err(e) = backup_stores(&home, &shared_store_path, &ctx.data_dir) {
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
                if let Err(e) = shared_store.migration_mark_applied(m.id(), scope, ms) {
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

fn cmd_list(shared_store: &Store) -> i32 {
    let applied = shared_store.migrations_list_applied().unwrap_or_default();
    for m in REGISTRY.iter() {
        let status = if applied.contains(&m.id().to_string()) { "applied" } else { "pending" };
        println!("{} [{}] — {}", m.id(), status, m.description());
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
