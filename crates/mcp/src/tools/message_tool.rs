// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Message: one tool for messages to one agent or many and for each
//! repository's coordination channel, with an `action`
//! (docs/specs/SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md §4.2, slice S4). It
//! replaces SendMessage, FleetBroadcast, PostToChannel and ReadChannel; their
//! handlers (fleet.rs, overlap.rs) run each action unchanged, so a message is
//! signed and delivered exactly as before.

use super::family::{pick, relabeled, Act};
use super::*;

pub(super) const ACTS: &[Act] = &[
    Act { action: "send", tool: "SendMessage", rename: &[] },
    Act { action: "broadcast", tool: "FleetBroadcast", rename: &[] },
    Act { action: "post", tool: "PostToChannel", rename: &[] },
    Act { action: "read", tool: "ReadChannel", rename: &[] },
];

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "Message" => {
            let (act, args) = pick("Message", ACTS, arguments)?;
            let result = match act.tool {
                "SendMessage" | "FleetBroadcast" => fleet::call(act.tool, &args, cx).await,
                _ => overlap::call(act.tool, &args, cx).await,
            };
            relabeled("Message", ACTS, result)
        }
        _ => Err(not_in_family()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_is_in_the_schema() {
        let schema: Value = serde_json::from_str(crate::tool_schemas::MESSAGE_TOOL).unwrap();
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
