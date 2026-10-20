// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The backup taken before a batch of migrations, and its pruning. Moved out
//! of `runner.rs` to keep it under the 1500-line limit.

use std::path::Path;

/// Copy the stores a migration batch may write to
/// `<home>/shared/backups/pre-migration-<version>-<ts>/`, as a splash step.
/// `objects.db` is skipped when this process copied it forward from the
/// previous version: that version's folder still holds it untouched.
pub(super) fn backup_stores(home: &Path, shared_store_path: &Path, data_dir: &Path) -> std::io::Result<()> {
    let skip_objects = crate::bootstrap::carry_forward::carried_forward_this_run();
    crate::boot_timing::step("backup", "Backing up your data", || {
        backup_stores_to(home, shared_store_path, data_dir, skip_objects)
    })
}

fn backup_stores_to(home: &Path, shared_store_path: &Path, data_dir: &Path, skip_objects: bool) -> std::io::Result<()> {
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

    // VACUUM INTO, not a file copy: other processes hold the shared stores open
    // and a file copy misses their -wal. Global migrations write identity-store.db.
    let objects_db = data_dir.join("db").join("objects.db");
    let identity_store = home.join("shared").join("identity-store.db");
    for (src, name) in [(shared_store_path, "store.db"), (objects_db.as_path(), "objects.db"), (identity_store.as_path(), "identity-store.db")] {
        if src.exists() && !(skip_objects && name == "objects.db") {
            crate::backend::storage::db_copy::copy_database(src, &backup_dir.join(name)).map_err(std::io::Error::other)?;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn db(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        rusqlite::Connection::open(path).unwrap().execute_batch("CREATE TABLE t (v);").unwrap();
    }

    fn backed_up(home: &Path) -> Vec<String> {
        let dir = std::fs::read_dir(home.join("shared/backups")).unwrap().next().unwrap().unwrap().path();
        let mut names: Vec<String> =
            std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn the_backup_covers_the_shared_stores_and_objects_db() {
        let home = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        db(&home.path().join("shared/store.db"));
        db(&home.path().join("shared/identity-store.db"));
        db(&data.path().join("db/objects.db"));
        backup_stores_to(home.path(), &home.path().join("shared/store.db"), data.path(), false).unwrap();
        assert_eq!(backed_up(home.path()), vec!["identity-store.db", "objects.db", "store.db"]);
    }

    #[test]
    fn after_a_copy_forward_objects_db_is_not_backed_up_again() {
        let home = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        db(&home.path().join("shared/store.db"));
        db(&data.path().join("db/objects.db"));
        backup_stores_to(home.path(), &home.path().join("shared/store.db"), data.path(), true).unwrap();
        assert_eq!(backed_up(home.path()), vec!["store.db"]);
    }
}
