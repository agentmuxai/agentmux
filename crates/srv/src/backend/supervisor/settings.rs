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
/// `supervisor:cooldown`: seconds between a turn's end and its continuation.
pub const KEY_COOLDOWN: &str = "supervisor:cooldown";
/// `supervisor:hourly`: continuations across all panes in a rolling hour.
pub const KEY_HOURLY: &str = "supervisor:hourly";
pub const KEY_NOTIFY_NEEDS_YOU: &str = "supervisor:notify:needsyou";
pub const KEY_NOTIFY_LIMIT: &str = "supervisor:notify:limit";
pub const KEY_NOTIFY_FAILED: &str = "supervisor:notify:failed";
pub const KEY_NOTIFY_DONE: &str = "supervisor:notify:done";

/// Every key the Supervisor reads, for callers that watch for a change.
pub const KEYS: [&str; 12] = [
    KEY_ENABLED,
    KEY_STEADY_CONTINUATIONS,
    KEY_STEADY_MINUTES,
    KEY_DRIVE_CONTINUATIONS,
    KEY_DRIVE_MINUTES,
    KEY_NO_PROGRESS,
    KEY_COOLDOWN,
    KEY_HOURLY,
    KEY_NOTIFY_NEEDS_YOU,
    KEY_NOTIFY_LIMIT,
    KEY_NOTIFY_FAILED,
    KEY_NOTIFY_DONE,
];

// The defaults are the schema's (schema/settings.json); the settings-defaults
// test checks they agree.
const DEFAULT_ENABLED: bool = false;
const DEFAULT_STEADY_CONTINUATIONS: u32 = 5;
const DEFAULT_STEADY_MINUTES: u32 = 0;
const DEFAULT_DRIVE_CONTINUATIONS: u32 = 15;
const DEFAULT_DRIVE_MINUTES: u32 = 120;
const DEFAULT_NO_PROGRESS: u32 = 2;
const DEFAULT_COOLDOWN: u32 = 30;
const DEFAULT_HOURLY: u32 = 30;
const DEFAULT_NOTIFY: bool = true;

/// Bounds a value is clamped to (a hand edit can be anything).
const CONTINUATIONS_RANGE: (u32, u32) = (1, 100);
const MINUTES_RANGE: (u32, u32) = (0, 1440);
const NO_PROGRESS_RANGE: (u32, u32) = (1, 10);
const COOLDOWN_RANGE: (u32, u32) = (0, 600);
const HOURLY_RANGE: (u32, u32) = (0, 1000);

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
    /// Seconds a continuation waits after its turn ended (0 sends at once).
    pub cooldown_secs: u32,
    /// Continuations across every pane in a rolling hour (0 is no cap).
    pub hourly: u32,
    pub notify_needs_you: bool,
    pub notify_limit: bool,
    pub notify_failed: bool,
    pub notify_done: bool,
}

/// The groups of Supervisor notices the notify settings narrow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeGroup {
    /// It needs you: a question, "needs you", "blocked".
    NeedsYou,
    /// It hit a limit: continuations, time, no progress, the same reply twice, the fleet budget.
    Limit,
    /// It couldn't continue: delivery failed, the agent can't run.
    Failed,
    /// It found the work done.
    Done,
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
            cooldown_secs: DEFAULT_COOLDOWN,
            hourly: DEFAULT_HOURLY,
            notify_needs_you: DEFAULT_NOTIFY,
            notify_limit: DEFAULT_NOTIFY,
            notify_failed: DEFAULT_NOTIFY,
            notify_done: DEFAULT_NOTIFY,
        }
    }
}

/// Whether this out-of-range `(key, value)` hasn't been logged yet: settings
/// are read at every decision, and one bad value would repeat its warning.
fn first_clamp(key: &str, value: u64) -> bool {
    static LOGGED: std::sync::Mutex<Vec<(String, u64)>> = std::sync::Mutex::new(Vec::new());
    let mut logged = LOGGED.lock().unwrap_or_else(|e| e.into_inner());
    if logged.iter().any(|(k, v)| k == key && *v == value) {
        return false;
    }
    if logged.len() >= 64 {
        logged.clear();
    }
    logged.push((key.to_string(), value));
    true
}

fn number(config: &ConfigState, key: &str, default: u32, (min, max): (u32, u32)) -> u32 {
    match config.extra_u64(key) {
        None => default,
        Some(v) => {
            let clamped = v.clamp(u64::from(min), u64::from(max)) as u32;
            if u64::from(clamped) != v && first_clamp(key, v) {
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
            cooldown_secs: number(config, KEY_COOLDOWN, DEFAULT_COOLDOWN, COOLDOWN_RANGE),
            hourly: number(config, KEY_HOURLY, DEFAULT_HOURLY, HOURLY_RANGE),
            notify_needs_you: config.extra_bool(KEY_NOTIFY_NEEDS_YOU).unwrap_or(DEFAULT_NOTIFY),
            notify_limit: config.extra_bool(KEY_NOTIFY_LIMIT).unwrap_or(DEFAULT_NOTIFY),
            notify_failed: config.extra_bool(KEY_NOTIFY_FAILED).unwrap_or(DEFAULT_NOTIFY),
            notify_done: config.extra_bool(KEY_NOTIFY_DONE).unwrap_or(DEFAULT_NOTIFY),
        }
    }

    /// The settings now; the defaults before the config is attached.
    pub fn current() -> Self {
        #[cfg(test)]
        if let Some(s) = SETTINGS_IN_TEST.with(|c| c.get()) {
            return s;
        }
        CONFIG.get().map(|c| Self::from_config(c)).unwrap_or_default()
    }

    /// Whether the OS notification for `group` is on. The rows, lines and
    /// history show every stop regardless; the global `notify:os:*` toggles
    /// still apply on top.
    pub fn notifies(&self, group: NoticeGroup) -> bool {
        match group {
            NoticeGroup::NeedsYou => self.notify_needs_you,
            NoticeGroup::Limit => self.notify_limit,
            NoticeGroup::Failed => self.notify_failed,
            NoticeGroup::Done => self.notify_done,
        }
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
    static SETTINGS_IN_TEST: std::cell::Cell<Option<SupervisorSettings>> = const { std::cell::Cell::new(None) };
}

/// Make [`SupervisorSettings::current`] return `settings` for the calling test thread only.
#[cfg(test)]
pub fn set_settings_for_test(settings: SupervisorSettings) {
    SETTINGS_IN_TEST.with(|c| c.set(Some(settings)));
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

/// Parse a `setconfig` request into its keys, logging who changed a
/// Supervisor key (anyone who can write settings may; none is anonymous).
pub fn parse_setconfig(
    data: serde_json::Value,
    agent_id: &str,
) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    let keys: serde_json::Map<String, serde_json::Value> =
        serde_json::from_value(data).map_err(|e| format!("setconfig: {e}"))?;
    log_change(&keys, agent_id);
    Ok(keys)
}
