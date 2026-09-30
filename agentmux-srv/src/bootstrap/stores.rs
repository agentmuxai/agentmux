// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Split out of bootstrap.rs unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.3).

use super::*;

/// Step 2: Parse CLI args and build config. Dispatches the `migrate`
/// subcommand (and exits) before touching AUTH_KEY-gated config.
pub fn load_config() -> config::Config {
    let args = CliArgs::parse();

    // Dispatch migrate subcommand before loading config (no AUTH_KEY needed).
    if let Some(config::SrvCommand::Migrate { dry_run, list, verify }) = &args.command {
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
        let code = migrations::run_migrate_command(&data_dir, *dry_run, *list, *verify);
        std::process::exit(code);
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

    // Set up data directory (uses AGENTMUX_DATA_HOME or default)
    if !config.data_home.as_os_str().is_empty() {
        std::env::set_var("AGENTMUX_DATA_HOME", &config.data_home);
    }
    if !config.config_home.as_os_str().is_empty() {
        std::env::set_var("AGENTMUX_CONFIG_HOME", &config.config_home);
    }
    if !config.app_path.is_empty() {
        std::env::set_var("AGENTMUX_APP_PATH", &config.app_path);
    }

    base::ensure_mux_data_dir().unwrap_or_else(|e| {
        tracing::error!("Failed to ensure data dir: {}", e);
        std::process::exit(1);
    });
    // One srv per set of databases, however it was started (headless already
    // took this in `headless::prepare_env`; that makes this a no-op).
    base::acquire_data_dir_lock(&base::get_mux_data_dir()).unwrap_or_else(|e| {
        tracing::error!("{e}");
        eprintln!("agentmux-srv: {e}");
        std::process::exit(1);
    });
    base::ensure_mux_db_dir().unwrap_or_else(|e| {
        tracing::error!("Failed to ensure db dir: {}", e);
        std::process::exit(1);
    });

    // Startup diagnostics
    tracing::info!(
        data_dir = %base::get_mux_data_dir().display(),
        db_dir = %base::get_mux_db_dir().display(),
        app_path = %config.app_path,
        instance_id = %config.instance_id,
        secret_store = crate::identity::secret_store::backend_name(),
        "backend directories initialized"
    );

    // Apply any pending migrations in-process before opening stores.
    // This ensures 0011_shared_store_backfill (and any future Global migrations)
    // have run before id_store binds to the shared store — avoiding apparent data
    // loss on first boot after an upgrade. Fast-path: no-op when already current.
    //
    // id_store binding below checks shared_store.migration_is_applied("0011_shared_store_backfill")
    // directly rather than a coarse migration_ok flag: if an early migration (e.g.
    // 0011) succeeds but a later one fails, the shared store is already backfilled
    // and safe to use — falling back to per-channel would strand writes made this
    // session when the later migration succeeds on next boot.
    let mux_data_dir = base::get_mux_data_dir();

    // Open databases
    let db_dir = base::get_mux_db_dir();

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
    let code_version = std::env::var("AGENTMUX_VERSION")
        .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string());
    // Snapshots live under the agentmux home root (sibling of `channels/`)
    // so they survive channel switches and aren't counted against any one
    // channel's data dir. Honor AGENTMUX_HOME_OVERRIDE for tests; else
    // default to the OS-level `~/.agentmux/`. Matches `resolve_root` in
    // agentmux-common — kept inline here to avoid threading the full
    // DataPaths plumbing into main.rs for one path.
    let snapshots_dir = std::env::var_os("AGENTMUX_HOME_OVERRIDE")
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(base::get_mux_data_dir)
        .join("snapshots");
    match maybe_snapshot_pre_migration(
        &db_dir,
        &snapshots_dir,
        &channel,
        &code_version,
        OBJECT_SCHEMA_VERSION,
    ) {
        Ok(Some(path)) => tracing::info!(snapshot = %path.display(), "pre-migration snapshot written"),
        Ok(None) => {}
        Err(e) => tracing::warn!("pre-migration snapshot failed (continuing without backup): {}", e),
    }

    // Run in-process migrations AFTER snapshot so the rollback aid captures
    // the pre-DDL state. count_pending_migrations emits AGENTMUXSRV-MIGRATING
    // first so the launcher/sidecar extend their ESTART deadline before the
    // (potentially slow) migration work begins.
    // Strict check, lenient reaction: this line only sizes the supervisors'
    // ESTART deadline, and the migration batch runs unconditionally right
    // below, so an unreadable store costs nothing here beyond a less precise
    // hint — `readable_pending()` reproduces exactly the number this call
    // site used before. It is logged rather than swallowed because the same
    // question becomes a real decision point once the boot gate lands:
    // `SPEC_FAST_STARTUP_UPGRADE_OWNS_MIGRATIONS_AND_UPDATES_2026_09_15.md`
    // invariant S1b requires that gate to fail closed, and a store that
    // cannot be read here is precisely the case it must catch.
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
        eprintln!("AGENTMUXSRV-MIGRATING migrations:{}", pre_migration_count);
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
            // (agentmux-common/src/srv_stderr.rs). Exit code 1 is what
            // docs/exe-return-codes.md already documented for this case.
            tracing::error!("startup: migration failed — refusing to start: {}", e);
            eprintln!("{}", agentmux_common::srv_stderr::migration_failed_line(&e));
            std::process::exit(1);
        }
    }

    let mstore_raw = Store::open(&db_dir.join("objects.db")).unwrap_or_else(|e| {
        tracing::error!("Failed to open object store: {}", e);
        std::process::exit(1);
    });
    // Attach the cross-version named-agent registry. Falls back to a
    // disabled registry when the shared home can't be resolved (CI,
    // unusual envs); mutations still hit SQLite, just don't mirror.
    // See docs/specs/SPEC_SHARED_AGENT_REGISTRY_2026_05_12.md.
    if let Some(root) = registry::resolve_shared_registry_dir() {
        match registry::Registry::open(root.clone()) {
            Ok(reg) => {
                tracing::info!(root = %root.display(), "registry: shared agent registry attached");
                // Best-effort catch-up pass, run every startup (not just the
                // one-shot m0010 migration): fills session_id for any agent
                // record that still lacks one, e.g. one named after m0010
                // already ran. Idempotent — skips records that already carry
                // a non-empty session_id. Without this, an agent created
                // after the migration ran would never get a resumable
                // session_id in the registry, so a cross-tab/cross-restart
                // open would silently orphan its conversation (the still-open
                // half of docs/retro/retro-cross-channel-conversation-continuity-regression-2026-06-16.md).
                if let Some(shared_dir) = root.parent().and_then(|p| p.parent()) {
                    let filled = crate::boot_timing::time("registry_session_backfill", || {
                        backend::session_backfill::backfill_session_ids(&reg, shared_dir)
                    });
                    if filled > 0 {
                        tracing::info!(filled, "registry: backfilled session_id for cross-channel resume (startup pass)");
                    }
                }
                mstore_raw.set_registry(Arc::new(reg));
                if let Some(base) = std::env::var_os("AGENTMUX_AGENTS_DIR") {
                    if !base.is_empty() {
                        mstore_raw
                            .set_registry_agents_base(std::path::PathBuf::from(base));
                    }
                }
            }
            Err(e) => tracing::warn!(
                root = %root.display(),
                error = %e,
                "registry: failed to open shared agent registry — SQLite remains authoritative"
            ),
        }
    } else {
        tracing::warn!("registry: could not resolve shared registry dir — mirror disabled");
    }
    // Attach the GLOBAL (cross-channel) agent-definition store. Sibling of the
    // instance registry above — since P0.3b both live under ~/.agentmux/shared/
    // (definitions/ and registry/), so user agents created in one channel are
    // visible in every channel. Best-effort: disabled when the shared dir can't
    // be resolved. See SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md (P0.2/P0.3).
    // Captured for the transcript backfill below (after the global transcript
    // store opens): the user-agent definition ids to seed conversations for.
    let mut backfill_def_ids: Vec<String> = Vec::new();
    if let Some(def_dir) = registry::resolve_shared_definitions_dir() {
        match registry::DefinitionStore::open(def_dir.clone()) {
            Ok(def_store) => {
                // Capture user-agent ids for the transcript backfill (below).
                backfill_def_ids = crate::boot_timing::time("def_store_list_active", || {
                    def_store
                        .list_active()
                        .map(|v| v.into_iter().map(|r| r.data.id).collect())
                        .unwrap_or_default()
                });
                mstore_raw.set_def_registry(Arc::new(def_store));
                tracing::info!(dir = %def_dir.display(), "def registry: global definition store attached");
            }
            Err(e) => tracing::warn!(
                dir = %def_dir.display(),
                error = %e,
                "def registry: failed to open global definition store — definitions stay channel-local"
            ),
        }
    } else {
        tracing::warn!("def registry: could not resolve shared definitions dir — global definitions disabled");
    }
    // Both host-global trees are attached now, so they can be reconciled
    // against each other. Best-effort catch-up pass run every startup, same
    // shape as the session_id backfill above: drops instance records whose
    // definition is tombstoned, which the picker would otherwise keep
    // rendering as an iconless "(missing definition)" row — a deleted agent
    // whose card never went away. New deletes sweep the registry themselves
    // (`Store::agent_def_delete`); this is for the orphans already on disk.
    if let (Some(reg), Some(defs)) = (
        mstore_raw.shared_agent_registry(),
        mstore_raw.shared_def_registry(),
    ) {
        let pruned = backend::registry_reconcile::prune_tombstoned_instance_records(&reg, &defs);
        if pruned > 0 {
            tracing::info!(pruned, "registry: dropped instance records for deleted agents (startup pass)");
        }
    }
    // Identity M4a: token → UID for request attribution, on this channel's
    // object store only (spec §6.5.3).
    match mstore_raw.attach_token_index() {
        Ok(_) => tracing::info!("identity: agent token index attached"),
        Err(e) => tracing::warn!(error = %e, "identity: agent token index unavailable — requests stay unattributed"),
    }
    // W3-S (SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md §2.2): the channel-wide
    // WAN identity store. Opened and its instance minted here, once, so every
    // spawn signs as the same instance. Best-effort and never redirected: if
    // the file can't be opened or the instance can't be minted, WAN signing
    // is simply off for this process (agents get no WAN key), which degrades
    // every WAN jekt to today's unsigned `TRUST=network-claimed`.
    match backend::storage::wan_identity::resolve_wan_identity_path() {
        Some(path) => match backend::storage::wan_identity::WanIdentityStore::open(&path).and_then(|store| {
            let instance = store.instance_ensure(&crate::backend::reactive::registry::local_host_label())?;
            Ok((store, instance))
        }) {
            Ok((store, instance)) => {
                tracing::info!(
                    path = %path.display(),
                    instance = %instance.instance_id,
                    host_hint = %instance.host_hint,
                    "wan identity: store attached"
                );
                let store = Arc::new(store);
                backend::storage::wan_identity::install_global(store.clone());
                mstore_raw.set_wan_identity(store);
            }
            Err(e) => tracing::warn!(
                path = %path.display(),
                error = %e,
                "wan identity: store unavailable — WAN jekt signing is off for this process"
            ),
        },
        None => tracing::warn!("wan identity: channel dir unresolved — WAN jekt signing is off for this process"),
    }
    let mstore = Arc::new(mstore_raw);
    // Identity M4a: lets spawn sites tell a row-backed block from a row-less
    // one when the environment carries no UID.
    backend::identity_spawn::attach_row_store(mstore.clone());
    let filestore = Arc::new(FileStore::open(&db_dir.join("filestore.db")).unwrap_or_else(|e| {
        tracing::error!("Failed to open file store: {}", e);
        std::process::exit(1);
    }));
    // GLOBAL transcript store — backs the `agent:<defId>:current` zone so a
    // conversation loads when the agent is opened from any build/channel
    // (finishes the cross-channel arc #1387–#1396). A second FileStore over an
    // independent SQLite/WAL file is safe alongside the per-channel one. This is
    // best-effort: if the shared root can't be resolved, or the store can't be
    // opened, we log and fall back to the per-channel `filestore` (global
    // transcripts disabled) — never fatal, unlike the per-channel store above.
    // See `docs/analysis/ANALYSIS_CROSS_CHANNEL_CONVERSATION_HISTORY_2026_06_14.md`.
    let global_transcript_store: Option<Arc<FileStore>> =
        match registry::resolve_shared_transcripts_dir() {
            Some(dir) => {
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    tracing::warn!(dir = %dir.display(), error = %e, "global transcripts: failed to create dir — disabled, falling back to per-channel store");
                    None
                } else {
                    match FileStore::open(&dir.join("filestore.db")) {
                        Ok(fs) => {
                            tracing::info!(dir = %dir.display(), "global transcripts: store attached");
                            Some(Arc::new(fs))
                        }
                        Err(e) => {
                            tracing::warn!(dir = %dir.display(), error = %e, "global transcripts: failed to open store — disabled, falling back to per-channel store");
                            None
                        }
                    }
                }
            }
            None => {
                tracing::warn!("global transcripts: could not resolve shared transcripts dir — disabled, falling back to per-channel store");
                None
            }
        };
    // GLOBAL shared store — identity accounts, memory bundles, drone
    // definitions, MuxBus credentials. Best-effort: disabled when the shared
    // root can't be resolved. Falls back to mstore so behavior is unchanged
    // from today. See SPEC_GLOBAL_IDENTITY_MEMORY_DRONE_2026_06_24.md.
    let shared_store: Option<Arc<Store>> = match registry::resolve_shared_store_path() {
        Some(path) => {
            // Ensure the parent dir (shared/) exists before opening.
            let parent = path.parent().unwrap_or_else(|| path.as_path());
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::warn!(path = %path.display(), error = %e, "shared store: failed to create dir — disabled");
                None
            } else {
                match Store::open_shared(&path) {
                    Ok(s) => {
                        let reason = agentmux_common::isolated_auth_reason();
                        if reason.is_isolated() {
                            tracing::info!(path = %path.display(), reason = reason.as_str(), "shared store: attached (ISOLATED — channel-scoped)");
                        } else {
                            tracing::info!(path = %path.display(), reason = reason.as_str(), "shared store: attached");
                        }
                        Some(Arc::new(s))
                    }
                    Err(e) => {
                        tracing::warn!(path = %path.display(), error = %e, "shared store: failed to open — identity/memory/drone/muxbus stay per-channel");
                        None
                    }
                }
            }
        }
        None => {
            tracing::warn!("shared store: could not resolve shared store path — disabled");
            None
        }
    };
    // id_store: routes identity/memory/drone/muxbus ops to the shared store when
    // available AND the shared-store backfill migration has been applied.
    // Checking the specific migration (not a coarse migration_ok flag) avoids
    // a split-brain when 0011 succeeded but a later migration failed: the shared
    // store is already backfilled, so using per-channel would strand writes made
    // this session once the later migration succeeds on next boot.
    let id_store: Arc<Store> = match shared_store.as_ref() {
        Some(ss) if ss.migration_is_applied("0011_shared_store_backfill") => ss.clone(),
        Some(_) => {
            tracing::warn!("id_store: 0011_shared_store_backfill not yet applied — using per-channel store");
            mstore.clone()
        }
        None => mstore.clone(),
    };

    // Permanently-global identity store (agent→account links, memory
    // bundles, drone definitions, muxbus/agent M2M credentials, native
    // memory, cron jobs) — see
    // docs/specs/SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md. Deliberately NOT
    // gated by isolated_auth_enabled() at all, unlike shared_store above:
    // none of this store's tables have a legitimate per-channel-isolation
    // use case. Best-effort, same fallback-to-mstore terms as id_store —
    // a resolution/open failure degrades to today's per-channel behavior
    // rather than being fatal.
    let identity_store: Arc<Store> = match registry::resolve_identity_store_path() {
        Some(path) => {
            let parent = path.parent().unwrap_or(path.as_path());
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::warn!(path = %path.display(), error = %e, "identity store: failed to create dir — falling back to per-channel store");
                mstore.clone()
            } else {
                match Store::open_identity_store(&path) {
                    Ok(s) => {
                        tracing::info!(path = %path.display(), "identity store: attached (always global)");
                        Arc::new(s)
                    }
                    Err(e) => {
                        tracing::warn!(path = %path.display(), error = %e, "identity store: failed to open — falling back to per-channel store");
                        mstore.clone()
                    }
                }
            }
        }
        None => {
            tracing::warn!("identity store: could not resolve path — falling back to per-channel store");
            mstore.clone()
        }
    };
    // #3603: memory resolution finds the account an agent is linked to (the
    // directory its spawn runs Claude in) through these, read-only.
    crate::server::native_memory_handlers::attach_identity_stores(id_store.clone(), identity_store.clone());

    // Install the process-global handle so the block-controller stdout-reader
    // hot path can mirror agent `output` into the global zone without threading
    // the store through `resync_controller` and every controller constructor.
    // Global Memory's own record (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md
    // §2.1.6): write-through from id_store, attached before this start's
    // seeding so seeded entries keep their seeder's attribution.
    let global_memory_recorder = global_transcript_store
        .as_ref()
        .map(|fs| crate::backend::global_memory_record::attach(fs.clone(), id_store.clone()));
    if let Some(ref fs) = global_transcript_store {
        crate::backend::agent_session::set_global_transcript_store(fs.clone());
        // Heal global snapshots poisoned before the normalize-on-mirror fix: a
        // channel-local `sourceBlockId` mirrored into the global zone makes a
        // cross-channel open render empty (the read fallback can't anchor a block
        // that doesn't exist in the opening channel). Idempotent + cheap. See
        // docs/retro/retro-legacy-agent-history-cross-channel-2026-06-16.md.
        let healed = crate::boot_timing::time("transcript_snapshot_heal", || {
            backend::agent_session::heal_global_snapshot_source_block_ids(fs, &backfill_def_ids)
        });
        if healed > 0 {
            tracing::info!(healed, "global transcripts: healed poisoned snapshot sourceBlockIds");
        }
    }

    // Saga durability — see SPEC_SAGA_DURABILITY_2026-05-01.md.
    // Backed by its own SQLite file (`sagas.db`) so saga writes
    // commit independently of the mstore connection. Failure here
    // is fatal: without the log, a srv crash mid-saga leaves
    // unrecoverable state divergence.
    let saga_log = Arc::new(
        crate::sagas::log::SagaLog::open(&db_dir.join("sagas.db")).unwrap_or_else(|e| {
            tracing::error!("Failed to open saga log: {}", e);
            std::process::exit(1);
        }),
    );
    // Seed `saga_id_alloc` from the highest persisted saga_id so
    // restarts don't reuse IDs from prior runs (reagent P1 + codex
    // P1 PR #631). With this seed + the plain INSERT (no OR REPLACE)
    // in `start_saga`, ID collisions become impossible by
    // construction.
    let saga_id_seed = crate::boot_timing::time("saga_id_seed_scan", || saga_log.max_saga_id()).unwrap_or_else(|e| {
        tracing::warn!(
            "[saga] failed to read MAX(saga_id) for allocator seed: {} — defaulting to 0; ID collisions on restart possible until next successful query",
            e
        );
        0
    });
    if saga_id_seed > 0 {
        tracing::info!(
            "[saga] seeded saga_id_alloc from durable log: next saga_id = {}",
            saga_id_seed + 1
        );
    }

    // Bootstrap data (creates Client/Window/Workspace/Tab on first launch)
    let first_launch = wcore::ensure_initial_data(&mstore).unwrap_or_else(|e| {
        tracing::error!("Failed to ensure initial data: {}", e);
        std::process::exit(1);
    });
    if first_launch {
        tracing::info!("First launch: created initial data");
    }

    // Seed ~/.agentmux/.gitignore so accidental git operations inside the
    // data directory (e.g. an agent running `git init` or `git clone` in its
    // cwd) don't stage anything by default. Idempotent — written once per
    // install; we don't overwrite an existing user-customized file.
    if let Some(home) = dirs::home_dir() {
        let data_dir = home.join(".agentmux");
        if data_dir.is_dir() {
            let gitignore = data_dir.join(".gitignore");
            if !gitignore.exists() {
                let _ = std::fs::write(&gitignore, "*\n!.gitignore\n");
            }
        }
    }

    // Session recovery (Phase 4.2): scan for agent blocks that still have
    // `session:active_pid` from a previous run — those sessions were killed
    // by a crash/reboot. Transfer to `session:was_interrupted` so the
    // frontend can show a reconnect banner.
    let orphan_count = backend::blockcontroller::session_recovery::scan_orphans(&mstore);
    if orphan_count > 0 {
        tracing::info!(
            orphan_count = orphan_count,
            "session_recovery: flagged {} interrupted sessions for user reconnect",
            orphan_count
        );
    }

    // Auto-seed agent definitions on first launch (or empty DB)
    crate::boot_timing::time("agent_auto_seed", || backend::agent_seed::auto_seed_on_startup(&mstore));

    // Keep AgentMux's own Operator Config (is_system=1 Global Memory) in
    // sync with the shipped manifest on every startup — not a one-time
    // seed, since stale Operator Config content would mislead an agent
    // rather than merely being absent. See
    // docs/specs/SPEC_SYSTEM_TIER_GLOBAL_MEMORY_SEEDING_2026_09_15.md.
    //
    // Seeded into id_store, NOT mstore: Global Memory reads/writes
    // (agent_open.rs's bundle_list_global, bundle.rs's handlers via
    // state.id_store) all route through id_store, which points at the
    // shared store once 0011_shared_store_backfill has run and only falls
    // back to mstore before that. Seeding mstore directly would leave these
    // rows invisible under the normal shared-store configuration once that
    // migration has applied. Codex P1, PR #3244.
    backend::operator_config_seed::auto_seed_on_startup(&id_store);
    // Then record whatever the record doesn't have yet: the baseline the
    // first time, and what older builds changed since.
    if let Some(ref recorder) = global_memory_recorder {
        recorder.sync_all();
    }

    // The starter Skills catalog is seeded by migrations::m0015_seed_starter_skills
    // (run once ever per channel, tracked in db_migrations) — not here. See
    // that migration's doc comment for why catalog-emptiness was dropped as
    // the gate.

    // Gap-repair (backfilling a db_agent_definitions row missing its
    // db_agents mirror) is gone as of the definition flip: every
    // agent_def_* write lands on db_agents directly now, so that gap can no
    // longer open. Removing the per-boot pass was required, not optional —
    // once agent_def_delete/instance_delete stopped also clearing
    // db_agent_definitions, a repair pass still reading it would have
    // resurrected every deleted agent on the next boot.

    // Let the cross-instance agent registry publish each agent's Ed25519
    // PUBLIC key alongside its entry, so a peer instance in a *different
    // channel* can verify that agent's jekts — the one same-machine tier that
    // had no verification path at all before
    // `SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md`. Host-tier's HMAC key is
    // symmetric and deliberately NOT published (spec §5).
    //
    // Installed here rather than next to `init_local_auth_key` in
    // `load_config` because that runs before any store is open, and this is a
    // per-agent lookup rather than a process constant. Registry writes before
    // this point simply publish an empty key, which readers treat as "cannot
    // check."
    crate::backend::reactive::registry::init_jekt_public_key_resolver({
        let mstore = mstore.clone();
        move |agent_id| mstore.agent_lan_public_key_load(agent_id).ok().flatten()
    });

    Stores {
        mstore,
        filestore,
        global_transcript_store,
        shared_store,
        id_store,
        identity_store,
        saga_log,
        saga_id_seed,
    }
}
