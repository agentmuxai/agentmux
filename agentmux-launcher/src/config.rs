// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// LSD-4 — operator-overridable launcher config loaded from
// `<user_home_dir>/config.toml` (typically `~/.agentmux/config.toml`).
//
// Spec: `docs/specs/SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md`
//   - §3.6 retention default 7 days, configurable via `[saga.launcher]`
//   - §4 PR4 scope: read this file at startup, apply
//     `vacuum_older_than(now - retention_days)` once before the
//     coordinator starts.
//
// Design:
//   - File is OPTIONAL. Missing file, unreadable file, or malformed
//     TOML all fall back to the compiled-in defaults; we log a WARN
//     line so operators see the misconfiguration but the launcher
//     keeps starting. The saga log's behavior shouldn't be load-
//     bearing on a config file the user may never have created.
//   - Sections are namespaced under `[saga.launcher]` so future srv
//     bits can land at `[saga.srv]` without colliding. Top-level
//     `unknown_keys` are tolerated (serde default) so an older
//     launcher reading a newer config doesn't crash.
//
// Example `~/.agentmux/config.toml`:
//
//     [saga.launcher]
//     retention_days = 14
//
// Tests live below in `#[cfg(test)] mod tests` — exercise the parse
// path on a `tempfile::NamedTempFile` containing valid + malformed
// TOML.

use std::path::Path;

use serde::Deserialize;

/// LSD spec §3.6 default — 7 days. Tunable via config file.
pub const DEFAULT_SAGA_RETENTION_DAYS: i64 = 7;

/// Top-level launcher config schema. Only the saga subtree is
/// populated today; future PRs append siblings.
#[derive(Debug, Default, Deserialize)]
struct LauncherConfig {
    #[serde(default)]
    saga: SagaConfig,
}

#[derive(Debug, Default, Deserialize)]
struct SagaConfig {
    #[serde(default)]
    launcher: SagaLauncherConfig,
}

#[derive(Debug, Default, Deserialize)]
struct SagaLauncherConfig {
    /// Days to retain terminal sagas (`completed` / `failed` /
    /// `failed_compensation`) before the startup vacuum sweeps them.
    /// In-flight sagas (`running` / `compensating`) are never vacuumed
    /// regardless of age — see `vacuum_older_than` SQL filter.
    retention_days: Option<i64>,
}

/// Read `<user_home_dir>/config.toml` if it exists and return the
/// configured saga retention days, falling back to
/// `DEFAULT_SAGA_RETENTION_DAYS` on any error. The optional `log_warn`
/// closure receives a human-readable diagnostic line per failure path
/// (file unreadable / malformed / negative value) so callers can
/// route it through `crate::log()` without introducing a tracing dep.
pub fn load_saga_retention_days(
    user_home_dir: &Path,
    mut log_warn: impl FnMut(&str),
) -> i64 {
    let path = user_home_dir.join("config.toml");
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Missing file is the expected case for fresh installs;
            // no warning needed.
            return DEFAULT_SAGA_RETENTION_DAYS;
        }
        Err(e) => {
            log_warn(&format!(
                "[config] failed to read {}: {} — using default retention {} days",
                path.display(),
                e,
                DEFAULT_SAGA_RETENTION_DAYS
            ));
            return DEFAULT_SAGA_RETENTION_DAYS;
        }
    };
    match toml::from_str::<LauncherConfig>(&raw) {
        Ok(cfg) => match cfg.saga.launcher.retention_days {
            Some(d) if d > 0 => d,
            Some(d) => {
                log_warn(&format!(
                    "[config] [saga.launcher] retention_days = {} is non-positive; using default {} days",
                    d, DEFAULT_SAGA_RETENTION_DAYS
                ));
                DEFAULT_SAGA_RETENTION_DAYS
            }
            None => DEFAULT_SAGA_RETENTION_DAYS,
        },
        Err(e) => {
            log_warn(&format!(
                "[config] failed to parse {}: {} — using default retention {} days",
                path.display(),
                e,
                DEFAULT_SAGA_RETENTION_DAYS
            ));
            DEFAULT_SAGA_RETENTION_DAYS
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::fs;
    use tempfile::TempDir;

    // Tests collect each warning passed through `log_warn` into a
    // `RefCell<Vec<String>>` and assert on the resulting list. Keeps
    // the closure trivial without requiring a generic `FnMut` factory.

    fn run(home: &Path) -> (i64, Vec<String>) {
        let warnings: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let days = load_saga_retention_days(home, |w| warnings.borrow_mut().push(w.to_string()));
        (days, warnings.into_inner())
    }

    #[test]
    fn missing_file_returns_default_no_warning() {
        let dir = TempDir::new().unwrap();
        let (days, warns) = run(dir.path());
        assert_eq!(days, DEFAULT_SAGA_RETENTION_DAYS);
        assert!(warns.is_empty(), "missing file is silent, got: {warns:?}");
    }

    #[test]
    fn valid_config_returns_configured_value() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("config.toml"),
            "[saga.launcher]\nretention_days = 14\n",
        )
        .unwrap();
        let (days, warns) = run(dir.path());
        assert_eq!(days, 14);
        assert!(warns.is_empty());
    }

    #[test]
    fn malformed_toml_logs_warning_and_returns_default() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("config.toml"), "not = toml = bro\n").unwrap();
        let (days, warns) = run(dir.path());
        assert_eq!(days, DEFAULT_SAGA_RETENTION_DAYS);
        assert_eq!(warns.len(), 1);
        assert!(warns[0].contains("failed to parse"), "got: {}", warns[0]);
    }

    #[test]
    fn missing_section_returns_default_no_warning() {
        // Empty + irrelevant keys are tolerated.
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("config.toml"),
            "[some_other_section]\nfoo = 1\n",
        )
        .unwrap();
        let (days, warns) = run(dir.path());
        assert_eq!(days, DEFAULT_SAGA_RETENTION_DAYS);
        assert!(warns.is_empty());
    }

    #[test]
    fn non_positive_retention_logs_warning_and_returns_default() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("config.toml"),
            "[saga.launcher]\nretention_days = 0\n",
        )
        .unwrap();
        let (days, warns) = run(dir.path());
        assert_eq!(days, DEFAULT_SAGA_RETENTION_DAYS);
        assert_eq!(warns.len(), 1);
        assert!(warns[0].contains("non-positive"));

        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("config.toml"),
            "[saga.launcher]\nretention_days = -3\n",
        )
        .unwrap();
        let (days, warns) = run(dir.path());
        assert_eq!(days, DEFAULT_SAGA_RETENTION_DAYS);
        assert_eq!(warns.len(), 1);
    }

}
