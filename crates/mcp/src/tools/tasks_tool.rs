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
            if let Some(host) = remote_op::host_arg(arguments) {
                return remote(&host, arguments, cx).await;
            }
            let (act, args) = pick("Tasks", ACTS, arguments)?;
            relabeled("Tasks", ACTS, work::call(act.tool, &args, cx).await)
        }
        _ => Err(not_in_family()),
    }
}

/// A non-blank string argument.
fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// Fields of `enqueue` that travel to another machine's board. Not `target_group` (a group is local),
/// not `for: "me"` (this agent has no backlog there), and never a filer: srv names the agent itself.
const ENQUEUE_FIELDS: &[&str] = &["title", "payload", "kind", "target_agent", "priority", "max_attempts", "not_before", "repo", "hold", "review", "after", "first", "ui_change"];

/// `enqueue` and `list` with a `host`: this agent's request to that machine's board.
async fn remote(host: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let (op, params, reason) = match str_arg(arguments, "action").as_deref() {
        Some("enqueue") => {
            let params = enqueue_params(arguments)?;
            let title = params["title"].as_str().unwrap_or("").chars().take(60).collect::<String>();
            ("tasks.enqueue", params, format!("Tasks enqueue: {title}"))
        }
        Some("list") => ("tasks.list", list_params(arguments), "Tasks list".to_string()),
        other => anyhow::bail!(
            "Tasks: with host only enqueue and list run on another machine (you asked for {other:?}); leave host out to act on this machine's board"
        ),
    };
    let answer = remote_op::remote_op_call(cx, host, op, params, &reason).await?;
    match remote_op::explain_not_done(host, op, &answer) {
        Some(problem) => anyhow::bail!("{problem}"),
        None => Ok(remote_op::pretty(host, &answer)),
    }
}

/// The params of `tasks.enqueue` from the tool's arguments; what can't be asked of another machine is an error.
fn enqueue_params(arguments: &Value) -> Result<Value> {
    if arguments.get("for").and_then(Value::as_str).is_some() {
        anyhow::bail!("Tasks enqueue: for \"me\" files into your own backlog and can't go to another machine; name target_agent there, or leave host out");
    }
    if str_arg(arguments, "target_group").is_some() {
        anyhow::bail!("Tasks enqueue: target_group is a group on this machine and can't be used with host; name target_agent there");
    }
    if str_arg(arguments, "title").is_none() || str_arg(arguments, "payload").is_none() {
        anyhow::bail!("Tasks enqueue: title and payload are required");
    }
    let mut params = serde_json::Map::new();
    for key in ENQUEUE_FIELDS {
        if let Some(v) = arguments.get(*key).filter(|v| !v.is_null()) {
            params.insert((*key).to_string(), v.clone());
        }
    }
    Ok(Value::Object(params))
}

fn list_params(arguments: &Value) -> Value {
    let mut params = serde_json::Map::new();
    for key in ["state", "limit"] {
        if let Some(v) = arguments.get(key).filter(|v| !v.is_null()) {
            params.insert(key.to_string(), v.clone());
        }
    }
    Value::Object(params)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_enqueue_for_another_machine_carries_only_what_that_board_takes() {
        let args = json!({ "action": "enqueue", "host": "starpower", "title": "Verify", "payload": "run it", "target_agent": "clare", "first": "spec",
            "reason": "x", "created_by": "forged", "files": ["a.png"] });
        let p = enqueue_params(&args).unwrap();
        assert_eq!(p, json!({ "title": "Verify", "payload": "run it", "target_agent": "clare", "first": "spec" }));
    }

    #[test]
    fn what_only_makes_sense_here_is_refused_with_the_way_out() {
        let base = |extra: Value| {
            let mut a = json!({ "action": "enqueue", "title": "t", "payload": "p" });
            a.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            a
        };
        assert!(enqueue_params(&base(json!({ "for": "me" }))).unwrap_err().to_string().contains("own backlog"));
        assert!(enqueue_params(&base(json!({ "target_group": "g" }))).unwrap_err().to_string().contains("target_agent"));
        assert!(enqueue_params(&json!({ "action": "enqueue", "title": "t" })).unwrap_err().to_string().contains("required"));
    }

    #[test]
    fn a_list_asks_only_for_a_state_and_a_limit() {
        assert_eq!(list_params(&json!({ "action": "list", "host": "h", "state": "open", "limit": 5, "id": "9" })), json!({ "state": "open", "limit": 5 }));
        assert_eq!(list_params(&json!({ "action": "list" })), json!({}));
    }

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
