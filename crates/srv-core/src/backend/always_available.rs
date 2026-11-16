// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The switch for agents that outlive their pane
//! (`docs/specs/SPEC_ALWAYS_AVAILABLE_AGENTS_2026_10_27.md` §4, D9). Off, closing a pane deletes the
//! agent's block as it always has; on, closing it parks the block (`Command::ParkBlock`) and the
//! agent keeps running. On unless the setting says `false`; it is removed a release after it became
//! the default.

use std::collections::HashMap;

/// The setting. On by default since P4c; `false` turns it off.
pub const SETTING_ALWAYS_AVAILABLE: &str = "agents:always_available";

/// Whether closing a pane parks the agent's block instead of deleting it.
pub fn enabled(extra: &HashMap<String, serde_json::Value>) -> bool {
    match extra.get(SETTING_ALWAYS_AVAILABLE) {
        Some(serde_json::Value::Bool(on)) => *on,
        // A hand-edited file: "false", "off", "no" and 0 mean off too, so a safety switch is not left on by a typo of type.
        Some(serde_json::Value::String(s)) => !matches!(s.trim().to_ascii_lowercase().as_str(), "false" | "off" | "no" | "0"),
        Some(serde_json::Value::Number(n)) => n.as_f64() != Some(0.0),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_is_on_unless_the_setting_is_false() {
        let mut extra = HashMap::new();
        assert!(enabled(&extra), "the default");
        extra.insert(SETTING_ALWAYS_AVAILABLE.to_string(), serde_json::json!("maybe"));
        assert!(enabled(&extra), "a value that says nothing about off leaves the default");
        for off in [serde_json::json!("no"), serde_json::json!("False"), serde_json::json!(" off "), serde_json::json!("0"), serde_json::json!(0)] {
            extra.insert(SETTING_ALWAYS_AVAILABLE.to_string(), off.clone());
            assert!(!enabled(&extra), "{off} is off");
        }
        extra.insert(SETTING_ALWAYS_AVAILABLE.to_string(), serde_json::json!(false));
        assert!(!enabled(&extra));
        extra.insert(SETTING_ALWAYS_AVAILABLE.to_string(), serde_json::json!(true));
        assert!(enabled(&extra));
    }
}
