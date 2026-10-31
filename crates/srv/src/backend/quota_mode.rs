// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! Quota mode: a fleet setting instead of a broadcast (`SPEC_CHEAPER_FLEET_MESSAGING_2026_10_30.md` §5).
//!
//! `fleet:quota_mode` is `off` (the default) or `tight`. While tight, `Tasks claim` with no id takes only a
//! task aimed at the claimer or with `priority >= fleet:quota_min_priority` (default 1); a claim by
//! id still works, because the agent was told to. A change of mode posts a bulletin under the key `mode`, so
//! no one broadcasts "slow down" again.

use std::collections::HashMap;

pub const SETTING_MODE: &str = "fleet:quota_mode";
pub const SETTING_MIN_PRIORITY: &str = "fleet:quota_min_priority";
/// The floor when the setting names none.
pub const DEFAULT_MIN_PRIORITY: i64 = 1;

/// The priority floor of an unnamed claim while the mode is tight, `None` while it is off.
pub fn claim_floor(extra: &HashMap<String, serde_json::Value>) -> Option<i64> {
    let tight = extra.get(SETTING_MODE).and_then(|v| v.as_str()).is_some_and(|m| m.trim().eq_ignore_ascii_case("tight"));
    tight.then(|| extra.get(SETTING_MIN_PRIORITY).and_then(|v| v.as_i64()).unwrap_or(DEFAULT_MIN_PRIORITY))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_floor_applies_only_while_the_mode_is_tight() {
        let mut extra = HashMap::new();
        assert_eq!(claim_floor(&extra), None, "off by default");
        extra.insert(SETTING_MODE.to_string(), serde_json::json!("off"));
        assert_eq!(claim_floor(&extra), None);
        extra.insert(SETTING_MODE.to_string(), serde_json::json!(" Tight "));
        assert_eq!(claim_floor(&extra), Some(DEFAULT_MIN_PRIORITY));
        extra.insert(SETTING_MIN_PRIORITY.to_string(), serde_json::json!(3));
        assert_eq!(claim_floor(&extra), Some(3));
        extra.insert(SETTING_MODE.to_string(), serde_json::json!(true));
        assert_eq!(claim_floor(&extra), None, "anything but the word tight is off");
    }
}
