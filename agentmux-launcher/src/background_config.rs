// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Background-service mode (and the tray it implies), resolved by the launcher
//! at startup — `docs/specs/SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md` §4.1.
//!
//! ## Why this module exists
//!
//! `tray::start_if_enabled` reads `AGENTMUX_TRAY` + `AGENTMUX_BACKGROUND_SERVICE`
//! from the **launcher's own** environment. Before this module, nothing ever
//! put them there:
//!
//! - `--background` (what every auto-start artifact passes) was only mapped to
//!   env on the **host** `Command` (`host_spawn.rs`), so an auto-started
//!   instance got background semantics in the host but no tray icon — the
//!   "invisible background service" WS4 exists to prevent.
//! - There was no setting at all, so the tray was reachable only by exporting
//!   two env vars before launching.
//!
//! Resolving once here, before any thread or child exists, and writing the
//! result into the launcher's own env fixes both: the tray reads it, the macOS
//! headless-pump branch in `main` reads it, and every child (host) inherits it.
//!
//! ## Precedence
//!
//! Either env var already present (developer override) → on. Otherwise
//! `--background` → on. Otherwise `settings.json` `"app:runinbackground"`,
//! which **defaults to on**: a missing file (fresh install) or a missing key
//! reads as on, and only an explicit `false` turns it off. **Fail-safe:** a
//! settings file that exists but cannot be read or parsed, or holds a non-bool
//! value, resolves to *off*; the user's real choice is unknown, and a broken
//! read must never silently turn a foreground app into a resident one.
//!
//! Default-on is safe only because a resident process always shows an icon: if
//! the tray cannot start (e.g. Linux without a StatusNotifier host), the host
//! is spawned without background mode (`tray::unavailable`), so closing the
//! last window still quits.
//!
//! The two switches are always set together, preserving `tray::should_enable`'s
//! deliberate pairing (a tray without background mode would lie about whether
//! the app is still running).

use std::path::{Path, PathBuf};

/// Settings key. Flat `namespace:key`, like every other `settings.json` entry.
pub const SETTINGS_KEY: &str = "app:runinbackground";

const ENV_BACKGROUND: &str = "AGENTMUX_BACKGROUND_SERVICE";
const ENV_TRAY: &str = "AGENTMUX_TRAY";

/// Pure decision, so every input combination is unit-testable.
pub fn should_run_in_background(env_already_set: bool, background_flag: bool, setting: bool) -> bool {
    env_already_set || background_flag || setting
}

/// Read `"app:runinbackground"` from a settings.json file.
/// Missing file or missing key → `true` (the default). Unreadable file, parse
/// error or non-bool value → `false` (fail-safe).
pub fn settings_run_in_background(settings_path: &Path) -> bool {
    let text = match std::fs::read_to_string(settings_path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
        Err(_) => return false,
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    match json.get(SETTINGS_KEY) {
        None => true,
        Some(v) => v.as_bool().unwrap_or(false),
    }
}

/// Resolve background mode and, when on, export both switches into the
/// launcher's own environment. Returns the resolved value.
///
/// MUST be called from `main` before any thread is spawned (`set_var` is not
/// thread-safe on every platform) and before the tray/supervisor start.
pub fn apply(args: &[String]) -> bool {
    let env_already_set =
        std::env::var_os(ENV_BACKGROUND).is_some() || std::env::var_os(ENV_TRAY).is_some();
    let flag = crate::autostart::background_requested(args);
    // Only touch the filesystem when nothing cheaper already decided it.
    let setting = !env_already_set
        && !flag
        && std::panic::catch_unwind(|| {
            config_dir()
                .map(|d| settings_run_in_background(&d.join("settings.json")))
                .unwrap_or(false)
        })
        .unwrap_or(false);

    let on = should_run_in_background(env_already_set, flag, setting);
    if on {
        std::env::set_var(ENV_BACKGROUND, "1");
        std::env::set_var(ENV_TRAY, "1");
    }
    on
}

/// Best-effort config dir (holds settings.json), resolved the same way
/// `splash_config` does — early and failure-tolerant.
fn config_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    let version = env!("CARGO_PKG_VERSION");
    crate::data_dir::resolve_paths(exe_dir, version)
        .ok()
        .map(|p| p.config_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_tmp(tag: &str, content: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "agentmux-bgcfg-{tag}-{}.json",
            std::process::id()
        ));
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn any_single_source_turns_it_on() {
        assert!(should_run_in_background(true, false, false));
        assert!(should_run_in_background(false, true, false));
        assert!(should_run_in_background(false, false, true));
        assert!(!should_run_in_background(false, false, false));
    }

    #[test]
    fn setting_true_reads_on() {
        let p = write_tmp("on", r#"{"app:runinbackground": true}"#);
        assert!(settings_run_in_background(&p));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn an_explicit_false_turns_it_off() {
        let p = write_tmp("off", r#"{"app:runinbackground": false}"#);
        assert!(!settings_run_in_background(&p));
        let _ = std::fs::remove_file(&p);
    }

    /// On by default: a fresh install has no settings.json, and an existing
    /// one without the key never chose.
    #[test]
    fn a_missing_file_or_key_defaults_on() {
        assert!(settings_run_in_background(Path::new("/no/such/agentmux-settings.json")));
        let p = write_tmp("absent", r#"{"term:fontsize": 14}"#);
        assert!(settings_run_in_background(&p));
        let _ = std::fs::remove_file(&p);
        let p = write_tmp("empty-obj", "{}");
        assert!(settings_run_in_background(&p));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn fail_safe_off_on_bad_or_wrong_type() {
        let p = write_tmp("bad", "not json {");
        assert!(!settings_run_in_background(&p));
        let _ = std::fs::remove_file(&p);
        // A string "true" is not a bool — must not enable a resident process.
        let p = write_tmp("str", r#"{"app:runinbackground": "true"}"#);
        assert!(!settings_run_in_background(&p));
        let _ = std::fs::remove_file(&p);
    }

    /// The regression this module exists for: `--background` must make the
    /// launcher's OWN tray gate pass, not just the host's env.
    #[test]
    fn background_flag_reaches_launcher_tray_gate() {
        let args = vec!["agentmux".to_string(), crate::autostart::BACKGROUND_FLAG.to_string()];
        // This is the only test in the crate that touches these two vars, so
        // no cross-test lock is needed; restore them afterwards regardless.
        let prev_bg = std::env::var_os(ENV_BACKGROUND);
        let prev_tray = std::env::var_os(ENV_TRAY);
        std::env::remove_var(ENV_BACKGROUND);
        std::env::remove_var(ENV_TRAY);

        assert!(apply(&args));
        assert!(crate::tray::should_enable(
            crate::tray::tray_opt_in_from_env(),
            crate::tray::background_service_from_env()
        ));

        match prev_bg {
            Some(v) => std::env::set_var(ENV_BACKGROUND, v),
            None => std::env::remove_var(ENV_BACKGROUND),
        }
        match prev_tray {
            Some(v) => std::env::set_var(ENV_TRAY, v),
            None => std::env::remove_var(ENV_TRAY),
        }
    }
}
