// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Split out of bootstrap.rs unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.3).

use super::*;

/// Step 6: Emit AGENTMUXSRV-ESTART on stderr (exact format from cmd/server/main-server.go:617)
/// pending_migrations reflects any migrations that failed during the in-process
/// run above. Non-zero causes the status-bar to show a "Migration failed —
/// restart to retry" message. Zero is the expected steady-state.
pub fn emit_estart(ws_port: u16, web_port: u16, version: &str, build_time: &str, instance_id: &str) {
    let pending_migrations = migrations::count_pending_migrations(&base::get_mux_data_dir());
    eprintln!(
        "AGENTMUXSRV-ESTART ws:127.0.0.1:{} web:127.0.0.1:{} version:{} buildtime:{} instance:{} pending_migrations:{}",
        ws_port, web_port, version, build_time, instance_id, pending_migrations
    );
}

/// Steps 8 & 9: spawn the stdin-watch thread (exit on EOF — matching Go's
/// stdinReadWatch) and the SIGINT/SIGTERM handler. Both cancel the returned
/// token, which the WAL checkpoint loop and the final server select! also
/// watch for graceful shutdown.
pub fn install_shutdown_handlers(watch_stdin: bool) -> tokio_util::sync::CancellationToken {
    // 8. Spawn stdin watch thread (exit on EOF — matching Go's stdinReadWatch).
    //    Not in headless mode: nothing owns srv's stdin there (a container's
    //    is /dev/null), so EOF would mean "shut down now".
    let stdin_token = tokio_util::sync::CancellationToken::new();
    let stdin_shutdown = stdin_token.clone();
    if watch_stdin {
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
    }

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
    wal_mstore: Arc<Store>,
    wal_filestore: Arc<FileStore>,
) {
    tokio::spawn(async move {
        const INTERVAL: std::time::Duration = std::time::Duration::from_secs(30 * 60);
        loop {
            tokio::select! {
                _ = tokio::time::sleep(INTERVAL) => {}
                _ = token.cancelled() => break,
            }
            if let Err(e) = wal_mstore.checkpoint() {
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
