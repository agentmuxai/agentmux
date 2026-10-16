// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agents: one tool for the other agents (who is reachable, what they are
//! doing, opening them) and the caller's own history, with an `action`
//! (docs/specs/SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md §4.2, slice S4).
//! It replaces DiscoverAgents, FleetList (the same call), ListConversations,
//! GetAgentTranscript, OpenAgent, SearchHistory and SupervisorNudge; their
//! handlers (fleet.rs, history.rs, panes.rs) run each action unchanged.
//! `fork` is its own (SPEC_AGENT_WORKERS_2026_10_15.md §3.1): srv launches
//! the worker, then this process sends it the brief, signed as the caller.

use agentmux_common::api_types::ForkAgentRequest;

use super::family::{pick, public_params, relabeled, Act};
use super::*;

pub(super) const ACTS: &[Act] = &[
    Act { action: "list", tool: "DiscoverAgents", rename: &[] },
    Act { action: "conversations", tool: "ListConversations", rename: &[] },
    Act { action: "transcript", tool: "GetAgentTranscript", rename: &[] },
    Act { action: "open", tool: "OpenAgent", rename: &[("agent", "agent_id")] },
    Act { action: "fork", tool: "ForkAgent", rename: &[] },
    Act { action: "search_history", tool: "SearchHistory", rename: &[] },
    // The handler's own `action` is the decision: `pick` takes Agents'
    // `action` out before it renames.
    Act { action: "nudge", tool: "SupervisorNudge", rename: &[("agent", "target_agent"), ("decision", "action")] },
];

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "Agents" => {
            let (act, args) = pick("Agents", ACTS, arguments)?;
            // Your own history only (see AGENTS_TOOL): another agent's goes
            // through the transcript_request rules, never through a search.
            if act.action == "search_history" && args.get("agent").is_some() {
                anyhow::bail!(
                    "Agents search_history searches only your own history and takes no agent; \
                     another agent's conversation is shared through its own visibility setting"
                );
            }
            let result = match act.tool {
                "DiscoverAgents" | "SupervisorNudge" => fleet::call(act.tool, &args, cx).await,
                "OpenAgent" => panes::call(act.tool, &args, cx).await,
                "ForkAgent" => fork(&args, cx).await,
                _ => history::call(act.tool, &args, cx).await,
            };
            let result = result.map_err(|e| anyhow::anyhow!(public_params(&format!("{e:#}"), act.rename)));
            relabeled("Agents", ACTS, result)
        }
        _ => Err(not_in_family()),
    }
}

/// `key`'s value, if it is a non-empty string.
fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// `fork`: srv forks this agent and opens the worker next to it
/// (`/api/v1/agent/fork`), then the brief goes to the worker as this
/// agent's own message, signed with its key, so it arrives host-verified
/// and the worker can answer with Message send.
async fn fork(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    require_agent_env(cx.local_url, cx.auth_key, cx.block_id)?;
    let brief = str_arg(arguments, "brief").ok_or_else(|| anyhow::anyhow!("missing required parameter: brief"))?;
    let request = ForkAgentRequest {
        auth: sign_ui_automation_auth()?,
        name: str_arg(arguments, "name"),
        model: str_arg(arguments, "model"),
        effort: str_arg(arguments, "effort"),
        brief: brief.clone(),
        user_instruction: str_arg(arguments, "user_instruction").unwrap_or_default(),
    };
    let url = format!("{}/api/v1/agent/fork", cx.local_url.trim_end_matches('/'));
    // No timeout of ours: srv may be waiting on the user's answer.
    let resp = cx
        .client
        .post(&url)
        .header(AUTH_KEY_HEADER, cx.auth_key)
        .json(&request)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    let mut body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        anyhow::bail!("ForkAgent: {}", body["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("srv answered {status}: {body}")));
    }
    let to = body["message_to"].as_str().unwrap_or_default().to_string();
    let leader = std::env::var("AGENTMUX_AGENT_ID").unwrap_or_default();
    let sent = super::fleet::call("SendMessage", &json!({ "to": to, "message": brief_message(&leader, &brief) }), cx).await;
    body["brief"] = json!(match sent {
        Ok(outcome) => outcome,
        Err(e) => format!("NOT SENT: {e:#}. The worker is open but has no task: send it the brief with Message send"),
    });
    Ok(serde_json::to_string_pretty(&body)?)
}

/// The worker's first message: who it reports to, then the leader's brief.
fn brief_message(leader: &str, brief: &str) -> String {
    format!(
        "You are a worker of {leader}, started to do the task below. Report to {leader} with Message send          when you finish, get stuck, or need a decision.

{brief}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_is_in_the_schema() {
        let schema: Value = serde_json::from_str(crate::tool_schemas::AGENTS_TOOL).unwrap();
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

    #[test]
    fn nudge_and_open_take_the_agent_as_their_handlers_name_it() {
        let (act, args) = pick("Agents", ACTS, &json!({ "action": "nudge", "agent": "Korp", "decision": "decline", "reason": "done" })).unwrap();
        assert_eq!(act.tool, "SupervisorNudge");
        assert_eq!(args, json!({ "target_agent": "Korp", "action": "decline", "reason": "done" }));
        let (act, args) = pick("Agents", ACTS, &json!({ "action": "open", "agent": "Korp", "focus": false })).unwrap();
        assert_eq!(act.tool, "OpenAgent");
        assert_eq!(args, json!({ "agent_id": "Korp", "focus": false }));
        let (_, args) = pick("Agents", ACTS, &json!({ "action": "transcript", "agent": "Korp" })).unwrap();
        assert_eq!(args, json!({ "agent": "Korp" }));
    }

    #[test]
    fn the_brief_tells_the_worker_who_it_reports_to() {
        let m = brief_message("loap-3", "Implement SPEC_X.");
        assert!(m.starts_with("You are a worker of loap-3, started to do the task below. Report to loap-3 with Message send"), "{m}");
        assert!(m.ends_with("

Implement SPEC_X."), "{m}");
    }

    #[test]
    fn fork_runs_its_own_handler_with_its_arguments() {
        let (act, args) = pick("Agents", ACTS, &json!({ "action": "fork", "name": "Loap-4", "model": "sonnet", "brief": "b" })).unwrap();
        assert_eq!(act.tool, "ForkAgent");
        assert_eq!(args, json!({ "name": "Loap-4", "model": "sonnet", "brief": "b" }));
    }

    /// The search reaches only the caller's own history: an agent is refused
    /// before anything is sent.
    #[tokio::test]
    async fn search_history_takes_no_agent() {
        let client = reqwest::Client::new();
        let loops = LoopRegistry::default();
        let counter = std::sync::atomic::AtomicU64::new(0);
        let cx = ToolCtx { local_url: "", auth_key: "", block_id: "", client: &client, loops: &loops, loop_counter: &counter };
        let e = call("Agents", &json!({ "action": "search_history", "query": "x", "agent": "Korp" }), &cx)
            .await
            .unwrap_err()
            .to_string();
        assert!(e.starts_with("Agents search_history searches only your own history"), "{e}");
    }
}
