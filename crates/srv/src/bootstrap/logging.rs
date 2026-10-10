// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Split out of the single-file bootstrap module unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.3).

/// Route `log` crate records into `tracing`, at info and above.
///
/// The subscriber is built by hand (`registry()...set_global_default`), which does
/// not install the `log` -> `tracing` bridge that `.init()` would. So every record
/// from a dependency that logs through the `log` crate was dropped, at every level:
/// `mdns-sd` included, whose `error!("bind a socket to {}: {}. Skipped.")` is exactly
/// what was missing when an instance could hear its peers and not be heard
/// (Area54, 2026-10-02), and which cost several cloud round trips to find by hand.
///
/// The cap is `Info`: the `log` macros skip anything above it before formatting, so
/// the chatty `debug!`/`trace!` records of `mdns-sd`, `hyper` and friends cost
/// nothing, and the `EnvFilter` below still decides what is written. Returns whether
/// the bridge was installed (it is not if another `log` logger already is).
pub(crate) fn install_log_bridge() -> bool {
    tracing_log::LogTracer::builder().with_max_level(log::LevelFilter::Info).init().is_ok()
}

/// Initialize tracing with dual output: JSON rolling file + human-readable stderr.
/// Returns a guard that must be held for the lifetime of the app to ensure log flushing.
pub fn init_logging() -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter};

    // Prefer AGENTMUX_LOG_DIR (set by the launcher, per-instance-scoped —
    // agentmux-common::DataPaths::to_env_vars) so srv's log file lands in the
    // SAME per-channel directory agentmux-cef's host log already uses, and
    // the same directory the shell-integration pointer lookups
    // (bash.sh/zsh.sh/fish.fish/pwsh.ps1's `$AGENTMUX_LOG_DIR/current-<target>-
    // v<version>.path`) already assume both host AND srv write into. Before
    // this, srv unconditionally wrote to the SHARED ~/.agentmux/logs/ root
    // regardless of channel — every instance at the same version interleaved
    // into one file, with no way to attribute a line back to its instance
    // (see docs/reports/REPORT_MUXSPECT_MUXLOG_CROSS_CHANNEL_INSPECTION_2026_08_22.md
    // §2.2-2.3). Read inline (not via DataPaths::from_env(), which requires
    // several other vars to all be present) to match this file's existing
    // lightweight-env-var-read style for a single value (see the
    // AGENTMUX_CHANNEL/AGENTMUX_HOME_OVERRIDE reads below) and to keep
    // working the same as before when AGENTMUX_LOG_DIR isn't set (a
    // standalone/no-launcher run).
    // Version is embedded in the filename for side-by-side coexistence.
    let version = env!("CARGO_PKG_VERSION");
    let log_dir = std::env::var_os("AGENTMUX_LOG_DIR")
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_default()
                .join(".agentmux")
                .join("logs")
        });
    let _ = std::fs::create_dir_all(&log_dir);

    // Delete log files older than 7 days to prevent unbounded growth.
    agentmux_common::log_retention::cleanup_old_logs(&log_dir, 7);

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

    install_log_bridge();
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

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
        type Writer = Capture;
        fn make_writer(&'a self) -> Capture {
            self.clone()
        }
    }

    /// The failure this exists for: a `log::error!` from `mdns-sd` must reach the srv
    /// log. Before the bridge it was dropped without a trace.
    #[test]
    fn a_log_crate_error_from_mdns_sd_reaches_tracing_but_debug_noise_does_not() {
        if !super::install_log_bridge() {
            // Another `log` logger is already installed in this test process, so the
            // bridge cannot be exercised here; production has none before this runs.
            eprintln!("skipped: a log logger is already installed in this process");
            return;
        }
        let out = Capture::default();
        let subscriber = tracing_subscriber::fmt().with_writer(out.clone()).with_max_level(tracing::Level::INFO).finish();
        tracing::subscriber::with_default(subscriber, || {
            log::error!(target: "mdns_sd::service_daemon", "bind a socket to 192.168.1.26: socket bind to 0.0.0.0:5353 failed. Skipped.");
            log::debug!(target: "mdns_sd::service_daemon", "send outgoing query: 1 questions");
        });
        let text = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
        assert!(text.contains("bind a socket to 192.168.1.26"), "the error must be logged: {text:?}");
        assert!(text.contains("mdns_sd"), "and attributed to its crate: {text:?}");
        assert!(!text.contains("send outgoing query"), "debug stays out (the bridge caps at info): {text:?}");
    }
}
