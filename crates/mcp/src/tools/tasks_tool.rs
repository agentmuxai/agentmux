// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tasks: one tool for the shared task queue (Muxqueue), with an `action`
//! (docs/specs/SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md §4.2, slice S3). It
//! replaces the six Work* tools, renamed so the queue can't be confused with
//! Presence's claims; their handlers (work.rs) run each action unchanged.

use super::family::{pick, Act};
use super::*;

pub(super) const ACTS: &[Act] = &[
    Act { action: "enqueue", tool: "WorkEnqueue", rename: &[] },
    Act { action: "claim", tool: "WorkClaim", rename: &[] },
    Act { action: "heartbeat", tool: "WorkHeartbeat", rename: &[] },
    Act { action: "complete", tool: "WorkComplete", rename: &[] },
    Act { action: "release", tool: "WorkRelease", rename: &[] },
    Act { action: "list", tool: "WorkList", rename: &[] },
];

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "Tasks" => {
            let (act, args) = pick("Tasks", ACTS, arguments)?;
            work::call(act.tool, &args, cx).await
        }
        _ => Err(not_in_family()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_is_in_the_schema() {
        let schema: Value = serde_json::from_str(crate::tool_schemas::TASKS_TOOL).unwrap();
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
