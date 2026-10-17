// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agents: one tool for the other agents (who is reachable, what they are
//! doing, opening them) and the caller's own history, with an `action`
//! (docs/specs/SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md §4.2, slice S4).
//! It replaces DiscoverAgents, FleetList (the same call), ListConversations,
//! GetAgentTranscript, OpenAgent and SearchHistory; their handlers
//! (fleet.rs, history.rs, panes.rs) run each action unchanged.
//! `spawn` (the agent-initiated action that makes a worker, not the CLI's
//! process start) is its own (SPEC_AGENT_WORKERS_2026_10_15.md §3.1): srv
//! launches the worker, then this process sends it the brief, signed as the
//! caller.

use agentmux_common::api_types::SpawnAgentRequest;

use super::family::{pick, public_params, relabeled, Act};
use super::*;

pub(super) const ACTS: &[Act] = &[
    Act { action: "list", tool: "DiscoverAgents", rename: &[] },
    Act { action: "conversations", tool: "ListConversations", rename: &[] },
    Act { action: "transcript", tool: "GetAgentTranscript", rename: &[] },
    Act { action: "open", tool: "OpenAgent", rename: &[("agent", "agent_id")] },
    Act { action: "spawn", tool: "SpawnAgent", rename: &[] },
    Act { action: "workers", tool: "AgentWorkers", rename: &[] },
    Act { action: "search_history", tool: "SearchHistory", rename: &[] },
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
                "DiscoverAgents" => fleet::call(act.tool, &args, cx).await,
                "OpenAgent" => panes::call(act.tool, &args, cx).await,
                "SpawnAgent" => spawn(&args, cx).await,
                "AgentWorkers" => workers(cx).await,
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

/// How long `spawn` waits for srv: the user's approval window (srv's
/// `ask_user`, 120 s and 15 s of grace), then an `agent.open` that may
/// install the worker's CLI (150 s), with room to spare. A wait that ran out
/// while srv carried on would launch a worker that never gets its brief.
const SPAWN_WAIT: std::time::Duration = std::time::Duration::from_secs(360);

/// `spawn`: srv builds a worker from this agent's definition and opens it next to it
/// (`/api/v1/agent/spawn`), then the brief goes to the worker as this
/// agent's own message, signed with its key, so it arrives host-verified
/// and the worker can answer with Message send.
async fn spawn(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    require_agent_env(cx.local_url, cx.auth_key, cx.block_id)?;
    let brief = str_arg(arguments, "brief").ok_or_else(|| anyhow::anyhow!("missing required parameter: brief"))?;
    let request = SpawnAgentRequest {
        auth: sign_ui_automation_auth()?,
        name: str_arg(arguments, "name"),
        model: str_arg(arguments, "model"),
        effort: str_arg(arguments, "effort"),
        brief: brief.clone(),
        account: str_arg(arguments, "account"),
        user_instruction: str_arg(arguments, "user_instruction").unwrap_or_default(),
    };
    let url = format!("{}/api/v1/agent/spawn", cx.local_url.trim_end_matches('/'));
    // Past the shared client's 10 s: srv may wait on the user's answer and
    // then on the worker's CLI install (SPAWN_WAIT).
    let resp = cx
        .client
        .post(&url)
        .timeout(SPAWN_WAIT)
        .header(AUTH_KEY_HEADER, cx.auth_key)
        .json(&request)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    let mut body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        anyhow::bail!("SpawnAgent: {}", body["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("srv answered {status}: {body}")));
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
/// It names the tool exactly: a worker that went looking for "SendMessage"
/// found Claude Code's own cross-session tool, whose message reaches the
/// leader unverified and isn't counted as a report.
fn brief_message(leader: &str, brief: &str) -> String {
    format!(
        "You are a worker of {leader}, started to do the task below. Report to {leader} with the agentmux \
         MCP server's Message tool (mcp__agentmux__Message in Claude Code; action \"send\", to \"{leader}\"), \
         never a built-in SendMessage, when you finish, get stuck, or need a decision.\n\n{brief}"
    )
}

/// `workers`: this agent's own workers, from srv (`/api/v1/agent/workers`).
async fn workers(cx: &ToolCtx<'_>) -> Result<String> {
    require_agent_env(cx.local_url, cx.auth_key, cx.block_id)?;
    let request = agentmux_common::api_types::AgentWorkersRequest { auth: sign_ui_automation_auth()? };
    let url = format!("{}/api/v1/agent/workers", cx.local_url.trim_end_matches('/'));
    let resp = cx
        .client
        .post(&url)
        .header(AUTH_KEY_HEADER, cx.auth_key)
        .json(&request)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        anyhow::bail!("AgentWorkers: {}", body["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("srv answered {status}: {body}")));
    }
    Ok(serde_json::to_string_pretty(&body)?)
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
    fn open_takes_the_agent_as_its_handler_names_it() {
        let (act, args) = pick("Agents", ACTS, &json!({ "action": "open", "agent": "Korp", "focus": false })).unwrap();
        assert_eq!(act.tool, "OpenAgent");
        assert_eq!(args, json!({ "agent_id": "Korp", "focus": false }));
        let (_, args) = pick("Agents", ACTS, &json!({ "action": "transcript", "agent": "Korp" })).unwrap();
        assert_eq!(args, json!({ "agent": "Korp" }));
    }

    #[test]
    fn the_brief_tells_the_worker_who_it_reports_to() {
        let m = brief_message("loap-3", "Implement SPEC_X.");
        assert_eq!(
            m,
            "You are a worker of loap-3, started to do the task below. Report to loap-3 with the agentmux \
             MCP server's Message tool (mcp__agentmux__Message in Claude Code; action \"send\", to \"loap-3\"), \
             never a built-in SendMessage, when you finish, get stuck, or need a decision.\n\nImplement SPEC_X."
        );
        assert!(!m.contains("  "), "{m}");
    }

    #[test]
    fn spawn_runs_its_own_handler_with_its_arguments() {
        let (act, args) = pick("Agents", ACTS, &json!({ "action": "spawn", "name": "Loap-4", "model": "sonnet", "brief": "b" })).unwrap();
        assert_eq!(act.tool, "SpawnAgent");
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
