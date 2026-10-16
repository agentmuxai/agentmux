// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Identity: one tool for an agent's presets and linked provider accounts,
//! with an `action` (docs/specs/SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md
//! §4.2, slice S4). It replaces PresetList, PresetGet, IdentityAccounts and
//! IdentityValidate; their handlers (presets.rs, accounts.rs) run each action
//! unchanged.

use super::family::{pick, relabeled, Act};
use super::*;

pub(super) const ACTS: &[Act] = &[
    Act { action: "presets", tool: "PresetList", rename: &[] },
    Act { action: "preset", tool: "PresetGet", rename: &[] },
    Act { action: "accounts", tool: "IdentityAccounts", rename: &[] },
    Act { action: "validate", tool: "IdentityValidate", rename: &[] },
];

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "Identity" => {
            let (act, args) = pick("Identity", ACTS, arguments)?;
            let result = match act.tool {
                "PresetList" | "PresetGet" => presets::call(act.tool, &args, cx).await,
                _ => accounts::call(act.tool, &args, cx).await,
            };
            relabeled("Identity", ACTS, result)
        }
        _ => Err(not_in_family()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_is_in_the_schema() {
        let schema: Value = serde_json::from_str(crate::tool_schemas::IDENTITY_TOOL).unwrap();
        let mut listed: Vec<&str> = schema["inputSchema"]["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        let mut want = super::super::family::action_names(ACTS);
        listed.sort();
        want.sort();
        assert_eq!(listed, want);
    }
}
