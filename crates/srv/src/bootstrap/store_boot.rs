// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps of [`open_stores_and_migrate`](super::open_stores_and_migrate) that prepare the data
//! folder, seed the first launch and install the key resolvers. Moved out of that function without
//! a change in behaviour.

use super::stores::prepare_in_process;
use super::*;
use crate::backend::memory::global_record::Recorder;

/// Sets the data folder up: environment, locks, the stamp and the startup diagnostics.
pub(super) fn prepare_data_dir(config: &config::Config, version: &str) {
    // Before anything is created: a data folder of its own must not mean the real shared folder.
    super::shared_root::guard_shared_folder(&config.data_home, version);

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
    // What this srv can do, beside the lock that names its pid, for a fork
    // slug migration's precondition (`slug_migration::stamp`).
    crate::backend::slug_migration::stamp::write(&base::get_mux_data_dir());

    // Startup diagnostics. `home` is the folder this instance's whole data lives under: the real
    // ~/.agentmux unless `AGENTMUX_HOME_OVERRIDE` names another (`task dev` always does).
    let home_override = std::env::var_os("AGENTMUX_HOME_OVERRIDE").filter(|v| !v.is_empty());
    tracing::info!(
        data_dir = %base::get_mux_data_dir().display(),
        db_dir = %base::get_mux_db_dir().display(),
        app_path = %config.app_path,
        instance_id = %config.instance_id,
        secret_store = crate::identity::secret_store::backend_name(),
        home = %home_override.as_ref().map(|h| h.to_string_lossy().into_owned()).unwrap_or_else(|| {
            agentmux_common::data_paths::agentmux_root().map(|p| p.display().to_string()).unwrap_or_default()
        }),
        home_isolated = home_override.is_some(),
        "backend directories initialized"
    );
}

/// Runs the read-only data gate under a supervisor, or prepares the data in this process.
pub(super) fn gate_or_prepare_data(mux_data_dir: &std::path::Path, db_dir: &std::path::Path) {
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

    let code_version =
        std::env::var("AGENTMUX_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string());
    // Under a supervisor that prepares data (the launcher sets
    // AGENTMUX_SUPERVISOR_PREPARES=1), srv never copies, snapshots or migrates
    // at boot (SPEC_FAST_STARTUP_UPGRADE_OWNS_MIGRATIONS_AND_UPDATES_2026_09_15
    // §10, S1c). A read-only check runs before anything writes the stores; if
    // the data needs preparing, srv says why and exits, and the launcher runs
    // `migrate --prepare` and starts it again. Elsewhere (headless srv,
    // `task dev:standalone`) srv still prepares in-process.
    let supervised =
        std::env::var(agentmux_common::srv_stderr::SUPERVISOR_PREPARES_ENV).as_deref() == Ok("1");
    if supervised {
        if let Some(reason) = crate::boot_timing::time("data_gate", || {
            migrations::data_needs_preparing(mux_data_dir, &code_version)
        }) {
            tracing::info!("startup: data needs preparing, exiting for the launcher: {reason}");
            eprintln!(
                "{}",
                agentmux_common::srv_stderr::needs_prepare_line(&reason)
            );
            std::process::exit(agentmux_common::srv_stderr::NEEDS_PREPARE_EXIT);
        }
    } else {
        prepare_in_process(mux_data_dir, db_dir, &code_version);
    }
}

/// Opens the per-channel object store.
pub(super) fn open_objects_db(db_dir: &std::path::Path) -> Store {
    crate::boot_timing::time("open_objects_db", || {
        Store::open(&db_dir.join("objects.db"))
    })
    .unwrap_or_else(|e| {
        tracing::error!("Failed to open object store: {}", e);
        std::process::exit(1);
    })
}

/// Creates the first launch's workspace and initial data.
pub(super) fn ensure_initial_data(mstore: &Arc<Store>) {
    // The user's workspace (SPEC_WORKSPACES_2026_10_23.md §4.2): Default
    // Workspace, or a new version's copy of the previous version's. Before the
    // initial data, which a copied workspace with windows replaces.
    let opened_workspace = crate::server::service::user_workspace::prepare_at_startup(
        mstore,
        crate::server::service::session_other_builds::ScanRoots::live().as_ref(),
    );
    // Bootstrap data (creates Client/Window/Workspace/Tab on first launch)
    let first_launch = wcore::ensure_initial_data(mstore).unwrap_or_else(|e| {
        tracing::error!("Failed to ensure initial data: {}", e);
        std::process::exit(1);
    });
    if first_launch {
        tracing::info!("First launch: created initial data");
        // Its window may offer a layout another build saved.
        crate::server::service::session_other_builds::remember_first_launch_default(mstore);
    }
    // A window whose close a quit outran: the session record restores it.
    crate::server::service::forget_windows_left_by_quit(mstore);
    crate::server::service::user_workspace::adopt_current(mstore, opened_workspace);

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
}

/// Flags interrupted sessions and runs the startup seeds.
pub(super) fn recover_and_seed(
    mstore: &Arc<Store>,
    id_store: &Arc<Store>,
    identity_store: &Arc<Store>,
    global_memory_recorder: &Option<Recorder>,
) {
    // Session recovery (Phase 4.2): scan for agent blocks that still have
    // `session:active_pid` from a previous run — those sessions were killed
    // by a crash/reboot. Transfer to `session:was_interrupted` so the
    // frontend can show a reconnect banner.
    let orphan_count = backend::blockcontroller::session_recovery::scan_orphans(mstore);
    if orphan_count > 0 {
        tracing::info!(
            orphan_count = orphan_count,
            "session_recovery: flagged {} interrupted sessions for user reconnect",
            orphan_count
        );
        backend::blockcontroller::session_recovery::note_interrupted(
            &base::get_mux_data_dir(),
            orphan_count,
            std::env::var(agentmux_common::incidents::RESTART_ID_ENV)
                .ok()
                .as_deref(),
        );
    }

    // Auto-seed agent definitions on first launch (or empty DB)
    crate::boot_timing::time("agent_auto_seed", || {
        backend::agent_seed::auto_seed_on_startup(mstore)
    });

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
    backend::operator_config_seed::auto_seed_on_startup(id_store);
    // Then record whatever the record doesn't have yet: the baseline the
    // first time, and what older builds changed since.
    if let Some(ref recorder) = global_memory_recorder {
        recorder.sync_all();
    }

    // No starter Skills are seeded: the migration that did is retired, and
    // 0041 removed the ones it had left in existing catalogs.
    // The skills AgentMux ships are a different thing: a few, each created once and then the
    // user's (`backend::builtin_skills`).
    backend::builtin_skills::seed_on_startup(identity_store);

    // Gap-repair (backfilling a db_agent_definitions row missing its
    // db_agents mirror) is gone as of the definition flip: every
    // agent_def_* write lands on db_agents directly now, so that gap can no
    // longer open. Removing the per-boot pass was required, not optional —
    // once agent_def_delete/instance_delete stopped also clearing
    // db_agent_definitions, a repair pass still reading it would have
    // resurrected every deleted agent on the next boot.
}

/// Lets the cross-instance agent registry publish each agent's public key.
pub(super) fn install_key_resolvers(mstore: &Arc<Store>) {
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
    // Identity M4d-5: the block's agent UID and that UID's public key, beside
    // the name-keyed one.
    crate::backend::reactive::registry::init_uid_key_resolver({
        let mstore = mstore.clone();
        move |block_id| {
            let uid = mstore
                .instance_get_active_for_block(block_id)
                .ok()
                .flatten()?
                .id;
            let key = mstore.agent_uid_lan_public_key_load(&uid).ok().flatten()?;
            Some((uid, key))
        }
    });
}
