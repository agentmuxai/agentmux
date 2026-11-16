// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The claude.ai account's connectors (Gmail, Calendar, Drive, Docs) are off in Claude agents by default.
//!
//! Claude Code loads the connectors of the signed-in claude.ai account into every session. AgentMux has its
//! own Google tools (`Gmail`, `Calendar`, `Drive`, once the person grants them), so the claude.ai ones only
//! add a second, unauthorized copy of each, and agents keep telling the person to authorize them in claude.ai.
//!
//! The switch is an environment variable the CLI reads at launch (`ENABLE_CLAUDEAI_MCP_SERVERS`, also the
//! CLI's `disableClaudeAiConnectors` setting). Its name is the CLI's, not ours: when the pinned CLI version
//! moves, the canary test below and this note are rechecked against it.
//!
//! Setting `agent:claudeaiconnectors`: `false` (the default) launches Claude agents with the variable set to
//! `false`; `true` leaves it unset, so the CLI loads the connectors as it does outside AgentMux. It takes
//! effect at an agent's next launch.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use super::wconfig::ConfigState;

/// The setting. Off by default.
pub const SETTING_CLAUDEAI_CONNECTORS: &str = "agent:claudeaiconnectors";

/// The CLI's own switch: `false` keeps the claude.ai connectors out of the session.
pub const ENV_SWITCH: &str = "ENABLE_CLAUDEAI_MCP_SERVERS";

static CONFIG: OnceLock<Arc<ConfigState>> = OnceLock::new();

/// Attach the live settings, so [`SETTING_CLAUDEAI_CONNECTORS`] is read at each launch (bootstrap, through
/// `cli_messaging::install_config`).
pub fn install_config(config: Arc<ConfigState>) {
    let _ = CONFIG.set(config);
}

/// Whether the connectors stay on: only a boolean `true` turns them on.
pub fn enabled(extra: &HashMap<String, serde_json::Value>) -> bool {
    extra.get(SETTING_CLAUDEAI_CONNECTORS).and_then(|v| v.as_bool()).unwrap_or(false)
}

/// Keep the connectors out of a launch environment: a Claude agent's, when the setting is off. It
/// overwrites what the environment already carries (a persisted `cmd:env`), as the setting is the only way
/// to turn them back on. Any other provider's launch is left alone.
pub fn apply(env_vars: &mut HashMap<String, String>, provider: &str, connectors_on: bool) {
    if provider == "claude" && !connectors_on {
        env_vars.insert(ENV_SWITCH.to_string(), "false".to_string());
    }
}

/// [`apply`] for a launch of `provider` (already resolved from a legacy alias), reading the setting now.
/// Called from `cli_messaging::apply_for_spawn`, which resolves the block's provider for both switches.
pub fn apply_for_spawn(env_vars: &mut HashMap<String, String>, provider: &str) {
    let on = CONFIG.get().is_some_and(|c| enabled(&c.get_settings().extra));
    apply(env_vars, provider, on);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extra(value: Option<serde_json::Value>) -> HashMap<String, serde_json::Value> {
        value.into_iter().map(|v| (SETTING_CLAUDEAI_CONNECTORS.to_string(), v)).collect()
    }

    #[test]
    fn the_connectors_are_off_unless_the_setting_is_true() {
        assert!(!enabled(&extra(None)));
        assert!(!enabled(&extra(Some(serde_json::json!("yes")))));
        assert!(!enabled(&extra(Some(serde_json::json!(false)))));
        assert!(enabled(&extra(Some(serde_json::json!(true)))));
    }

    /// The canary: the switch is present, as `false`, for a Claude agent while the setting is off, and absent
    /// once it is on. If the CLI renames its switch, this is where the new name goes.
    #[test]
    fn a_claude_launch_carries_the_switch_with_the_setting_off_and_not_with_it_on() {
        let mut off = HashMap::new();
        apply(&mut off, "claude", false);
        assert_eq!(off.get(ENV_SWITCH).map(String::as_str), Some("false"));

        let mut on = HashMap::new();
        apply(&mut on, "claude", true);
        assert!(!on.contains_key(ENV_SWITCH));
    }

    #[test]
    fn the_switch_overwrites_a_persisted_value_and_other_providers_are_left_alone() {
        let mut persisted = HashMap::from([(ENV_SWITCH.to_string(), "true".to_string())]);
        apply(&mut persisted, "claude", false);
        assert_eq!(persisted.get(ENV_SWITCH).map(String::as_str), Some("false"));

        let mut codex = HashMap::new();
        apply(&mut codex, "codex", false);
        assert!(codex.is_empty());
    }
}
