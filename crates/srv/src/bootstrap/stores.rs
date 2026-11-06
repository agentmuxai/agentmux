// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Split out of the single-file bootstrap module unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.3).

use super::*;
use super::store_attach::*;
use super::store_boot::*;

/// Step 2: Parse CLI args and build config. Dispatches the `migrate`
/// subcommand (and exits) before touching AUTH_KEY-gated config.
pub fn load_config() -> config::Config {
    let args = CliArgs::parse();

    // Dispatch migrate subcommand before loading config (no AUTH_KEY needed).
    if let Some(config::SrvCommand::Migrate { dry_run, list, verify, prepare }) = &args.command {
        // Same precedence as `Config::from_env_and_args`: the CLI flag, then
        // the canonical `AGENTMUX_DATA_DIR` the launcher exports. Without
        // that fallback a launcher-spawned `migrate --verify` opened a
        // different objects.db than the daemon's and could "verify" the
        // wrong channel and exit 0 (codex P1 on #3058). The retired
        // `AGENTMUX_DATA_HOME` and the bare default stay last for the
        // standalone / pre-unification shapes.
        let data_dir: std::path::PathBuf = args.wavedata
            .clone()
            .or_else(|| std::env::var_os("AGENTMUX_DATA_DIR").filter(|s| !s.is_empty()).map(std::path::PathBuf::from))
            .or_else(|| std::env::var_os("AGENTMUX_DATA_HOME").map(std::path::PathBuf::from))
            .unwrap_or_else(|| std::path::PathBuf::from(base::get_mux_data_dir()));
        let data_dir = migrate_data_dir(data_dir);
        let code = migrations::run_migrate_command(&data_dir, *dry_run, *list, *verify, *prepare);
        std::process::exit(code);
    }

    if let Some(config::SrvCommand::MigrateForkSlugs { plan: _, apply, rollback, restore }) = &args.command {
        use crate::backend::rpc_types::ForkSlugMigrationMode as Mode;
        let (mode, journal) = match (apply, rollback, restore) {
            (true, _, _) => (Mode::Apply, None),
            (_, Some(j), _) => (Mode::Rollback, Some(j.as_str())),
            (_, _, Some(j)) => (Mode::Restore, Some(j.as_str())),
            _ => (Mode::Plan, None),
        };
        std::process::exit(crate::backend::slug_migration::run_cli(mode, journal));
    }

    let config = config::Config::from_env_and_args(&args).unwrap_or_else(|e| {
        tracing::error!("Failed to load config: {}", e);
        std::process::exit(1);
    });

    // Make the per-launch auth_key available to the cross-instance agent
    // registry writer. Peers performing an HTTP forward of a missed inject
    // use this to authenticate against the writing instance's sidecar.
    // Must happen after Config::from_env_and_args (which removes
    // AGENTMUX_AUTH_KEY from the process env) but before anything calls
    // `agent_registry::write`.
    crate::backend::reactive::registry::init_local_auth_key(&config.auth_key);

    config
}

/// The data folder `migrate` works on: the one the daemon opens given the same
/// environment. The daemon turns its data folder into `AGENTMUX_DATA_HOME` and
/// opens `base::get_mux_db_dir()`, which resolves through `agentmux_root()`,
/// where `AGENTMUX_HOME_OVERRIDE` comes first, except for a data folder inside
/// it (an isolated instance's version folder, task #3569). Without the same
/// resolution, the launcher's `migrate --prepare` under a home override
/// migrated the version's folder while srv's boot check read `<override>/db`,
/// and srv never started (Korp@narko, after #5252).
fn migrate_data_dir(requested: std::path::PathBuf) -> std::path::PathBuf {
    if !requested.as_os_str().is_empty() {
        std::env::set_var("AGENTMUX_DATA_HOME", &requested);
    }
    base::get_mux_data_dir()
}

/// Output of [`open_stores_and_migrate`] — every store/log handle the rest
/// of bootstrap and `AppState` need.
pub struct Stores {
    pub mstore: Arc<Store>,
    pub filestore: Arc<FileStore>,
    pub global_transcript_store: Option<Arc<FileStore>>,
    pub shared_store: Option<Arc<Store>>,
    pub id_store: Arc<Store>,
    /// Permanently-global identity store (agent→account links, memory
    /// bundles, drone definitions, muxbus/agent M2M credentials, native
    /// memory, cron jobs) — see
    /// `docs/specs/SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md`. Distinct from
    /// `id_store`: never redirected by `isolated_auth_enabled()`. Falls
    /// back to `mstore` (never `None`) on the same best-effort terms as
    /// `id_store`'s own fallback, so a resolution/open failure degrades to
    /// today's per-channel behavior instead of panicking.
    pub identity_store: Arc<Store>,
    /// Where the Kanban queue, Presence claims, coordination posts and the
    /// Jobs run ledger live: `identity_store`, unless this is a `task dev` build
    /// (or `AGENTMUX_ISOLATED_WORK=1`), which has a `work-store.db` of its own
    /// (`crate::registry::work_store`).
    pub work_store: Arc<Store>,
    /// Where scheduled jobs live: `shared_store` (`None` when it can't be
    /// resolved), or `work_store` in a build that has its own.
    pub cron_store: Option<Arc<Store>>,
    pub saga_log: Arc<crate::sagas::log::SagaLog>,
    pub saga_id_seed: u64,
}

/// Step 4: Initialize backend (matching Go cmd/server/main-server.go:374-590).
/// Sets up the data directory, runs in-process migrations, opens every
/// SQLite-backed store, attaches the shared/global registries, and performs
/// the various one-time startup seed/repair passes.
pub fn open_stores_and_migrate(config: &config::Config, version: &str, build_time: &str) -> Stores {
    base::set_version(version);
    base::set_build_time(build_time);

    prepare_data_dir(config, version);
    let mux_data_dir = base::get_mux_data_dir();
    let db_dir = base::get_mux_db_dir();
    gate_or_prepare_data(&mux_data_dir, &db_dir);

    let mstore_raw = open_objects_db(&db_dir);
    attach_agent_registry(&mstore_raw);
    let backfill_def_ids = attach_definition_store(&mstore_raw);
    reconcile_registries(&mstore_raw);
    attach_token_index_and_wan_identity(&mstore_raw);
    let mstore = Arc::new(mstore_raw);
    // Identity M4a: lets spawn sites tell a row-backed block from a row-less
    // one when the environment carries no UID.
    backend::identity_spawn::attach_row_store(mstore.clone());
    let filestore = open_filestore(&db_dir);
    let global_transcript_store = open_global_transcripts();
    let shared_store = open_shared_store();
    let id_store = pick_id_store(&shared_store, &mstore);
    adopt_earlier_auth(&shared_store, &id_store);
    let identity_store = open_identity_store(&mstore);
    let (work_store, cron_store) = open_work_store(&identity_store, shared_store.clone(), &db_dir);
    // #3603: memory resolution finds the account an agent is linked to (the
    // directory its spawn runs Claude in) through these, read-only.
    crate::server::native_memory_handlers::attach_identity_stores(
        id_store.clone(),
        identity_store.clone(),
    );
    // Which agents get GitHub tokens from AgentMux Cloud's broker is decided by the accounts here.
    crate::identity::github_tokens::attach_store(id_store.clone());
    let global_memory_recorder =
        attach_global_memory(&id_store, &global_transcript_store, &backfill_def_ids);
    let (saga_log, saga_id_seed) = open_saga_log(&db_dir);
    ensure_initial_data(&mstore);
    recover_and_seed(&mstore, &id_store, &identity_store, &global_memory_recorder);
    install_key_resolvers(&mstore);

    Stores {
        mstore,
        filestore,
        global_transcript_store,
        shared_store,
        id_store,
        identity_store,
        work_store,
        cron_store,
        saga_log,
        saga_id_seed,
    }
}

/// After a repair of the shared store (`filestore::repair`): put back, from the per-channel copies of
/// it, the conversation of each agent whose zone lost rows. Runs inside the repair, before the store
/// is opened for use.
pub(super) fn restore_repaired_history(fs: &FileStore, zones: &[String]) {
    let Some(home) = registry::resolve_global_shared_root().and_then(|shared| shared.parent().map(std::path::Path::to_path_buf)) else { return };
    let stats = crate::backend::transcript_backfill::restore_damaged(&home, fs, zones);
    tracing::info!(?stats, "history repair: conversations put back from the per-channel copies");
}

/// The stores the work queue and scheduled jobs use. Installed and `task
/// package` builds share the host's (the identity store and the shared store,
/// as before); a `task dev` build gets a `work-store.db` of its own, so it never
/// reads or changes the operator's real queue or jobs. If that store can't be
/// opened srv exits, as it does for `objects.db`: falling back to the host's
/// files would be the leak this exists to close.
fn open_work_store(
    identity_store: &Arc<Store>,
    shared_store: Option<Arc<Store>>,
    db_dir: &std::path::Path,
) -> (Arc<Store>, Option<Arc<Store>>) {
    let Some(path) = registry::resolve_isolated_work_store_path(db_dir) else {
        return (identity_store.clone(), shared_store);
    };
    let opened = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .map_err(|e| e.to_string())
        .and_then(|()| Store::open_identity_store(&path).map_err(|e| e.to_string()));
    let store = Arc::new(opened.unwrap_or_else(|e| {
        tracing::error!(path = %path.display(), "Failed to open the channel's work store: {}", e);
        std::process::exit(1);
    }));
    let reason = crate::registry::work_store::isolated_work_reason();
    tracing::info!(path = %path.display(), reason = reason.as_str(), "work store: attached (channel-scoped)");
    (store.clone(), Some(store))
}

/// What an unsupervised srv does before opening its stores: copy a previous
/// version's data forward, snapshot, and run pending migrations (the same
/// work `agentmux-srv migrate --prepare` does for the launcher).
pub(super) fn prepare_in_process(mux_data_dir: &std::path::Path, db_dir: &std::path::Path, code_version: &str) {
    // Before the pending-migration count below, which opens the shared store and so runs its schema setup.
    if let Err(e) = migrations::refuse_if_real_home_would_move() {
        tracing::error!("startup: refusing to start: {}", e);
        eprintln!("{}", agentmux_common::srv_stderr::migration_failed_line(&e));
        std::process::exit(1);
    }
    // Pre-migration snapshot (Increment B.2 lean cut from
    // SPEC_DATA_CHANNELS §3.4). Run BEFORE Store::open AND before
    // count_pending_migrations/run_pending_migrations — both open
    // objects.db via Store::open which runs DDL/schema setup, mutating
    // the file. The snapshot must capture the pre-DDL state so it is a
    // valid rollback aid for a buggy forward migration.
    //
    // Failures are logged and ignored — refusing to boot when the
    // snapshot can't be written would be worse than booting without a
    // backup (the safety lock still prevents downgrade corruption).
    let channel = std::env::var("AGENTMUX_CHANNEL").unwrap_or_else(|_| "stable".to_string());
    // Snapshots live in this version's own data directory, `<data>/snapshots/`:
    // `get_mux_data_dir()` answers with the data directory once
    // AGENTMUX_DATA_HOME is set above (#3372). They sit beside the databases
    // they copy, and pruning keeps 5 there. AGENTMUX_HOME_OVERRIDE wins in
    // tests. (Before #3372 they went to `~/.agentmux/snapshots/`; older
    // snapshots stay there.)
    let snapshots_dir = std::env::var_os("AGENTMUX_HOME_OVERRIDE")
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(base::get_mux_data_dir)
        .join("snapshots");
    // A new version's first start begins from the previous version's data,
    // then migrates it like any upgrade (carry_forward.rs). No snapshot after
    // a copy-forward: the previous version's folder is that data, untouched.
    let carried = super::carry_forward::from_previous_version(&db_dir, &code_version).is_some();
    let snapshot = if carried {
        Ok(None)
    } else {
        maybe_snapshot_pre_migration(&db_dir, &snapshots_dir, &channel, &code_version, OBJECT_SCHEMA_VERSION)
    };
    match snapshot {
        Ok(Some(path)) => tracing::info!(snapshot = %path.display(), "pre-migration snapshot written"),
        Ok(None) => {}
        Err(e) => tracing::warn!("pre-migration snapshot failed (continuing without backup): {}", e),
    }

    // Run in-process migrations AFTER snapshot so the rollback aid captures
    // the pre-DDL state. count_pending_migrations emits AGENTMUXSRV-MIGRATING
    // first so the launcher/sidecar extend their ESTART deadline before the
    // (potentially slow) migration work begins.
    // Strict check, lenient reaction: this runs only when no supervisor
    // prepares data (headless srv, `task dev:standalone`), where the line
    // only sizes the ESTART deadline and the migration batch runs right below
    // anyway, so an unreadable store costs nothing here beyond a less precise
    // hint — `readable_pending()` reproduces exactly the number this call site
    // used before. Under the launcher the same question is the fail-closed
    // boot gate instead (`migrations/gate.rs`, invariant S1b of
    // `SPEC_FAST_STARTUP_UPGRADE_OWNS_MIGRATIONS_AND_UPDATES_2026_09_15.md`).
    let pre_migration_count = match migrations::try_count_pending_migrations(&mux_data_dir) {
        Ok(n) => n,
        Err(e) => {
            let fallback = e.readable_pending();
            // Distinguished because they mean different things to whoever
            // reads the log: one store was readable and the count is
            // partial, versus nothing was readable and the count is a
            // placeholder.
            match &e {
                migrations::PendingCountError::ChannelStoreUnreadable { .. } => tracing::warn!(
                    "startup: channel-scoped migrations could not be counted: {} — sizing the ESTART deadline with {} global",
                    e,
                    fallback
                ),
                _ => tracing::warn!(
                    "startup: no pending-migration count available: {} — sizing the ESTART deadline as if none were pending",
                    e
                ),
            }
            fallback
        }
    };
    if pre_migration_count > 0 {
        eprintln!("{}", agentmux_common::srv_stderr::migrating_line("migrations", pre_migration_count));
    }
    match migrations::run_pending_migrations(&mux_data_dir) {
        Ok(0) => {}
        Ok(n) => tracing::info!(applied = n, "startup: applied pending migrations"),
        Err(e) => {
            // SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03 Phase 1 (F4): a failed
            // migration is fatal. This used to be a `warn!` and the server
            // booted anyway against a store in an unknown state, which turned
            // any migration bug into a silent one. Note the asymmetry it left:
            // failing to OPEN the store (just below) always exited 1; failing
            // to MIGRATE it did not.
            //
            // The tagged stderr line lets the launcher / CEF host fail fast
            // with the real reason instead of timing out on ESTART 30s later
            // (crates/common/src/srv_stderr.rs). Exit code 1 is what
            // docs/exe-return-codes.md already documented for this case.
            tracing::error!("startup: migration failed — refusing to start: {}", e);
            eprintln!("{}", agentmux_common::srv_stderr::migration_failed_line(&e));
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sets env vars for one test and puts them back after.
    struct EnvRestore(Vec<(&'static str, Option<std::ffi::OsString>)>);
    impl EnvRestore {
        fn set(vars: &[(&'static str, Option<&std::path::Path>)]) -> Self {
            let saved = vars.iter().map(|(k, _)| (*k, std::env::var_os(k))).collect();
            for (k, v) in vars {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
            EnvRestore(saved)
        }
    }
    impl Drop for EnvRestore {
        fn drop(&mut self) {
            for (k, v) in &self.0 {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
    }

    /// `migrate` works on the folder whose `db/` the daemon opens, so the
    /// launcher's `migrate --prepare` prepares what srv's boot check reads.
    #[test]
    fn migrate_works_on_the_daemons_data_folder_with_and_without_a_home_override() {
        let _guard = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = tempfile::tempdir().unwrap();
        let version_dir = root.path().join("channels").join("x").join("versions").join("1.0.0").join("data");

        // No override: the folder asked for, as before.
        let _env = EnvRestore::set(&[("AGENTMUX_HOME_OVERRIDE", None), ("AGENTMUX_DATA_HOME", None)]);
        assert_eq!(migrate_data_dir(version_dir.clone()), version_dir);
        assert_eq!(base::get_mux_db_dir(), version_dir.join("db"), "the daemon's db folder");

        // A home override and a version folder inside it (an isolated
        // instance): the versioned layout, as without an override.
        let _env = EnvRestore::set(&[("AGENTMUX_HOME_OVERRIDE", Some(root.path())), ("AGENTMUX_DATA_HOME", None)]);
        assert_eq!(migrate_data_dir(version_dir.clone()), version_dir);
        assert_eq!(base::get_mux_db_dir(), version_dir.join("db"));

        // A folder outside the override isn't the instance's: the override's own, as before.
        let elsewhere = tempfile::tempdir().unwrap();
        let _env = EnvRestore::set(&[("AGENTMUX_HOME_OVERRIDE", Some(root.path())), ("AGENTMUX_DATA_HOME", None)]);
        assert_eq!(migrate_data_dir(elsewhere.path().to_path_buf()), root.path());
        assert_eq!(migrate_data_dir(elsewhere.path().to_path_buf()).join("db"), base::get_mux_db_dir());
    }

    /// Task #3569: under a root override each version gets its own folder and
    /// the next one starts from the previous one's data, so an upgrade's data
    /// can be tested in isolation.
    #[test]
    fn an_isolated_instance_carries_its_data_forward_between_versions() {
        let _guard = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = tempfile::tempdir().unwrap();
        let versions = root.path().join("channels").join("x").join("versions");
        let (old, new) = (versions.join("1.0.0").join("data"), versions.join("1.1.0").join("data"));
        std::fs::create_dir_all(old.join("db")).unwrap();
        let conn = rusqlite::Connection::open(old.join("db").join("objects.db")).unwrap();
        conn.execute_batch("CREATE TABLE t(v TEXT); INSERT INTO t VALUES ('from 1.0.0');").unwrap();
        drop(conn);

        let _env = EnvRestore::set(&[("AGENTMUX_HOME_OVERRIDE", Some(root.path())), ("AGENTMUX_DATA_HOME", None)]);
        std::env::set_var("AGENTMUX_DATA_HOME", &new); // what open_stores_and_migrate does with the launcher's folder
        let db_dir = base::get_mux_db_dir();
        assert_eq!(db_dir, new.join("db"), "not <override>/db, which every version would share");
        std::fs::create_dir_all(&db_dir).unwrap();
        let from = super::carry_forward::from_previous_version(&db_dir, "1.1.0").expect("carried from 1.0.0");
        assert!(from.starts_with(&versions.join("1.0.0")), "carried from {from:?}");
        let conn = rusqlite::Connection::open(db_dir.join("objects.db")).unwrap();
        let v: String = conn.query_row("SELECT v FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(v, "from 1.0.0");
    }
}
