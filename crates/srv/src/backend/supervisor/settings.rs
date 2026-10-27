// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Supervisor's settings: the master switch and the limits. Read from the
//! live config when a decision is made, never from a snapshot taken at start,
//! so a change in the Settings pane or by hand in settings.json applies to the
//! next decision. docs/specs/SPEC_SWARM_SECTIONS_SUPERVISOR_SETTINGS_AND_KANBAN_2026_10_24.md §5, §6.2.

use std::sync::{Arc, OnceLock};

use crate::backend::wconfig::ConfigState;

use super::{Limits, SupervisorMode};

/// `supervisor:enabled`: the master switch. While off nothing is judged or
/// continued and a mode can't be set (Off is always accepted).
pub const KEY_ENABLED: &str = "supervisor:enabled";
pub const KEY_STEADY_CONTINUATIONS: &str = "supervisor:steady:continuations";
pub const KEY_STEADY_MINUTES: &str = "supervisor:steady:minutes";
pub const KEY_DRIVE_CONTINUATIONS: &str = "supervisor:drive:continuations";
pub const KEY_DRIVE_MINUTES: &str = "supervisor:drive:minutes";
pub const KEY_NO_PROGRESS: &str = "supervisor:noprogress";

/// Every key the Supervisor reads, for callers that watch for a change.
pub const KEYS: [&str; 6] = [
    KEY_ENABLED,
    KEY_STEADY_CONTINUATIONS,
    KEY_STEADY_MINUTES,
    KEY_DRIVE_CONTINUATIONS,
    KEY_DRIVE_MINUTES,
    KEY_NO_PROGRESS,
];

// The defaults are the schema's (schema/settings.json); the settings-defaults
// test checks they agree.
const DEFAULT_ENABLED: bool = false;
const DEFAULT_STEADY_CONTINUATIONS: u32 = 5;
const DEFAULT_STEADY_MINUTES: u32 = 0;
const DEFAULT_DRIVE_CONTINUATIONS: u32 = 15;
const DEFAULT_DRIVE_MINUTES: u32 = 120;
const DEFAULT_NO_PROGRESS: u32 = 2;

/// Bounds a value is clamped to (a hand edit can be anything).
const CONTINUATIONS_RANGE: (u32, u32) = (1, 100);
const MINUTES_RANGE: (u32, u32) = (0, 1440);
const NO_PROGRESS_RANGE: (u32, u32) = (1, 10);

static CONFIG: OnceLock<Arc<ConfigState>> = OnceLock::new();

/// Attach the live settings (bootstrap). Until then the defaults apply.
pub fn install_config(config: Arc<ConfigState>) {
    let _ = CONFIG.set(config);
}

/// What the settings say now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SupervisorSettings {
    pub enabled: bool,
    pub steady_continuations: u32,
    pub steady_minutes: u32,
    pub drive_continuations: u32,
    pub drive_minutes: u32,
    pub no_progress: u32,
}

impl Default for SupervisorSettings {
    fn default() -> Self {
        SupervisorSettings {
            enabled: DEFAULT_ENABLED,
            steady_continuations: DEFAULT_STEADY_CONTINUATIONS,
            steady_minutes: DEFAULT_STEADY_MINUTES,
            drive_continuations: DEFAULT_DRIVE_CONTINUATIONS,
            drive_minutes: DEFAULT_DRIVE_MINUTES,
            no_progress: DEFAULT_NO_PROGRESS,
        }
    }
}

fn number(config: &ConfigState, key: &str, default: u32, (min, max): (u32, u32)) -> u32 {
    match config.extra_u64(key) {
        None => default,
        Some(v) => {
            let clamped = v.clamp(u64::from(min), u64::from(max)) as u32;
            if u64::from(clamped) != v {
                tracing::warn!(key, value = v, used = clamped, "supervisor: setting out of range, clamped");
            }
            clamped
        }
    }
}

impl SupervisorSettings {
    pub fn from_config(config: &ConfigState) -> Self {
        SupervisorSettings {
            enabled: config.extra_bool(KEY_ENABLED).unwrap_or(DEFAULT_ENABLED),
            steady_continuations: number(config, KEY_STEADY_CONTINUATIONS, DEFAULT_STEADY_CONTINUATIONS, CONTINUATIONS_RANGE),
            steady_minutes: number(config, KEY_STEADY_MINUTES, DEFAULT_STEADY_MINUTES, MINUTES_RANGE),
            drive_continuations: number(config, KEY_DRIVE_CONTINUATIONS, DEFAULT_DRIVE_CONTINUATIONS, CONTINUATIONS_RANGE),
            drive_minutes: number(config, KEY_DRIVE_MINUTES, DEFAULT_DRIVE_MINUTES, MINUTES_RANGE),
            no_progress: number(config, KEY_NO_PROGRESS, DEFAULT_NO_PROGRESS, NO_PROGRESS_RANGE),
        }
    }

    /// The settings now; the defaults before the config is attached.
    pub fn current() -> Self {
        CONFIG.get().map(|c| Self::from_config(c)).unwrap_or_default()
    }

    /// `mode`'s limits under these settings. Off has none.
    pub fn limits(&self, mode: SupervisorMode) -> Limits {
        let minutes = |m: u32| (m > 0).then(|| u64::from(m) * 60 * 1000);
        match mode {
            SupervisorMode::Off => Limits { continuations: 0, run_ms: None, no_progress: 0 },
            SupervisorMode::Steady => Limits {
                continuations: self.steady_continuations,
                run_ms: minutes(self.steady_minutes),
                no_progress: self.no_progress,
            },
            SupervisorMode::Drive => Limits {
                continuations: self.drive_continuations,
                run_ms: minutes(self.drive_minutes),
                no_progress: self.no_progress,
            },
        }
    }
}

#[cfg(test)]
thread_local! {
    static ENABLED_IN_TEST: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Set the master switch for the calling test thread only.
#[cfg(test)]
pub fn set_enabled_for_test(on: bool) {
    ENABLED_IN_TEST.with(|c| c.set(Some(on)));
}

/// Whether the master switch is on now.
pub fn enabled() -> bool {
    #[cfg(test)]
    if let Some(on) = ENABLED_IN_TEST.with(|c| c.get()) {
        return on;
    }
    CONFIG.get().and_then(|c| c.extra_bool(KEY_ENABLED)).unwrap_or(DEFAULT_ENABLED)
}

/// The sentence a refused mode change gives.
pub const SWITCHED_OFF: &str = "The Supervisor is switched off. Turn it on with supervisor:enabled in the settings.";

/// Log a settings write that touches the Supervisor's keys, with who made it
/// (`agent_id` is empty for the app itself).
pub fn log_change(new_keys: &serde_json::Map<String, serde_json::Value>, agent_id: &str) {
    let changed: Vec<String> =
        KEYS.iter().filter_map(|k| new_keys.get(*k).map(|v| format!("{k}={v}"))).collect();
    if changed.is_empty() {
        return;
    }
    let caller = if agent_id.is_empty() { "the app" } else { agent_id };
    tracing::info!(caller, changed = %changed.join(", "), "supervisor: settings changed");
}
