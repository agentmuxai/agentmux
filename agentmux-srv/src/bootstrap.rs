// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Server bootstrap sequence, extracted from `main()`.
//!
//! Each function here corresponds to one (or a small cluster of) the
//! numbered phases that used to live inline in `async fn main()`:
//! crash-monitor branch -> parent-process watchdog -> logging init ->
//! PATH enrichment -> CLI/config parsing -> data-dir + migration setup ->
//! DB/store opening -> event bus + subagent watcher + background task
//! spawns -> TCP listener binds -> router/AppState build -> ESTART stderr
//! emission -> stdin-watch thread -> SIGINT/SIGTERM handler.
//!
//! This module is purely a relocation of that logic into named functions;
//! `main()` calls them in the same order with the same effect.

use std::sync::Arc;

use clap::Parser;
use tokio::net::TcpListener;

use crate::backend;
use crate::backend::eventbus::EventBus;
use crate::backend::reactive::{self, Poller, PollerConfig};
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::migrations::OBJECT_SCHEMA_VERSION;
use crate::backend::storage::snapshot::maybe_snapshot_pre_migration;
use crate::backend::storage::store::Store;
use crate::backend::wps::Broker;
use crate::backend::wconfig;
use crate::backend::{base, docsite, sysinfo, wcore};
use crate::config::{self, CliArgs};
use crate::event_log;
use crate::messaging;
use crate::migrations;
use crate::persist;
use crate::persist_subscriber;
use crate::registry;
use crate::server::{self, AppState};
use crate::state;

/// Start a ppid polling watchdog on Linux/macOS.
/// If the parent process dies, getppid() changes (reparented to init/launchd).
/// This is safer than PR_SET_PDEATHSIG which tracks the parent *thread*, not process,
/// and can fire spuriously with async runtimes like Tokio.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn start_ppid_watchdog() {
    let original_ppid = unsafe { libc::getppid() };
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let current_ppid = unsafe { libc::getppid() };
            if current_ppid != original_ppid {
                eprintln!(
                    "parent process died (ppid changed {} -> {}), shutting down",
                    original_ppid, current_ppid
                );
                std::process::exit(0);
            }
        }
    });
}

/// Event-driven parent process watcher using kqueue (macOS) or pidfd (Linux).
/// Monitors a specific PID and exits when that process terminates.
/// Falls back to PPID polling on older Linux kernels without pidfd support.
#[cfg(target_os = "macos")]
fn start_parent_watcher(parent_pid: u32) {
    std::thread::spawn(move || {
        unsafe {
            let kq = libc::kqueue();
            if kq < 0 {
                eprintln!(
                    "kqueue() failed (errno={}), falling back to ppid watchdog",
                    *libc::__error()
                );
                let _ = kq;
                start_ppid_watchdog();
                return;
            }

            // Register EVFILT_PROC + NOTE_EXIT on the parent PID.
            let mut changelist: [libc::kevent; 1] = std::mem::zeroed();
            changelist[0] = libc::kevent {
                ident: parent_pid as usize,
                filter: libc::EVFILT_PROC,
                flags: libc::EV_ADD | libc::EV_ONESHOT,
                fflags: libc::NOTE_EXIT,
                data: 0,
                udata: std::ptr::null_mut(),
            };

            let ret = libc::kevent(
                kq,
                changelist.as_ptr(),
                1,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            );

            if ret < 0 {
                let errno = *libc::__error();
                libc::close(kq);
                if errno == libc::ESRCH {
                    // Parent already dead
                    eprintln!(
                        "parent process {} already exited (ESRCH during kqueue registration), shutting down",
                        parent_pid
                    );
                    std::process::exit(0);
                }
                eprintln!(
                    "kevent() registration failed (errno={}), falling back to ppid watchdog",
                    errno
                );
                start_ppid_watchdog();
                return;
            }

            eprintln!("kqueue EVFILT_PROC registered for parent pid {}", parent_pid);

            // Race condition guard: check if the parent is still alive after registering.
            // If it died between our registration and this check, we might miss the event.
            if libc::kill(parent_pid as i32, 0) != 0 && *libc::__error() == libc::ESRCH {
                libc::close(kq);
                eprintln!(
                    "parent process {} already exited (post-registration check), shutting down",
                    parent_pid
                );
                std::process::exit(0);
            }

            // Block until the parent exits.
            let mut eventlist: [libc::kevent; 1] = std::mem::zeroed();
            let n = libc::kevent(
                kq,
                std::ptr::null(),
                0,
                eventlist.as_mut_ptr(),
                1,
                std::ptr::null(),
            );
            libc::close(kq);

            if n > 0 {
                eprintln!(
                    "parent process {} exited (kqueue EVFILT_PROC), shutting down",
                    parent_pid
                );
            } else {
                eprintln!(
                    "kevent() wait returned {} (errno={}), shutting down",
                    n,
                    *libc::__error()
                );
            }
            std::process::exit(0);
        }
    });
}

/// Event-driven parent process watcher using pidfd_open (Linux 5.3+).
/// Falls back to PPID polling on older kernels without pidfd support.
#[cfg(target_os = "linux")]
fn start_parent_watcher(parent_pid: u32) {
    std::thread::spawn(move || {
        unsafe {
            // Try pidfd_open (syscall 434 on x86_64, 434 on aarch64)
            let pidfd = libc::syscall(libc::SYS_pidfd_open, parent_pid as libc::c_int, 0 as libc::c_int);

            if pidfd < 0 {
                let errno = *libc::__errno_location();
                if errno == libc::ESRCH {
                    // Parent already dead
                    eprintln!(
                        "parent process {} already exited (ESRCH from pidfd_open), shutting down",
                        parent_pid
                    );
                    std::process::exit(0);
                }
                // ENOSYS means kernel doesn't support pidfd_open — fall back
                eprintln!(
                    "pidfd_open() failed (errno={}), falling back to ppid watchdog",
                    errno
                );
                start_ppid_watchdog();
                return;
            }

            let pidfd = pidfd as libc::c_int;

            // Race condition guard: verify parent is still alive
            if libc::kill(parent_pid as i32, 0) != 0 && *libc::__errno_location() == libc::ESRCH {
                libc::close(pidfd);
                eprintln!(
                    "parent process {} already exited (post-pidfd check), shutting down",
                    parent_pid
                );
                std::process::exit(0);
            }

            // poll() on the pidfd — blocks until the process exits
            let mut pfd = libc::pollfd {
                fd: pidfd,
                events: libc::POLLIN,
                revents: 0,
            };

            let ret = libc::poll(&mut pfd, 1, -1); // infinite timeout
            libc::close(pidfd);

            if ret > 0 {
                eprintln!(
                    "parent process {} exited (pidfd poll), shutting down",
                    parent_pid
                );
            } else {
                eprintln!(
                    "poll() on pidfd returned {} (errno={}), shutting down",
                    ret,
                    *libc::__errno_location()
                );
            }
            std::process::exit(0);
        }
    });
}

/// Step -1: Crash monitor branch — must be checked before any other initialization.
/// The monitor process runs a blocking minidumper::Server and exits when the
/// main process disconnects. It does not run any backend logic.
///
/// Returns `true` if this process was the crash-monitor branch (in which case
/// `main()` must return immediately without doing anything else).
pub fn maybe_run_crash_monitor() -> bool {
    #[cfg(windows)]
    {
        if std::env::args().any(|a| a == "--crash-monitor") {
            crate::crash_monitor::run_monitor();
            return true;
        }
    }
    false
}

/// Step 0: Start parent process watcher BEFORE tokio runtime does real work (Linux/macOS only).
/// On Windows, the frontend uses a Job Object with KILL_ON_JOB_CLOSE instead.
/// Uses getppid() to get the parent PID, then kqueue/pidfd to watch it (event-driven,
/// zero CPU). Falls back to PPID polling if kqueue/pidfd setup fails or parent is init/launchd.
pub fn install_process_watchers() {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let ppid = unsafe { libc::getppid() } as u32;
        if ppid <= 1 {
            // Parent is init/launchd — can't meaningfully watch it, use polling fallback
            start_ppid_watchdog();
        } else {
            start_parent_watcher(ppid);
        }
    }
}

/// Step 0b: Attach out-of-process crash dump handler (Windows only).
/// Spawns self with --crash-monitor and installs a VEH handler.
/// The returned guard must stay alive — dropping it uninstalls the VEH handler.
/// Non-fatal: if the monitor fails to start, the process continues normally
/// and WER LocalDumps still captures __fastfail crashes independently.
#[cfg(windows)]
pub fn install_crash_guard() -> Option<crate::crash_monitor::CrashHandlerGuard> {
    crate::crash_monitor::spawn_and_attach()
}

/// Initialize tracing with dual output: JSON rolling file + human-readable stderr.
/// Returns a guard that must be held for the lifetime of the app to ensure log flushing.
pub fn init_logging() -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter};

    // Always log to ~/.agentmux/logs/ so all logs (host + sidecar) land in one
    // discoverable directory. AGENTMUX_DATA_HOME controls the data dir, not logs.
    // Version is embedded in the filename for side-by-side coexistence.
    let version = env!("CARGO_PKG_VERSION");
    let log_dir = dirs::home_dir()
        .unwrap_or_default()
        .join(".agentmux")
        .join("logs");
    let _ = std::fs::create_dir_all(&log_dir);

    // Delete log files older than 7 days to prevent unbounded growth.
    cleanup_old_logs(&log_dir, 7);

    // Rolling daily log file with JSON structured output
    let log_prefix = format!("agentmuxsrv-v{}.log", version);
    let file_appender = tracing_appender::rolling::daily(&log_dir, &log_prefix);
    let (non_blocking_file, guard) = tracing_appender::non_blocking(file_appender);

    // Write pointer to current log file for zero-lookup agent discovery.
    // Version-qualified name so multi-instance doesn't clobber pointers.
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let current_filename = format!("{}.{}", log_prefix, today);
    let pointer_name = format!("current-srv-v{}.path", version);
    let _ = std::fs::write(log_dir.join(&pointer_name), &current_filename);

    // Spawn a background thread to refresh the pointer on UTC date rollover.
    // tracing_appender::rolling::daily creates a new file at midnight UTC.
    {
        let log_dir = log_dir.clone();
        let log_prefix = log_prefix.clone();
        let pointer_name = pointer_name.clone();
        std::thread::Builder::new()
            .name("srv-log-pointer".into())
            .spawn(move || {
                let mut last_date = chrono::Utc::now().format("%Y-%m-%d").to_string();
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(60));
                    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
                    if last_date != today {
                        last_date = today.clone();
                        let filename = format!("{}.{}", log_prefix, today);
                        let _ = std::fs::write(log_dir.join(&pointer_name), &filename);
                    }
                }
            })
            .ok();
    }

    let subscriber = tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("agentmuxsrv=info,info")),
        )
        .with(
            fmt::layer()
                .json()
                .with_writer(non_blocking_file)
                .with_target(true)
                .with_thread_ids(true),
        )
        .with(
            fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(true),
        );

    tracing::subscriber::set_global_default(subscriber).ok();

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        log_dir = %log_dir.display(),
        "agentmuxsrv starting"
    );

    guard
}

/// Delete log files (*.log.*) older than `days` to prevent unbounded growth.
/// Only touches files with `.log.` in the name — pointer files and other data are safe.
fn cleanup_old_logs(log_dir: &std::path::Path, days: u64) {
    let cutoff = std::time::SystemTime::now()
        - std::time::Duration::from_secs(days * 86400);
    let Ok(entries) = std::fs::read_dir(log_dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.to_string_lossy().contains(".log.") {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            if let Ok(modified) = meta.modified() {
                if modified < cutoff {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }
}

/// Step 1b: Direct-launch PATH fallback. The host enriches the srv's PATH when it
/// spawns it (sidecar.rs), so in normal operation this is a cheap no-op.
/// It only does work when the srv is launched directly with a stripped
/// launchd PATH (some dev paths), so installs/CLIs still resolve node/npm.
/// See SPEC_TOOLCHAIN_MANAGER_2026-06-15 §3.1.
pub fn enrich_path() {
    let path_source = agentmux_common::enrich_current_process_path();
    if path_source != agentmux_common::PathSource::Inherited {
        // Record the source for the Toolchain modal (the host sets this when it
        // spawns the srv; on a direct launch we set it here after enriching).
        std::env::set_var("AGENTMUX_PATH_SOURCE", path_source.as_str());
        tracing::info!(
            source = path_source.as_str(),
            "Enriched srv PATH on direct launch (stripped PATH detected)"
        );
    }
}

/// Step 2: Parse CLI args and build config. Dispatches the `migrate`
/// subcommand (and exits) before touching AUTH_KEY-gated config.
pub fn load_config() -> config::Config {
    let args = CliArgs::parse();

    // Dispatch migrate subcommand before loading config (no AUTH_KEY needed).
    if let Some(config::SrvCommand::Migrate { dry_run, list }) = &args.command {
        let data_dir: std::path::PathBuf = args.wavedata
            .as_deref()
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var("AGENTMUX_DATA_HOME").ok().map(std::path::PathBuf::from))
            .unwrap_or_else(|| std::path::PathBuf::from(base::get_wave_data_dir()));
        let code = migrations::run_migrate_command(&data_dir, *dry_run, *list);
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
    pub wstore: Arc<Store>,
    pub filestore: Arc<FileStore>,
    pub global_transcript_store: Option<Arc<FileStore>>,
    pub shared_store: Option<Arc<Store>>,
    pub id_store: Arc<Store>,
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
    if !config.data_home.is_empty() {
        std::env::set_var("AGENTMUX_DATA_HOME", &config.data_home);
    }
    if !config.config_home.is_empty() {
        std::env::set_var("AGENTMUX_CONFIG_HOME", &config.config_home);
    }
    if !config.app_path.is_empty() {
        std::env::set_var("AGENTMUX_APP_PATH", &config.app_path);
    }

    base::ensure_wave_data_dir().unwrap_or_else(|e| {
        tracing::error!("Failed to ensure data dir: {}", e);
        std::process::exit(1);
    });
    base::ensure_wave_db_dir().unwrap_or_else(|e| {
        tracing::error!("Failed to ensure db dir: {}", e);
        std::process::exit(1);
    });

    // Startup diagnostics
    tracing::info!(
        data_dir = %base::get_wave_data_dir().display(),
        db_dir = %base::get_wave_db_dir().display(),
        app_path = %config.app_path,
        instance_id = %config.instance_id,
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
    let wave_data_dir = base::get_wave_data_dir();

    // Open databases
    let db_dir = base::get_wave_db_dir();

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
        .unwrap_or_else(|| base::get_home_dir().join(".agentmux"))
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
    let pre_migration_count = migrations::count_pending_migrations(&wave_data_dir);
    if pre_migration_count > 0 {
        eprintln!("AGENTMUXSRV-MIGRATING migrations:{}", pre_migration_count);
    }
    match migrations::run_pending_migrations(&wave_data_dir) {
        Ok(0) => {}
        Ok(n) => tracing::info!(applied = n, "startup: applied pending migrations"),
        Err(e) => tracing::warn!("startup: migration error (continuing): {}", e),
    }

    let wstore_raw = Store::open(&db_dir.join("objects.db")).unwrap_or_else(|e| {
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
                    let filled = backend::session_backfill::backfill_session_ids(&reg, shared_dir);
                    if filled > 0 {
                        tracing::info!(filled, "registry: backfilled session_id for cross-channel resume (startup pass)");
                    }
                }
                wstore_raw.set_registry(Arc::new(reg));
                if let Some(base) = std::env::var_os("AGENTMUX_AGENTS_DIR") {
                    if !base.is_empty() {
                        wstore_raw
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
                backfill_def_ids = def_store
                    .list_active()
                    .map(|v| v.into_iter().map(|r| r.data.id).collect())
                    .unwrap_or_default();
                wstore_raw.set_def_registry(Arc::new(def_store));
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
    let wstore = Arc::new(wstore_raw);
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
    // root can't be resolved. Falls back to wstore so behavior is unchanged
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
            wstore.clone()
        }
        None => wstore.clone(),
    };

    // Install the process-global handle so the block-controller stdout-reader
    // hot path can mirror agent `output` into the global zone without threading
    // the store through `resync_controller` and every controller constructor.
    if let Some(ref fs) = global_transcript_store {
        crate::backend::agent_session::set_global_transcript_store(fs.clone());
        // Heal global snapshots poisoned before the normalize-on-mirror fix: a
        // channel-local `sourceBlockId` mirrored into the global zone makes a
        // cross-channel open render empty (the read fallback can't anchor a block
        // that doesn't exist in the opening channel). Idempotent + cheap. See
        // docs/retro/retro-legacy-agent-history-cross-channel-2026-06-16.md.
        let healed =
            backend::agent_session::heal_global_snapshot_source_block_ids(fs, &backfill_def_ids);
        if healed > 0 {
            tracing::info!(healed, "global transcripts: healed poisoned snapshot sourceBlockIds");
        }
    }

    // Saga durability — see SPEC_SAGA_DURABILITY_2026-05-01.md.
    // Backed by its own SQLite file (`sagas.db`) so saga writes
    // commit independently of the wstore connection. Failure here
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
    let saga_id_seed = saga_log.max_saga_id().unwrap_or_else(|e| {
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
    let first_launch = wcore::ensure_initial_data(&wstore).unwrap_or_else(|e| {
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
    let orphan_count = backend::blockcontroller::session_recovery::scan_orphans(&wstore);
    if orphan_count > 0 {
        tracing::info!(
            orphan_count = orphan_count,
            "session_recovery: flagged {} interrupted sessions for user reconnect",
            orphan_count
        );
    }

    // Auto-seed agent definitions on first launch (or empty DB)
    backend::agent_seed::auto_seed_on_startup(&wstore);

    // The starter Skills catalog is seeded by migrations::m0015_seed_starter_skills
    // (run once ever per channel, tracked in db_migrations) — not here. See
    // that migration's doc comment for why catalog-emptiness was dropped as
    // the gate.

    // Gap-repair: backfill definitions written after the Phase 3a marker
    // but before Phase 3b dual-write (they exist in db_agent_definitions
    // but not in db_agents, making them invisible to Phase 3b readers).
    match wstore.repair_agent_def_gaps() {
        Ok(0) => {}
        Ok(n) => {
            tracing::info!(count = n, "agents_consolidate: gap-repair backfilled missing definitions");
        }
        Err(e) => {
            tracing::warn!(error = %e, "agents_consolidate: gap-repair failed (non-fatal)");
        }
    }

    Stores {
        wstore,
        filestore,
        global_transcript_store,
        shared_store,
        id_store,
        saga_log,
        saga_id_seed,
    }
}

/// Output of [`spawn_background_subsystems`] — event/watcher infrastructure
/// that needs no `AppState` and can start before it is built.
pub struct BackgroundSubsystems {
    pub event_bus: Arc<EventBus>,
    pub broker: Arc<Broker>,
    pub editor_file_watcher: Arc<backend::editor_file_watcher::EditorFileWatcher>,
    pub media_file_watcher: Arc<backend::media_file_watcher::MediaFileWatcher>,
    pub fs_watch_pool: Arc<backend::fs_watch::FsWatchPool>,
    pub config_watcher: Arc<wconfig::ConfigWatcher>,
    pub reactive_handler: &'static reactive::ReactiveHandler,
    pub poller: Arc<Poller>,
    pub messagebus: Arc<backend::messagebus::MessageBus>,
    pub subagent_watcher: Arc<backend::subagent_watcher::SubagentWatcher>,
    pub history_service: Arc<backend::history::HistoryService>,
}

/// Event infrastructure + all background task spawns that don't need
/// `AppState` to exist yet: event bus/broker, editor file watcher, config
/// watcher, sysinfo/watchdog/activity loops, the reactive handler + poller,
/// the cloud push subscriber, messaging bridges (Discord/Telegram/Slack/
/// WhatsApp), docsite dir, messagebus, subagent watcher, history service,
/// and the session archiver.
pub fn spawn_background_subsystems(
    wstore: &Arc<Store>,
    filestore: &Arc<FileStore>,
    id_store: &Arc<Store>,
) -> BackgroundSubsystems {
    // Event infrastructure
    let event_bus = Arc::new(EventBus::new());
    let broker = Arc::new(Broker::new());

    // Bridge WPS events to WebSocket clients via EventBus
    let bridge = backend::eventbus::EventBusBridge::new(event_bus.clone());
    broker.set_client(Box::new(bridge));

    // Shared filesystem-watcher framework — constructed once, before any
    // consumer, all three of which now build on it (see
    // SPEC_SHARED_FS_WATCHER_FRAMEWORK_2026_08_07.md §5's migration order:
    // config_watcher_fs first, editor+media here, native memory as a future
    // consumer).
    let fs_watch_pool = backend::fs_watch::FsWatchPool::new();

    // Watches files open in editor/preview panes, publishing a per-block
    // wake signal on external changes. See SPEC_EDITOR_LIVE_FILE_RELOAD_2026_07_18.md.
    let editor_file_watcher = backend::editor_file_watcher::EditorFileWatcher::new(fs_watch_pool.clone(), broker.clone());

    // Watches directories a Media pane is pointed at, publishing a per-block
    // wake signal when a matching-extension file changes. See
    // SPEC_MEDIA_PANE_2026_07_26.md.
    let media_file_watcher = backend::media_file_watcher::MediaFileWatcher::new(fs_watch_pool.clone(), broker.clone());

    // Deploy shell integration scripts (muxlog.mjs/muxspect.mjs + rcfiles)
    // unconditionally at startup, not opportunistically from a specific
    // controller's spawn path. Previously the only call site was
    // `ShellController::start`'s interactive (empty-command) branch
    // (`blockcontroller/shell/lifecycle.rs`) — a user whose first-ever pane
    // in a fresh data dir is an Agent pane (persistent/subprocess/acp, which
    // never hits that branch) would never get the scripts deployed at all,
    // so even muxspect's own documented `node ~/.agentmux/shell/muxspect.mjs`
    // direct-path fallback would ENOENT (codex P1 on PR #2380 — a latent gap
    // in muxlog's identical deployment mechanism that PR newly depends on).
    // `deploy_scripts` is idempotent (skips if its version marker already
    // matches), so calling it here in addition to the existing call site is
    // safe, not a double-write race.
    backend::shellintegration::deploy_scripts(&backend::base::get_home_dir().join(".agentmux"));

    // Config watcher (created before sysinfo loop so it can read telemetry:interval)
    let config_watcher = Arc::new(wconfig::ConfigWatcher::with_config(wconfig::build_default_config()));

    // Load user's settings.json from disk (merges with defaults)
    backend::config_watcher_fs::load_settings_from_disk(&config_watcher);

    // Watch settings.json for changes and broadcast to WebSocket clients
    backend::config_watcher_fs::spawn_settings_watcher(
        fs_watch_pool.clone(),
        config_watcher.clone(),
        event_bus.clone(),
    );

    // Start sysinfo collection loop (interval configurable via telemetry:interval)
    let sysinfo_broker = broker.clone();
    let sysinfo_config = config_watcher.clone();
    tokio::spawn(async move {
        sysinfo::run_sysinfo_loop(sysinfo_broker, sysinfo_config, "local".to_string()).await;
    });

    // Start agent process watchdog (kills panes that exceed max-runtime or idle-output limits)
    let watchdog_config = config_watcher.clone();
    tokio::spawn(async move {
        backend::blockcontroller::watchdog::run_watchdog_loop(watchdog_config).await;
    });

    // Push a live Haiku activity summary per registered agent (swarm feed) —
    // reads reactive::get_global_handler() as its registry, so it needs no
    // AppState and can start before AppState is built (matches sysinfo/watchdog above).
    let activity_wstore = Arc::clone(wstore);
    let activity_filestore = Arc::clone(filestore);
    let activity_broker = broker.clone();
    tokio::spawn(async move {
        backend::reactive::activity_watcher::run_agent_summary_loop(
            activity_wstore, activity_filestore, activity_broker,
        ).await;
    });

    // Reactive handler (global singleton) + poller
    let reactive_handler = reactive::get_global_handler();
    reactive_handler.set_input_sender(Arc::new(|block_id: &str, data: &[u8]| {
        backend::blockcontroller::send_input(
            block_id,
            backend::blockcontroller::BlockInputUnion::data(data.to_vec()),
            None,
        )
    }));
    // Controller-aware delivery (SPEC_AGENT_CONTROL_PROTOCOL §6 / Phase 3): persistent
    // stream-json and ACP agents have no PTY, so muxbus Tier-1 keystroke injection
    // silently misses them. Route those through their structured channel (live stdin /
    // session/prompt) — which also steers the agent mid-turn — and fall back to PTY
    // keystrokes only for terminal-based agents.
    reactive_handler.set_message_sender(Arc::new(|block_id: &str, message: &str| {
        match backend::blockcontroller::deliver_agent_message(block_id, message) {
            Ok(backend::blockcontroller::AgentDelivery::Structured) => Ok(true),
            Ok(backend::blockcontroller::AgentDelivery::Pty) => Ok(false),
            Err(e) => Err(e),
        }
    }));
    let poller = Arc::new(Poller::new(
        PollerConfig {
            muxbus_url: None,
            muxbus_token: None,
            poll_interval_secs: reactive::DEFAULT_POLL_INTERVAL_SECS,
        },
        reactive_handler,
    ));

    // Cloud push subscriber — single WS connection per sidecar that the cloud
    // uses to push reactive injections instead of polling.
    // No-op until the user connects via muxbus.login.
    crate::muxbus::cloud_subscriber::CloudSubscriber::init_global(id_store.clone());

    // Discord messaging bridge — connects to Discord Gateway if configured.
    // Set messaging:discord:enabled + messaging:discord:token in settings.json to activate.
    {
        let settings = config_watcher.get_settings();
        if settings.messaging_discord_enabled {
            match settings.messaging_discord_token.clone() {
                Some(token) if !token.is_empty() => {
                    messaging::discord::DiscordBridge::init_global(
                        messaging::discord::DiscordConfig {
                            token,
                            channel_id: settings.messaging_discord_channel.clone(),
                            target_agent: settings.messaging_discord_target.clone(),
                            guild_id: settings.messaging_discord_guild.clone(),
                        },
                        reqwest::Client::new(),
                    );
                }
                _ => {
                    tracing::warn!(
                        "discord bridge: enabled but messaging:discord:token is not set in settings.json"
                    );
                }
            }
        }
    }

    // Telegram messaging bridge — long-polls getUpdates if configured.
    // Set messaging:telegram:enabled + messaging:telegram:token in settings.json to activate.
    {
        let settings = config_watcher.get_settings();
        if settings.messaging_telegram_enabled {
            match settings.messaging_telegram_token.clone() {
                Some(token) if !token.is_empty() => {
                    let allowed_chat_ids = settings
                        .messaging_telegram_allowed_chats
                        .split(',')
                        .filter_map(|s| s.trim().parse::<i64>().ok())
                        .collect::<Vec<_>>();
                    let default_chat_id = settings
                        .messaging_telegram_default_chat
                        .as_deref()
                        .and_then(|s| s.parse::<i64>().ok());
                    messaging::telegram::TelegramBridge::init_global(
                        messaging::telegram::TelegramConfig {
                            token,
                            allowed_chat_ids,
                            default_chat_id,
                            target_agent: settings.messaging_telegram_target.clone(),
                        },
                        reqwest::Client::new(),
                    );
                }
                _ => {
                    tracing::warn!(
                        "telegram bridge: enabled but messaging:telegram:token is not set in settings.json"
                    );
                }
            }
        }
    }

    // Slack messaging bridge — opens a Socket Mode connection if configured.
    // Set messaging:slack:enabled + messaging:slack:bot_token + messaging:slack:app_token
    // in settings.json to activate.
    {
        let settings = config_watcher.get_settings();
        if settings.messaging_slack_enabled {
            match (
                settings.messaging_slack_bot_token.clone(),
                settings.messaging_slack_app_token.clone(),
            ) {
                (Some(bot_token), Some(app_token))
                    if !bot_token.is_empty() && !app_token.is_empty() =>
                {
                    messaging::slack::SlackBridge::init_global(
                        messaging::slack::SlackConfig {
                            bot_token,
                            app_token,
                            channel_id: settings.messaging_slack_channel.clone(),
                            target_agent: settings.messaging_slack_target.clone(),
                        },
                        reqwest::Client::new(),
                    );
                }
                _ => {
                    tracing::warn!(
                        "slack bridge: enabled but messaging:slack:bot_token and/or \
                         messaging:slack:app_token is not set in settings.json"
                    );
                }
            }
        }
    }

    // WhatsApp Cloud API messaging bridge — outbound send + inbound webhook
    // receiver. Unlike Discord/Telegram/Slack there is no bridge-managed
    // network connection to open at startup: inbound delivery is passive
    // HTTP (the GET/POST /webhook/whatsapp routes, registered unauthenticated
    // in server/mod.rs — Meta cannot supply X-AuthKey), reachable only once
    // the operator's own tunnel is up and the callback URL is registered in
    // Meta's App Dashboard, both manual one-time steps this process does not
    // perform (v1 does not manage a tunnel subprocess — see
    // messaging/whatsapp/mod.rs). Set messaging:whatsapp:enabled +
    // access_token/app_secret/webhook_verify_token in settings.json to activate.
    // See docs/specs/SPEC_MESSAGING_INTEGRATION_WHATSAPP_2026_07_07.md.
    {
        let settings = config_watcher.get_settings();
        if settings.messaging_whatsapp_enabled {
            match (
                settings.messaging_whatsapp_access_token.clone(),
                settings.messaging_whatsapp_app_secret.clone(),
                settings.messaging_whatsapp_webhook_verify_token.clone(),
            ) {
                (Some(token), Some(secret), Some(verify_token))
                    if !token.is_empty() && !secret.is_empty() && !verify_token.is_empty() =>
                {
                    messaging::whatsapp::WhatsAppBridge::init_global(
                        messaging::whatsapp::WhatsAppConfig {
                            phone_number_id: settings.messaging_whatsapp_phone_number_id.clone(),
                            access_token: token,
                            app_secret: secret,
                            webhook_verify_token: verify_token,
                            target_agent: settings.messaging_whatsapp_target.clone(),
                            fallback_template: settings.messaging_whatsapp_fallback_template.clone(),
                            fallback_template_lang: settings
                                .messaging_whatsapp_fallback_template_lang
                                .clone()
                                .unwrap_or_else(|| "en_US".to_string()),
                        },
                        reqwest::Client::new(),
                    );
                    if settings.messaging_whatsapp_tunnel_domain.is_empty() {
                        tracing::warn!(
                            "whatsapp bridge: enabled but messaging:whatsapp:tunnel_domain is not set — \
                             point your own tunnel at this instance's webhook port and register \
                             https://<your-domain>/webhook/whatsapp in Meta App Dashboard > WhatsApp > Configuration"
                        );
                    } else {
                        tracing::info!(
                            "whatsapp bridge: initialized — webhook callback = https://{}/webhook/whatsapp \
                             — verify this is registered in Meta App Dashboard > WhatsApp > Configuration \
                             (v1 does not manage the tunnel subprocess; ensure it's already running)",
                            settings.messaging_whatsapp_tunnel_domain
                        );
                    }
                }
                _ => {
                    tracing::warn!(
                        "whatsapp bridge: enabled but one of messaging:whatsapp:{{access_token,app_secret,webhook_verify_token}} is not set in settings.json"
                    );
                }
            }
        }
    }

    // Set up docsite directory
    if let Some(app_path) = base::get_wave_app_path() {
        let docsite_dir = app_path.join("docsite");
        docsite::set_docsite_dir(docsite_dir);
    }

    // Local MessageBus for inter-agent communication
    let messagebus = Arc::new(backend::messagebus::MessageBus::new());

    // Subagent watcher — monitors Claude Code session dirs for spawned subagents
    let subagent_watcher = backend::subagent_watcher::SubagentWatcher::spawn(event_bus.clone(), wstore.clone());
    // Registered as a global so blockcontroller/persistent.rs's turn-end
    // reconciliation hook (SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_
    // 2026_07_20 Phase A) can reach it without threading an Arc through
    // PersistentSubprocessController::new and every one of its callers.
    backend::subagent_watcher::set_global(subagent_watcher.clone());

    // History service — discovers and indexes past CLI agent conversations
    let history_service = Arc::new(backend::history::HistoryService::new());

    // Session archiver — auto-archive sessions inactive for >7 days, cap at 2 GB.
    // Skip if home directory can't be determined (would otherwise fall back to a
    // relative path and create archives under the current working directory).
    if let Some(archive_dir) = backend::session_archive::default_archive_dir() {
        let archiver = Arc::new(backend::session_archive::SessionArchiver::new(
            wstore.clone(),
            filestore.clone(),
            7,                              // inactive days
            2 * 1024 * 1024 * 1024,         // 2 GB max
            archive_dir,
        ));
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                match archiver.sweep().await {
                    Ok(stats) => tracing::info!(?stats, "session archiver sweep complete"),
                    Err(e) => tracing::warn!(error = %e, "session archiver sweep failed"),
                }
            }
        });
    } else {
        tracing::warn!("session archiver: home dir unavailable, archiver disabled");
    }

    BackgroundSubsystems {
        event_bus,
        broker,
        editor_file_watcher,
        media_file_watcher,
        fs_watch_pool,
        config_watcher,
        reactive_handler,
        poller,
        messagebus,
        subagent_watcher,
        history_service,
    }
}

/// Output of [`bind_listeners_and_network`].
pub struct NetworkBundle {
    pub web_listener: TcpListener,
    pub ws_listener: TcpListener,
    pub web_addr: std::net::SocketAddr,
    pub ws_addr: std::net::SocketAddr,
    pub local_web_url: String,
    pub lan_discovery: Arc<backend::lan_discovery::LanDiscoveryController>,
    pub lsp_supervisor: Arc<backend::lsp::LspSupervisor>,
    pub process_tracker: Arc<backend::process_tracker::registry::AgentProcessRegistry>,
    pub process_broker: Arc<crate::broker::ProcessBroker>,
}

/// Step 5: Bind 2 TCP listeners (web + ws — separate ports matching Go), then
/// bring up LAN discovery (mDNS), the LSP supervisor, stale registry cleanup,
/// and the process tracker/broker.
pub async fn bind_listeners_and_network(
    config: &config::Config,
    config_watcher: &Arc<wconfig::ConfigWatcher>,
    event_bus: &Arc<EventBus>,
    broker: &Arc<Broker>,
    version: &str,
) -> NetworkBundle {
    // When LAN discovery is enabled the user has explicitly opted in to network
    // visibility. Bind to 0.0.0.0 so devices on the same network can reach the
    // port that mDNS advertises. The auth_key (X-AuthKey header, broadcast in
    // the mDNS TXT record) gates every API route.
    //
    // Known limitation: `bind_addr` is resolved once at startup. The toggle is
    // therefore only fully effective before launch (settings.json) or after a
    // sidecar restart. Both directions are affected:
    //   OFF→ON: mDNS re-advertises the LAN IP but listeners stay on 127.0.0.1,
    //           so remote devices cannot connect until restart.
    //   ON→OFF: mDNS is stopped but listeners remain on 0.0.0.0 and are still
    //           reachable on the LAN until restart (auth_key still gates routes).
    // A future improvement would re-bind listeners on toggle; deferred because
    // rebinding an active axum server is non-trivial.
    let bind_addr = if config_watcher.get_settings().network_lan_discovery {
        "0.0.0.0:0"
    } else {
        "127.0.0.1:0"
    };
    let web_listener = TcpListener::bind(bind_addr)
        .await
        .expect("failed to bind web listener");
    let ws_listener = TcpListener::bind(bind_addr)
        .await
        .expect("failed to bind ws listener");

    let web_addr = web_listener.local_addr().unwrap();
    let ws_addr = ws_listener.local_addr().unwrap();
    // Always use 127.0.0.1 for the local URL regardless of bind address.
    // When bound to 0.0.0.0, local_addr() returns 0.0.0.0:PORT which is not a
    // valid connect destination (fails on Windows and some Linux configs).
    let local_web_url = format!("http://127.0.0.1:{}", web_addr.port());

    // Make local backend URL available to child processes (PTY shells).
    // the muxbus client (agentbus-client package) reads AGENTMUX_LOCAL_URL for local PTY delivery
    // instead of routing through the cloud muxbus relay.
    std::env::set_var("AGENTMUX_LOCAL_URL", &local_web_url);

    // LAN discovery via mDNS — opt-in to avoid Windows Firewall prompt.
    // mDNS binds 0.0.0.0:5353 UDP which triggers the firewall dialog.
    // The setting defaults to false; users opt in via the HostPopover toggle
    // (or by editing settings.json). The controller supports live start/stop
    // so flipping the setting does not require an app restart.
    // See specs/lan-discovery-toggle.md.
    let hostname = whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_string());
    let lan_discovery = Arc::new(backend::lan_discovery::LanDiscoveryController::new(
        config.instance_id.clone(),
        hostname,
        version.to_string(),
        web_addr.port(),
        event_bus.clone(),
        config.auth_key.clone(),
    ));
    // Honor the current setting at boot — starts the daemon if enabled.
    lan_discovery.apply(config_watcher.get_settings().network_lan_discovery);

    // LSP supervisor — owns LSP server child processes. Nothing spawned
    // until the editor pane calls `lspstart`. Spec:
    // specs/SPEC_EDITOR_LSP_AND_THEMES_2026-05-26.md
    let lsp_supervisor = Arc::new(backend::lsp::LspSupervisor::new(event_bus.clone()));

    // Clean up stale cross-instance agent registry entries (entries older than 4h).
    backend::reactive::registry::cleanup_stale(
        &base::get_wave_data_dir(),
        4 * 60 * 60 * 1000,
    );

    // Same sweep for the host-global shared registry (Tier 2b, issue #1916)
    // — additionally drops entries whose owning PID no longer exists on this
    // host, since a channel that crashed without a clean unregister can
    // leave an entry behind indefinitely otherwise (no other channel's
    // startup would ever revisit it).
    if let Some(shared_dir) = registry::resolve_shared_reactive_dir() {
        backend::reactive::registry::cleanup_stale_shared(&shared_dir, 4 * 60 * 60 * 1000);
    }

    // Tracks agent-spawned OS processes per block. Registered trackers
    // live as long as their agent pane; the background poller emits
    // delta events (`agent:process-added`/`-exited`) to the frontend.
    let process_tracker = std::sync::Arc::new(
        backend::process_tracker::registry::AgentProcessRegistry::new(Some(broker.clone())),
    );
    backend::process_tracker::registry::set_global(process_tracker.clone());
    backend::process_tracker::registry::spawn_poller(process_tracker.clone());

    // Process Broker (Phase A) — unified read path over blockcontroller +
    // process_tracker. See docs/specs/REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK_2026_07_22.md.
    let process_broker = std::sync::Arc::new(crate::broker::ProcessBroker::new(Some(broker.clone())));
    crate::broker::process::set_global(process_broker.clone());

    NetworkBundle {
        web_listener,
        ws_listener,
        web_addr,
        ws_addr,
        local_web_url,
        lan_discovery,
        lsp_supervisor,
        process_tracker,
        process_broker,
    }
}

/// Output of [`spawn_reducer_plumbing`].
pub struct ReducerPlumbing {
    pub srv_state: Arc<tokio::sync::Mutex<state::State>>,
    pub srv_events_tx: tokio::sync::broadcast::Sender<agentmux_common::ipc::Event>,
    pub srv_event_log: Arc<event_log::EventLog>,
}

/// Phase E.2 / E.2c.2 — srv reducer plumbing, hoisted out of the
/// (conditional) pipe-IPC bind block so HTTP/WS RPC handlers in
/// dispatch_service can route through the reducer. State, event
/// bus, event log, and persist subscriber all live unconditionally;
/// the pipe IPC server is still conditional on
/// `AGENTMUX_SRV_PIPE_PATH` being set (absent in `task dev` mode).
///
/// Bootstraps reducer state from SQLite, spawns the disk writer (forensic
/// log of every reducer event), the persist subscriber (idempotent SQLite
/// write-back), the WaveObjUpdate bridge, and the subagent-watcher block-delete
/// cascade backstop.
pub async fn spawn_reducer_plumbing(
    wstore: &Arc<Store>,
    event_bus: &Arc<EventBus>,
    subagent_watcher: &Arc<backend::subagent_watcher::SubagentWatcher>,
) -> ReducerPlumbing {
    let wstore_for_persist = Arc::clone(wstore);
    let srv_state = std::sync::Arc::new(tokio::sync::Mutex::new(state::State::default()));
    let (srv_events_tx, _) =
        tokio::sync::broadcast::channel::<agentmux_common::ipc::Event>(1024);
    let srv_event_log = std::sync::Arc::new(event_log::EventLog::new(Some(
        base::get_wave_data_dir().join("srv-events.log"),
    )));

    // Bootstrap reducer state from SQLite. Always runs (even in
    // `task dev` where there's no pipe IPC server) so RPC handlers
    // dispatching through the reducer see populated state.
    persist::bootstrap_state_from_wstore(&srv_state, &wstore_for_persist).await;

    // Spawn the disk writer (forensic log of every reducer event)
    // and the persist subscriber (idempotent SQLite write-back).
    let disk_writer_rx = srv_events_tx.subscribe();
    let log_for_writer = std::sync::Arc::clone(&srv_event_log);
    tokio::spawn(event_log::run_disk_writer(log_for_writer, disk_writer_rx));
    let subscriber_rx = srv_events_tx.subscribe();
    persist_subscriber::spawn_persist_subscriber(
        subscriber_rx,
        std::sync::Arc::clone(&wstore_for_persist),
        std::sync::Arc::clone(&srv_state),
    );

    // Phase 1 of the WaveObjUpdate bridge: subscribe to srv_events_tx and
    // translate workspace mutations into `waveobj:update` WS broadcasts.
    // Fixes the workspace-rename reactivity gap where UpdateWorkspace
    // returned `success_empty()` and the response loop had nothing to
    // broadcast — see docs/specs/SPEC_OBJ_UPDATE_BRIDGE_2026-05-14.md.
    //
    // Watchdog: capture the JoinHandle and observe it from a sibling task
    // so a panic in the bridge's loop scaffolding (vs. an inner
    // dispatch_event panic, which is already caught per-event) is logged
    // loudly. Without this, a silent bridge death would manifest as
    // "renaming a workspace stopped propagating" with no log evidence.
    // (Per ReAgent P2 follow-up on PR #852.)
    let bridge_rx = srv_events_tx.subscribe();
    let bridge_handle = server::wave_obj_bridge::spawn_wave_obj_bridge(
        bridge_rx,
        std::sync::Arc::clone(&wstore_for_persist),
        std::sync::Arc::clone(event_bus),
    );
    tokio::spawn(async move {
        match bridge_handle.await {
            Ok(()) => tracing::info!(
                target: "wave-obj-bridge",
                "bridge task exited normally (events channel closed at srv shutdown)"
            ),
            Err(e) if e.is_panic() => tracing::error!(
                target: "wave-obj-bridge",
                "bridge task PANICKED at top level — frontend WOS will stop receiving updates until srv restart. Panic: {}",
                e
            ),
            Err(e) => tracing::error!(
                target: "wave-obj-bridge",
                "bridge task terminated unexpectedly (non-panic JoinError): {}",
                e
            ),
        }
    });

    // Block-delete cascade backstop for the subagent watcher: prunes a
    // closed block's subagents/dispatches on Event::BlockDeleted/TabDeleted/
    // WorkspaceDeleted, independent of whether the frontend's normal
    // /agentmux/reactive/unregister teardown path fires for this close (see
    // SubagentWatcher::prune_block's doc comment). Without this, closing an
    // agent pane left a ghost row in the Swarm pane until srv restart.
    let block_prune_rx = srv_events_tx.subscribe();
    backend::subagent_watcher::spawn_block_prune_subscriber(subagent_watcher.clone(), block_prune_rx);

    ReducerPlumbing {
        srv_state,
        srv_events_tx,
        srv_event_log,
    }
}

/// Assembles the final `AppState` from every bundle produced above, plus the
/// handful of state pieces (auth session manager, install sessions, container
/// manager, shell sessions, cron scheduler) that only ever existed inline in
/// the `AppState` struct literal.
pub fn build_app_state(
    config: &config::Config,
    version: String,
    stores: Stores,
    bg: BackgroundSubsystems,
    net: &NetworkBundle,
    reducer: &ReducerPlumbing,
) -> AppState {
    // Clone before move into AppState for cron_scheduler construction.
    let shared_store_for_cron = stores.shared_store.clone();
    let broker = bg.broker;

    AppState {
        auth_key: config.auth_key.clone(),
        boot_id: Arc::from(uuid::Uuid::new_v4().to_string()),
        version,
        app_path: config.app_path.clone(),
        wstore: stores.wstore,
        shared_store: stores.shared_store,
        id_store: stores.id_store,
        filestore: stores.filestore,
        global_transcript_store: stores.global_transcript_store,
        event_bus: bg.event_bus,
        broker: broker.clone(),
        reactive_handler: bg.reactive_handler,
        poller: bg.poller,
        config_watcher: bg.config_watcher,
        messagebus: bg.messagebus,
        subagent_watcher: bg.subagent_watcher,
        history_service: bg.history_service,
        lan_discovery: net.lan_discovery.clone(),
        lsp_supervisor: net.lsp_supervisor.clone(),
        local_web_url: net.local_web_url.clone(),
        // Bounded request timeout: cross-instance reactive-inject forwards
        // (Tier 2/2b/3) chain through this client, and an unbounded client
        // combined with a forwarding cycle (two channels each holding a
        // stale-but-PID-alive shared-registry entry pointing at the other)
        // could otherwise hang a request indefinitely. The hop-count guard
        // in server/reactive.rs bounds the CYCLE; this bounds each
        // individual HOP (reagent P1 on PR #2350).
        http_client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "failed to build http_client with timeout, using default");
                reqwest::Client::new()
            }),
        process_tracker: net.process_tracker.clone(),
        process_broker: net.process_broker.clone(),
        dock_snapshots: std::sync::Arc::new(crate::backend::dock_snapshot::DockSnapshotCache::new()),
        // Phase E.2c.2 — reducer state + event bus exposed to HTTP/WS
        // dispatch handlers. Workspace handlers route through the
        // reducer and publish events to `srv_events_tx`; the persist
        // subscriber writes back to SQLite asynchronously.
        srv_state: std::sync::Arc::clone(&reducer.srv_state),
        srv_events_tx: reducer.srv_events_tx.clone(),
        // Phase E.5.5 — saga-id allocator. Seeded from
        // `SagaLog::max_saga_id()` so restarts don't collide with
        // prior runs' IDs. First new saga after restart gets
        // `seed + 1`; on a fresh DB seed=0, first saga gets id 1.
        saga_id_alloc: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(stores.saga_id_seed)),
        saga_log: stores.saga_log,
        auth_session_manager: std::sync::Arc::new(
            crate::identity::auth_session::AuthSessionManager::new(),
        ),
        install_sessions: crate::server::install_handlers::InstallSessionRegistry::new(),
        container_manager: {
            // Self-healing: unlike a plain `Option<ContainerManager>` fixed at
            // boot, `ContainerRuntimeHandle` retries the connect on demand, so
            // a daemon that starts after this point is picked up by later
            // calls without an app restart. See
            // docs/retro/RETRO_DOCKER_DETECTION_DIVERGENCE_2026_07_04.md.
            let handle = std::sync::Arc::new(
                crate::backend::container::ContainerRuntimeHandle::connect_at_startup(),
            );
            let handle_check = handle.clone();
            tokio::spawn(async move {
                if handle_check.is_available().await {
                    tracing::info!("Docker daemon available — container agent panes enabled");
                } else {
                    tracing::warn!(
                        "Docker daemon not reachable at startup; container agent panes will \
                         become available automatically once Docker is running"
                    );
                }
            });
            handle
        },
        shell_sessions: crate::backend::shell_node::ShellSessionRegistry::new(),
        cron_scheduler: crate::backend::cron::CronScheduler::new(
            shared_store_for_cron,
            reqwest::Client::new(),
            net.local_web_url.clone(),
            config.auth_key.clone(),
            Arc::clone(&broker),
        ),
        editor_file_watcher: bg.editor_file_watcher,
        media_file_watcher: bg.media_file_watcher,
        fs_watch_pool: bg.fs_watch_pool,
    }
}

/// Phase E.1b — srv pipe IPC server. Bound when launcher passes
/// `AGENTMUX_SRV_PIPE_PATH`; absent in `task dev` mode (no
/// launcher in the loop).
///
/// Bind happens BEFORE the AGENTMUXSRV-ESTART line so the
/// launcher knows the pipe is ready when host starts. Non-fatal
/// if the bind fails — srv keeps running with HTTP/WS only.
#[cfg(target_os = "windows")]
pub fn bind_srv_pipe_ipc(
    version: &str,
    srv_state: std::sync::Arc<tokio::sync::Mutex<state::State>>,
    srv_events_tx: tokio::sync::broadcast::Sender<agentmux_common::ipc::Event>,
    srv_event_log: std::sync::Arc<event_log::EventLog>,
) {
    if let Ok(srv_pipe_path) = std::env::var("AGENTMUX_SRV_PIPE_PATH") {
        if !srv_pipe_path.is_empty() {
            match crate::srv_ipc::server::bind_first_pipe_instance(&srv_pipe_path) {
                Ok(first_pipe) => {
                    // Phase E.2c.2 — pipe IPC server reuses the
                    // hoisted srv_state / events_tx / event_log so
                    // pipe-originated commands and HTTP/WS-originated
                    // commands mutate the same canonical state.
                    let srv_ctx = crate::srv_ipc::ServerCtx {
                        srv_pid: std::process::id(),
                        srv_version: version.to_string(),
                        state: srv_state,
                        events_tx: srv_events_tx,
                        event_log: srv_event_log,
                    };
                    let _srv_ipc_handle = crate::srv_ipc::run_srv_ipc_server(
                        srv_pipe_path.clone(),
                        first_pipe,
                        srv_ctx,
                    );
                    tracing::info!(
                        target: "srv-ipc",
                        "[srv-ipc] bound + spawned on {}",
                        srv_pipe_path
                    );
                }
                Err(e) => {
                    tracing::error!(
                        target: "srv-ipc",
                        "[srv-ipc] bind failed on {}: {} — srv runs without pipe IPC",
                        srv_pipe_path,
                        e
                    );
                }
            }
        }
    }
}

/// Step 6: Emit AGENTMUXSRV-ESTART on stderr (exact format from cmd/server/main-server.go:617)
/// pending_migrations reflects any migrations that failed during the in-process
/// run above. Non-zero causes the status-bar to show a "Migration failed —
/// restart to retry" message. Zero is the expected steady-state.
pub fn emit_estart(ws_port: u16, web_port: u16, version: &str, build_time: &str, instance_id: &str) {
    let pending_migrations = migrations::count_pending_migrations(&base::get_wave_data_dir());
    eprintln!(
        "AGENTMUXSRV-ESTART ws:127.0.0.1:{} web:127.0.0.1:{} version:{} buildtime:{} instance:{} pending_migrations:{}",
        ws_port, web_port, version, build_time, instance_id, pending_migrations
    );
}

/// Steps 8 & 9: spawn the stdin-watch thread (exit on EOF — matching Go's
/// stdinReadWatch) and the SIGINT/SIGTERM handler. Both cancel the returned
/// token, which the WAL checkpoint loop and the final server select! also
/// watch for graceful shutdown.
pub fn install_shutdown_handlers() -> tokio_util::sync::CancellationToken {
    // 8. Spawn stdin watch thread (exit on EOF — matching Go's stdinReadWatch)
    let stdin_token = tokio_util::sync::CancellationToken::new();
    let stdin_shutdown = stdin_token.clone();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut stdin = std::io::stdin().lock();
        let mut buf = [0u8; 1024];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => {
                    eprintln!("stdin closed, shutting down");
                    stdin_shutdown.cancel();
                    break;
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("stdin read error: {}, shutting down", e);
                    stdin_shutdown.cancel();
                    break;
                }
            }
        }
    });

    // 9. Spawn signal handler (SIGINT/SIGTERM → graceful shutdown)
    let signal_token = stdin_token.clone();
    tokio::spawn(async move {
        let ctrl_c = tokio::signal::ctrl_c();
        #[cfg(unix)]
        {
            let mut sigterm =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
            tokio::select! {
                _ = ctrl_c => {
                    tracing::info!("received SIGINT, shutting down");
                }
                _ = sigterm.recv() => {
                    tracing::info!("received SIGTERM, shutting down");
                }
            }
        }
        #[cfg(not(unix))]
        {
            ctrl_c.await.ok();
            tracing::info!("received Ctrl+C, shutting down");
        }
        signal_token.cancel();
    });

    stdin_token
}

/// Periodic WAL checkpoint — prevents unbounded WAL file growth during
/// long-running sessions. Runs every 30 minutes while the srv is up.
/// busy_timeout=5000 (set at DB open) handles transient reader contention;
/// partial truncate on contention is safe — the remainder is picked up on
/// the next pass. (SPEC_WINDOWS_LIFECYCLE_ROBUSTNESS_2026_06_26 §4.E)
pub fn spawn_wal_checkpoint_loop(
    token: tokio_util::sync::CancellationToken,
    wal_wstore: Arc<Store>,
    wal_filestore: Arc<FileStore>,
) {
    tokio::spawn(async move {
        const INTERVAL: std::time::Duration = std::time::Duration::from_secs(30 * 60);
        loop {
            tokio::select! {
                _ = tokio::time::sleep(INTERVAL) => {}
                _ = token.cancelled() => break,
            }
            if let Err(e) = wal_wstore.checkpoint() {
                tracing::warn!(error = %e, "wal_checkpoint(TRUNCATE) on objects.db failed");
            } else {
                tracing::debug!("wal_checkpoint(TRUNCATE): objects.db ok");
            }
            if let Err(e) = wal_filestore.checkpoint() {
                tracing::warn!(error = %e, "wal_checkpoint(TRUNCATE) on filestore.db failed");
            } else {
                tracing::debug!("wal_checkpoint(TRUNCATE): filestore.db ok");
            }
        }
    });
}
