// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tasks: one tool for the shared task queue (Muxqueue), with an `action`
//! (docs/specs/SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md §4.2, slice S3). It
//! replaces the six Work* tools, renamed so the queue can't be confused with
//! Presence's claims; their handlers (work.rs) run each action unchanged.

use super::family::{pick, relabeled, Act};
use super::*;

pub(super) const ACTS: &[Act] = &[
    Act { action: "enqueue", tool: "WorkEnqueue", rename: &[] },
    Act { action: "claim", tool: "WorkClaim", rename: &[] },
    Act { action: "heartbeat", tool: "WorkHeartbeat", rename: &[] },
    Act { action: "complete", tool: "WorkComplete", rename: &[] },
    Act { action: "release", tool: "WorkRelease", rename: &[] },
    Act { action: "list", tool: "WorkList", rename: &[] },
    // Kanban (SPEC_WARDEN_KANBAN_2026_10_20.md §6.11).
    Act { action: "update", tool: "WorkUpdate", rename: &[] },
    Act { action: "ask", tool: "WorkAsk", rename: &[] },
    Act { action: "submit", tool: "WorkSubmit", rename: &[] },
    // SPEC_KANBAN_AGENTS_HOLD_AND_UNHOLD_2026_10_22.md §3.1.
    Act { action: "hold", tool: "WorkHold", rename: &[] },
    Act { action: "unhold", tool: "WorkUnhold", rename: &[] },
    // Any agent decides a task in Review, and makes the Kanban's other moves; each is recorded as the agent.
    Act { action: "accept", tool: "WorkAccept", rename: &[] },
    Act { action: "send_back", tool: "WorkSendBack", rename: &[] },
    Act { action: "edit", tool: "WorkEdit", rename: &[] },
    Act { action: "move", tool: "WorkMove", rename: &[] },
    Act { action: "start", tool: "WorkStart", rename: &[] },
    Act { action: "answer", tool: "WorkAnswer", rename: &[] },
    Act { action: "cancel", tool: "WorkCancel", rename: &[] },
    Act { action: "reopen", tool: "WorkReopen", rename: &[] },
    Act { action: "retry", tool: "WorkRetry", rename: &[] },
    Act { action: "take_back", tool: "WorkTakeBack", rename: &[] },
    Act { action: "reassign", tool: "WorkReassign", rename: &[] },
];

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "Tasks" => {
            let (act, args) = pick("Tasks", ACTS, arguments)?;
            relabeled("Tasks", ACTS, work::call(act.tool, &args, cx).await)
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
