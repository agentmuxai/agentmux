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
    // A dev build on the real home says why it won't touch the shared stores, ahead of the generic
    // reason: `migrate --prepare` refuses with the same words.
    if let Err(why) = refuse_on_real_home(REGISTRY, &shared, &identity) {
        return Some(why);
    }
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

/// `task dev` sets this to `real` when it runs on the person's own `~/.agentmux`
/// (`AGENTMUX_DEV_USE_REAL_HOME=1`), `isolated` otherwise.
const DEV_HOME_MODE_ENV: &str = "AGENTMUX_DEV_HOME_MODE";

/// How many pending migration ids the refusal names; the rest are counted.
const MAX_IDS_SHOWN: usize = 5;

#[cfg(test)]
thread_local! {
    /// Lets a test act as a dev build on the real home without touching the process environment.
    pub(super) static FORCE_REAL_HOME: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Is this a dev build running on the real home?
pub(super) fn dev_on_real_home() -> bool {
    #[cfg(test)]
    if FORCE_REAL_HOME.with(|f| f.get()) {
        return true;
    }
    std::env::var(DEV_HOME_MODE_ENV).as_deref() == Ok("real")
}

/// Refuses, before anything is locked, backed up or opened, when a dev build on
/// the real home would move the stores every installed build shares ahead of
/// them. Opening a store runs its schema setup and a Global migration rewrites
/// the shared store, and the installed build can't read a store that is ahead of
/// it. A channel-scoped migration only touches the dev build's own data, so it is
/// no reason to refuse. Read-only; a no-op for any other build.
pub(super) fn refuse_on_real_home(
    regs: &[&(dyn super::Migration + Sync)],
    shared_store: &Path,
    identity_store: &Path,
) -> Result<(), String> {
    if !dev_on_real_home() {
        return Ok(());
    }
    match real_home_would_move(regs, shared_store, identity_store) {
        Some(what) => Err(format!(
            "this dev build runs on your real AgentMux home and would move the shared stores ahead of the installed build: {what}. \
             Either run it on an isolated home (`task dev` without AGENTMUX_DEV_USE_REAL_HOME=1), \
             or start the installed AgentMux build first and let it migrate the shared stores."
        )),
        None => Ok(()),
    }
}

/// What a build of this version would change in the shared stores, or `None`.
/// A store that doesn't exist yet has nobody to move ahead of.
pub(super) fn real_home_would_move(
    regs: &[&(dyn super::Migration + Sync)],
    shared_store: &Path,
    identity_store: &Path,
) -> Option<String> {
    use crate::backend::storage::migrations::{IDENTITY_STORE_SCHEMA_VERSION, SHARED_STORE_SCHEMA_VERSION};
    let mut what = Vec::new();
    for (path, current) in [(shared_store, SHARED_STORE_SCHEMA_VERSION), (identity_store, IDENTITY_STORE_SCHEMA_VERSION)] {
        match schema_version(path) {
            Ok(Some(found)) if found < current => {
                what.push(format!("{} is at schema {found}, this build uses {current}", path.display()))
            }
            Err(e) => what.push(format!("{}'s schema version can't be read: {e}", path.display())),
            _ => {}
        }
    }
    if shared_store.exists() {
        match read_applied_ids(shared_store) {
            Ok(applied) => {
                let ids: Vec<&str> = regs
                    .iter()
                    .filter(|m| m.scope() == MigrationScope::Global && !applied.contains(m.id()))
                    .map(|m| m.id())
                    .collect();
                if !ids.is_empty() {
                    let more = ids.len().saturating_sub(MAX_IDS_SHOWN);
                    let more = if more > 0 { format!(" and {more} more") } else { String::new() };
                    what.push(format!("shared-store migration(s) pending: {}{more}", ids[..ids.len().min(MAX_IDS_SHOWN)].join(", ")));
                }
            }
            Err(e) => what.push(format!("the shared store's migrations can't be read: {e}")),
        }
    }
    (!what.is_empty()).then(|| what.join("; "))
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

    // ---- A dev build on the real home must not move the shared stores ahead ----

    use crate::migrations::{Migration, MigrationContext, MigrationError};

    struct Fake {
        id: &'static str,
        scope: MigrationScope,
    }
    impl Migration for Fake {
        fn id(&self) -> &'static str { self.id }
        fn scope(&self) -> MigrationScope { self.scope }
        fn description(&self) -> &'static str { "fake" }
        fn up(&self, _ctx: &MigrationContext) -> Result<(), MigrationError> { Ok(()) }
    }

    /// Acts as a dev build on the real home for the length of the test.
    struct OnRealHome;
    impl OnRealHome {
        fn enter() -> Self {
            FORCE_REAL_HOME.with(|f| f.set(true));
            Self
        }
    }
    impl Drop for OnRealHome {
        fn drop(&mut self) {
            FORCE_REAL_HOME.with(|f| f.set(false));
        }
    }

    /// A real home's shared and identity stores, current and with nothing pending.
    fn real_home(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let (shared, identity) = (dir.join("store.db"), dir.join("identity-store.db"));
        drop(Store::open_shared(&shared).unwrap());
        drop(Store::open_identity_store(&identity).unwrap());
        (shared, identity)
    }

    /// The files in `dir` and their sizes, without the `-shm` and `-wal` files a read-only open of a WAL database leaves.
    fn files(dir: &Path) -> Vec<(std::ffi::OsString, u64)> {
        let mut v: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap())
            .filter(|e| !e.file_name().to_string_lossy().ends_with("-shm") && !e.file_name().to_string_lossy().ends_with("-wal"))
            .map(|e| (e.file_name(), e.metadata().unwrap().len()))
            .collect();
        v.sort();
        v
    }

    #[test]
    fn a_dev_build_on_the_real_home_refuses_a_pending_global_migration_and_names_the_ways_out() {
        let dir = tempfile::tempdir().unwrap();
        let (shared, identity) = real_home(dir.path());
        let global = Fake { id: "9001_global_new", scope: MigrationScope::Global };
        let regs: Vec<&(dyn Migration + Sync)> = vec![&global];
        let _real = OnRealHome::enter();
        let why = refuse_on_real_home(&regs, &shared, &identity).unwrap_err();
        assert!(why.contains("9001_global_new"), "{why}");
        assert!(why.contains("isolated home") && why.contains("installed AgentMux build"), "{why}");
    }

    #[test]
    fn a_pending_channel_migration_is_no_reason_to_refuse() {
        let dir = tempfile::tempdir().unwrap();
        let (shared, identity) = real_home(dir.path());
        let channel = Fake { id: "9002_channel_new", scope: MigrationScope::Channel };
        let regs: Vec<&(dyn Migration + Sync)> = vec![&channel];
        let _real = OnRealHome::enter();
        assert_eq!(refuse_on_real_home(&regs, &shared, &identity), Ok(()));
    }

    #[test]
    fn a_shared_or_identity_store_behind_this_builds_schema_is_refused() {
        let _real = OnRealHome::enter();
        let dir = tempfile::tempdir().unwrap();
        let (shared, identity) = real_home(dir.path());
        assert_eq!(refuse_on_real_home(&[], &shared, &identity), Ok(()));
        rusqlite::Connection::open(&shared).unwrap().pragma_update(None, "user_version", 1).unwrap();
        assert!(refuse_on_real_home(&[], &shared, &identity).unwrap_err().contains("store.db is at schema 1"));

        let dir = tempfile::tempdir().unwrap();
        let (shared, identity) = real_home(dir.path());
        rusqlite::Connection::open(&identity).unwrap().pragma_update(None, "user_version", 1).unwrap();
        assert!(refuse_on_real_home(&[], &shared, &identity).unwrap_err().contains("identity-store.db is at schema 1"));
    }

    #[test]
    fn any_other_build_is_not_held_back_and_a_home_with_no_stores_yet_has_nobody_to_move_ahead_of() {
        let dir = tempfile::tempdir().unwrap();
        let (shared, identity) = real_home(dir.path());
        let global = Fake { id: "9001_global_new", scope: MigrationScope::Global };
        let regs: Vec<&(dyn Migration + Sync)> = vec![&global];
        assert_eq!(refuse_on_real_home(&regs, &shared, &identity), Ok(()), "not a dev build on the real home");
        let _real = OnRealHome::enter();
        let none = tempfile::tempdir().unwrap();
        assert_eq!(refuse_on_real_home(&regs, &none.path().join("store.db"), &none.path().join("identity-store.db")), Ok(()));
    }

    #[test]
    fn the_boot_gate_and_the_migration_batch_refuse_before_touching_anything() {
        let home = tempfile::tempdir().unwrap();
        let shared_dir = home.path().join("shared");
        std::fs::create_dir_all(&shared_dir).unwrap();
        let (shared, _identity) = real_home(&shared_dir);
        let data_dir = home.path().join("data");
        let global = Fake { id: "9001_global_new", scope: MigrationScope::Global };
        let regs: Vec<&(dyn Migration + Sync)> = vec![&global];
        let before = (files(&shared_dir), data_dir.exists());
        let _real = OnRealHome::enter();
        let err = crate::migrations::runner::apply_pending(&regs, home.path(), &shared, &data_dir, None).unwrap_err();
        assert!(err.contains("9001_global_new"), "{err}");
        assert_eq!((files(&shared_dir), data_dir.exists()), before, "no lock, no backup, no store opened, nothing migrated");
        let reason = pending_reason(&regs, &shared, &data_dir.join("db").join("objects.db"), &shared_dir.join("identity-store.db"));
        assert!(reason.is_some(), "the gate still sends the launcher to prepare, which then refuses");
    }
}
