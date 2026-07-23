// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tracing/logging setup for the CEF host process, extracted out of lib.rs's
// `run()` bootstrap sequence. Logically distinct from process bootstrap
// (crate-startup-adjacent, not host-lifecycle): sets up the dual file+stderr
// tracing subscriber and prunes old rolling log files.

/// Initialize tracing with dual output: rolling daily log file + human-readable stderr.
/// `log_dir` is resolved by the caller: `<portable-root>/data/logs/` in portable mode,
/// `~/.agentmux/logs/` in installed mode.
/// Returns a guard that must be held for the lifetime of the process to ensure log flushing.
pub(crate) fn init_logging(log_dir: &std::path::Path) -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter};

    let version = env!("CARGO_PKG_VERSION");
    let _ = std::fs::create_dir_all(log_dir);

    // Delete log files older than 7 days to prevent unbounded growth.
    cleanup_old_logs(log_dir, 7);

    let log_prefix = format!("agentmux-host-v{}.log", version);
    let file_appender = tracing_appender::rolling::daily(&log_dir, &log_prefix);
    let (non_blocking_file, guard) = tracing_appender::non_blocking(file_appender);

    // Write pointer to current log file for zero-lookup agent discovery.
    // Version-qualified name so multi-instance doesn't clobber pointers.
    // Uses UTC to match tracing_appender::rolling::daily's date suffix.
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let current_filename = format!("{}.{}", log_prefix, today);
    let absolute_path = log_dir.join(&current_filename);
    let pointer_name = format!("current-host-v{}.path", version);

    // Pointer #1: local — inside the instance's log dir. The basename
    // is enough here since the reader is colocated.
    let _ = std::fs::write(log_dir.join(&pointer_name), &current_filename);

    // Pointer #2: global — at `<root>/logs/<pointer_name>`. Writes the
    // ABSOLUTE PATH so legacy tooling (`muxlog host`) that lives outside
    // the instance dir can `cat $pointer | xargs tail -f` and reach the
    // real file. Skipped silently if the global dir can't be derived
    // (e.g. AGENTMUX_HOME_OVERRIDE unset in some test setups).
    if let Some(global_logs_dir) = log_dir.parent().and_then(|p| p.parent()).and_then(|p| p.parent()).map(|p| p.join("logs")) {
        let _ = std::fs::create_dir_all(&global_logs_dir);
        let _ = std::fs::write(
            global_logs_dir.join(&pointer_name),
            absolute_path.to_string_lossy().as_bytes(),
        );
    }

    // Synchronous init sentinel: append a single line directly to the
    // expected log path BEFORE the tracing subscriber is wired up. Without
    // this, a hang between subscriber-setup and the non-blocking writer's
    // first flush leaves the pointer file pointing at a never-created log
    // file (observed 2026-05-02 freeze investigation). The sentinel
    // guarantees the file exists once init_logging has run past
    // pointer-write — if the file is missing afterwards, we know
    // init_logging itself didn't get past this point.
    let sentinel_path = log_dir.join(&current_filename);
    let sentinel_line = format!(
        "{} INIT-SENTINEL agentmux-host v={} pid={} os={} arch={}\n",
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"),
        version,
        std::process::id(),
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&sentinel_path)
    {
        use std::io::Write;
        let _ = f.write_all(sentinel_line.as_bytes());
        let _ = f.flush();
    }

    let subscriber = tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
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
        version,
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        log_dir = %log_dir.display(),
        "AgentMux host starting"
    );

    guard
}

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
