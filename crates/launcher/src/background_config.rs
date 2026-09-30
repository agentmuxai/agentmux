// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The tray icon and background-service mode, resolved by the launcher at
//! startup — `docs/specs/SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md` §4.1.
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
//! ## Two switches, two defaults
//!
//! They are separate questions and must not be conflated (#3785 did, and
//! turned background mode on for everyone; see
//! `docs/reports/REPORT_TRAY_BACKGROUND_DEFAULT_ON_2026_09_28.md`):
//!
//! - **Tray icon** (`"app:showtray"`) — **on by default**. Shown while
//!   AgentMux runs. Only an explicit `false` hides it.
//! - **Background mode** (`"app:runinbackground"`) — keep running after the
//!   last window closes. **Opt-in**: only an explicit `true` turns it on, so a
//!   fresh install quits on last-window-close.
//!
//! Background mode always implies the tray: a resident process must show an
//! icon, so `app:showtray: false` is overridden while background mode is on.
//! And if the tray cannot start (e.g. Linux without a StatusNotifier host),
//! the host is spawned without background mode (`tray::unavailable`), so
//! closing the last window quits.
//!
//! ## Precedence
//!
//! Background: `AGENTMUX_BACKGROUND_SERVICE` already set (developer override)
//! → on; otherwise `--background` → on; otherwise the setting. Tray:
//! background on, or `AGENTMUX_TRAY` already set → on; otherwise the setting.
//! A settings file that cannot be read or parsed leaves both at their default
//! (the user's real choice is unknown): background off, tray on.

use std::path::{Path, PathBuf};

/// Settings keys. Flat `namespace:key`, like every other `settings.json` entry.
pub const BACKGROUND_KEY: &str = "app:runinbackground";
pub const TRAY_KEY: &str = "app:showtray";

const ENV_BACKGROUND: &str = "AGENTMUX_BACKGROUND_SERVICE";
const ENV_TRAY: &str = "AGENTMUX_TRAY";

/// The two settings as stored, with their defaults applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub run_in_background: bool,
    pub show_tray: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            run_in_background: false,
            show_tray: true,
        }
    }
}

/// What the launcher exports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolved {
    pub background: bool,
    pub tray: bool,
}

/// Pure decision, so every input combination is unit-testable.
pub fn resolve(env_background: bool, env_tray: bool, background_flag: bool, settings: Settings) -> Resolved {
    let background = env_background || background_flag || settings.run_in_background;
    let tray = background || env_tray || settings.show_tray;
    Resolved { background, tray }
}

/// Read both settings from a settings.json file. Each key keeps its default
/// unless it holds a bool; a missing or unreadable file yields the defaults.
pub fn read_settings(settings_path: &Path) -> Settings {
    let mut s = Settings::default();
    let Ok(text) = std::fs::read_to_string(settings_path) else {
        return s;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return s;
    };
    if let Some(v) = json.get(BACKGROUND_KEY).and_then(|v| v.as_bool()) {
        s.run_in_background = v;
    }
    if let Some(v) = json.get(TRAY_KEY).and_then(|v| v.as_bool()) {
        s.show_tray = v;
    }
    s
}

/// Resolve both switches and export the ones that are on into the launcher's
/// own environment.
///
/// MUST be called from `main` before any thread is spawned (`set_var` is not
/// thread-safe on every platform) and before the tray/supervisor start.
pub fn apply(args: &[String]) -> Resolved {
    let env_background = std::env::var_os(ENV_BACKGROUND).is_some();
    let env_tray = std::env::var_os(ENV_TRAY).is_some();
    let flag = crate::autostart::background_requested(args);
    let settings = std::panic::catch_unwind(|| {
        config_dir()
            .map(|d| read_settings(&d.join("settings.json")))
            .unwrap_or_default()
    })
    .unwrap_or_default();

    let r = resolve(env_background, env_tray, flag, settings);
    if r.background {
        std::env::set_var(ENV_BACKGROUND, "1");
    }
    if r.tray {
        std::env::set_var(ENV_TRAY, "1");
    }
    r
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

    fn read(tag: &str, content: &str) -> Settings {
        let p = write_tmp(tag, content);
        let s = read_settings(&p);
        let _ = std::fs::remove_file(&p);
        s
    }

    const DEFAULTS: Settings = Settings {
        run_in_background: false,
        show_tray: true,
    };

    /// A fresh install (no settings.json) and one that never touched either
    /// key: the tray is shown, closing the last window quits.
    #[test]
    fn defaults_are_tray_on_background_off() {
        assert_eq!(read_settings(Path::new("/no/such/agentmux-settings.json")), DEFAULTS);
        assert_eq!(read("absent", r#"{"term:fontsize": 14}"#), DEFAULTS);
        assert_eq!(read("empty-obj", "{}"), DEFAULTS);
        assert_eq!(
            resolve(false, false, false, DEFAULTS),
            Resolved { background: false, tray: true }
        );
    }

    #[test]
    fn each_key_is_read_independently() {
        assert_eq!(
            read("bg-on", r#"{"app:runinbackground": true}"#),
            Settings { run_in_background: true, show_tray: true }
        );
        assert_eq!(
            read("tray-off", r#"{"app:showtray": false}"#),
            Settings { run_in_background: false, show_tray: false }
        );
    }

    #[test]
    fn a_bad_file_or_wrong_type_keeps_the_defaults() {
        assert_eq!(read("bad", "not json {"), DEFAULTS);
        // A string "true" is not a bool — must not enable a resident process.
        assert_eq!(read("str", r#"{"app:runinbackground": "true", "app:showtray": "false"}"#), DEFAULTS);
    }

    #[test]
    fn background_is_opt_in_from_any_single_source() {
        assert!(resolve(true, false, false, DEFAULTS).background, "env override");
        assert!(resolve(false, false, true, DEFAULTS).background, "--background");
        let on = Settings { run_in_background: true, ..DEFAULTS };
        assert!(resolve(false, false, false, on).background, "setting");
        assert!(!resolve(false, true, false, DEFAULTS).background, "the tray alone never implies background");
    }

    /// A resident process must always show an icon.
    #[test]
    fn background_forces_the_tray_even_when_hidden_in_settings() {
        let hidden = Settings { run_in_background: false, show_tray: false };
        assert_eq!(resolve(false, false, false, hidden), Resolved { background: false, tray: false });
        assert_eq!(resolve(false, false, true, hidden), Resolved { background: true, tray: true });
        let both = Settings { run_in_background: true, show_tray: false };
        assert_eq!(resolve(false, false, false, both), Resolved { background: true, tray: true });
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

        assert!(apply(&args).background);
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
