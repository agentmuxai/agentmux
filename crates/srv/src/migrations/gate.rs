// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! srv's boot gate: whether this version's data needs `agentmux-srv migrate
//! --prepare` before srv may open it (S1c of
//! `docs/specs/SPEC_FAST_STARTUP_UPGRADE_OWNS_MIGRATIONS_AND_UPDATES_2026_09_15.md` §10).
//!
//! Read-only, and fail-closed (invariants S1b and S1c there): it only opens
//! stores read-only, creates nothing, and any doubt (a store it cannot read)
//! counts as "needs preparing", so srv never boots onto data it couldn't
//! check. A boot with nothing to prepare costs two read-only opens, in place
//! of the read-write pending count it replaces.

use std::path::Path;

use super::runner::read_applied_ids;
use super::{MigrationScope, REGISTRY};

/// Why `data_dir` needs preparing, or `None` when srv can open it as it is.
pub fn data_needs_preparing(data_dir: &Path, version: &str) -> Option<String> {
    let db_dir = data_dir.join("db");
    if let Some(source) = crate::bootstrap::carry_forward::source_for(&db_dir, version) {
        return Some(format!("this version has no data yet; {} has data to copy forward", source.display()));
    }
    // No shared root (CI, a bare cargo run): nothing to migrate, as for
    // `run_pending_migrations`.
    let shared = crate::registry::resolve_shared_store_path()?;
    let identity = crate::registry::resolve_identity_store_path()?;
    pending_reason(REGISTRY, &shared, &db_dir.join("objects.db"), &identity)
}

/// Why the stores `migrate --prepare` opens need work: a schema older than
/// this build's (its schema setup would change the file), a migration of
/// `regs` not yet applied, or a store that can't be read. `None` when they need
/// nothing. Read-only; `apply_pending` asks it too, to take its backup before
/// anything opens the stores.
///
/// `identity-store.db` only counts when it is readable and behind: srv starts
/// without it when it can't open it, and preparing couldn't repair it.
pub(super) fn pending_reason(
    regs: &[&(dyn super::Migration + Sync)],
    shared_store: &Path,
    objects_db: &Path,
    identity_store: &Path,
) -> Option<String> {
    use crate::backend::storage::migrations::{
        IDENTITY_STORE_SCHEMA_VERSION, OBJECT_SCHEMA_VERSION, SHARED_STORE_SCHEMA_VERSION,
    };
    let stores = [
        (shared_store, SHARED_STORE_SCHEMA_VERSION, true),
        (objects_db, OBJECT_SCHEMA_VERSION, true),
        (identity_store, IDENTITY_STORE_SCHEMA_VERSION, false),
    ];
    for (path, current, required) in stores {
        match schema_version(path) {
            Ok(Some(found)) if found < current => {
                return Some(format!("{} is at schema {found}, this build uses {current}", path.display()))
            }
            Err(e) if required => return Some(format!("{}'s schema version can't be read: {e}", path.display())),
            _ => {}
        }
    }
    let shared_applied = match read_applied_ids(shared_store) {
        Ok(ids) => ids,
        Err(e) => return Some(format!("the shared store's migrations can't be read: {e}")),
    };
    let channel_applied = match read_applied_ids(objects_db) {
        Ok(ids) => ids,
        Err(e) => return Some(format!("this version's migrations can't be read: {e}")),
    };
    let pending: Vec<&str> = regs
        .iter()
        .filter(|m| {
            let applied = match m.scope() {
                MigrationScope::Global => &shared_applied,
                MigrationScope::Channel => &channel_applied,
            };
            !applied.contains(m.id())
        })
        .map(|m| m.id())
        .collect();
    let first = pending.first()?;
    Some(format!("{} migration(s) pending, starting with {first}", pending.len()))
}

/// `PRAGMA user_version` of the database at `path`, read-only; `None` when
/// the file doesn't exist yet.
fn schema_version(path: &Path) -> Result<Option<i64>, String> {
    let Some(conn) = super::runner::open_readonly(path)? else { return Ok(None) };
    conn.query_row("PRAGMA user_version", [], |r| r.get(0)).map(Some).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::store::Store;

    /// An identity store that doesn't exist: nothing to prepare for it.
    fn no_identity(dir: &Path) -> std::path::PathBuf {
        dir.join("identity-store.db")
    }

    /// Stores with every registered migration recorded as applied.
    fn migrated(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let shared = dir.join("store.db");
        let objects = dir.join("objects.db");
        let shared_store = Store::open_shared(&shared).unwrap();
        let channel_store = Store::open(&objects).unwrap();
        for m in REGISTRY.iter() {
            let store = match m.scope() {
                MigrationScope::Global => &shared_store,
                MigrationScope::Channel => &channel_store,
            };
            store.migration_mark_applied(m.id(), m.scope().as_str(), 0).unwrap();
        }
        (shared, objects)
    }

    #[test]
    fn fully_migrated_stores_need_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (shared, objects) = migrated(dir.path());
        assert_eq!(pending_reason(REGISTRY, &shared, &objects, &no_identity(dir.path())), None);
    }

    #[test]
    fn a_pending_migration_needs_preparing() {
        let dir = tempfile::tempdir().unwrap();
        let (shared, objects) = migrated(dir.path());
        let last = REGISTRY.last().unwrap();
        let path = match last.scope() {
            MigrationScope::Global => &shared,
            MigrationScope::Channel => &objects,
        };
        rusqlite::Connection::open(path).unwrap().execute("DELETE FROM db_migrations WHERE id = ?1", [last.id()]).unwrap();
        let reason = pending_reason(REGISTRY, &shared, &objects, &no_identity(dir.path())).expect("one migration is pending");
        assert!(reason.contains(last.id()), "{reason}");
    }

    #[test]
    fn a_fresh_install_needs_preparing_and_the_check_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (shared, objects) = (dir.path().join("store.db"), dir.path().join("objects.db"));
        assert!(pending_reason(REGISTRY, &shared, &objects, &no_identity(dir.path())).is_some());
        assert!(!shared.exists() && !objects.exists(), "the gate is read-only");
    }

    #[test]
    fn a_store_it_cannot_read_needs_preparing() {
        let dir = tempfile::tempdir().unwrap();
        let (shared, objects) = migrated(dir.path());
        std::fs::write(&objects, b"not a database").unwrap();
        let _ = std::fs::remove_file(objects.with_extension("db-wal"));
        assert!(pending_reason(REGISTRY, &shared, &objects, &no_identity(dir.path())).unwrap().contains("can't be read"));
    }

    /// A build that only raises the schema version (no new migration) still
    /// goes through preparation, so its schema change gets a backup first.
    #[test]
    fn an_older_schema_version_needs_preparing() {
        let dir = tempfile::tempdir().unwrap();
        let (shared, objects) = migrated(dir.path());
        rusqlite::Connection::open(&objects).unwrap().pragma_update(None, "user_version", 3).unwrap();
        let reason = pending_reason(REGISTRY, &shared, &objects, &no_identity(dir.path())).expect("schema 3 is behind");
        assert!(reason.contains("schema 3"), "{reason}");
    }

    /// The identity store has its own schema version; behind, it needs
    /// preparing like the other two. Unreadable, it doesn't stop srv.
    #[test]
    fn an_older_identity_store_needs_preparing_and_an_unreadable_one_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let (shared, objects) = migrated(dir.path());
        let identity = dir.path().join("identity-store.db");
        drop(Store::open_identity_store(&identity).unwrap());
        assert_eq!(pending_reason(REGISTRY, &shared, &objects, &identity), None);
        rusqlite::Connection::open(&identity).unwrap().pragma_update(None, "user_version", 2).unwrap();
        let reason = pending_reason(REGISTRY, &shared, &objects, &identity).expect("schema 2 is behind");
        assert!(reason.contains("identity-store.db"), "{reason}");
        std::fs::write(&identity, b"not a database").unwrap();
        let _ = std::fs::remove_file(identity.with_extension("db-wal"));
        assert_eq!(pending_reason(REGISTRY, &shared, &objects, &identity), None);
    }

    #[test]
    fn a_previous_version_to_copy_needs_preparing() {
        let root = tempfile::tempdir().unwrap();
        let versions = root.path().join("channels").join("stable").join("versions");
        let old = versions.join("0.1.0").join("data").join("db");
        std::fs::create_dir_all(&old).unwrap();
        drop(Store::open(&old.join("objects.db")).unwrap());
        let reason = data_needs_preparing(&versions.join("0.2.0").join("data"), "0.2.0").unwrap();
        assert!(reason.contains("copy forward"), "{reason}");
    }
}
