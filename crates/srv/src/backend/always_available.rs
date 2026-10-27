// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The switch for agents that outlive their pane
//! (`docs/specs/SPEC_ALWAYS_AVAILABLE_AGENTS_2026_10_27.md` §4, D9). Off, closing a pane deletes the
//! agent's block as it always has; on, closing it parks the block (`Command::ParkBlock`) and the
//! agent keeps running.

use std::collections::HashMap;

/// The setting. Off by default through P1 and P2.
pub const SETTING_ALWAYS_AVAILABLE: &str = "agents:always_available";

/// Whether closing a pane parks the agent's block instead of deleting it.
#[cfg_attr(not(test), allow(dead_code))]
pub fn enabled(extra: &HashMap<String, serde_json::Value>) -> bool {
    extra.get(SETTING_ALWAYS_AVAILABLE).and_then(|v| v.as_bool()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_is_off_unless_the_setting_is_true() {
        let mut extra = HashMap::new();
        assert!(!enabled(&extra));
        extra.insert(SETTING_ALWAYS_AVAILABLE.to_string(), serde_json::json!("yes"));
        assert!(!enabled(&extra), "only a boolean turns it on");
        extra.insert(SETTING_ALWAYS_AVAILABLE.to_string(), serde_json::json!(false));
        assert!(!enabled(&extra));
        extra.insert(SETTING_ALWAYS_AVAILABLE.to_string(), serde_json::json!(true));
        assert!(enabled(&extra));
    }
}
