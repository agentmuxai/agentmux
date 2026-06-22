// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! "Disable the splash" preference, resolved by the launcher *before* it spawns
//! the splash (and before the host/srv exist), per
//! `SPEC_SPLASH_USERINFO_AND_DISABLE_2026_06_21.md` §6.
//!
//! Precedence: env override wins; otherwise `settings.json` `"splash:disabled"`.
//! **Fail-safe:** any error reading/parsing settings resolves to *enabled* — a
//! broken read must never silently suppress the splash.

use std::path::{Path, PathBuf};

/// True when the splash should be suppressed entirely (no window created).
pub fn splash_disabled() -> bool {
    if let Some(forced) = env_override() {
        return forced; // env is authoritative (Some(true)=disable, Some(false)=force-on)
    }
    // Best-effort settings.json read. Wrapped so a resolver panic/IO error can
    // never take down launcher startup — on any failure we keep the splash.
    std::panic::catch_unwind(|| {
        config_dir()
            .map(|d| settings_splash_disabled(&d.join("settings.json")))
            .unwrap_or(false)
    })
    .unwrap_or(false)
}

/// `Some(true)` = disable, `Some(false)` = force-enable, `None` = no opinion.
fn env_override() -> Option<bool> {
    if let Ok(v) = std::env::var("AGENTMUX_NO_SPLASH") {
        if truthy(&v) {
            return Some(true);
        }
    }
    if let Ok(v) = std::env::var("AGENTMUX_SPLASH") {
        return match v.trim().to_ascii_lowercase().as_str() {
            "0" | "false" | "off" | "no" => Some(true),
            "1" | "true" | "on" | "yes" => Some(false),
            _ => None,
        };
    }
    None
}

fn truthy(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "on" | "yes"
    )
}

/// Read `"splash:disabled"` (flat `namespace:key`) from a settings.json file.
/// Missing file / parse error / missing key → `false` (enabled).
pub fn settings_splash_disabled(settings_path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(settings_path) else {
        return false;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    json.get("splash:disabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Best-effort config dir (holds settings.json) resolved the same way
/// `launcher_main` does — but early and failure-tolerant.
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
        let p = std::env::temp_dir().join(format!("agentmux-splashcfg-{tag}-{}.json", std::process::id()));
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn settings_disabled_true() {
        let p = write_tmp("on", r#"{"splash:disabled": true}"#);
        assert!(settings_splash_disabled(&p));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn settings_disabled_false_when_key_false() {
        let p = write_tmp("off", r#"{"splash:disabled": false}"#);
        assert!(!settings_splash_disabled(&p));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn settings_default_enabled_when_key_absent() {
        let p = write_tmp("absent", r#"{"term:fontsize": 14}"#);
        assert!(!settings_splash_disabled(&p));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn settings_fail_safe_enabled_on_missing_or_bad() {
        // Missing file and malformed JSON both resolve to enabled (false).
        assert!(!settings_splash_disabled(Path::new("/no/such/agentmux-settings.json")));
        let p = write_tmp("bad", "this is not json {");
        assert!(!settings_splash_disabled(&p));
        let _ = std::fs::remove_file(&p);
    }
}
