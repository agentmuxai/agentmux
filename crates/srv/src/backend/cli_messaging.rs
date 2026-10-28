// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! AgentMux agents message each other through jekt, so the Claude CLI's own
//! separate cross-session messaging is turned off by default.
//!
//! The switch is an environment variable the CLI reads at launch. Its name is an
//! internal of the CLI, not a documented setting, so it can change from one CLI
//! version to the next: when the pinned CLI version moves, the tests below (the
//! canary) and this note must be rechecked against the new CLI.
//!
//! Setting `agent:climessaging`: `false` (the default) launches Claude agents
//! with the variable set to `0`; `true` leaves it unset, so the CLI behaves as it
//! does outside AgentMux.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use super::obj::{meta_get_string, MetaMapType};
use super::wconfig::ConfigState;

/// The setting. Off by default.
pub const SETTING_CLI_MESSAGING: &str = "agent:climessaging";

/// The CLI's own gate for its cross-session messaging (set to `0` to turn it
/// off). An undocumented name: recheck on every CLI upgrade.
pub const ENV_GATE: &str = "CLAUDE_CODE_HARBOR_KITE";

static CONFIG: OnceLock<Arc<ConfigState>> = OnceLock::new();

/// Attach the live settings (bootstrap), so [`SETTING_CLI_MESSAGING`] is read at each launch.
pub fn install_config(config: Arc<ConfigState>) {
    let _ = CONFIG.set(config);
}

/// Whether the CLI's own messaging stays on: only a boolean `true` turns it on.
pub fn enabled(extra: &HashMap<String, serde_json::Value>) -> bool {
    extra.get(SETTING_CLI_MESSAGING).and_then(|v| v.as_bool()).unwrap_or(false)
}

/// Turn the CLI's messaging off in a launch environment: a Claude agent's, when the setting is off.
/// It overwrites what the environment already carries (a persisted `cmd:env`), as the setting is the
/// only way to turn it back on. Any other provider's launch is left alone.
pub fn apply(env_vars: &mut HashMap<String, String>, provider: &str, messaging_on: bool) {
    if provider == "claude" && !messaging_on {
        env_vars.insert(ENV_GATE.to_string(), "0".to_string());
    }
}

/// [`apply`] for a block's launch, reading the provider from its meta and the setting now.
pub fn apply_for_spawn(env_vars: &mut HashMap<String, String>, block_meta: &MetaMapType) {
    let provider = meta_get_string(block_meta, "agentProvider", "");
    let on = CONFIG.get().is_some_and(|c| enabled(&c.get_settings().extra));
    apply(env_vars, &provider, on);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extra(value: Option<serde_json::Value>) -> HashMap<String, serde_json::Value> {
        value.into_iter().map(|v| (SETTING_CLI_MESSAGING.to_string(), v)).collect()
    }

    #[test]
    fn the_cli_messaging_is_off_unless_the_setting_is_true() {
        assert!(!enabled(&extra(None)));
        assert!(!enabled(&extra(Some(serde_json::json!("yes")))));
        assert!(!enabled(&extra(Some(serde_json::json!(false)))));
        assert!(enabled(&extra(Some(serde_json::json!(true)))));
    }

    /// The canary: the gate is present, as `0`, for a Claude agent while the setting is off, and absent
    /// once it is on. If the CLI renames its gate, this is where the new name goes.
    #[test]
    fn a_claude_launch_carries_the_gate_with_the_setting_off_and_not_with_it_on() {
        let mut off = HashMap::new();
        apply(&mut off, "claude", false);
        assert_eq!(off.get(ENV_GATE).map(String::as_str), Some("0"));

        let mut on = HashMap::new();
        apply(&mut on, "claude", true);
        assert!(!on.contains_key(ENV_GATE));
    }

    #[test]
    fn the_gate_overwrites_a_persisted_value_and_other_providers_are_left_alone() {
        let mut persisted = HashMap::from([(ENV_GATE.to_string(), "1".to_string())]);
        apply(&mut persisted, "claude", false);
        assert_eq!(persisted.get(ENV_GATE).map(String::as_str), Some("0"));

        let mut codex = HashMap::new();
        apply(&mut codex, "codex", false);
        assert!(codex.is_empty());
    }

    #[test]
    fn a_block_meta_names_the_provider() {
        let mut meta = MetaMapType::new();
        meta.insert("agentProvider".to_string(), serde_json::json!("claude"));
        let mut env = HashMap::new();
        apply_for_spawn(&mut env, &meta);
        assert_eq!(env.get(ENV_GATE).map(String::as_str), Some("0"), "no settings installed: the default, off");
    }
}
