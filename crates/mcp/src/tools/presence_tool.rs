// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Presence: one tool for who is working on what, with an `action`
//! (docs/specs/SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md §4.2, slice S3). It
//! replaces WhoIsWorkingOn, ClaimWork and ReleaseWork; their handlers
//! (overlap.rs) run each action unchanged.

use super::family::{pick, relabeled, Act};
use super::*;

pub(super) const ACTS: &[Act] = &[
    Act { action: "who", tool: "WhoIsWorkingOn", rename: &[] },
    Act { action: "claim", tool: "ClaimWork", rename: &[] },
    Act { action: "release", tool: "ReleaseWork", rename: &[] },
];

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "Presence" => {
            let (act, args) = pick("Presence", ACTS, arguments)?;
            relabeled("Presence", ACTS, overlap::call(act.tool, &args, cx).await)
        }
        _ => Err(not_in_family()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_is_in_the_schema() {
        let schema: Value = serde_json::from_str(crate::tool_schemas::PRESENCE_TOOL).unwrap();
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
